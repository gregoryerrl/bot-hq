//! A delegated write's authority is checked again after waiting for the DB
//! writer lock, at the claim. Pausing and resuming cannot revive an old ticket.
use super::*;
use crate::storage::ControlActor;
use anyhow::Context;

#[derive(Clone)]
pub(crate) struct ControlClaim {
    pub actor: ControlActor,
    /// The capability slug this claim stands on — `control_bot_hq` for every
    /// `app_control` action, `message_sessions` for a peer relay. Rechecked
    /// (snapshot AND live role) at every claim.
    pub grant: &'static str,
    proof: McpTokenProof,
    source: Arc<ActivityTracker>,
    source_pause: u64,
    target_session: String,
    target: Option<(Arc<ActivityTracker>, u64)>,
    expected_choice_sha: Option<String>,
    allow_target_paused: bool,
}

impl ControlClaim {
    /// The claim's own grant, rechecked on THIS connection once the writer
    /// lock is held: the role can be edited while a write waits for it.
    pub(crate) async fn reverify_on(&self, conn: &mut sqlx::SqliteConnection) -> Result<()> {
        let current = Storage::granted_actor_on(conn, &self.actor.session_id, &self.actor.slug, self.grant).await?;
        anyhow::ensure!(current == self.actor, "caller identity changed");
        Ok(())
    }

    pub(crate) fn check(&self, bridge: &SignalingBridge) -> Result<()> {
        anyhow::ensure!(
            bridge.token_proof_is_current(&self.actor.session_id, &self.actor.slug, &self.proof),
            "caller runtime was replaced or its credential was revoked"
        );
        let trackers = bridge
            .session_activity
            .try_lock()
            .map_err(|_| anyhow::anyhow!("session control state is changing; retry the action"))?;
        let source = trackers
            .get(&self.actor.session_id)
            .and_then(Weak::upgrade)
            .context("caller runtime stopped")?;
        anyhow::ensure!(
            Arc::ptr_eq(&source, &self.source)
                && source.pause_generation() == self.source_pause
                && !source.holds_wakes(),
            "caller was paused, cancelled or restarted before the action claimed"
        );
        let target = trackers.get(&self.target_session).and_then(Weak::upgrade);
        match (&self.target, target) {
            (Some((old, generation)), Some(now))
                if Arc::ptr_eq(old, &now)
                    && now.pause_generation() == *generation
                    && (self.allow_target_paused || !now.holds_wakes()) => {}
            (None, None) => {}
            _ => anyhow::bail!(
                "target session was paused, cancelled or restarted; read its state before retrying"
            ),
        }
        Ok(())
    }
}

impl SignalingBridge {
    #[cfg(test)]
    pub(crate) fn observe_control_tickets(&self, signal: Arc<tokio::sync::Notify>) {
        *self.control_ticket_observer.lock().unwrap() = Some(signal);
    }

    pub(crate) async fn control_claim(
        &self,
        actor: ControlActor,
        proof: Option<&McpTokenProof>,
        target_session: String,
        expected_choice_sha: String,
    ) -> Result<ControlClaim> {
        anyhow::ensure!(
            expected_choice_sha.len() == 64
                && expected_choice_sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "expected_choice_sha256 must be the hash returned by app_control choice"
        );
        self.control_ticket(
            actor,
            proof,
            target_session,
            Some(expected_choice_sha),
            false,
        )
        .await
    }

    pub(crate) async fn control_message_ticket(
        &self,
        actor: ControlActor,
        proof: Option<&McpTokenProof>,
        target: String,
    ) -> Result<ControlClaim> {
        self.control_ticket(actor, proof, target, None, true).await
    }

    /// A peer relay's ticket (Plan Batch 6): the `message_sessions` grant,
    /// a target that may be paused or dormant — the row waits for it.
    pub(crate) async fn relay_ticket(
        &self,
        actor: ControlActor,
        proof: Option<&McpTokenProof>,
        target: String,
    ) -> Result<ControlClaim> {
        self.ticket(actor, crate::storage::RELAY_GRANT, proof, target, None, true).await
    }

