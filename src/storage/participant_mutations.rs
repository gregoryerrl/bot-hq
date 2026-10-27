//! Transactional roster preparation and recovery. The host must hold the ring's
//! safe boundary before claiming/preparing, and retain it through runtime swap
//! and completion/rollback. No process or channel cursor is touched here.

use super::*;
use super::participant_changes::{read_change, selection_fingerprint, validate_request};
use super::participants::{participant_from_row, role_from_row};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use std::collections::HashSet;

/// Frozen spawn inputs from the SAME transaction that checked the selection
/// fingerprint. Model credentials stay in memory, never in the recovery row.
pub struct PreparedParticipantChange {
    pub participant: Participant,
    pub previous: Option<Participant>,
    pub model: Option<Model>,
    pub role: Option<Role>,
    /// The channel frontier captured with this preparation revision. New
    /// members join here; replacements keep their existing delivered cursor.
    pub history_frontier: i64,
}

/// Observed candidate state, kept in memory until the lifecycle owner publishes
/// the prepared change. No model credentials or MCP token belong here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantSpawnMetadata {
    pub runtime: String,
    pub runtime_version: String,
    pub resume_id: Option<String>,
    pub account_dir: Option<String>,
    pub account_email: Option<String>,
    pub account_org: Option<String>,
    pub effort: Option<String>,
    pub ultracode: Option<bool>,
    pub system_prompt: String,
}

#[derive(Serialize, Deserialize)]
struct Recovery {
    version: u8,
    action: RecoveryAction,
    /// Legacy checkpoints can still recover. Only a newly prepared runtime
    /// may publish, against the actual post-mutation revision it observed.
    #[serde(default)]
    publication_revision: Option<i64>,
    #[serde(default)]
    history_frontier: Option<i64>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RecoveryAction {
    Added { participant_id: i64 },
    Removed { participant_id: i64, enabled: i64 },
    Model { participant_id: i64, after_model: String, after_runtime: String, before: ModelState },
}

/// Only the fields a model switch owns. Restoring these never overwrites a
/// peer, a label, a capability grant, user-selected knobs or channel history.
#[derive(Serialize, Deserialize, sqlx::FromRow)]
struct ModelState {
    model_id: Option<String>,
    runtime: String,
    claude_session_id: Option<String>,
    effort_at_spawn: Option<String>,
    ultracode_at_spawn: Option<i64>,
    spawn_knobs_recorded: i64,
    account_dir_at_spawn: Option<String>,
    account_email_at_spawn: Option<String>,
    account_org_at_spawn: Option<String>,
    runtime_version_at_spawn: String,
}

#[derive(Debug)]
struct RecoveryConflict;

impl std::fmt::Display for RecoveryConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("participant changed outside this operation; refusing to overwrite the newer state")
    }
}
impl std::error::Error for RecoveryConflict {}

async fn participant_optional(conn: &mut SqliteConnection, session: &str, id: i64) -> Result<Option<Participant>> {
    let row = sqlx::query("SELECT * FROM session_participants WHERE session_id = ? AND id = ?")
        .bind(session).bind(id).fetch_optional(conn).await?;
    Ok(row.as_ref().map(participant_from_row))
}

async fn participant(conn: &mut SqliteConnection, session: &str, id: i64) -> Result<Participant> {
    participant_optional(conn, session, id).await?.context("participant is no longer in this session")
}

fn recovery(row: &ParticipantChange) -> Result<Recovery> {
    let body = row.recovery_json.as_deref().context("participant change has no recovery checkpoint")?;
    let snapshot: Recovery = serde_json::from_str(body).context("invalid participant change recovery checkpoint")?;
    if snapshot.version != 1 { anyhow::bail!("unsupported participant change recovery version"); }
    match (&snapshot.action, row.request()?) {
        (RecoveryAction::Added { .. }, ParticipantChangeRequest::Add { .. }) => {}
        (RecoveryAction::Removed { participant_id: a, .. }, ParticipantChangeRequest::Remove { participant_id: b }) if *a == b => {}
        (RecoveryAction::Model { participant_id: a, after_model, .. }, ParticipantChangeRequest::SwitchModel { participant_id: b, model_id }) if *a == b && *after_model == model_id => {}
        _ => anyhow::bail!("recovery checkpoint does not match its participant change"),
    }
    Ok(snapshot)
}

async fn check_prepared(conn: &mut SqliteConnection, row: &ParticipantChange, snapshot: &Recovery) -> Result<()> {
    let (id, expected_enabled, expected_model, expected_runtime) = match &snapshot.action {
        RecoveryAction::Added { participant_id } => {
            let ParticipantChangeRequest::Add { model_id, .. } = row.request()? else { unreachable!() };
            (*participant_id, true, Some(model_id), None)
        }
        RecoveryAction::Removed { participant_id, .. } => (*participant_id, false, None, None),
        RecoveryAction::Model { participant_id, after_model, after_runtime, .. } =>
            (*participant_id, true, Some(after_model.clone()), Some(after_runtime.as_str())),
    };
    let Some(current) = participant_optional(conn, &row.session_id, id).await? else {
        return Err(RecoveryConflict.into());
    };
    if current.enabled != expected_enabled
        || expected_model.as_ref().is_some_and(|m| current.model_id.as_ref() != Some(m))
        || expected_runtime.is_some_and(|r| current.runtime != r) {
        return Err(RecoveryConflict.into());
    }
    Ok(())
}

