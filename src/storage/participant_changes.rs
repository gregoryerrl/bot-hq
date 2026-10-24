//! Durable, validated intent for changing an existing session's participants.
//! This layer never starts/stops a process or mutates a roster. The runtime
//! owns the safe boundary, recovery snapshot, authority recheck and execution.

use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticipantChangeRequest {
    Add { role_id: i64, model_id: String, label: Option<String> },
    Remove { participant_id: i64 },
    SwitchModel { participant_id: i64, model_id: String },
}

/// Supplied by the authenticated host caller, never accepted as UI payload.
/// Provenance is not permission: an agent-origin request still needs live
/// application-control authorization in the runtime before it can execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticipantChangeOrigin {
    User,
    Agent { session_id: String, participant_id: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ParticipantChange {
    pub id: String,
    pub session_id: String,
    pub expected_revision: i64,
    pub selection_fingerprint: String,
    pub request_json: String,
    pub origin_json: String,
    pub status: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub recovery_json: Option<String>,
}

impl ParticipantChange {
    pub fn request(&self) -> Result<ParticipantChangeRequest> {
        serde_json::from_str(&self.request_json).context("reading the participant change request")
    }

    pub fn origin(&self) -> Result<ParticipantChangeOrigin> {
        serde_json::from_str(&self.origin_json).context("reading the participant change origin")
    }
}

const COLUMNS: &str = "id, session_id, expected_revision, selection_fingerprint, request_json, origin_json, status, \
    created_at, started_at, finished_at, error, recovery_json";

async fn read_change(conn: &mut SqliteConnection, id: &str) -> Result<Option<ParticipantChange>> {
    Ok(sqlx::query_as(&format!("SELECT {COLUMNS} FROM session_participant_changes WHERE id = ?"))
        .bind(id).fetch_optional(conn).await?)
}

/// Freeze the meaning of a selected saved model/role without copying its
/// credentials or full prompt into the request ledger. Timestamps alone can
/// collide or change on no-op saves; hash the actual settings instead.
async fn selection_fingerprint(conn: &mut SqliteConnection, request: &ParticipantChangeRequest) -> Result<String> {
    let mut selections = Vec::new();
    let (model, role) = match request {
        ParticipantChangeRequest::Add { model_id, role_id, .. } => (Some(model_id), Some(*role_id)),
        ParticipantChangeRequest::SwitchModel { model_id, participant_id } => {
            let role: Option<i64> = sqlx::query_scalar("SELECT role_id FROM session_participants WHERE id = ?")
                .bind(participant_id).fetch_one(&mut *conn).await?;
            (Some(model_id), role)
        }
        ParticipantChangeRequest::Remove { .. } => (None, None),
    };
    if let Some(id) = model {
        let mut model: Model = sqlx::query_as("SELECT * FROM models WHERE id = ?")
            .bind(id).fetch_one(&mut *conn).await?;
        model.created_at.clear();
        model.updated_at.clear();
        selections.push(serde_json::to_value(model)?);
    }
    if let Some(id) = role {
        let role: (String, String, Option<String>, String, String, Option<String>, bool) = sqlx::query_as(
            "SELECT slug, display_name, description_prompt, capabilities, participation_mode, default_model_id, archived FROM roles WHERE id = ?",
        ).bind(id).fetch_one(&mut *conn).await?;
        selections.push(serde_json::to_value(role)?);
    }
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&selections)?)))
}

