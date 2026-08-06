//! `roles` / `session_participants` / `participant_cursors` /
//! `participant_deliveries` — the session-focused model's persistence layer.
//!
//! Batch B3 of the redesign. Roles are user-owned templates; a participant is a
//! model plus an **invite-time snapshot** of one, so editing a role never
//! widens a live participant mid-turn. Cursors make delivery an auditable fact
//! instead of a side effect, and deliveries record what a policy withheld —
//! policies gate delivery, never persistence.
//!
//! **Schema note:** these tables ship in migration 0044, which is deliberately
//! NOT armed yet (see `docs/plans/2026-08-06-session-participants-runbook.md`).
//! `sqlx::migrate!` embeds `migrations/` at compile time, so the tests below
//! apply the reviewed draft to an in-memory DB instead — exercising the exact
//! schema the migration produces without arming anything.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    pub description_prompt: Option<String>,
    pub capabilities: String,
    pub participation_mode: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub id: i64,
    pub session_id: String,
    pub slug: String,
    pub display_name: String,
    pub role_id: Option<i64>,
    pub model_id: Option<String>,
    pub runtime: String,
    /// Invite-time snapshot, NOT a live read of the role.
    pub capabilities: String,
    pub participation_mode: String,
    pub turn_position: i64,
    pub done_vote: bool,
    pub enabled: bool,
}

const ROLE_COLUMNS: &str =
    "id, slug, display_name, description_prompt, capabilities, participation_mode, builtin";

const PARTICIPANT_COLUMNS: &str = "id, session_id, slug, display_name, role_id, model_id, \
     runtime, capabilities, participation_mode, turn_position, done_vote, enabled";

fn role_from_row(r: &sqlx::sqlite::SqliteRow) -> Role {
    use sqlx::Row;
    Role {
        id: r.get("id"),
        slug: r.get("slug"),
        display_name: r.get("display_name"),
        description_prompt: r.get("description_prompt"),
        capabilities: r.get("capabilities"),
        participation_mode: r.get("participation_mode"),
        builtin: r.get::<i64, _>("builtin") != 0,
    }
}

fn participant_from_row(r: &sqlx::sqlite::SqliteRow) -> Participant {
    use sqlx::Row;
    Participant {
        id: r.get("id"),
        session_id: r.get("session_id"),
        slug: r.get("slug"),
        display_name: r.get("display_name"),
        role_id: r.get("role_id"),
        model_id: r.get("model_id"),
        runtime: r.get("runtime"),
        capabilities: r.get("capabilities"),
        participation_mode: r.get("participation_mode"),
        turn_position: r.get("turn_position"),
        done_vote: r.get::<i64, _>("done_vote") != 0,
        enabled: r.get::<i64, _>("enabled") != 0,
    }
}

impl Storage {
    // ---- roles ----------------------------------------------------------

    pub async fn list_roles(&self) -> Result<Vec<Role>> {
        let rows = sqlx::query(&format!("SELECT {ROLE_COLUMNS} FROM roles ORDER BY slug"))
            .fetch_all(&self.pool)
            .await
            .context("listing roles")?;
        Ok(rows.iter().map(role_from_row).collect())
    }