async fn restore(conn: &mut SqliteConnection, row: &ParticipantChange, snapshot: &Recovery) -> Result<()> {
    check_prepared(conn, row, snapshot).await?;
    match &snapshot.action {
        RecoveryAction::Added { participant_id } => {
            // Keep the row, cursor and any attempted-spawn messages as history.
            sqlx::query("UPDATE session_participants SET enabled = 0, done_vote = 0 WHERE session_id = ? AND id = ?")
                .bind(&row.session_id).bind(participant_id).execute(&mut *conn).await?;
        }
        RecoveryAction::Removed { participant_id, enabled } => {
            sqlx::query("UPDATE session_participants SET enabled = ?, done_vote = 0 WHERE session_id = ? AND id = ?")
                .bind(enabled).bind(&row.session_id).bind(participant_id).execute(&mut *conn).await?;
        }
        RecoveryAction::Model { participant_id, before: s, .. } => {
            sqlx::query("UPDATE session_participants SET model_id = ?, runtime = ?, claude_session_id = ?, \
                effort_at_spawn = ?, ultracode_at_spawn = ?, spawn_knobs_recorded = ?, account_dir_at_spawn = ?, \
                account_email_at_spawn = ?, account_org_at_spawn = ?, runtime_version_at_spawn = ?, done_vote = 0 \
                WHERE session_id = ? AND id = ?")
                .bind(&s.model_id).bind(&s.runtime).bind(&s.claude_session_id)
                .bind(&s.effort_at_spawn).bind(s.ultracode_at_spawn).bind(s.spawn_knobs_recorded)
                .bind(&s.account_dir_at_spawn).bind(&s.account_email_at_spawn).bind(&s.account_org_at_spawn)
                .bind(&s.runtime_version_at_spawn).bind(&row.session_id).bind(participant_id)
                .execute(&mut *conn).await?;
        }
    }
    Ok(())
}

/// Called in the SAME transaction as close/reopen. A pending request cancels;
/// an applying request restores its owned fields before becoming failed. A
/// newer external edit is preserved and recorded as a conflict, not a reason
/// to prevent closing. Other storage/checkpoint failures still propagate.
pub(super) async fn settle_open_change(conn: &mut SqliteConnection, session: &str, reason: &str) -> Result<bool> {
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM session_participant_changes WHERE session_id = ? AND status IN ('pending', 'applying')")
        .bind(session).fetch_optional(&mut *conn).await?;
    let Some(id) = id else { return Ok(false) };
    let row = read_change(conn, &id).await?.context("participant change disappeared")?;
    let detail = if row.status == "pending" {
        "pending participant change cancelled"
    } else if row.recovery_json.is_some() {
        "unfinished participant change rolled back to its previous configuration"
    } else {
        "unfinished participant change failed before preparation; configuration unchanged"
    };
    match settle(conn, &row, &format!("{reason}; {detail}.")).await {
        Ok(()) => {}
        // Close/reopen supersedes the operation. A conflict is checked before
        // any restore write: preserve the newer state, retire old intent, and
        // let the lifecycle transition commit. Never swallow SQL/IO failures.
        Err(error) if error.is::<RecoveryConflict>() => {
            sqlx::query("UPDATE session_participant_changes SET status = 'failed', error = ?, finished_at = ? WHERE id = ? AND session_id = ?")
                .bind(format!("{reason}; participant change failed: {error}. Newer participant state preserved."))
                .bind(now_utc()).bind(&row.id).bind(session).execute(&mut *conn).await?;
        }
        Err(error) => return Err(error),
    }
    Ok(true)
}

async fn settle(conn: &mut SqliteConnection, row: &ParticipantChange, reason: &str) -> Result<()> {
    if row.status == "applying" && row.recovery_json.is_some() {
        restore(conn, row, &recovery(row)?).await?;
    }
    sqlx::query("UPDATE session_participant_changes SET status = ?, error = ?, finished_at = ? WHERE id = ? AND session_id = ?")
        .bind(if row.status == "pending" { "cancelled" } else { "failed" })
        .bind(reason).bind(now_utc()).bind(&row.id).bind(&row.session_id).execute(conn).await?;
    Ok(())
}

