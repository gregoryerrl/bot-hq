//! `sessions` table: lifecycle (create/get/close/list) + per-session spawn
//! metadata (frozen model names, claude-code resume UUIDs).

use super::*;

/// The `sessions` columns every `query_as::<_, Session>` SELECT must list, in
/// `Session` field order (also the flattened prefix for `SessionWithPreview`).
/// Centralized so adding a column is one edit, not four — a missing column
/// fails `query_as` at runtime, not compile time.
const SESSION_COLUMNS: &str = "id, title, working_repo_path, created_at, closed_at, \
    archived, slot0_model_at_spawn, slot1_model_at_spawn, base_repo_path, created_by_plugin, \
    (SELECT COUNT(*) > 1 FROM session_participants p \
     WHERE p.session_id = sessions.id AND p.enabled <> 0) AS multi_participant";

/// The Dashboard's active-sessions read with the Quickview preview — a
/// function so a test can EXPLAIN the production string (see
/// [`Storage::list_active_sessions_with_preview`]).
fn list_active_sessions_with_preview_sql() -> String {
    format!(
        "SELECT s.*, substr(m.content, 1, 200) AS last_message, m.author AS last_author \
         FROM (SELECT {SESSION_COLUMNS} FROM sessions \
               WHERE archived = 0 AND closed_at IS NULL) AS s \
         LEFT JOIN messages m ON m.id = \
             (SELECT MAX(m2.id) FROM messages m2 \
               WHERE m2.session_id = s.id AND m2.kind = 'text') \
         ORDER BY COALESCE(\
             (SELECT MAX(m3.created_at) FROM messages m3 WHERE m3.session_id = s.id), \
             s.created_at) DESC, \
             s.id ASC"
    )
}

impl Storage {
    pub async fn create_session(
        &self,
        id: &str,
        title: &str,
        working_repo_path: Option<&str>,
    ) -> Result<Session> {
        // Blank-but-present paths ('' from a repo-less project row) must store
        // as NULL: every consumer treats Some as "has a repo", and a phantom
        // path hard-errors action_gate / hook install. Migration 0019 repaired
        // pre-guard rows.
        let working_repo_path = working_repo_path.filter(|p| !p.trim().is_empty());
        sqlx::query(
            "INSERT INTO sessions (id, title, working_repo_path, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(title)
        .bind(working_repo_path)
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .with_context(|| format!("creating session {id}"))?;
        self.get_session(id)
            .await?
            .context("session row vanished immediately after insert")
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let row = sqlx::query_as::<_, Session>(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// **Declare the session's halt (rc3 D35).** One slot on the session
    /// itself, by construction — the user's "there can never be 2 halts",
    /// finally as schema rather than as a display rule. A later declaration
    /// replaces the earlier: the freshest recap is the one the user reads.
    /// Not remotely a tray row: the tray is for questions.
    pub async fn declare_session_halt(
        &self,
        session_id: &str,
        agent: &str,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET halt_declared_by = ?, halt_reason = ?,              halt_declared_at = ? WHERE id = ?",
        )
        .bind(agent)
        .bind(reason)
        .bind(now_utc())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("declaring halt for session {session_id}"))?;
        Ok(())
    }

    /// Persist (or clear) this session's staged message — the Stage slot
    /// (rc3 B1-F11).
    ///
    /// One slot per session, replace-on-restage, exactly like the halt. Kept
    /// OFF the `Session` row struct deliberately: that struct derives `FromRow`
    /// and is read by four SELECTs (three `query_as::<_, Session>` sites plus
    /// `SessionWithPreview`'s flattened prefix), all through `SESSION_COLUMNS`
    /// — so a new field is one edit there, but a widened row type still costs
    /// the generated TS bindings and every consumer. This column has two
    /// readers and one writer, so it pays for that nowhere.
    pub async fn set_staged_message(&self, session_id: &str, text: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE sessions SET staged_message = ? WHERE id = ?")
            .bind(text)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("staging a message for {session_id}"))?;
        Ok(())
    }