    pub async fn role_by_slug(&self, slug: &str) -> Result<Option<Role>> {
        let row = sqlx::query(&format!("SELECT {ROLE_COLUMNS} FROM roles WHERE slug = ?"))
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("loading role {slug}"))?;
        Ok(row.as_ref().map(role_from_row))
    }

    // ---- participants ---------------------------------------------------

    /// Invite a participant, snapshotting the role's capabilities and
    /// participation mode. Passing them explicitly (rather than reading the
    /// role here) keeps the snapshot honest: the caller composes what this
    /// participant actually runs with, including any session-policy ceiling.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_participant(
        &self,
        session_id: &str,
        slug: &str,
        display_name: &str,
        role_id: Option<i64>,
        model_id: Option<&str>,
        capabilities: &str,
        participation_mode: &str,
        turn_position: i64,
    ) -> Result<i64> {
        let id = sqlx::query(
            "INSERT INTO session_participants \
             (session_id, slug, display_name, role_id, model_id, capabilities, \
              participation_mode, turn_position) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(slug)
        .bind(display_name)
        .bind(role_id)
        .bind(model_id)
        .bind(capabilities)
        .bind(participation_mode)
        .bind(turn_position)
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting participant {slug} into {session_id}"))?
        .last_insert_rowid();
        // Every participant reads the channel, so every participant has a cursor.
        sqlx::query("INSERT INTO participant_cursors (participant_id) VALUES (?)")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("seeding participant cursor")?;
        Ok(id)
    }

    /// Roster in turn order — the order the sequencer advances through and the
    /// order the UI renders.
    pub async fn participants_for_session(&self, session_id: &str) -> Result<Vec<Participant>> {
        let rows = sqlx::query(&format!(
            "SELECT {PARTICIPANT_COLUMNS} FROM session_participants \
             WHERE session_id = ? ORDER BY turn_position, id"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("listing participants for {session_id}"))?;
        Ok(rows.iter().map(participant_from_row).collect())
    }

    pub async fn participant_by_slug(
        &self,
        session_id: &str,
        slug: &str,
    ) -> Result<Option<Participant>> {
        let row = sqlx::query(&format!(
            "SELECT {PARTICIPANT_COLUMNS} FROM session_participants \
             WHERE session_id = ? AND slug = ?"
        ))
        .bind(session_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .context("loading participant")?;
        Ok(row.as_ref().map(participant_from_row))
    }

    // ---- turn cycle -----------------------------------------------------

    /// The participant after `current` in the ring, skipping anyone not in the
    /// rotation. Observers are SKIPPED rather than given a no-op turn: a wake
    /// that cannot produce output is pure waste. `None` when nobody is active.
    pub async fn next_active_participant(
        &self,
        session_id: &str,
        current_position: Option<i64>,
    ) -> Result<Option<Participant>> {
        let roster = self.participants_for_session(session_id).await?;
        let active: Vec<&Participant> = roster
            .iter()
            .filter(|p| p.enabled && p.participation_mode == "active")
            .collect();
        if active.is_empty() {
            return Ok(None);
        }
        let next = match current_position {
            // Wrap to the first active participant past `current`.
            Some(pos) => active
                .iter()
                .find(|p| p.turn_position > pos)
                .copied()
                .unwrap_or(active[0]),
            // A user message resets the cycle to the first active participant.
            None => active[0],
        };
        Ok(Some(next.clone()))
    }

    pub async fn set_done_vote(&self, participant_id: i64, done: bool) -> Result<()> {
        sqlx::query("UPDATE session_participants SET done_vote = ? WHERE id = ?")
            .bind(i64::from(done))
            .bind(participant_id)
            .execute(&self.pool)
            .await
            .context("setting done vote")?;
        Ok(())
    }

    /// Any substantive output resets EVERY vote — a stale "done" must not
    /// accumulate into a false arrival.
    pub async fn clear_done_votes(&self, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE session_participants SET done_vote = 0 WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .context("clearing done votes")?;
        Ok(())
    }

    /// Consensus halt: every ACTIVE participant has declared done.
    pub async fn all_active_voted_done(&self, session_id: &str) -> Result<bool> {
        let roster = self.participants_for_session(session_id).await?;
        let mut any = false;
        for p in roster.iter().filter(|p| p.enabled && p.participation_mode == "active") {
            any = true;
            if !p.done_vote {
                return Ok(false);
            }
        }
        Ok(any)
    }

    // ---- channel cursors + deliveries -----------------------------------

    pub async fn cursor_for(&self, participant_id: i64) -> Result<i64> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT last_read_message_id FROM participant_cursors WHERE participant_id = ?",
        )
        .bind(participant_id)
        .fetch_optional(&self.pool)
        .await
        .context("reading cursor")?;
        Ok(row.map(|r| r.0).unwrap_or(0))
    }

    /// Cursors only ever move FORWARD. A rewind would re-deliver messages an
    /// agent has already acted on — the staleness class this redesign removes.
    pub async fn advance_cursor(&self, participant_id: i64, message_id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE participant_cursors \
             SET last_read_message_id = MAX(last_read_message_id, ?), \
                 updated_at = datetime('now') \
             WHERE participant_id = ?",
        )
        .bind(message_id)
        .bind(participant_id)
        .execute(&self.pool)
        .await
        .context("advancing cursor")?;
        Ok(())
    }

    /// Record an delivery outcome. `withheld_reason = None` means delivered.
    /// A withheld message still has a row — policies gate delivery, never
    /// persistence.
    pub async fn record_delivery(
        &self,
        participant_id: i64,
        message_id: i64,
        withheld_reason: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO participant_deliveries \
             (participant_id, message_id, delivered_at, withheld_reason) \
             VALUES (?, ?, CASE WHEN ?3 IS NULL THEN datetime('now') ELSE NULL END, ?3)",
        )
        .bind(participant_id)
        .bind(message_id)
        .bind(withheld_reason)
        .execute(&self.pool)
        .await
        .context("recording delivery")?;
        Ok(())
    }

    /// What a participant was NOT shown, and why. The query that makes
    /// "what did participant X actually receive?" answerable.
    pub async fn withheld_for_participant(
        &self,
        participant_id: i64,
    ) -> Result<Vec<(i64, String)>> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT message_id, withheld_reason FROM participant_deliveries \
             WHERE participant_id = ? AND withheld_reason IS NOT NULL \
             ORDER BY message_id",
        )
        .bind(participant_id)
        .fetch_all(&self.pool)
        .await
        .context("listing withheld deliveries")?;
        Ok(rows)
    }
}