async fn validate_request(
    conn: &mut SqliteConnection,
    session_id: &str,
    expected_revision: i64,
    request: &ParticipantChangeRequest,
) -> Result<()> {
    let (revision, closed, archived): (i64, Option<String>, bool) = sqlx::query_as(
        "SELECT roster_revision, closed_at, archived FROM sessions WHERE id = ?",
    ).bind(session_id).fetch_optional(&mut *conn).await?.context("session does not exist")?;
    if closed.is_some() || archived { anyhow::bail!("reopen the session before changing its participants"); }
    if revision != expected_revision {
        anyhow::bail!("the session roster changed; refresh the participants and try again");
    }
    let model = match request {
        ParticipantChangeRequest::Add { model_id, .. } | ParticipantChangeRequest::SwitchModel { model_id, .. } => Some(model_id),
        _ => None,
    };
    if let Some(model) = model {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models WHERE id = ?)")
            .bind(model).fetch_one(&mut *conn).await?;
        // The registry has no enabled flag; availability is an existing row.
        if !exists { anyhow::bail!("the selected saved model no longer exists"); }
    }
    let (enabled, active): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(participation_mode = 'active'), 0) \
         FROM session_participants WHERE session_id = ? AND enabled <> 0",
    ).bind(session_id).fetch_one(&mut *conn).await?;
    match request {
        ParticipantChangeRequest::Add { role_id, label, .. } => {
            let (archived, mode): (bool, String) = sqlx::query_as(
                "SELECT archived, participation_mode FROM roles WHERE id = ?",
            ).bind(role_id).fetch_optional(&mut *conn).await?.context("the selected role no longer exists")?;
            if archived { anyhow::bail!("the selected role is archived"); }
            if !PARTICIPATION_MODES.contains(&mode.as_str()) { anyhow::bail!("the selected role has an invalid participation mode"); }
            if enabled >= MAX_SESSION_PARTICIPANTS as i64 { anyhow::bail!("a session can have at most {MAX_SESSION_PARTICIPANTS} enabled participants"); }
            if active == 0 && mode != MODE_ACTIVE { anyhow::bail!("the session needs at least one active participant"); }
            if let Some(label) = label {
                if label.is_empty() || label.len() > 256 || label.contains(char::is_control) {
                    anyhow::bail!("a participant label must be 1–256 bytes with no control characters");
                }
                let taken: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id = ? \
                     AND (lower(trim(label)) = ? OR lower(slug) = ?))",
                ).bind(session_id).bind(label.to_ascii_lowercase()).bind(label.to_ascii_lowercase())
                    .fetch_one(&mut *conn).await?;
                if taken { anyhow::bail!("that participant label or handle is already used in this session"); }
            }
        }
        ParticipantChangeRequest::Remove { participant_id } | ParticipantChangeRequest::SwitchModel { participant_id, .. } => {
            let (is_enabled, mode, current_model): (bool, String, Option<String>) = sqlx::query_as(
                "SELECT enabled, participation_mode, model_id FROM session_participants WHERE id = ? AND session_id = ?",
            ).bind(participant_id).bind(session_id).fetch_optional(&mut *conn).await?
                .context("participant does not belong to this session")?;
            if !is_enabled { anyhow::bail!("the participant has already been removed"); }
            match request {
                ParticipantChangeRequest::Remove { .. } if enabled <= 1 || active - i64::from(mode == MODE_ACTIVE) < 1 => {
                    anyhow::bail!("cannot remove the session's last active participant");
                }
                ParticipantChangeRequest::SwitchModel { model_id, .. } if current_model.as_ref() == Some(model_id) => {
                    anyhow::bail!("the participant already uses that saved model");
                }
                _ => {}
            }
        }
    }
    Ok(())
}

impl Storage {
    pub async fn roster_revision(&self, session_id: &str) -> Result<i64> {
        sqlx::query_scalar("SELECT roster_revision FROM sessions WHERE id = ?")
            .bind(session_id).fetch_optional(&self.pool).await?.context("session does not exist")
    }