    /// Admission point for a synchronous application operation. Async workers
    /// must additionally validate when they take ownership (as startup does).
    pub(crate) async fn authorize_control_effect(&self, claim: &ControlClaim) -> Result<()> {
        let storage = self.storage_handle().await.context("storage is unavailable")?;
        let mut tx = storage.pool().begin_with("BEGIN IMMEDIATE").await?;
        let current = Storage::controller_actor_on(&mut tx, &claim.actor.session_id, &claim.actor.slug).await?;
        anyhow::ensure!(current == claim.actor, "controller identity changed");
        claim.check(self)?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn create_controlled_session_row(
        &self,
        claim: &ControlClaim,
        id: &str,
        title: &str,
        working: Option<&str>,
    ) -> Result<()> {
        anyhow::ensure!(id == claim.target_session, "creation target differs from its claim");
        let storage = self.storage_handle().await.context("storage is unavailable")?;
        let mut tx = storage.pool().begin_with("BEGIN IMMEDIATE").await?;
        let actor = Storage::controller_actor_on(&mut tx, &claim.actor.session_id, &claim.actor.slug).await?;
        anyhow::ensure!(actor == claim.actor, "controller identity changed");
        claim.check(self)?;
        Storage::insert_session_on(&mut tx, id, title, working).await?;
        let notice = format!("Session created by delegated controller {} of {}.", actor.slug, actor.session_id);
        sqlx::query("INSERT INTO messages (session_id, origin, kind, content, author, created_at) VALUES (?, 'system', 'system_notice', ?, 'user', ?)")
            .bind(id).bind(notice).bind(crate::storage::now_utc()).execute(&mut *tx).await?;
        claim.check(self)?;
        tx.commit().await?;
        Ok(())
    }

    async fn control_ticket(
        &self,
        actor: ControlActor,
        proof: Option<&McpTokenProof>,
        target_session: String,
        expected_choice_sha: Option<String>,
        allow_target_paused: bool,
    ) -> Result<ControlClaim> {
        self.ticket(actor, crate::storage::CONTROL_GRANT, proof, target_session, expected_choice_sha, allow_target_paused).await
    }

    async fn ticket(
        &self,
        actor: ControlActor,
        grant: &'static str,
        proof: Option<&McpTokenProof>,
        target_session: String,
        expected_choice_sha: Option<String>,
        allow_target_paused: bool,
    ) -> Result<ControlClaim> {
        let proof = proof
            .context("an authenticated HTTP caller is required for delegated writes")?
            .clone();
        let trackers = self.session_activity.lock().await;
        let source = trackers
            .get(&actor.session_id)
            .and_then(Weak::upgrade)
            .context("caller runtime is not registered")?;
        let target = trackers
            .get(&target_session)
            .and_then(Weak::upgrade)
            .map(|a| {
                let epoch = a.pause_generation();
                (a, epoch)
            });
        let claim = ControlClaim {
            actor,
            grant,
            proof,
            source_pause: source.pause_generation(),
            source,
            target_session,
            target,
            expected_choice_sha,
            allow_target_paused,
        };
        drop(trackers);
        claim.check(self)?;
        #[cfg(test)]
        if let Some(signal) = &*self.control_ticket_observer.lock().unwrap() {
            signal.notify_one();
        }
        Ok(claim)
    }

    pub(super) async fn begin_session_control(
        &self,
        claim: &ControlClaim,
        id: &str,
    ) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>> {
        anyhow::ensure!(id == claim.target_session, "lifecycle target differs from its claim");
        let storage = self.storage_handle().await.context("storage is unavailable")?;
        let mut tx = storage.pool().begin_with("BEGIN IMMEDIATE").await?;
        let actor = Storage::controller_actor_on(&mut tx, &claim.actor.session_id, &claim.actor.slug).await?;
        anyhow::ensure!(actor == claim.actor, "controller identity changed");
        claim.check(self)?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)")
            .bind(id).fetch_one(&mut *tx).await?;
        anyhow::ensure!(exists, "target session does not exist");
        Ok(tx)
    }