/// One row of the session channel, as a participant reads it.
///
/// `envelope` carries what used to be invisible string mutation — the phase
/// banner, sender role, blocking-findings notice, ack tags. Today those are
/// concatenated into the wire and never persisted, so the user cannot see what
/// an agent actually received; here they are a rendered field beside the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMessage {
    pub id: i64,
    pub session_id: String,
    pub participant_id: Option<i64>,
    pub origin: String,
    pub kind: String,
    pub content: String,
    pub envelope: Option<String>,
    pub created_at: String,
}

const CHANNEL_COLUMNS: &str =
    "id, session_id, participant_id, origin, kind, content, envelope, created_at";

fn channel_from_row(r: &sqlx::sqlite::SqliteRow) -> ChannelMessage {
    use sqlx::Row;
    ChannelMessage {
        id: r.get("id"),
        session_id: r.get("session_id"),
        participant_id: r.get("participant_id"),
        origin: r.get("origin"),
        kind: r.get("kind"),
        content: r.get("content"),
        envelope: r.get("envelope"),
        created_at: r.get("created_at"),
    }
}

impl Storage {
    /// Post to the session channel — the write half of "the channel is the
    /// transport". Every wire into a participant goes through here, including
    /// host-authored injections (`origin = "system"`), which today are written
    /// straight to stdin and never recorded at all.
    ///
    /// Writes the legacy `author` column too, so readers that have not migrated
    /// yet keep working (migration revision 3). `system` rows are stored as
    /// `author = 'user'` because that is exactly how today's system notices are
    /// already persisted — legacy readers must not encounter a new author value.
    pub async fn post_to_channel(
        &self,
        session_id: &str,
        origin: &str,
        participant_slug: Option<&str>,
        kind: &str,
        content: &str,
        envelope: Option<&str>,
    ) -> Result<i64> {
        let participant_id = match (origin, participant_slug) {
            ("participant", Some(slug)) => {
                self.participant_by_slug(session_id, slug).await?.map(|p| p.id)
            }
            _ => None,
        };
        let legacy_author = match origin {
            "participant" => participant_slug.unwrap_or("user"),
            // 'system' and 'user' both land as 'user' for legacy readers.
            _ => "user",
        };
        let id = sqlx::query(
            "INSERT INTO messages \
             (session_id, participant_id, origin, kind, content, envelope, author, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))",
        )
        .bind(session_id)
        .bind(participant_id)
        .bind(origin)
        .bind(kind)
        .bind(content)
        .bind(envelope)
        .bind(legacy_author)
        .execute(&self.pool)
        .await
        .with_context(|| format!("posting to channel for {session_id}"))?
        .last_insert_rowid();
        Ok(id)
    }