    /// Persist intent only. BEGIN IMMEDIATE makes validation and the one-open
    /// request invariant atomic across independent callers/connections.
    pub async fn request_participant_change(
        &self, session_id: &str, id: &str, expected_revision: i64,
        mut request: ParticipantChangeRequest, origin: ParticipantChangeOrigin,
    ) -> Result<ParticipantChange> {
        if id.is_empty() || id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            anyhow::bail!("invalid participant change request ID");
        }
        if let ParticipantChangeRequest::Add { label, .. } = &mut request {
            *label = label.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        }
        let request_json = serde_json::to_string(&request)?;
        let origin_json = serde_json::to_string(&origin)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(existing) = read_change(&mut tx, id).await? {
            if existing.session_id != session_id || existing.expected_revision != expected_revision
                || existing.request_json != request_json || existing.origin_json != origin_json {
                anyhow::bail!("request ID already belongs to a different participant change");
            }
            tx.commit().await?;
            return Ok(existing);
        }
        validate_request(&mut tx, session_id, expected_revision, &request).await?;
        let fingerprint = selection_fingerprint(&mut tx, &request).await?;
        let busy: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM session_participant_changes WHERE session_id = ? AND status IN ('pending', 'applying'))",
        ).bind(session_id).fetch_one(&mut *tx).await?;
        if busy { anyhow::bail!("this session already has a pending participant change; finish or cancel it first"); }
        sqlx::query("INSERT INTO session_participant_changes (id, session_id, expected_revision, selection_fingerprint, request_json, origin_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(id).bind(session_id).bind(expected_revision).bind(fingerprint).bind(request_json).bind(origin_json).bind(now_utc())
            .execute(&mut *tx).await?;
        let row = read_change(&mut tx, id).await?.context("participant change disappeared")?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn participant_change(&self, session_id: &str, id: &str) -> Result<Option<ParticipantChange>> {
        let mut conn = self.pool.acquire().await?;
        Ok(read_change(&mut conn, id).await?.filter(|row| row.session_id == session_id))
    }

    pub async fn open_participant_change(&self, session_id: &str) -> Result<Option<ParticipantChange>> {
        Ok(sqlx::query_as(&format!("SELECT {COLUMNS} FROM session_participant_changes WHERE session_id = ? AND status IN ('pending', 'applying')"))
            .bind(session_id).fetch_optional(&self.pool).await?)
    }

    /// The runtime calls this only AFTER holding the safe boundary. Exactly
    /// one claimant wins; a request invalidated while waiting becomes failed.
    pub async fn claim_participant_change(&self, session_id: &str, id: &str) -> Result<Option<ParticipantChange>> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some(row) = read_change(&mut tx, id).await?.filter(|row| row.session_id == session_id && row.status == "pending") else {
            return Ok(None);
        };
        let request = row.request()?;
        let validation = async {
            validate_request(&mut tx, session_id, row.expected_revision, &request).await?;
            if selection_fingerprint(&mut tx, &request).await? != row.selection_fingerprint {
                anyhow::bail!("the selected role or model settings changed; refresh and try again");
            }
            Ok::<(), anyhow::Error>(())
        }.await;
        if let Err(error) = validation {
            sqlx::query("UPDATE session_participant_changes SET status = 'failed', error = ?, finished_at = ? WHERE id = ?")
                .bind(error.to_string()).bind(now_utc()).bind(id).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(None);
        }
        sqlx::query("UPDATE session_participant_changes SET status = 'applying', started_at = ? WHERE id = ?")
            .bind(now_utc()).bind(id).execute(&mut *tx).await?;
        let row = read_change(&mut tx, id).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn cancel_participant_change(&self, session_id: &str, id: &str) -> Result<bool> {
        Ok(sqlx::query("UPDATE session_participant_changes SET status = 'cancelled', finished_at = ? WHERE id = ? AND session_id = ? AND status = 'pending'")
            .bind(now_utc()).bind(id).bind(session_id).execute(&self.pool).await?.rows_affected() == 1)
    }

    /// A host-owned recovery checkpoint must precede any roster mutation.
    /// Write once: a retry must not replace the original rollback information.
    pub async fn checkpoint_participant_change(&self, session_id: &str, id: &str, recovery: &serde_json::Value) -> Result<bool> {
        let body = serde_json::to_string(recovery)?;
        if !recovery.is_object() || body.len() > 256 * 1024 { anyhow::bail!("invalid participant change recovery checkpoint"); }
        Ok(sqlx::query("UPDATE session_participant_changes SET recovery_json = ? WHERE id = ? AND session_id = ? AND status = 'applying' AND recovery_json IS NULL")
            .bind(body).bind(id).bind(session_id).execute(&self.pool).await?.rows_affected() == 1)
    }

    /// None = completed; Some = failed (caller supplies a redacted explanation).
    /// Successful completion requires a recovery checkpoint. The runtime must
    /// restore/settle it before reporting failure; this is not a rollback API.
    pub async fn finish_participant_change(&self, session_id: &str, id: &str, error: Option<&str>) -> Result<bool> {
        Ok(sqlx::query("UPDATE session_participant_changes SET status = ?, error = ?, finished_at = ? \
            WHERE id = ? AND session_id = ? AND status = 'applying' AND (? OR recovery_json IS NOT NULL)")
            .bind(if error.is_some() { "failed" } else { "applied" }).bind(error).bind(now_utc())
            .bind(id).bind(session_id).bind(error.is_some()).execute(&self.pool).await?.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(s: &Storage) -> (i64, i64, i64) {
        for id in ["m1", "m2"] {
            s.upsert_model(&Model {
                id: id.into(), display_name: id.into(), provider: "anthropic".into(),
                model_name: "claude-opus-5".into(), base_url: None, auth_token: None,
                created_at: String::new(), updated_at: String::new(), context_window: None,
                cli_settings: None, claude_config_dir: None, runtime: "claude_code".into(),
            }).await.unwrap();
        }
        s.create_session("s1", "test", None).await.unwrap();
        s.ensure_session_roster("s1", 2).await.unwrap();
        let p = s.participants_for_session("s1").await.unwrap();
        s.set_participant_model(p[0].id, Some("m1")).await.unwrap();
        (p[0].id, p[1].id, p[0].role_id.unwrap())
    }

    fn switch(id: i64) -> ParticipantChangeRequest {
        ParticipantChangeRequest::SwitchModel { participant_id: id, model_id: "m2".into() }
    }

    #[tokio::test]
    async fn duplicate_requests_are_one_durable_intent_and_do_not_change_the_roster() {
        let s = Storage::memory().await.unwrap();
        let (id, _, _) = seed(&s).await;
        let rev = s.roster_revision("s1").await.unwrap();
        let before = s.participants_for_session("s1").await.unwrap();
        let (a, b) = tokio::join!(
            s.request_participant_change("s1", "op1", rev, switch(id), ParticipantChangeOrigin::User),
            s.request_participant_change("s1", "op1", rev, switch(id), ParticipantChangeOrigin::User),
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(s.participants_for_session("s1").await.unwrap(), before);
        assert_eq!(s.roster_revision("s1").await.unwrap(), rev);
        assert!(s.request_participant_change("s1", "op2", rev, switch(id), ParticipantChangeOrigin::User).await.unwrap_err().to_string().contains("pending"));
        assert!(s.request_participant_change("s1", "op1", rev, ParticipantChangeRequest::Remove { participant_id: id }, ParticipantChangeOrigin::User).await.is_err());
        assert!(s.request_participant_change("s1", "op1", rev, switch(id), ParticipantChangeOrigin::Agent { session_id: "driver".into(), participant_id: 42 }).await.is_err(), "a replay cannot replace the authenticated origin");
        assert!(s.participant_change("another-session", "op1").await.unwrap().is_none());
        assert!(!s.cancel_participant_change("another-session", "op1").await.unwrap());
        assert!(!s.finish_participant_change("s1", "op1", None).await.unwrap(), "pending is not completed");
    }

    #[tokio::test]
    async fn claim_and_cancel_have_one_winner_and_checkpoints_are_write_once() {
        let s = Storage::memory().await.unwrap();
        let (id, _, _) = seed(&s).await;
        let rev = s.roster_revision("s1").await.unwrap();
        s.request_participant_change("s1", "race", rev, switch(id), ParticipantChangeOrigin::User).await.unwrap();
        let (claim, cancel) = tokio::join!(s.claim_participant_change("s1", "race"), s.cancel_participant_change("s1", "race"));
        let claim = claim.unwrap();
        let cancel = cancel.unwrap();
        assert_ne!(claim.is_some(), cancel, "one atomic transition wins");
        let row = s.participant_change("s1", "race").await.unwrap().unwrap();
        assert_eq!(row.status, if cancel { "cancelled" } else { "applying" });
        if claim.is_some() {
            assert!(s.claim_participant_change("s1", "race").await.unwrap().is_none());
            s.finish_participant_change("s1", "race", Some("test ends this attempt")).await.unwrap();
        }
        s.request_participant_change("s1", "complete", rev, switch(id), ParticipantChangeOrigin::User).await.unwrap();
        assert!(s.claim_participant_change("s1", "complete").await.unwrap().is_some());
        assert!(!s.finish_participant_change("s1", "complete", None).await.unwrap(), "success cannot precede a recovery checkpoint");
        let checkpoint = serde_json::json!({"participant_id":id,"old_model":"m1"});
        assert!(s.checkpoint_participant_change("s1", "complete", &checkpoint).await.unwrap());
        assert!(!s.checkpoint_participant_change("s1", "complete", &serde_json::json!({"overwrite":true})).await.unwrap());
        assert!(s.finish_participant_change("s1", "complete", None).await.unwrap());
        assert!(!s.finish_participant_change("s1", "complete", Some("late failure")).await.unwrap());
        let row = s.participant_change("s1", "complete").await.unwrap().unwrap();
        assert_eq!(row.status, "applied");
        assert_eq!(row.recovery_json.unwrap(), checkpoint.to_string());
        assert!(row.started_at.unwrap().ends_with('Z') && row.finished_at.unwrap().ends_with('Z'));
    }

    #[tokio::test]
    async fn stale_rosters_fail_at_claim_but_runtime_bookkeeping_is_not_a_roster_edit() {
        let s = Storage::memory().await.unwrap();
        let (id, _, _) = seed(&s).await;
        let rev = s.roster_revision("s1").await.unwrap();
        s.request_participant_change("s1", "stale", rev, switch(id), ParticipantChangeOrigin::User).await.unwrap();
        sqlx::query("UPDATE session_participants SET done_vote = 1, claude_session_id = 'thread-id' WHERE id = ?")
            .bind(id).execute(s.pool()).await.unwrap();
        assert_eq!(s.roster_revision("s1").await.unwrap(), rev);
        sqlx::query("UPDATE session_participants SET label = 'Changed' WHERE id = ?")
            .bind(id).execute(s.pool()).await.unwrap();
        assert!(s.roster_revision("s1").await.unwrap() > rev);
        assert!(s.claim_participant_change("s1", "stale").await.unwrap().is_none());
        let row = s.participant_change("s1", "stale").await.unwrap().unwrap();
        assert_eq!(row.status, "failed");
        assert!(row.error.unwrap().contains("refresh"));
        assert!(s.request_participant_change("s1", "old-dialog", rev, switch(id), ParticipantChangeOrigin::User).await.is_err());
        assert!(s.open_participant_change("s1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn roster_request_guards_cover_ownership_roles_models_labels_cap_and_last_active() {
        let s = Storage::memory().await.unwrap();
        let (first, second, role_id) = seed(&s).await;
        let rev = s.roster_revision("s1").await.unwrap();
        let invalid = [
            ParticipantChangeRequest::Remove { participant_id: i64::MAX },
            ParticipantChangeRequest::Add { role_id: i64::MAX, model_id: "m1".into(), label: None },
            ParticipantChangeRequest::Add { role_id, model_id: "missing".into(), label: None },
            ParticipantChangeRequest::Add { role_id, model_id: "m1".into(), label: Some(" hands ".into()) },
            ParticipantChangeRequest::SwitchModel { participant_id: first, model_id: "m1".into() },
        ];
        for (i, request) in invalid.into_iter().enumerate() {
            assert!(s.request_participant_change("s1", &format!("invalid-{i}"), rev, request, ParticipantChangeOrigin::User).await.is_err());
        }
        s.set_role_archived(role_id, true).await.unwrap();
        let add = ParticipantChangeRequest::Add { role_id, model_id: "m1".into(), label: None };
        assert!(s.request_participant_change("s1", "archived", rev, add.clone(), ParticipantChangeOrigin::User).await.is_err());
        s.set_role_archived(role_id, false).await.unwrap();
        for slot in 2..MAX_SESSION_PARTICIPANTS {
            s.insert_participant("s1", &format!("extra-{slot}"), "Extra", Some(role_id), Some("m1"), "[]", MODE_ACTIVE, slot as i64).await.unwrap();
        }
        let rev = s.roster_revision("s1").await.unwrap();
        assert!(s.request_participant_change("s1", "full", rev, add, ParticipantChangeOrigin::User).await.unwrap_err().to_string().contains("at most"));
        sqlx::query("UPDATE session_participants SET enabled = 0 WHERE session_id = 's1' AND id NOT IN (?, ?)")
            .bind(first).bind(second).execute(s.pool()).await.unwrap();
        sqlx::query("UPDATE session_participants SET participation_mode = 'on_mention' WHERE id = ?")
            .bind(second).execute(s.pool()).await.unwrap();
        let rev = s.roster_revision("s1").await.unwrap();
        assert!(s.request_participant_change("s1", "last", rev, ParticipantChangeRequest::Remove { participant_id: first }, ParticipantChangeOrigin::User).await.unwrap_err().to_string().contains("last active"));
        s.close_session("s1", false).await.unwrap();
        assert!(s.request_participant_change("s1", "closed", rev, switch(first), ParticipantChangeOrigin::User).await.unwrap_err().to_string().contains("reopen"));
    }

    #[tokio::test]
    async fn selection_invalidated_while_waiting_fails_without_mutating_the_roster() {
        let s = Storage::memory().await.unwrap();
        let (_, _, role_id) = seed(&s).await;
        let before = s.participants_for_session("s1").await.unwrap();
        let rev = s.roster_revision("s1").await.unwrap();
        s.request_participant_change("s1", "role", rev, ParticipantChangeRequest::Add {
            role_id, model_id: "m2".into(), label: Some("New agent".into()),
        }, ParticipantChangeOrigin::User).await.unwrap();
        s.set_role_archived(role_id, true).await.unwrap();
        assert!(s.claim_participant_change("s1", "role").await.unwrap().is_none());
        assert!(s.participant_change("s1", "role").await.unwrap().unwrap().error.unwrap().contains("archived"));
        s.set_role_archived(role_id, false).await.unwrap();
        s.request_participant_change("s1", "model", rev, ParticipantChangeRequest::Add {
            role_id, model_id: "m2".into(), label: None,
        }, ParticipantChangeOrigin::User).await.unwrap();
        s.delete_model("m2").await.unwrap();
        assert!(s.claim_participant_change("s1", "model").await.unwrap().is_none());
        assert!(s.participant_change("s1", "model").await.unwrap().unwrap().error.unwrap().contains("model"));
        assert_eq!(s.participants_for_session("s1").await.unwrap(), before);
        assert_eq!(s.ensure_session_roster("s1", 8).await.unwrap(), 0, "ordinary respawn must never append");
    }

    #[tokio::test]
    async fn saved_selection_changes_are_detected_without_timestamp_or_secret_copies() {
        let s = Storage::memory().await.unwrap();
        let (id, _, role_id) = seed(&s).await;
        let rev = s.roster_revision("s1").await.unwrap();
        for field in ["model", "role"] {
            let row = s.request_participant_change("s1", field, rev, switch(id), ParticipantChangeOrigin::User).await.unwrap();
            assert_eq!(row.selection_fingerprint.len(), 64);
            if field == "model" {
                // Intentionally leave updated_at unchanged: it is not the proof.
                sqlx::query("UPDATE models SET context_window = 123456, auth_token = 'fixture-credential' WHERE id = 'm2'")
                    .execute(s.pool()).await.unwrap();
            } else {
                sqlx::query("UPDATE roles SET description_prompt = COALESCE(description_prompt, '') || ' changed' WHERE id = ?")
                    .bind(role_id).execute(s.pool()).await.unwrap();
            }
            assert_eq!(s.roster_revision("s1").await.unwrap(), rev, "saved selections are separate from roster edits");
            assert!(s.claim_participant_change("s1", field).await.unwrap().is_none());
            let row = s.participant_change("s1", field).await.unwrap().unwrap();
            assert_eq!(row.status, "failed");
            assert!(row.error.as_deref().unwrap().contains("settings changed"));
            assert!(!format!("{row:?}").contains("fixture-credential"), "credentials never enter the ledger");
        }
    }

    #[tokio::test]
    async fn requests_and_recovery_checkpoints_survive_a_storage_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("changes.db");
        let s = Storage::open(&path).await.unwrap();
        // Disk storage is a fresh install (one neutral role); this fixture
        // explicitly chooses the same pair Storage::memory installs for tests.
        s.install_role_preset().await.unwrap();
        let neutral = s.role_by_slug("agent").await.unwrap().unwrap();
        s.set_role_archived(neutral.id, true).await.unwrap();
        let (id, _, _) = seed(&s).await;
        let rev = s.roster_revision("s1").await.unwrap();
        let reopened = Storage::open(&path).await.unwrap();
        let (a, b) = tokio::join!(
            s.request_participant_change("s1", "durable", rev, switch(id), ParticipantChangeOrigin::User),
            reopened.request_participant_change("s1", "durable", rev, switch(id), ParticipantChangeOrigin::User),
        );
        assert_eq!(a.unwrap(), b.unwrap(), "idempotency holds across independent connection pools");
        assert_eq!(reopened.open_participant_change("s1").await.unwrap().unwrap().request().unwrap(), switch(id));
        reopened.claim_participant_change("s1", "durable").await.unwrap().unwrap();
        let checkpoint = serde_json::json!({"old_model":"m1"});
        reopened.checkpoint_participant_change("s1", "durable", &checkpoint).await.unwrap();
        let again = Storage::open(&path).await.unwrap();
        let row = again.open_participant_change("s1").await.unwrap().unwrap();
        assert_eq!(row.status, "applying");
        assert_eq!(row.recovery_json.as_deref(), Some(checkpoint.to_string().as_str()));
        assert_eq!(row.origin().unwrap(), ParticipantChangeOrigin::User);
        assert!(again.finish_participant_change("s1", "durable", Some("recovered by the host")).await.unwrap());
        assert_eq!(again.participant_change("s1", "durable").await.unwrap().unwrap().status, "failed");
    }
}