impl Storage {
    /// Runtime recovery after an owned rollback. The worker still holds the
    /// same ring reservation; a close or a later roster edit wins in storage.
    pub(crate) async fn publish_recovered_participant_spawn(&self, session: &str, id: &str,
        participant_id: i64, revision: i64, history_frontier: i64, metadata: &ParticipantSpawnMetadata) -> Result<bool> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some(row) = read_change(&mut tx, id).await?.filter(|r| r.session_id == session && r.status == "failed") else { return Ok(false) };
        let target = match row.request()? {
            ParticipantChangeRequest::Remove { participant_id } | ParticipantChangeRequest::SwitchModel { participant_id, .. } => participant_id,
            ParticipantChangeRequest::Add { .. } => return Ok(false),
        };
        if target != participant_id { return Ok(false); }
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ? AND closed_at IS NULL AND archived = 0 AND roster_revision = ?) \
            AND NOT EXISTS(SELECT 1 FROM session_participant_changes WHERE session_id = ? AND status IN ('pending', 'applying'))")
            .bind(session).bind(revision).bind(session).fetch_one(&mut *tx).await?;
        if !valid { return Ok(false); }
        let cursor: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT last_read_message_id FROM participant_cursors WHERE participant_id = ?), 0)")
            .bind(participant_id).fetch_one(&mut *tx).await?;
        if cursor != history_frontier { return Ok(false); }
        let current = participant(&mut tx, session, participant_id).await?;
        if !current.enabled || current.runtime != metadata.runtime { return Ok(false); }
        sqlx::query("UPDATE session_participants SET runtime_version_at_spawn = ?, claude_session_id = ?, \
            account_dir_at_spawn = ?, account_email_at_spawn = ?, account_org_at_spawn = ?, \
            effort_at_spawn = ?, ultracode_at_spawn = ?, spawn_knobs_recorded = 1, prompt = ? WHERE session_id = ? AND id = ?")
            .bind(&metadata.runtime_version).bind(&metadata.resume_id).bind(&metadata.account_dir)
            .bind(&metadata.account_email).bind(&metadata.account_org).bind(&metadata.effort)
            .bind(metadata.ultracode.map(i64::from)).bind(&metadata.system_prompt).bind(session).bind(participant_id)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The only write of a candidate's observed metadata. Close/recovery and
    /// publication serialize in one transaction: a late probe/candidate cannot
    /// overwrite a rolled-back participant or reopen an already-failed intent.
    pub async fn publish_participant_spawn(
        &self, session: &str, id: &str, participant_id: i64,
        metadata: &ParticipantSpawnMetadata,
    ) -> Result<bool> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some(row) = read_change(&mut tx, id).await?
            .filter(|r| r.session_id == session && r.status == "applying" && r.recovery_json.is_some())
            else { return Ok(false) };
        let snapshot = recovery(&row)?;
        let target = match &snapshot.action {
            RecoveryAction::Added { participant_id } | RecoveryAction::Model { participant_id, .. } => *participant_id,
            RecoveryAction::Removed { .. } => anyhow::bail!("a removal cannot publish spawn metadata"),
        };
        if target != participant_id { anyhow::bail!("spawn metadata names another participant"); }
        let open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ? AND closed_at IS NULL AND archived = 0)")
            .bind(session).fetch_one(&mut *tx).await?;
        if !open { return Ok(false); }
        let revision: i64 = sqlx::query_scalar("SELECT roster_revision FROM sessions WHERE id = ?")
            .bind(session).fetch_one(&mut *tx).await?;
        let prepared_revision = snapshot.publication_revision
            .context("an older interrupted change must be recovered before publishing its agent")?;
        if prepared_revision != revision {
            anyhow::bail!("the roster changed while this participant was starting; recover the change");
        }
        check_prepared(&mut tx, &row, &snapshot).await?;
        let frontier = snapshot.history_frontier.context("an older interrupted change must recover before publishing a history snapshot")?;
        let cursor: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT last_read_message_id FROM participant_cursors WHERE participant_id = ?), 0)")
            .bind(target).fetch_one(&mut *tx).await?;
        anyhow::ensure!(cursor == frontier, "the participant's delivery cursor changed during startup; recover before publishing");
        let current = participant(&mut tx, session, target).await?;
        if current.runtime != metadata.runtime { anyhow::bail!("spawn metadata belongs to another runtime"); }
        sqlx::query("UPDATE session_participants SET runtime_version_at_spawn = ?, claude_session_id = ?, \
            account_dir_at_spawn = ?, account_email_at_spawn = ?, account_org_at_spawn = ?, \
            effort_at_spawn = ?, ultracode_at_spawn = ?, spawn_knobs_recorded = 1, prompt = ? \
            WHERE session_id = ? AND id = ?")
            .bind(&metadata.runtime_version).bind(&metadata.resume_id)
            .bind(&metadata.account_dir).bind(&metadata.account_email).bind(&metadata.account_org)
            .bind(&metadata.effort).bind(metadata.ultracode.map(i64::from)).bind(&metadata.system_prompt)
            .bind(session).bind(target).execute(&mut *tx).await?;
        sqlx::query("UPDATE session_participant_changes SET status = 'applied', finished_at = ? WHERE id = ? AND session_id = ?")
            .bind(now_utc()).bind(id).bind(session).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// A claimed request prepares ONCE: snapshot and row mutation commit
    /// together. The runtime holds a safe boundary and owns process recovery.
    pub async fn prepare_participant_change(&self, session: &str, id: &str) -> Result<PreparedParticipantChange> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = read_change(&mut tx, id).await?.filter(|r| r.session_id == session && r.status == "applying")
            .context("participant change is not claimed by the runtime")?;
        if row.recovery_json.is_some() { anyhow::bail!("participant change is already prepared; finish or recover it"); }
        let request = row.request()?;
        validate_request(&mut tx, session, row.expected_revision, &request).await?;
        if selection_fingerprint(&mut tx, &request).await? != row.selection_fingerprint {
            anyhow::bail!("selected role or model settings changed before preparation");
        }
        let previous = match &request {
            ParticipantChangeRequest::Remove { participant_id } | ParticipantChangeRequest::SwitchModel { participant_id, .. } => Some(participant(&mut tx, session, *participant_id).await?),
            _ => None,
        };
        let (model_id, role_id) = match &request {
            ParticipantChangeRequest::Add { model_id, role_id, .. } => (Some(model_id), Some(*role_id)),
            ParticipantChangeRequest::SwitchModel { model_id, .. } => (Some(model_id), previous.as_ref().and_then(|p| p.role_id)),
            _ => (None, None),
        };
        let model: Option<Model> = if let Some(id) = model_id {
            Some(sqlx::query_as("SELECT * FROM models WHERE id = ?").bind(id).fetch_one(&mut *tx).await?)
        } else { None };
        if model.as_ref().is_some_and(|m| !["claude_code", "codex"].contains(&m.runtime.as_str())) {
            anyhow::bail!("the selected model has an unsupported runtime");
        }
        let role = if let Some(id) = role_id {
            Some(role_from_row(&sqlx::query("SELECT * FROM roles WHERE id = ?").bind(id).fetch_one(&mut *tx).await?))
        } else { None };
        let now = now_utc();
        let (changed_id, action) = match &request {
            ParticipantChangeRequest::Add { label, .. } => {
                let role = role.as_ref().context("new participant has no role")?;
                let model = model.as_ref().context("new participant has no model")?;
                let existing: Vec<(String, Option<String>)> = sqlx::query_as("SELECT slug, label FROM session_participants WHERE session_id = ?")
                    .bind(session).fetch_all(&mut *tx).await?;
                let taken: HashSet<String> = existing.into_iter().flat_map(|(slug, label)| {
                    std::iter::once(slug.to_ascii_lowercase()).chain(label.map(|l| l.trim().to_ascii_lowercase()))
                }).collect();
                let slug = participant_slug(&role.slug, &taken);
                let last: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(turn_position), -1) FROM session_participants WHERE session_id = ?")
                    .bind(session).fetch_one(&mut *tx).await?;
                let position = last.checked_add(1).context("participant turn order overflow")?;
                let id = sqlx::query("INSERT INTO session_participants \
                    (session_id, slug, display_name, role_id, model_id, runtime, capabilities, participation_mode, turn_position, enabled, joined_at, label) \
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)")
                    .bind(session).bind(slug).bind(&role.display_name).bind(role.id).bind(&model.id)
                    .bind(&model.runtime).bind(&role.capabilities).bind(&role.participation_mode).bind(position)
                    .bind(&now).bind(label).execute(&mut *tx).await?.last_insert_rowid();
                // A late invite joins at this channel frontier. This is NOT a
                // delivery: old rows stay stored and receive no delivery receipt.
                let frontier: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM messages WHERE session_id = ?")
                    .bind(session).fetch_one(&mut *tx).await?;
                sqlx::query("INSERT INTO participant_cursors (participant_id, last_read_message_id, updated_at) VALUES (?, ?, ?)")
                    .bind(id).bind(frontier).bind(&now).execute(&mut *tx).await?;
                (id, RecoveryAction::Added { participant_id: id })
            }
            ParticipantChangeRequest::Remove { participant_id } => {
                let enabled: i64 = sqlx::query_scalar("SELECT enabled FROM session_participants WHERE session_id = ? AND id = ?")
                    .bind(session).bind(participant_id).fetch_one(&mut *tx).await?;
                sqlx::query("UPDATE session_participants SET enabled = 0 WHERE session_id = ? AND id = ?")
                    .bind(session).bind(participant_id).execute(&mut *tx).await?;
                (*participant_id, RecoveryAction::Removed { participant_id: *participant_id, enabled })
            }
            ParticipantChangeRequest::SwitchModel { participant_id, model_id } => {
                let before: ModelState = sqlx::query_as("SELECT * FROM session_participants WHERE session_id = ? AND id = ?")
                    .bind(session).bind(participant_id).fetch_one(&mut *tx).await?;
                let runtime = &model.as_ref().context("model switch has no model")?.runtime;
                sqlx::query("UPDATE session_participants SET model_id = ?, runtime = ?, claude_session_id = NULL, \
                    effort_at_spawn = NULL, ultracode_at_spawn = NULL, spawn_knobs_recorded = 0, account_dir_at_spawn = NULL, \
                    account_email_at_spawn = NULL, account_org_at_spawn = NULL, runtime_version_at_spawn = '' WHERE session_id = ? AND id = ?")
                    .bind(model_id).bind(runtime).bind(session).bind(participant_id).execute(&mut *tx).await?;
                (*participant_id, RecoveryAction::Model { participant_id: *participant_id, after_model: model_id.clone(), after_runtime: runtime.clone(), before })
            }
        };
        let publication_revision: i64 = sqlx::query_scalar("SELECT roster_revision FROM sessions WHERE id = ?")
            .bind(session).fetch_one(&mut *tx).await?;
        let history_frontier: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT last_read_message_id FROM participant_cursors WHERE participant_id = ?), 0)")
            .bind(changed_id).fetch_one(&mut *tx).await?;
        let checkpoint = serde_json::to_string(&Recovery { version: 1, action, publication_revision: Some(publication_revision), history_frontier: Some(history_frontier) })?;
        if checkpoint.len() > 256 * 1024 { anyhow::bail!("participant recovery checkpoint exceeds 256 KiB; nothing changed"); }
        sqlx::query("UPDATE session_participant_changes SET recovery_json = ? WHERE id = ? AND session_id = ? AND recovery_json IS NULL")
            .bind(checkpoint).bind(id).bind(session).execute(&mut *tx).await?;
        // A different electorate/context cannot inherit an old consensus.
        // Retractions keep the vote audit; rollback never resurrects ballots.
        sqlx::query("UPDATE session_participants SET done_vote = 0 WHERE session_id = ?")
            .bind(session).execute(&mut *tx).await?;
        sqlx::query("UPDATE phase_votes SET retracted_at = ? WHERE session_id = ? AND retracted_at IS NULL \
            AND phase_epoch = (SELECT phase_epoch FROM sessions WHERE id = ?)")
            .bind(&now).bind(session).bind(session).execute(&mut *tx).await?;
        let participant = participant(&mut tx, session, changed_id).await?;
        tx.commit().await?;
        Ok(PreparedParticipantChange { participant, previous, model, role, history_frontier })
    }

    /// Cold-start use only, before spawning a session. Pending intent survives
    /// restart; an interrupted applying operation is rolled back, never replayed.
    pub async fn recover_interrupted_participant_change(&self, session: &str) -> Result<Option<ParticipantChange>> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let id: Option<String> = sqlx::query_scalar("SELECT id FROM session_participant_changes WHERE session_id = ? AND status = 'applying'")
            .bind(session).fetch_optional(&mut *tx).await?;
        let Some(id) = id else { return Ok(None) };
        let row = read_change(&mut tx, &id).await?.context("participant change disappeared")?;
        let reason = if row.recovery_json.is_some() {
            "Participant change interrupted; restored its previous configuration."
        } else {
            "Participant change interrupted before preparation; configuration unchanged."
        };
        settle(&mut tx, &row, reason).await?;
        let result = read_change(&mut tx, &id).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub(super) async fn complete_participant_change(&self, session: &str, id: &str, error: Option<&str>) -> Result<bool> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some(row) = read_change(&mut tx, id).await?.filter(|r| r.session_id == session && r.status == "applying") else { return Ok(false) };
        if let Some(reason) = error {
            settle(&mut tx, &row, reason).await?;
        } else {
            if row.recovery_json.is_none() { return Ok(false); }
            let open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ? AND closed_at IS NULL AND archived = 0)")
                .bind(session).fetch_one(&mut *tx).await?;
            if !open { return Ok(false); }
            check_prepared(&mut tx, &row, &recovery(&row)?).await?;
            sqlx::query("UPDATE session_participant_changes SET status = 'applied', finished_at = ? WHERE id = ? AND session_id = ?")
                .bind(now_utc()).bind(id).bind(session).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture() -> (Storage, Vec<Participant>) {
        let s = Storage::memory().await.unwrap();
        for (id, runtime) in [("old", "claude_code"), ("new", "codex")] {
            s.upsert_model(&Model {
                favorite: false,
                id: id.into(), display_name: id.into(), provider: "fixture".into(), model_name: id.into(),
                base_url: None, auth_token: Some("fixture-secret".into()), created_at: String::new(), updated_at: String::new(),
                context_window: Some(100_000), cli_settings: None, claude_config_dir: Some(format!("/accounts/{id}")), runtime: runtime.into(),
            }).await.unwrap();
        }
        s.create_session("s1", "test", None).await.unwrap();
        s.ensure_session_roster("s1", 2).await.unwrap();
        sqlx::query("UPDATE session_participants SET model_id = 'old', claude_session_id = 'old-thread-' || slug, \
            account_dir_at_spawn = '/old-account', account_email_at_spawn = 'old@example.com', account_org_at_spawn = 'old-org', \
            runtime_version_at_spawn = 'old-version', effort = 'high', ultracode = 0, effort_at_spawn = 'high', \
            ultracode_at_spawn = 0, spawn_knobs_recorded = 1 WHERE session_id = 's1'")
            .execute(s.pool()).await.unwrap();
        let p = s.participants_for_session("s1").await.unwrap();
        (s, p)
    }

    async fn claim(s: &Storage, id: &str, request: ParticipantChangeRequest) {
        let rev = s.roster_revision("s1").await.unwrap();
        s.request_participant_change("s1", id, rev, request, ParticipantChangeOrigin::User).await.unwrap();
        assert!(s.claim_participant_change("s1", id).await.unwrap().is_some());
    }

    fn switch(p: &Participant) -> ParticipantChangeRequest {
        ParticipantChangeRequest::SwitchModel { participant_id: p.id, model_id: "new".into() }
    }

    fn spawn_metadata() -> ParticipantSpawnMetadata {
        ParticipantSpawnMetadata {
            runtime: "codex".into(), runtime_version: "observed-version".into(),
            resume_id: Some("ready-thread".into()), account_dir: Some("/new-account".into()),
            account_email: Some("new@example.com".into()), account_org: Some("new-org".into()),
            effort: Some("low".into()), ultracode: Some(false), system_prompt: "candidate prompt".into(),
        }
    }

    #[tokio::test]
    async fn late_additions_join_their_own_channel_frontier_without_delivery_receipts() {
        let (s, peers) = fixture().await;
        let before = s.post_to_channel("s1", "user", None, "text", "old direction", None).await.unwrap().message_id();
        s.create_session("other", "other", None).await.unwrap();
        s.post_to_channel("other", "user", None, "text", "not this session", None).await.unwrap();
        claim(&s, "add", ParticipantChangeRequest::Add { role_id: peers[0].role_id.unwrap(), model_id: "new".into(), label: None }).await;
        let prepared = s.prepare_participant_change("s1", "add").await.unwrap();
        assert_eq!(prepared.history_frontier, before);
        assert_eq!(s.cursor_for(prepared.participant.id).await.unwrap(), before);
        let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM participant_deliveries WHERE participant_id = ?")
            .bind(prepared.participant.id).fetch_one(s.pool()).await.unwrap();
        assert_eq!(receipts, 0);
        let saved = s.participant_change("s1", "add").await.unwrap().unwrap();
        assert_eq!(recovery(&saved).unwrap().history_frontier, Some(before));
        s.finish_participant_change("s1", "add", Some("startup failed")).await.unwrap();
        assert!(!s.participant_by_id(prepared.participant.id).await.unwrap().unwrap().enabled);
        assert_eq!(s.cursor_for(prepared.participant.id).await.unwrap(), before);
    }

    #[tokio::test]
    async fn changed_delivery_frontier_refuses_publication_without_rewinding_the_cursor() {
        let (s, peers) = fixture().await;
        let old = s.post_to_channel("s1", "user", None, "text", "old", None).await.unwrap().message_id();
        s.commit_delivery(peers[0].id, &[(old, None)]).await.unwrap();
        claim(&s, "switch", switch(&peers[0])).await;
        let prepared = s.prepare_participant_change("s1", "switch").await.unwrap();
        assert_eq!(prepared.history_frontier, old);
        let later = s.post_to_channel("s1", "user", None, "text", "later", None).await.unwrap().message_id();
        s.commit_delivery(peers[0].id, &[(later, None)]).await.unwrap();
        assert!(s.publish_participant_spawn("s1", "switch", peers[0].id, &spawn_metadata()).await.unwrap_err().to_string().contains("cursor changed"));
        s.finish_participant_change("s1", "switch", Some("recover")).await.unwrap();
        assert_eq!(s.cursor_for(peers[0].id).await.unwrap(), later, "recovery never rolls back actual delivery");
    }

    #[tokio::test]
    async fn checkpoints_without_history_frontiers_recover_but_cannot_publish() {
        let (s, peers) = fixture().await;
        claim(&s, "switch", switch(&peers[0])).await;
        s.prepare_participant_change("s1", "switch").await.unwrap();
        let saved = s.participant_change("s1", "switch").await.unwrap().unwrap();
        let mut json: serde_json::Value = serde_json::from_str(saved.recovery_json.as_ref().unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("history_frontier");
        sqlx::query("UPDATE session_participant_changes SET recovery_json = ? WHERE id = 'switch'").bind(json.to_string()).execute(s.pool()).await.unwrap();
        assert!(s.publish_participant_spawn("s1", "switch", peers[0].id, &spawn_metadata()).await.is_err());
        s.recover_interrupted_participant_change("s1").await.unwrap().unwrap();
        assert_eq!(s.participant_by_id(peers[0].id).await.unwrap().unwrap(), peers[0]);
    }

    #[tokio::test]
    async fn spawn_publication_checks_target_runtime_revision_and_never_touches_a_peer() {
        let (s, peers) = fixture().await;
        claim(&s, "switch", switch(&peers[0])).await;
        s.prepare_participant_change("s1", "switch").await.unwrap();
        let metadata = spawn_metadata();
        assert!(s.publish_participant_spawn("s1", "switch", peers[1].id, &metadata).await.is_err());
        let wrong_runtime = ParticipantSpawnMetadata { runtime: "claude_code".into(), ..metadata.clone() };
        assert!(s.publish_participant_spawn("s1", "switch", peers[0].id, &wrong_runtime).await.is_err());
        sqlx::query("UPDATE session_participants SET label = 'newer user label' WHERE id = ?")
            .bind(peers[0].id).execute(s.pool()).await.unwrap();
        assert!(s.publish_participant_spawn("s1", "switch", peers[0].id, &metadata).await.is_err());
        assert!(s.finish_participant_change("s1", "switch", Some("roster changed")).await.unwrap());
        assert!(!s.publish_participant_spawn("s1", "switch", peers[0].id, &metadata).await.unwrap());
        assert_eq!(s.participant_by_id(peers[0].id).await.unwrap().unwrap().label.as_deref(), Some("newer user label"));
        claim(&s, "retry", switch(&peers[0])).await;
        s.prepare_participant_change("s1", "retry").await.unwrap();
        assert!(s.publish_participant_spawn("s1", "retry", peers[0].id, &metadata).await.unwrap());
        assert_eq!(s.participant_change("s1", "retry").await.unwrap().unwrap().status, "applied");
        assert_eq!(s.participant_by_id(peers[1].id).await.unwrap().unwrap(), peers[1]);
        let prompt: String = sqlx::query_scalar("SELECT prompt FROM session_participants WHERE id = ?")
            .bind(peers[0].id).fetch_one(s.pool()).await.unwrap();
        assert_eq!(prompt, metadata.system_prompt);
    }

    #[tokio::test]
    async fn spawn_publication_failure_commits_neither_metadata_nor_success() {
        let (s, peers) = fixture().await;
        s.store_participant_prompt(peers[0].id, "before").await.unwrap();
        claim(&s, "switch", switch(&peers[0])).await;
        let prepared = s.prepare_participant_change("s1", "switch").await.unwrap();
        // Fail AFTER the metadata UPDATE, at the success mark: this proves
        // the first write rolls back with the second, not just a pre-write refusal.
        sqlx::query("CREATE TRIGGER refuse_candidate BEFORE UPDATE OF status ON session_participant_changes \
            WHEN NEW.status = 'applied' BEGIN SELECT RAISE(ABORT, 'fixture rejects success'); END")
            .execute(s.pool()).await.unwrap();
        assert!(s.publish_participant_spawn("s1", "switch", peers[0].id, &spawn_metadata()).await.is_err());
        assert_eq!(s.participant_change("s1", "switch").await.unwrap().unwrap().status, "applying");
        assert_eq!(s.participant_by_id(peers[0].id).await.unwrap().unwrap(), prepared.participant);
        let prompt: String = sqlx::query_scalar("SELECT prompt FROM session_participants WHERE id = ?")
            .bind(peers[0].id).fetch_one(s.pool()).await.unwrap();
        assert_eq!(prompt, "before");
    }

    #[tokio::test]
    async fn spawn_publication_and_close_have_one_consistent_winner() {
        let (s, peers) = fixture().await;
        claim(&s, "switch", switch(&peers[0])).await;
        s.prepare_participant_change("s1", "switch").await.unwrap();
        let metadata = spawn_metadata();
        let (published, closed) = tokio::join!(
            s.publish_participant_spawn("s1", "switch", peers[0].id, &metadata),
            s.close_session("s1", false));
        closed.unwrap();
        let published = published.unwrap();
        let now = s.participant_by_id(peers[0].id).await.unwrap().unwrap();
        let change = s.participant_change("s1", "switch").await.unwrap().unwrap();
        assert!(s.get_session("s1").await.unwrap().unwrap().closed_at.is_some());
        if published {
            assert_eq!(change.status, "applied");
            assert_eq!(now.claude_session_id, metadata.resume_id);
        } else {
            assert_eq!(change.status, "failed");
            assert_eq!(now, peers[0]);
        }
    }

    #[tokio::test]
    async fn older_checkpoints_without_a_publication_revision_can_still_recover() {
        let (s, peers) = fixture().await;
        claim(&s, "switch", switch(&peers[0])).await;
        s.prepare_participant_change("s1", "switch").await.unwrap();
        let row = s.participant_change("s1", "switch").await.unwrap().unwrap();
        let mut json: serde_json::Value = serde_json::from_str(row.recovery_json.as_ref().unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("publication_revision");
        sqlx::query("UPDATE session_participant_changes SET recovery_json = ? WHERE id = ?")
            .bind(json.to_string()).bind(&row.id).execute(s.pool()).await.unwrap();
        assert!(s.publish_participant_spawn("s1", "switch", peers[0].id, &spawn_metadata()).await.is_err());
        s.recover_interrupted_participant_change("s1").await.unwrap().unwrap();
        assert_eq!(s.participant_by_id(peers[0].id).await.unwrap().unwrap(), peers[0]);
    }

    #[tokio::test]
    async fn addition_appends_a_unique_identity_and_failed_additions_remain_history() {
        let (s, peers) = fixture().await;
        let add = ParticipantChangeRequest::Add { role_id: peers[0].role_id.unwrap(), model_id: "new".into(), label: None };
        claim(&s, "add", add.clone()).await;
        let prepared = s.prepare_participant_change("s1", "add").await.unwrap();
        assert_eq!(prepared.participant.slug, "hands-2");
        assert_eq!(prepared.participant.turn_position, peers[1].turn_position + 1);
        assert_eq!(prepared.participant.capabilities, prepared.role.unwrap().capabilities);
        assert_eq!(prepared.participant.runtime, "codex");
        assert_eq!(s.cursor_for(prepared.participant.id).await.unwrap(), 0);
        assert_eq!(&s.participants_for_session("s1").await.unwrap()[..2], peers.as_slice());
        assert_eq!(s.ensure_session_roster("s1", 8).await.unwrap(), 0);
        let saved = s.participant_change("s1", "add").await.unwrap().unwrap();
        assert!(!saved.recovery_json.as_ref().unwrap().contains("fixture-secret"));
        let mut changed = s.get_model("new").await.unwrap().unwrap();
        changed.context_window = Some(200_000);
        s.upsert_model(&changed).await.unwrap();
        assert_eq!(prepared.model.unwrap().context_window, Some(100_000), "spawn uses frozen selected settings");
        s.post_to_channel("s1", "participant", Some(&prepared.participant.slug), "text", "orientation attempt", None).await.unwrap();
        assert!(s.finish_participant_change("s1", "add", Some("spawn failed")).await.unwrap());
        let history = s.participants_for_session("s1").await.unwrap();
        assert!(!history[2].enabled);
        assert_eq!(s.messages_for_session("s1", None).await.unwrap().len(), 1);
        claim(&s, "add-again", add).await;
        let next = s.prepare_participant_change("s1", "add-again").await.unwrap();
        assert_eq!(next.participant.slug, "hands-3", "a historical identity is never reused");
        assert!(s.finish_participant_change("s1", "add-again", None).await.unwrap());
    }

    #[tokio::test]
    async fn model_switch_resets_only_spawn_state_and_failure_restores_it_exactly() {
        let (s, before) = fixture().await;
        let msg = s.insert_user_message("s1", MessageKind::Text, "keep this history").await.unwrap();
        sqlx::query("UPDATE participant_cursors SET last_read_message_id = ? WHERE participant_id = ?")
            .bind(msg.message_id()).bind(before[0].id).execute(s.pool()).await.unwrap();
        let cursor = s.cursor_for(before[0].id).await.unwrap();
        claim(&s, "switch", switch(&before[0])).await;
        let prepared = s.prepare_participant_change("s1", "switch").await.unwrap();
        let p = prepared.participant;
        assert_eq!((p.id, p.slug.as_str(), p.role_id), (before[0].id, before[0].slug.as_str(), before[0].role_id));
        assert_eq!(p.model_id.as_deref(), Some("new"));
        assert_eq!(p.runtime, "codex");
        assert!(p.claude_session_id.is_none() && p.account_dir_at_spawn.is_none() && p.account_email_at_spawn.is_none() && p.account_org_at_spawn.is_none());
        assert!(!p.spawn_knobs_recorded && p.effort_at_spawn.is_none() && p.ultracode_at_spawn.is_none() && p.runtime_version_at_spawn.is_empty());
        assert_eq!((p.effort, p.ultracode, p.capabilities), (before[0].effort.clone(), before[0].ultracode, before[0].capabilities.clone()));
        assert_eq!(s.participants_for_session("s1").await.unwrap()[1], before[1]);
        assert_eq!(s.cursor_for(before[0].id).await.unwrap(), cursor);
        assert!(s.finish_participant_change("s1", "switch", Some("failed to start model")).await.unwrap());
        assert_eq!(s.participants_for_session("s1").await.unwrap(), before);
        assert_eq!(s.cursor_for(before[0].id).await.unwrap(), cursor);
        assert_eq!(s.messages_for_session("s1", None).await.unwrap().len(), 1);
        assert!(!s.finish_participant_change("s1", "switch", None).await.unwrap());
    }

    #[tokio::test]
    async fn removal_preserves_gates_findings_and_history_but_invalidates_old_votes() {
        let (s, peers) = fixture().await;
        s.insert_finding("s1", "finding", &peers[1].slug, FindingSeverity::Blocking, "keep this finding", None).await.unwrap();
        s.insert_tray_entry("s1", "gate", &peers[1].slug, QuestionKind::Approval, "keep this gate", Some(&["Approve".into(), "Reject".into()]), None, Some("true")).await.unwrap();
        s.set_done_vote(peers[0].id, true).await.unwrap();
        s.cast_phase_vote("s1", peers[1].id, "Verify", "work", 0).await.unwrap();
        claim(&s, "remove", ParticipantChangeRequest::Remove { participant_id: peers[1].id }).await;
        let p = s.prepare_participant_change("s1", "remove").await.unwrap();
        assert!(!p.participant.enabled);
        assert!(!s.participants_for_session("s1").await.unwrap()[0].done_vote);
        let retracted: Option<String> = sqlx::query_scalar("SELECT retracted_at FROM phase_votes WHERE participant_id = ?")
            .bind(peers[1].id).fetch_one(s.pool()).await.unwrap();
        assert!(retracted.is_some(), "ballots remain as retracted audit, never deleted");
        assert_eq!(s.count_open_blocking_findings("s1").await.unwrap(), 1);
        let status: String = sqlx::query_scalar("SELECT status FROM session_tray WHERE choice_id = 'gate'").fetch_one(s.pool()).await.unwrap();
        assert_eq!(status, "pending");
        assert!(s.finish_participant_change("s1", "remove", Some("runtime detach failed")).await.unwrap());
        assert!(s.participants_for_session("s1").await.unwrap()[1].enabled);
        let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM phase_votes WHERE retracted_at IS NULL").fetch_one(s.pool()).await.unwrap();
        assert_eq!(live, 0, "rollback cannot resurrect consensus");
    }

    #[tokio::test]
    async fn close_settles_pending_claimed_and_prepared_changes_before_reopen() {
        for stage in ["pending", "claimed", "prepared"] {
            let (s, before) = fixture().await;
            let rev = s.roster_revision("s1").await.unwrap();
            s.request_participant_change("s1", "close", rev, switch(&before[0]), ParticipantChangeOrigin::User).await.unwrap();
            if stage != "pending" { s.claim_participant_change("s1", "close").await.unwrap().unwrap(); }
            if stage == "prepared" { s.prepare_participant_change("s1", "close").await.unwrap(); }
            s.close_session("s1", false).await.unwrap();
            assert!(s.open_participant_change("s1").await.unwrap().is_none(), "close must settle {stage}");
            assert_eq!(s.participants_for_session("s1").await.unwrap(), before);
            s.reopen_session("s1").await.unwrap();
            assert!(s.claim_participant_change("s1", "close").await.unwrap().is_none(), "old intent cannot execute on reopen");
            assert!(!s.finish_participant_change("s1", "close", None).await.unwrap());
        }
        // A legacy close that did not settle: the reopen itself owns recovery.
        let (s, before) = fixture().await;
        claim(&s, "legacy", switch(&before[0])).await;
        s.prepare_participant_change("s1", "legacy").await.unwrap();
        sqlx::query("UPDATE sessions SET closed_at = ? WHERE id = 's1'").bind(now_utc()).execute(s.pool()).await.unwrap();
        assert!(s.reopen_session("s1").await.unwrap());
        assert_eq!(s.participants_for_session("s1").await.unwrap(), before);
        assert_eq!(s.participant_change("s1", "legacy").await.unwrap().unwrap().status, "failed");
    }

    #[tokio::test]
    async fn close_and_reopen_preserve_newer_edits_and_fail_the_conflicting_change() {
        for (legacy_reopen, removed_target) in [(false, false), (true, false), (false, true), (true, true)] {
            let (s, before) = fixture().await;
            claim(&s, "conflict-on-close", switch(&before[0])).await;
            s.prepare_participant_change("s1", "conflict-on-close").await.unwrap();
            if removed_target {
                sqlx::query("DELETE FROM session_participants WHERE id = ?").bind(before[0].id).execute(s.pool()).await.unwrap();
            } else {
                s.set_participant_model(before[0].id, Some("outside-edit")).await.unwrap();
            }
            let newer = s.participants_for_session("s1").await.unwrap();
            if legacy_reopen {
                sqlx::query("UPDATE sessions SET closed_at = ? WHERE id = 's1'").bind(now_utc()).execute(s.pool()).await.unwrap();
                assert!(s.reopen_session("s1").await.unwrap());
            } else {
                s.close_session("s1", false).await.unwrap();
                assert!(s.get_session("s1").await.unwrap().unwrap().closed_at.is_some());
            }
            assert_eq!(s.participants_for_session("s1").await.unwrap(), newer, "the newer edit must survive");
            let row = s.participant_change("s1", "conflict-on-close").await.unwrap().unwrap();
            assert_eq!(row.status, "failed");
            assert!(row.error.unwrap().contains("newer"), "the conflict must be recorded");
            assert!(s.open_participant_change("s1").await.unwrap().is_none());
            assert!(!s.finish_participant_change("s1", "conflict-on-close", None).await.unwrap(), "a late worker cannot finish old intent");
        }
    }

    #[tokio::test]
    async fn close_does_not_acknowledge_a_database_failure_as_recovery() {
        let (s, before) = fixture().await;
        claim(&s, "storage-error", switch(&before[0])).await;
        s.prepare_participant_change("s1", "storage-error").await.unwrap();
        sqlx::query("CREATE TRIGGER refuse_fixture_restore BEFORE UPDATE OF model_id ON session_participants \
            WHEN OLD.model_id = 'new' AND NEW.model_id = 'old' BEGIN SELECT RAISE(ABORT, 'fixture storage failure'); END")
            .execute(s.pool()).await.unwrap();
        assert!(s.close_session("s1", false).await.is_err());
        assert!(s.get_session("s1").await.unwrap().unwrap().closed_at.is_none());
        assert_eq!(s.participant_change("s1", "storage-error").await.unwrap().unwrap().status, "applying");
        sqlx::query("DROP TRIGGER refuse_fixture_restore").execute(s.pool()).await.unwrap();
        s.close_session("s1", false).await.unwrap();
        assert_eq!(s.participants_for_session("s1").await.unwrap(), before);
    }

    #[tokio::test]
    async fn recovery_rolls_back_interrupted_work_and_never_overwrites_a_newer_selection() {
        let (s, before) = fixture().await;
        claim(&s, "interrupted", switch(&before[0])).await;
        s.prepare_participant_change("s1", "interrupted").await.unwrap();
        let row = s.recover_interrupted_participant_change("s1").await.unwrap().unwrap();
        assert_eq!(row.status, "failed");
        assert_eq!(s.participants_for_session("s1").await.unwrap(), before);
        assert!(s.recover_interrupted_participant_change("s1").await.unwrap().is_none());
        claim(&s, "conflict", switch(&before[0])).await;
        s.prepare_participant_change("s1", "conflict").await.unwrap();
        s.set_participant_model(before[0].id, Some("outside-edit")).await.unwrap();
        assert!(s.recover_interrupted_participant_change("s1").await.is_err());
        assert_eq!(s.participants_for_session("s1").await.unwrap()[0].model_id.as_deref(), Some("outside-edit"));
        assert_eq!(s.participant_change("s1", "conflict").await.unwrap().unwrap().status, "applying", "a failed recovery remains visible, not falsely completed");
    }

    #[tokio::test]
    async fn preparation_rechecks_selections_and_rolls_back_the_whole_transaction_on_error() {
        let (s, before) = fixture().await;
        claim(&s, "changed", switch(&before[0])).await;
        sqlx::query("UPDATE models SET context_window = 42 WHERE id = 'new'").execute(s.pool()).await.unwrap();
        assert!(s.prepare_participant_change("s1", "changed").await.is_err());
        assert_eq!(s.participants_for_session("s1").await.unwrap(), before);
        s.finish_participant_change("s1", "changed", Some("selection changed")).await.unwrap();
        // Oversized old spawn metadata must not leave the new model half-set.
        sqlx::query("UPDATE session_participants SET account_dir_at_spawn = ? WHERE id = ?")
            .bind("x".repeat(256 * 1024)).bind(before[0].id).execute(s.pool()).await.unwrap();
        claim(&s, "large", switch(&before[0])).await;
        let revision = s.roster_revision("s1").await.unwrap();
        assert!(s.prepare_participant_change("s1", "large").await.err().unwrap().to_string().contains("256 KiB"));
        assert_eq!(s.roster_revision("s1").await.unwrap(), revision);
        assert_eq!(s.participants_for_session("s1").await.unwrap()[0].model_id.as_deref(), Some("old"));
        assert!(s.participant_change("s1", "large").await.unwrap().unwrap().recovery_json.is_none());
    }
}