    /// Everything in the channel after `after_id`, oldest first — the read half.
    /// A participant waking on its turn reads exactly this, from its cursor, so
    /// context completeness is structural rather than a forwarding discipline.
    pub async fn channel_after(
        &self,
        session_id: &str,
        after_id: i64,
    ) -> Result<Vec<ChannelMessage>> {
        let rows = sqlx::query(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM messages \
             WHERE session_id = ? AND id > ? ORDER BY id ASC"
        ))
        .bind(session_id)
        .bind(after_id)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("reading channel for {session_id}"))?;
        Ok(rows.iter().map(channel_from_row).collect())
    }

    /// What this participant has not read yet. The query that makes "what did
    /// participant X actually receive?" answerable — a cursor range, not
    /// archaeology across a side table of drop records.
    pub async fn unread_for_participant(
        &self,
        participant_id: i64,
    ) -> Result<Vec<ChannelMessage>> {
        let Some(p) = self.participant_by_id(participant_id).await? else {
            return Ok(Vec::new());
        };
        let cursor = self.cursor_for(participant_id).await?;
        self.channel_after(&p.session_id, cursor).await
    }

    pub async fn participant_by_id(&self, id: i64) -> Result<Option<Participant>> {
        let row = sqlx::query(&format!(
            "SELECT {PARTICIPANT_COLUMNS} FROM session_participants WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("loading participant by id")?;
        Ok(row.as_ref().map(participant_from_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0044 is armed, so the stock in-memory backend has these tables — the
    /// transitional `storage_with_0044()` scaffold that applied the draft by
    /// hand is gone. Kept as a named alias so the tests still read as
    /// "storage that has 0044", which is the property they depend on.
    async fn storage_with_0044() -> Storage {
        Storage::memory().await.unwrap()
    }

    #[tokio::test]
    async fn the_draft_seeds_the_users_two_roles() {
        let s = storage_with_0044().await;
        let roles = s.list_roles().await.unwrap();
        assert_eq!(roles.len(), 2, "hands + eyes");
        let hands = s.role_by_slug("hands").await.unwrap().expect("hands");
        assert!(hands.builtin);
        assert_eq!(hands.participation_mode, "active");
        assert!(hands.capabilities.contains("edit_files"));
        let eyes = s.role_by_slug("eyes").await.unwrap().expect("eyes");
        assert!(eyes.capabilities.contains("file_finding"));
        assert!(
            !eyes.capabilities.contains("edit_files"),
            "EYES must not be seeded with write access"
        );
    }

    #[tokio::test]
    async fn inviting_a_participant_seeds_its_cursor() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let pid = s
            .insert_participant("s1", "brian", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();
        // Every participant reads the channel, so a cursor exists from birth —
        // a participant without one would be invisibly undeliverable.
        assert_eq!(s.cursor_for(pid).await.unwrap(), 0);
        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].turn_position, 0);
        assert!(!roster[0].done_vote);
    }

    #[tokio::test]
    async fn cursors_only_move_forward() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let pid = s
            .insert_participant("s1", "a", "A", None, None, "[]", "active", 0)
            .await
            .unwrap();
        s.advance_cursor(pid, 10).await.unwrap();
        assert_eq!(s.cursor_for(pid).await.unwrap(), 10);
        // A rewind would re-deliver messages already acted on — refuse it.
        s.advance_cursor(pid, 4).await.unwrap();
        assert_eq!(s.cursor_for(pid).await.unwrap(), 10, "cursor must not rewind");
    }

    #[tokio::test]
    async fn the_ring_skips_observers_and_wraps() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_participant("s1", "a", "A", None, None, "[]", "active", 0).await.unwrap();
        s.insert_participant("s1", "obs", "Obs", None, None, "[]", "observer", 1).await.unwrap();
        s.insert_participant("s1", "c", "C", None, None, "[]", "active", 2).await.unwrap();

        // A user message resets to the first active participant.
        let first = s.next_active_participant("s1", None).await.unwrap().unwrap();
        assert_eq!(first.slug, "a");
        // The observer is SKIPPED, not given a no-op turn.
        let second = s.next_active_participant("s1", Some(0)).await.unwrap().unwrap();
        assert_eq!(second.slug, "c", "observer must not take a turn");
        // The ring wraps.
        let third = s.next_active_participant("s1", Some(2)).await.unwrap().unwrap();
        assert_eq!(third.slug, "a");
    }

    #[tokio::test]
    async fn consensus_needs_every_active_participant_and_ignores_observers() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let a = s.insert_participant("s1", "a", "A", None, None, "[]", "active", 0).await.unwrap();
        let c = s.insert_participant("s1", "c", "C", None, None, "[]", "active", 1).await.unwrap();
        s.insert_participant("s1", "obs", "O", None, None, "[]", "observer", 2).await.unwrap();

        assert!(!s.all_active_voted_done("s1").await.unwrap());
        s.set_done_vote(a, true).await.unwrap();
        assert!(!s.all_active_voted_done("s1").await.unwrap(), "one vote is not consensus");
        s.set_done_vote(c, true).await.unwrap();
        assert!(
            s.all_active_voted_done("s1").await.unwrap(),
            "observers must not be required to vote — 1 active + 3 observers \
             would otherwise need 4 yields to halt"
        );
        // Any substantive output resets every vote.
        s.clear_done_votes("s1").await.unwrap();
        assert!(!s.all_active_voted_done("s1").await.unwrap());
    }

    #[tokio::test]
    async fn a_withheld_delivery_is_still_a_visible_row() {
        // The redesign's core inversion: today a suppressed forward vanishes.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let pid = s
            .insert_participant("s1", "a", "A", None, None, "[]", "active", 0)
            .await
            .unwrap();
        s.record_delivery(pid, 1, None).await.unwrap();
        s.record_delivery(pid, 2, Some("spin")).await.unwrap();
        let withheld = s.withheld_for_participant(pid).await.unwrap();
        assert_eq!(withheld, vec![(2, "spin".to_string())]);
    }

    #[tokio::test]
    async fn every_legacy_message_query_still_works_after_0044() {
        // **This is the app-boot check, in miniature.** The runbook's one
        // outstanding unknown was whether the app can boot against the new
        // schema; SQL guards cannot answer that, because the failure mode is a
        // query that no longer matches the table.
        //
        // It caught a real defect: `origin` was NOT NULL with no default, so
        // every legacy `insert_message` — which knows nothing about `origin` —
        // would have failed on insert and the app would not have started. The
        // column is transitional-nullable because of this test.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();

        // The write path, untouched by B3a.
        let id = s
            .insert_message("s1", Author::Brian, MessageKind::Text, "hello")
            .await
            .unwrap();
        assert!(id > 0, "legacy insert_message must still work post-0044");
        s.insert_message("s1", Author::User, MessageKind::Text, "hi back")
            .await
            .unwrap();

        // The read paths that key on `author`.
        assert_eq!(
            s.count_user_messages("s1").await.unwrap(),
            1,
            "count_user_messages keys on author = 'user'"
        );
        let msgs = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 2, "both rows readable through the legacy path");
        assert_eq!(msgs[0].author, Author::Brian.as_str());
    }

    #[tokio::test]
    async fn the_channel_records_what_a_participant_receives() {
        // The redesign's central inversion. Today a peer forward is built,
        // pushed to stdin, and never persisted — so "what did Rain actually
        // read?" is unanswerable. Here it is a cursor range.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let eyes = s.role_by_slug("eyes").await.unwrap().unwrap();
        let b = s
            .insert_participant("s1", "brian", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();
        let r = s
            .insert_participant("s1", "rain", "Rain", Some(eyes.id), None,
                                &eyes.capabilities, "active", 1)
            .await
            .unwrap();

        let m1 = s.post_to_channel("s1", "user", None, "text", "do the thing", None)
            .await.unwrap();
        let m2 = s.post_to_channel("s1", "participant", Some("brian"), "text", "done",
                                   Some(r#"{"phase":"Apply"}"#)).await.unwrap();

        // Rain has read nothing yet, so both are unread — including the message
        // she was not "forwarded". Context completeness is structural.
        let unread = s.unread_for_participant(r).await.unwrap();
        assert_eq!(unread.len(), 2);
        assert_eq!(unread[0].id, m1);
        assert_eq!(unread[0].origin, "user");
        assert_eq!(unread[1].id, m2);
        assert_eq!(unread[1].participant_id, Some(b), "attributed to its author");
        assert_eq!(
            unread[1].envelope.as_deref(),
            Some(r#"{"phase":"Apply"}"#),
            "the envelope is a visible field, not string mutation"
        );

        // After reading, the cursor advances and the backlog empties.
        s.advance_cursor(r, m2).await.unwrap();
        assert!(s.unread_for_participant(r).await.unwrap().is_empty());

        // Brian, who never read, still has both — cursors are per participant.
        assert_eq!(s.unread_for_participant(b).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn host_injections_become_visible_rows() {
        // The six invisible wires (apply-entry nudge, reconcile directive, idle
        // nudge, phase notices, peer prefix, spawn prompt) post as `system`.
        // Legacy readers must not meet a new author value, so a system row is
        // stored as author='user' — which is exactly how today's system notices
        // are already persisted.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let id = s
            .post_to_channel("s1", "system", None, "system_notice",
                             "[System: your previous turn was force-interrupted]", None)
            .await
            .unwrap();
        let rows = s.channel_after("s1", 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].origin, "system");
        assert!(rows[0].participant_id.is_none());
        // And the legacy read path still sees it, unchanged.
        let legacy = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].author, "user", "legacy readers meet no new author value");
    }

    #[tokio::test]
    async fn a_session_with_no_active_participants_has_no_next_turn() {
        // An all-observer session must not wedge the sequencer on an unwrap.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_participant("s1", "o", "O", None, None, "[]", "observer", 0).await.unwrap();
        assert!(s.next_active_participant("s1", None).await.unwrap().is_none());
        assert!(!s.all_active_voted_done("s1").await.unwrap(), "no actives = no consensus");
    }
}