    /// The staged message, if this session has one — read at spawn so a relaunch
    /// mid-stage resumes with the user's words still in the slot.
    pub async fn staged_message(&self, session_id: &str) -> Result<Option<String>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT staged_message FROM sessions WHERE id = ?")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await
                .with_context(|| format!("reading the staged message for {session_id}"))?;
        Ok(row.and_then(|(text,)| text))
    }

    /// Clear the session's halt slot. Returns whether one was set — the
    /// caller's cue to tell the UI the state changed.
    pub async fn clear_session_halt(&self, session_id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE sessions SET halt_declared_by = NULL, halt_reason = NULL,              halt_declared_at = NULL WHERE id = ? AND halt_reason IS NOT NULL",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("clearing halt for session {session_id}"))?;
        Ok(res.rows_affected() > 0)
    }

    /// The session's declared halt, if any: `(declared_by, reason, declared_at)`.
    pub async fn session_halt(
        &self,
        session_id: &str,
    ) -> Result<Option<(String, String, String)>> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT halt_declared_by, halt_reason, halt_declared_at                  FROM sessions WHERE id = ?",
            )
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|(by, reason, at)| {
            Some((by?, reason?, at.unwrap_or_default()))
        }))
    }

    /// **The boot orphan sweep** (2026-08-15): a restart over a mid-turn
    /// session kills the turn without a stop — the busy map dies with the
    /// process, the box reopens bannerless, and the watchdog needs its whole
    /// grace period to notice (lived in `s-d6352684` when a relaunch landed
    /// 90 s into a turn). At startup, every open session whose LAST recorded
    /// activity state was `busy` or `cancelling` gets a host halt saying so —
    /// restarts land inside the every-stop-is-a-HALT model like everything
    /// else. A session already wearing a halt keeps its own recap (an agent's
    /// words beat the generic ones). Returns how many sessions were halted.
    pub async fn halt_orphaned_busy_sessions(&self) -> Result<usize> {
        let orphans: Vec<String> = sqlx::query_scalar(
            "SELECT s.id FROM sessions s \
             WHERE s.closed_at IS NULL AND s.archived = 0 \
               AND s.halt_reason IS NULL \
               AND (SELECT a.state FROM activity_events a \
                    WHERE a.session_id = s.id \
                    ORDER BY a.id DESC LIMIT 1) IN ('busy', 'cancelling')",
        )
        .fetch_all(&self.pool)
        .await
        .context("scanning for restart-orphaned sessions")?;
        for id in &orphans {
            self.declare_session_halt(
                id,
                "system",
                "The app restarted while a turn was in flight — that turn was \
                 lost, but the participants keep their memory. Send a message \
                 to resume where they left off.",
            )
            .await?;
        }
        Ok(orphans.len())
    }

    /// Rename a session. The live `SessionHandle.title` snapshot is NOT
    /// touched (it only feeds spawn-time logs); the UI re-reads the row.
    pub async fn rename_session(&self, id: &str, title: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("renaming session {id}"))?;
        Ok(())
    }

    /// Close the row. Also clears the turn holder: a closed session holds no
    /// turn, and 27 closed rows still named one on 2026-08-17 — an ending state
    /// that read like a session mid-turn (round 7).
    pub async fn close_session(&self, id: &str, archive: bool) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET closed_at = ?, archived = ?, \
                 current_turn_participant_id = NULL \
             WHERE id = ? AND closed_at IS NULL",
        )
        .bind(now_utc())
        .bind(if archive { 1 } else { 0 })
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("closing session {id}"))?;
        Ok(())
    }

    /// **Archive an already-closed row** (round 11). `close_session` applies
    /// `archive` only on the close itself (`WHERE closed_at IS NULL`), so a
    /// later "close and archive" that JOINS an in-flight epilogue — whose
    /// winner closed the row unarchived — has no way to honour the archive
    /// half without this. Returns whether a row moved (`false` for unknown or
    /// still-open).
    pub async fn archive_session(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE sessions SET archived = 1 WHERE id = ? AND closed_at IS NOT NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("archiving session {id}"))?;
        Ok(res.rows_affected() > 0)
    }

    /// **Reopen a closed row** (round 10, B4 — the user's pick: "a Reopen button
    /// for closed sessions"). Clears `closed_at` AND `archived` (the dashboard
    /// filters on both), and empties the halt slot — the halt a session closed
    /// under is not a stop the reopened session declared. Returns whether a row
    /// moved: `false` for an unknown or still-open session, which is what makes
    /// a double click harmless.
    ///
    /// The other half of the reopen — the roster respawn — is
    /// `AppState::reopen_session`, and it is the ONLY path that may spawn a
    /// roster for a row that was closed: `ensure_session_started` refuses closed
    /// rows since round 10, so viewing an archived session no longer revives
    /// its participants (four such rosters were alive at once on 2026-08-18,
    /// spawned by clicks through the Archive to copy session ids).
    pub async fn reopen_session(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE sessions SET closed_at = NULL, archived = 0, \
                 halt_declared_by = NULL, halt_reason = NULL, halt_declared_at = NULL \
             WHERE id = ? AND closed_at IS NOT NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("reopening session {id}"))?;
        Ok(res.rows_affected() > 0)
    }

    /// Active sessions: not archived, not closed. Ordered by LAST ACTIVITY —
    /// the newest message timestamp, falling back to `created_at` for
    /// message-less (just-created) sessions — so the dashboard surfaces the
    /// session you (or an agent) touched most recently first. `id ASC` is the
    /// tiebreaker so equal timestamps can't make tiles swap places between
    /// refreshes.
    pub async fn list_active_sessions(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query_as::<_, Session>(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions \
             WHERE archived = 0 AND closed_at IS NULL \
             ORDER BY COALESCE(\
                 (SELECT MAX(m.created_at) FROM messages m WHERE m.session_id = sessions.id), \
                 created_at) DESC, \
                 id ASC"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Like `list_active_sessions` but each row also carries a cheap preview of
    /// its latest `kind='text'` message (content capped at 200 chars + author),
    /// for the dashboard Quickview. Dashboard-only consumer (`list_sessions`),
    /// refetched on every (2.5 s-throttled) message batch while a session runs.
    ///
    /// ONE preview lookup per session (round 9): the newest text row is found
    /// once — `MAX(id)` over the `(session_id, id)` index — and joined by
    /// primary key, projecting both preview columns from that row. It used to
    /// be two byte-identical correlated subqueries (one per column), each an
    /// index walk that read full `messages` rows until it met a text row.
    /// `SESSION_COLUMNS` is unqualified and names `sessions.id` in its own
    /// subquery, so it stays inside a `FROM sessions` derived table and the
    /// join happens outside it.
    pub async fn list_active_sessions_with_preview(&self) -> Result<Vec<SessionWithPreview>> {
        let rows = sqlx::query_as::<_, SessionWithPreview>(&list_active_sessions_with_preview_sql())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Closed sessions (both just-closed and archived), most-recently-closed
    /// first. Surfaces in the Settings → Archive tab. `id ASC` tiebreaks equal
    /// `closed_at` values (`close_session` writes `now_utc()`, millisecond
    /// RFC3339-Z, so ties are rare but possible) for stable ordering.
    pub async fn list_closed_sessions(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query_as::<_, Session>(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions \
             WHERE closed_at IS NOT NULL \
             ORDER BY closed_at DESC, id ASC"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Record the user's main repo for a worktree-isolated session. Called at
    /// create time (before spawn) together with the worktree placement —
    /// `working_repo_path` then carries the worktree path and this column the
    /// repo the worktree was carved from. `None` clears (direct mode).
    pub async fn set_session_base_repo(
        &self,
        session_id: &str,
        base_repo_path: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE sessions SET base_repo_path = ? WHERE id = ?")
            .bind(base_repo_path)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording base repo on session {session_id}"))?;
        Ok(())
    }

    /// Stamp the plugin that created a session — set once by the
    /// `plugin_sessions` capability's create arm, immediately after the row
    /// exists and before the plugin learns the session id. Read by
    /// `require_owned_session` so a plugin can drive only sessions it created.
    pub async fn set_session_created_by(&self, session_id: &str, plugin_id: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET created_by_plugin = ? WHERE id = ?")
            .bind(plugin_id)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording creator plugin on session {session_id}"))?;
        Ok(())
    }

    /// Convert a worktree-isolated session to direct mode: point
    /// `working_repo_path` back at the base repo and clear `base_repo_path`.
    /// Used when the worktree can't be materialized at spawn — the row must
    /// follow the fallback or row-readers (action_gate) and the live session
    /// would disagree about where the session runs.
    pub async fn convert_session_to_direct(&self, session_id: &str, repo: &str) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET working_repo_path = ?, base_repo_path = NULL WHERE id = ?",
        )
        .bind(repo)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("converting session {session_id} to direct mode"))?;
        Ok(())
    }

    /// Record the model one TURN SLOT spawned with (slot 0 → `slot0_model_at_spawn`,
    /// slot 1 → `slot1_model_at_spawn`; migration 0060 renamed the pair).
    ///
    /// The session header reads those two columns, so they are still written —
    /// but positionally, off the roster's turn order, rather than off an agent
    /// name. Slots past 1 have nowhere to go and are simply not recorded; the
    /// live model for any participant is on its own row.
    pub async fn set_session_spawn_model_slot(
        &self,
        session_id: &str,
        slot: usize,
        model: &str,
    ) -> Result<()> {
        let column = match slot {
            0 => "slot0_model_at_spawn",
            1 => "slot1_model_at_spawn",
            _ => return Ok(()),
        };
        sqlx::query(&format!("UPDATE sessions SET {column} = ? WHERE id = ?"))
            .bind(model)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording the slot-{slot} spawn model on {session_id}"))?;
        Ok(())
    }

    /// NULL out one turn slot's spawn model — the header's "this session has no
    /// participant in that slot" state.
    pub async fn clear_session_spawn_model_slot(
        &self,
        session_id: &str,
        slot: usize,
    ) -> Result<()> {
        let column = match slot {
            0 => "slot0_model_at_spawn",
            1 => "slot1_model_at_spawn",
            _ => return Ok(()),
        };
        sqlx::query(&format!("UPDATE sessions SET {column} = NULL WHERE id = ?"))
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("clearing the slot-{slot} spawn model on {session_id}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    /// **A closed session reopens on a button, not on a view** (round 10, B4).
    /// The storage half: `reopen_session` clears BOTH `closed_at` and
    /// `archived` (the dashboard's active filter is `archived = 0 AND
    /// closed_at IS NULL`), empties the halt slot the session closed under, and
    /// is a no-op on an open or unknown row.
    #[tokio::test]
    async fn reopen_clears_closed_archived_and_the_halt_slot() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s.declare_session_halt("s1", "hands", "done for now").await.unwrap();
        // Close it archived, as the Archive tab's rows are.
        s.close_session("s1", true).await.unwrap();
        let row = s.get_session("s1").await.unwrap().unwrap();
        assert!(row.closed_at.is_some() && row.archived == 1);
        assert!(s.session_halt("s1").await.unwrap().is_some(), "the halt outlives the close");
        assert!(
            s.list_active_sessions().await.unwrap().is_empty(),
            "closed + archived: off the dashboard"
        );

        assert!(s.reopen_session("s1").await.unwrap(), "a closed row moves");
        let row = s.get_session("s1").await.unwrap().unwrap();
        assert!(row.closed_at.is_none(), "reopened: closed_at cleared");
        assert_eq!(row.archived, 0, "reopened: archived cleared too, or the dashboard still hides it");
        assert!(
            s.session_halt("s1").await.unwrap().is_none(),
            "the halt the session closed under is not the reopened session's stop"
        );
        assert_eq!(
            s.list_active_sessions().await.unwrap().len(),
            1,
            "back on the dashboard"
        );

        // A second click, or a reopen of an open row, moves nothing.
        assert!(!s.reopen_session("s1").await.unwrap());
        assert!(!s.reopen_session("nope").await.unwrap());
    }

    /// **A closed row can be archived after the fact** (round 11). The
    /// "close and archive" that joins an in-flight epilogue arrives after the
    /// winner closed the row unarchived; `close_session` cannot re-apply the
    /// flag (`closed_at IS NULL`), so this is the archive half of a join.
    /// A still-open row is not archived — closing is the other path's job.
    #[tokio::test]
    async fn archive_session_flags_a_closed_row_and_leaves_an_open_one() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        assert!(!s.archive_session("s1").await.unwrap(), "open: not archived here");
        assert_eq!(s.get_session("s1").await.unwrap().unwrap().archived, 0);
        s.close_session("s1", false).await.unwrap();
        assert!(s.archive_session("s1").await.unwrap(), "closed unarchived → archived");
        assert_eq!(s.get_session("s1").await.unwrap().unwrap().archived, 1);
        assert!(!s.archive_session("nope").await.unwrap());
    }

    /// **The staged message survives the process** (B1-F11, migration 0058).
    ///
    /// One slot, replace-on-restage, cleared by delivery or unstage — the halt's
    /// shape, for the same reason: the user gets one composed message pending at
    /// a time, and a second replaces it rather than queueing behind it.
    #[tokio::test]
    async fn a_staged_message_round_trips_and_replaces() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        assert_eq!(
            s.staged_message("s1").await.unwrap(),
            None,
            "a fresh session has nothing staged"
        );

        s.set_staged_message("s1", Some("first draft")).await.unwrap();
        assert_eq!(
            s.staged_message("s1").await.unwrap().as_deref(),
            Some("first draft")
        );

        // Re-staging replaces; it does not queue.
        s.set_staged_message("s1", Some("what they actually meant"))
            .await
            .unwrap();
        assert_eq!(
            s.staged_message("s1").await.unwrap().as_deref(),
            Some("what they actually meant")
        );

        // Delivery / unstage empties the slot.
        s.set_staged_message("s1", None).await.unwrap();
        assert_eq!(s.staged_message("s1").await.unwrap(), None);
    }

    /// The boot orphan sweep: a session whose last recorded state was busy
    /// gets the restart halt; idle, already-halted, and closed sessions are
    /// untouched — an agent's own recap is never overwritten by the generic.
    #[tokio::test]
    async fn boot_sweep_halts_only_restart_orphans() {
        let s = Storage::memory().await.unwrap();
        // Orphan: open, last state busy, no halt.
        s.create_session("s-orphan", "t", None).await.unwrap();
        s.insert_activity_event("s-orphan", "idle", false, false).await.unwrap();
        s.insert_activity_event("s-orphan", "busy", true, false).await.unwrap();
        // Clean: open but last state idle.
        s.create_session("s-idle", "t", None).await.unwrap();
        s.insert_activity_event("s-idle", "busy", true, false).await.unwrap();
        s.insert_activity_event("s-idle", "idle", false, false).await.unwrap();
        // Already declared: busy at kill, but an agent's halt is on the slot.
        s.create_session("s-declared", "t", None).await.unwrap();
        s.insert_activity_event("s-declared", "busy", true, false).await.unwrap();
        s.declare_session_halt("s-declared", "hands", "my own recap").await.unwrap();
        // Closed mid-busy: not open, not swept.
        s.create_session("s-closed", "t", None).await.unwrap();
        s.insert_activity_event("s-closed", "busy", true, false).await.unwrap();
        s.close_session("s-closed", false).await.unwrap();

        assert_eq!(s.halt_orphaned_busy_sessions().await.unwrap(), 1);
        let halt = s.session_halt("s-orphan").await.unwrap();
        assert!(
            halt.is_some_and(|(by, reason, _)| by == "system"
                && reason.contains("restarted while a turn was in flight")),
            "the orphan wears the restart halt"
        );
        assert!(s.session_halt("s-idle").await.unwrap().is_none());
        assert_eq!(
            s.session_halt("s-declared").await.unwrap().unwrap().1,
            "my own recap",
            "an agent's recap is never overwritten"
        );
        assert!(s.session_halt("s-closed").await.unwrap().is_none());
        // Idempotent: the swept orphan now wears a halt, so a second sweep
        // finds nothing.
        assert_eq!(s.halt_orphaned_busy_sessions().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn active_and_closed_lists_partition_sessions() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-a", "Active one", None).await.unwrap();
        s.create_session("s-b", "Closed one", None).await.unwrap();
        s.close_session("s-b", false).await.unwrap();
        s.create_session("s-c", "Archived one", None).await.unwrap();
        s.close_session("s-c", true).await.unwrap();

        // Active list: only the never-closed session.
        let active: Vec<String> = s
            .list_active_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(active, vec!["s-a"]);

        // Closed list: both the plain-closed and the archived session.
        let closed = s.list_closed_sessions().await.unwrap();
        let closed_ids: Vec<&str> = closed.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(closed.len(), 2);
        assert!(closed_ids.contains(&"s-b"));
        assert!(closed_ids.contains(&"s-c"));
        // Archived flag preserved so the UI can badge it.
        assert_eq!(closed.iter().find(|x| x.id == "s-c").unwrap().archived, 1);
        assert_eq!(closed.iter().find(|x| x.id == "s-b").unwrap().archived, 0);
    }

    /// A closed session holds no turn. `set_current_turn` is written by the ring
    /// at every handover and cleared by `halt`, but a session closed mid-turn
    /// kept its holder forever — 27 closed rows named one on 2026-08-17.
    #[tokio::test]
    async fn closing_a_session_clears_its_turn_holder() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-t", "mid-turn", None).await.unwrap();
        s.set_current_turn("s-t", Some(7)).await;
        let holder: (Option<i64>,) =
            sqlx::query_as("SELECT current_turn_participant_id FROM sessions WHERE id = ?")
                .bind("s-t")
                .fetch_one(&s.pool)
                .await
                .unwrap();
        assert_eq!(holder.0, Some(7), "the ring's write lands");
        s.close_session("s-t", false).await.unwrap();
        let holder: (Option<i64>,) =
            sqlx::query_as("SELECT current_turn_participant_id FROM sessions WHERE id = ?")
                .bind("s-t")
                .fetch_one(&s.pool)
                .await
                .unwrap();
        assert_eq!(holder.0, None, "closing clears the holder");
    }

    #[tokio::test]
    async fn active_sessions_order_by_last_activity() {
        use crate::storage::{MessageKind};
        let s = Storage::memory().await.unwrap();
        s.create_session("s-old", "Older", None).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        s.create_session("s-new", "Newer", None).await.unwrap();

        // No messages anywhere → creation order, newest first.
        let ids: Vec<String> = s
            .list_active_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(ids, vec!["s-new", "s-old"]);

        // Activity on the older session bumps it to the top.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        s.insert_user_message("s-old", MessageKind::Text, "hi")
            .await
            .unwrap();
        let ids: Vec<String> = s
            .list_active_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(ids, vec!["s-old", "s-new"]);
    }

    #[tokio::test]
    async fn preview_carries_latest_text_message() {
        use crate::storage::{MessageKind};
        let s = Storage::memory().await.unwrap();
        s.create_session("s-msg", "Has messages", None)
            .await
            .unwrap();
        s.create_session("s-empty", "No messages", None)
            .await
            .unwrap();

        // Newest TEXT message wins; a later tool_use must NOT shadow it.
        s.insert_user_message("s-msg", MessageKind::Text, "first prompt")
            .await
            .unwrap();
        s.post_to_channel("s-msg", "participant", Some("hands"), MessageKind::Text.as_str(), "hands reply", None)
            .await
            .unwrap();
        s.post_to_channel("s-msg", "participant", Some("hands"), MessageKind::ToolUse.as_str(), "{\"tool\":\"x\"}", None)
            .await
            .unwrap();

        let rows = s.list_active_sessions_with_preview().await.unwrap();
        let msg = rows.iter().find(|r| r.session.id == "s-msg").unwrap();
        assert_eq!(msg.last_message.as_deref(), Some("hands reply"));
        assert_eq!(msg.last_author.as_deref(), Some("hands"));

        // A session with no text messages → None preview, not an error.
        let empty = rows.iter().find(|r| r.session.id == "s-empty").unwrap();
        assert!(empty.last_message.is_none());
        assert!(empty.last_author.is_none());
    }

    /// The preview read finds the newest text row ONCE and by index — pinned
    /// over the production string (round 9): the join's `MAX(id)` subquery must
    /// seek the session index, and the row itself is fetched by primary key.
    #[tokio::test]
    async fn the_preview_read_finds_the_newest_text_row_once_by_index() {
        let s = Storage::memory().await.unwrap();
        let sql = format!("EXPLAIN QUERY PLAN {}", super::list_active_sessions_with_preview_sql());
        let rows: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(&sql).fetch_all(s.pool()).await.unwrap();
        let plan = rows.iter().map(|r| r.3.as_str()).collect::<Vec<_>>().join(" | ");
        let seeks = plan.matches("SEARCH m2 USING COVERING INDEX idx_messages_session_id").count()
            + plan.matches("SEARCH m2 USING INDEX idx_messages_session_id").count();
        assert_eq!(seeks, 1, "one MAX(id) seek for the preview row, got: {plan}");
        assert!(
            plan.contains("SEARCH m USING INTEGER PRIMARY KEY"),
            "the preview row is fetched by primary key, got: {plan}"
        );
    }

    #[tokio::test]
    async fn create_session_normalizes_blank_repo_path_to_null() {
        let s = Storage::memory().await.unwrap();
        let created = s.create_session("s-blank", "T", Some("")).await.unwrap();
        assert!(created.working_repo_path.is_none());
        let ws = s.create_session("s-ws", "T", Some("  ")).await.unwrap();
        assert!(ws.working_repo_path.is_none());
        // A real path still round-trips.
        let real = s
            .create_session("s-real", "T", Some("/tmp/repo"))
            .await
            .unwrap();
        assert_eq!(real.working_repo_path.as_deref(), Some("/tmp/repo"));
    }

    #[tokio::test]
    async fn migration_0017_purges_emma_seed() {
        // 0001 seeds an 'emma' session + agent_config; 0017 deletes both. A
        // freshly migrated DB must come up Emma-free.
        let s = Storage::memory().await.unwrap();
        assert!(s.get_session("emma").await.unwrap().is_none());
        assert!(s.get_agent_config("emma").await.unwrap().is_none());
    }
}