    pub(crate) async fn close_controlled_session_row(&self, claim: &ControlClaim, id: &str, archive: bool, audit: i64) -> Result<()> {
        let mut tx = self.begin_session_control(claim, id).await?;
        Storage::close_session_on(&mut tx, id, archive).await?;
        if archive {
            sqlx::query("UPDATE sessions SET archived = 1 WHERE id = ? AND closed_at IS NOT NULL")
                .bind(id).execute(&mut *tx).await?;
        }
        let notice = format!("Session closed{} by delegated controller {} of {}.", if archive { " and archived" } else { "" }, claim.actor.slug, claim.actor.session_id);
        sqlx::query("INSERT INTO messages (session_id, origin, kind, content, author, created_at) VALUES (?, 'system', 'system_notice', ?, 'user', ?)")
            .bind(id).bind(notice).bind(crate::storage::now_utc()).execute(&mut *tx).await?;
        // Persist the close outcome BEFORE teardown can kill a controller
        // closing its own session. This describes the row, not cleanup success.
        sqlx::query("UPDATE application_control_audit SET outcome = 'ok', detail = 'Session row closed; runtime cleanup follows.', finished_at = ? WHERE id = ? AND outcome = 'started'")
            .bind(crate::storage::now_utc()).bind(audit).execute(&mut *tx).await?;
        claim.check(self)?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn reopen_controlled_session_row(&self, claim: &ControlClaim, id: &str) -> Result<bool> {
        let mut tx = self.begin_session_control(claim, id).await?;
        let moved = Storage::reopen_session_on(&mut tx, id).await?;
        claim.check(self)?;
        tx.commit().await?;
        Ok(moved)
    }

    pub(crate) async fn record_controlled_pause(&self, claim: &ControlClaim, id: &str, audit: i64) -> Result<()> {
        let mut tx = self.begin_session_control(claim, id).await?;
        let notice = format!("Pause requested by delegated controller {} of {}.", claim.actor.slug, claim.actor.session_id);
        sqlx::query("INSERT INTO messages (session_id, origin, kind, content, author, created_at) VALUES (?, 'system', 'system_notice', ?, 'user', ?)")
            .bind(id).bind(notice).bind(crate::storage::now_utc()).execute(&mut *tx).await?;
        sqlx::query("UPDATE application_control_audit SET outcome = 'ok', detail = 'Pause requested; runtime interruption follows.', finished_at = ? WHERE id = ? AND outcome = 'started'")
            .bind(crate::storage::now_utc()).bind(audit).execute(&mut *tx).await?;
        claim.check(self)?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn request_controlled_participant_change(&self, claim: &ControlClaim, session: &str, id: &str, revision: i64, request: crate::storage::ParticipantChangeRequest) -> Result<crate::storage::ParticipantChange> {
        let mut tx = self.begin_session_control(claim, session).await?;
        let origin = crate::storage::ParticipantChangeOrigin::Agent { session_id: claim.actor.session_id.clone(), participant_id: claim.actor.participant_id };
        let row = Storage::request_participant_change_on(&mut tx, session, id, revision, request, origin).await?;
        claim.check(self)?;
        tx.commit().await?;
        Ok(row)
    }

    pub(crate) async fn claim_controlled_participant_change(&self, claim: &ControlClaim, session: &str, id: &str) -> Result<Option<crate::storage::ParticipantChange>> {
        let mut tx = self.begin_session_control(claim, session).await?;
        let row = Storage::claim_participant_change_on(&mut tx, session, id).await?;
        if let Some(row) = &row {
            anyhow::ensure!(row.origin()? == crate::storage::ParticipantChangeOrigin::Agent { session_id: claim.actor.session_id.clone(), participant_id: claim.actor.participant_id }, "participant change belongs to another caller");
        }
        claim.check(self)?;
        tx.commit().await?;
        Ok(row)
    }

    pub(crate) async fn cancel_controlled_participant_change(&self, claim: &ControlClaim, session: &str, id: &str) -> Result<bool> {
        let mut tx = self.begin_session_control(claim, session).await?;
        let cancelled = Storage::cancel_participant_change_on(&mut tx, session, id).await?;
        claim.check(self)?;
        tx.commit().await?;
        Ok(cancelled)
    }

    pub(crate) async fn claim_delegated_answer(
        &self,
        storage: &Storage,
        choice_id: &str,
        picked: &str,
        claim: &ControlClaim,
    ) -> Result<u64> {
        use sha2::{Digest, Sha256};
        // The role can be edited while this waits. Validate on THIS connection
        // once we own the writer lock, rather than reusing the earlier read.
        let mut tx = storage.pool().begin_with("BEGIN IMMEDIATE").await?;
        let current =
            Storage::controller_actor_on(&mut tx, &claim.actor.session_id, &claim.actor.slug)
                .await?;
        anyhow::ensure!(current == claim.actor, "controller identity changed");
        let row: crate::storage::SessionTrayEntry =
            sqlx::query_as("SELECT * FROM session_tray WHERE choice_id = ?")
                .bind(choice_id)
                .fetch_optional(&mut *tx)
                .await?
                .context("choice does not exist")?;
        anyhow::ensure!(
            row.session_id == claim.target_session && row.status == "pending",
            "choice is no longer pending in the expected session"
        );
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&row)?));
        anyhow::ensure!(
            claim.expected_choice_sha.as_deref() == Some(hash.as_str()),
            "choice changed since it was read; read the choice again before answering"
        );
        claim.check(self)?;
        let count = sqlx::query("UPDATE session_tray SET status = 'answered', picked_option = ?, answered_at = ?, decision_actor_json = ? WHERE choice_id = ? AND status = 'pending'")
            .bind(picked).bind(crate::storage::now_utc()).bind(serde_json::to_string(&claim.actor)?)
            .bind(choice_id).execute(&mut *tx).await?.rows_affected();
        // A Pause during the database write rolls this tentative answer back.
        claim.check(self)?;
        tx.commit().await?;
        Ok(count)
    }

    pub(crate) async fn persist_controlled_message(
        &self,
        storage: &Storage,
        claim: &ControlClaim,
        request_id: &str,
        text: &str,
    ) -> Result<i64> {
        use sha2::{Digest, Sha256};
        anyhow::ensure!(
            !request_id.is_empty()
                && request_id.len() <= 128
                && request_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
            "request_id must be a stable identifier of at most 128 characters"
        );
        anyhow::ensure!(
            !text.trim().is_empty() && text.len() <= 64_000,
            "message must be non-empty and at most 64 KB"
        );
        let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
        let mut tx = storage.pool().begin_with("BEGIN IMMEDIATE").await?;
        let actor =
            Storage::controller_actor_on(&mut tx, &claim.actor.session_id, &claim.actor.slug)
                .await?;
        anyhow::ensure!(actor == claim.actor, "controller identity changed");
        let open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ? AND closed_at IS NULL AND archived = 0)")
            .bind(&claim.target_session).fetch_one(&mut *tx).await?;
        anyhow::ensure!(
            open,
            "target session is missing or closed; reopen it before sending"
        );
        claim.check(self)?;
        let prior: Option<(String, String, i64)> = sqlx::query_as("SELECT target_session, content_sha256, message_id FROM delegated_messages WHERE source_session = ? AND source_participant = ? AND request_id = ?")
            .bind(&actor.session_id).bind(actor.participant_id).bind(request_id).fetch_optional(&mut *tx).await?;
        if let Some((target, hash, id)) = prior {
            anyhow::ensure!(
                target == claim.target_session && hash == digest,
                "request_id already belongs to a different message"
            );
            let retained: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ? AND session_id = ?)",
            )
            .bind(id)
            .bind(&target)
            .fetch_one(&mut *tx)
            .await?;
            anyhow::ensure!(
                retained,
                "the original message is no longer retained; use a new request_id to send again"
            );
            claim.check(self)?;
            return Ok(id);
        }
        let body = crate::policy::secret_scan::redact_string(format!(
            "Delegated message from {} of {}:\n{}",
            actor.slug, actor.session_id, text
        ));
        let id = sqlx::query("INSERT INTO messages (session_id, participant_id, origin, kind, content, envelope, author, created_at) VALUES (?, NULL, 'system', 'system_notice', ?, NULL, 'user', ?)")
            .bind(&claim.target_session).bind(body).bind(crate::storage::now_utc()).execute(&mut *tx).await?.last_insert_rowid();
        sqlx::query("INSERT INTO delegated_messages (source_session, source_participant, request_id, target_session, content_sha256, message_id) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&actor.session_id).bind(actor.participant_id).bind(request_id).bind(&claim.target_session).bind(digest).bind(id).execute(&mut *tx).await?;
        claim.check(self)?;
        tx.commit().await?;
        Ok(id)
    }
}
