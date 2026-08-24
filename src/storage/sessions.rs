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

/// The ONE dashboard ordering, prefix-parameterized for the two SQL shapes
/// that must never disagree (1.0.0 Batch 3, tray c38a216b — the user:
/// "first create - first on list", plus drag-to-SWAP).
///
/// Last-activity DESC was the old order, and it is exactly why "the cards
/// switch all over the place": any session speaking re-sorted the grid.
/// `sort_key` (migration 0071, seeded from creation order, exchanged by
/// [`Storage::swap_session_order`]) is the order now; the NULL clause sinks a
/// key-less row to the END (SQLite would otherwise sort NULLs FIRST and a
/// stray unkeyed session would squat the top slot), where created-order
/// tiebreaks keep it stable.
///
/// Expressed ONCE because the review that scoped this batch found the old
/// ORDER BY duplicated across both strings with only one of them test-pinned —
/// drift between them was green. Both builders call this; the test pins both.
fn session_order_by(prefix: &str) -> String {
    format!(
        "ORDER BY ({prefix}sort_key IS NULL) ASC, {prefix}sort_key ASC, \
         {prefix}created_at ASC, {prefix}id ASC"
    )
}

/// The Dashboard's active-sessions read with the Quickview preview — a
/// function so a test can EXPLAIN the production string (see
/// [`Storage::list_active_sessions_with_preview`]).
fn list_active_sessions_with_preview_sql() -> String {
    let order = session_order_by("s.");
    format!(
        "SELECT s.*, substr(m.content, 1, 200) AS last_message, m.author AS last_author \
         FROM (SELECT {SESSION_COLUMNS}, sort_key FROM sessions \
               WHERE archived = 0 AND closed_at IS NULL) AS s \
         LEFT JOIN messages m ON m.id = \
             (SELECT MAX(m2.id) FROM messages m2 \
               WHERE m2.session_id = s.id AND m2.kind = 'text') \
         {order}"
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
        // sort_key = MAX+1: a new session takes the END of the user's
        // arrangement (Batch 3) — created-order by default, never displacing
        // an explicit swap. The subselect sees the pre-insert table, which is
        // exactly the "everything already there" the new row appends to.
        sqlx::query(
            "INSERT INTO sessions (id, title, working_repo_path, created_at, sort_key) \
             VALUES (?, ?, ?, ?, (SELECT COALESCE(MAX(sort_key), 0) + 1 FROM sessions))",
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
    /// Returns whether a row was actually written — `false` means the session
    /// is closed (or unknown) and the declare was REFUSED, which the caller
    /// must treat as "no halt exists", never as success (828147ad).
    pub async fn declare_session_halt(
        &self,
        session_id: &str,
        agent: &str,
        reason: &str,
    ) -> Result<bool> {
        // An ordinary halt replaces a temporary one's wake time too: one slot.
        // `closed_at IS NULL`: a CLOSED session refuses the declare (round 13).
        // The live specimen: teardown's kill reached the pump before the row
        // close, and the pump's died-mid-turn declaration stamped a ghost
        // "stopped mid-turn — send a message to respawn them" onto the closed
        // row (s-a73699ec, s-b1d2591b — every agent-initiated close of a busy
        // session). Teardown now closes the row before it kills; this
        // predicate is the half that holds even if a straggler declare lands
        // after that.
        sqlx::query(
            "UPDATE sessions SET halt_declared_by = ?, halt_reason = ?,              halt_declared_at = ?, halt_wake_at = NULL              WHERE id = ? AND closed_at IS NULL",
        )
        .bind(agent)
        .bind(reason)
        .bind(now_utc())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("declaring halt for session {session_id}"))
        .map(|r| r.rows_affected() > 0)
    }

    /// A TEMPORARY halt (round 12, migration 0069): the same slot, plus the
    /// RFC3339-Z instant the host wakes the declarer at. The banner counts
    /// down to `wake_at`; `clear_session_halt` clears it with the rest.
    /// Same `bool` contract as [`Self::declare_session_halt`].
    pub async fn declare_temporary_session_halt(
        &self,
        session_id: &str,
        agent: &str,
        reason: &str,
        wake_at: &str,
    ) -> Result<bool> {
        // Same closed-row refusal as `declare_session_halt` (round 13).
        sqlx::query(
            "UPDATE sessions SET halt_declared_by = ?, halt_reason = ?,              halt_declared_at = ?, halt_wake_at = ?              WHERE id = ? AND closed_at IS NULL",
        )
        .bind(agent)
        .bind(reason)
        .bind(now_utc())
        .bind(wake_at)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("declaring temporary halt for session {session_id}"))
        .map(|r| r.rows_affected() > 0)
    }

    /// The wake instant of the session's TEMPORARY halt, or `None` for no halt
    /// / an ordinary one.
    pub async fn session_halt_wake_at(&self, session_id: &str) -> Result<Option<String>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT halt_wake_at FROM sessions WHERE id = ?")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(w,)| w))
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
            "UPDATE sessions SET halt_declared_by = NULL, halt_reason = NULL,              halt_declared_at = NULL, halt_wake_at = NULL              WHERE id = ? AND halt_reason IS NOT NULL",
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
            // The WHERE above already filters closed rows; the bool is moot.
            let _ = self.declare_session_halt(
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
    /// filters on both) and **fills the halt slot with a system-declared halt**
    /// (1.0.0-readiness Batch 1, issues.md 2026-08-24 + dissect s-43567984 #1):
    /// a reopened session has no task yet, and the halt is what keeps it
    /// honest while the user types one — the idle watchdog gates on the slot
    /// (`watchdog::idle_unflagged_decision`), so the respawned roster is never
    /// nudged into "declare state", which is exactly the path that made a
    /// fresh agent re-close the session the user had just reopened. All FOUR
    /// halt columns are overwritten — a surviving `halt_wake_at` from the
    /// pre-close halt would be a timer that wakes the ring unbidden.
    ///
    /// `ipav_phase` resets with it: the restored phase chip was meaningless
    /// for the whole second half of the dissected session (a reopen almost
    /// always starts a NEW task). Tradeoff, stated: a reopen-to-continue loses
    /// the chip and the roster re-votes its way back in one boundary.
    ///
    /// Returns whether a row moved: `false` for an unknown or still-open
    /// session, which is what makes a double click harmless.
    ///
    /// The other half of the reopen — the announce row + roster respawn — is
    /// `AppState::reopen_session`, and it is the ONLY path that may spawn a
    /// roster for a row that was closed: `ensure_session_started` refuses closed
    /// rows since round 10, so viewing an archived session no longer revives
    /// its participants (four such rosters were alive at once on 2026-08-18,
    /// spawned by clicks through the Archive to copy session ids).
    pub async fn reopen_session(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE sessions SET closed_at = NULL, archived = 0, \
                 halt_declared_by = 'system', \
                 halt_reason = 'Session reopened — waiting for your prompt.', \
                 halt_declared_at = ?, halt_wake_at = NULL, \
                 ipav_phase = NULL \
             WHERE id = ? AND closed_at IS NOT NULL",
        )
        .bind(now_utc())
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("reopening session {id}"))?;
        Ok(res.rows_affected() > 0)
    }

    /// SWAP two sessions' dashboard positions — the user's literal pick
    /// (tray c38a216b: "Swap (literal exchange of two slots)", over both
    /// reviewers' move recommendation): dragging tile A onto tile B exchanges
    /// exactly those two `sort_key`s and nothing else shifts.
    ///
    /// One transaction. A NULL key (a row that somehow missed 0071's backfill
    /// and the create-time MAX+1) is assigned an end-of-list key first —
    /// backfill-on-touch — so the exchange is always between two real
    /// integers. Returns false when either id is unknown, leaving both rows
    /// untouched.
    pub async fn swap_session_order(&self, a: &str, b: &str) -> Result<bool> {
        if a == b {
            return Ok(false);
        }
        let mut tx = self.pool.begin().await?;
        let mut keys = Vec::with_capacity(2);
        for id in [a, b] {
            let row: Option<Option<i64>> =
                sqlx::query_scalar("SELECT sort_key FROM sessions WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some(key) = row else {
                // Unknown id: the tx drops unchanged.
                return Ok(false);
            };
            let key = match key {
                Some(k) => k,
                None => {
                    let assigned: i64 = sqlx::query_scalar(
                        "UPDATE sessions \
                         SET sort_key = (SELECT COALESCE(MAX(sort_key), 0) + 1 FROM sessions) \
                         WHERE id = ? RETURNING sort_key",
                    )
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
                    assigned
                }
            };
            keys.push((id, key));
        }
        for (id, new_key) in [(keys[0].0, keys[1].1), (keys[1].0, keys[0].1)] {
            sqlx::query("UPDATE sessions SET sort_key = ? WHERE id = ?")
                .bind(new_key)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// The session's persisted diff anchor (migration 0070). `None` until the
    /// first spawn over a git repo captures one.
    pub async fn session_start_sha(&self, id: &str) -> Result<Option<String>> {
        let row: Option<Option<String>> =
            sqlx::query_scalar("SELECT session_start_sha FROM sessions WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.flatten())
    }

    /// Persist the diff anchor, WRITE-ONCE (1.0.0 Batch 1, T7): the predicate
    /// keeps the FIRST spawn's capture — a reopen or restart re-running the
    /// spawn path cannot rebaseline the Apply-tab diff to a later HEAD. The
    /// bool says whether this call was the one that wrote it.
    pub async fn set_session_start_sha_if_absent(&self, id: &str, sha: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE sessions SET session_start_sha = ? \
             WHERE id = ? AND session_start_sha IS NULL",
        )
        .bind(sha)
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("persisting session_start_sha for {id}"))?;
        Ok(res.rows_affected() > 0)
    }

    /// Active sessions: not archived, not closed. Ordered by the USER'S
    /// arrangement — `sort_key` seeded from creation order, exchanged by
    /// [`Self::swap_session_order`] (Batch 3; see [`session_order_by`]). The
    /// old last-activity order made tiles trade places whenever any session
    /// spoke.
    pub async fn list_active_sessions(&self) -> Result<Vec<Session>> {
        let order = session_order_by("");
        let rows = sqlx::query_as::<_, Session>(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions \
             WHERE archived = 0 AND closed_at IS NULL \
             {order}"
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
    use super::{list_active_sessions_with_preview_sql, session_order_by};
    use crate::storage::{MessageKind, Storage};

    /// **A closed session refuses a halt declaration** (round 13). The ghost
    /// specimen: the close path's kill raced ahead of the row close, and the
    /// pump's died-mid-turn declaration stamped "stopped mid-turn — send a
    /// message to respawn them" onto an already-closing session's row — every
    /// agent-initiated close of a busy session carried it (s-a73699ec,
    /// s-b1d2591b). Both declare variants must no-op once `closed_at` is set;
    /// an OPEN session still takes both.
    #[tokio::test]
    async fn a_closed_session_refuses_both_halt_declarations() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s.declare_session_halt("s1", "hands", "live halt").await.unwrap();
        assert!(s.session_halt("s1").await.unwrap().is_some(), "open row takes it");
        s.clear_session_halt("s1").await.unwrap();

        s.close_session("s1", false).await.unwrap();
        assert!(
            !s.declare_session_halt("s1", "hands", "ghost").await.unwrap(),
            "the refusal is REPORTED, not a silent Ok (828147ad)"
        );
        assert!(
            s.session_halt("s1").await.unwrap().is_none(),
            "a closed row keeps no ghost halt"
        );
        assert!(
            !s.declare_temporary_session_halt("s1", "hands", "ghost", "2027-01-01T00:00:00Z")
                .await
                .unwrap()
        );
        assert!(s.session_halt("s1").await.unwrap().is_none());
        assert!(s.session_halt_wake_at("s1").await.unwrap().is_none());
    }

    /// **A closed session reopens on a button, INTO a system halt** (round 10
    /// B4 + 1.0.0 Batch 1). The storage half: `reopen_session` clears BOTH
    /// `closed_at` and `archived` (the dashboard's active filter is
    /// `archived = 0 AND closed_at IS NULL`), REPLACES the halt the session
    /// closed under with a system-declared "waiting for your prompt" halt —
    /// all four columns overwritten, so a pre-close TEMPORARY halt's timer
    /// cannot wake the reopened ring — resets the persisted IPAV phase, and
    /// is a no-op on an open or unknown row.
    #[tokio::test]
    async fn reopen_sets_a_system_halt_and_resets_the_phase() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        // The pre-close state this must NOT leak: an agent's TEMPORARY halt
        // (wake timer armed) and a persisted Verify phase.
        s.declare_temporary_session_halt("s1", "hands", "done for now", "2099-01-01T00:00:00Z")
            .await
            .unwrap();
        s.set_persisted_ipav_phase("s1", "verify").await.unwrap();
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
        let (by, reason, at) = s
            .session_halt("s1")
            .await
            .unwrap()
            .expect("a reopened session is HALTED — the watchdog gates on this slot");
        assert_eq!(by, "system", "the halt is the host's, not a ghost of the agent's");
        assert!(
            reason.contains("waiting for your prompt"),
            "the recap says what the session waits for: {reason}"
        );
        assert!(!at.is_empty(), "declared_at stamped");
        let wake: Option<String> =
            sqlx::query_scalar("SELECT halt_wake_at FROM sessions WHERE id = 's1'")
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert!(
            wake.is_none(),
            "the pre-close temporary halt's timer must not survive the reopen: {wake:?}"
        );
        assert_eq!(
            s.persisted_ipav_phase("s1").await.unwrap(),
            None,
            "the restored phase chip misled the whole second half of s-43567984 — a reopen resets it"
        );
        assert_eq!(
            s.list_active_sessions().await.unwrap().len(),
            1,
            "back on the dashboard"
        );

        // A second click, or a reopen of an open row, moves nothing — and the
        // no-op must not re-stamp the halt either.
        assert!(!s.reopen_session("s1").await.unwrap());
        assert!(!s.reopen_session("nope").await.unwrap());
    }

    /// **The diff anchor is write-once** (1.0.0 Batch 1, T7 — migration 0070).
    /// The first capture wins; a respawn's later HEAD cannot rebaseline it,
    /// and a reopen leaves it standing.
    #[tokio::test]
    async fn session_start_sha_is_write_once_and_survives_reopen() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        assert_eq!(s.session_start_sha("s1").await.unwrap(), None);
        assert!(s.set_session_start_sha_if_absent("s1", "aaa111").await.unwrap());
        assert!(
            !s.set_session_start_sha_if_absent("s1", "bbb222").await.unwrap(),
            "a second capture must not rebaseline the anchor"
        );
        assert_eq!(s.session_start_sha("s1").await.unwrap().as_deref(), Some("aaa111"));
        s.close_session("s1", false).await.unwrap();
        assert!(s.reopen_session("s1").await.unwrap());
        assert_eq!(
            s.session_start_sha("s1").await.unwrap().as_deref(),
            Some("aaa111"),
            "reopen keeps the original anchor — the whole session's work stays in the diff"
        );
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

    /// **The dashboard order is the user's arrangement, not activity**
    /// (1.0.0 Batch 3, ideas.md 2026-08-24 + tray c38a216b). Creation order
    /// seeds it; a message in an OLD session must not move its tile; SWAP
    /// exchanges exactly two slots. Asserted through BOTH SQL shapes — the
    /// preview path is the one the dashboard actually calls
    /// (`tauri_cmd/sessions.rs::list_sessions`), and the review that scoped
    /// this batch found the old order duplicated with only the other one
    /// pinned.
    #[tokio::test]
    async fn dashboard_order_is_the_users_arrangement_not_activity() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-1", "first", None).await.unwrap();
        s.create_session("s-2", "second", None).await.unwrap();
        s.create_session("s-3", "third", None).await.unwrap();
        // The OLD order's trigger: the oldest session speaks last.
        s.post_to_channel("s-1", "user", None, MessageKind::Text.as_str(), "hi", None)
            .await
            .unwrap();

        let plain: Vec<String> = s
            .list_active_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(plain, vec!["s-1", "s-2", "s-3"], "creation order, whatever spoke");
        let preview: Vec<String> = s
            .list_active_sessions_with_preview()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.session.id)
            .collect();
        assert_eq!(preview, vec!["s-1", "s-2", "s-3"], "the dashboard's own path agrees");

        // SWAP is a literal two-slot exchange: 1<->3, 2 untouched.
        assert!(s.swap_session_order("s-1", "s-3").await.unwrap());
        let swapped: Vec<String> = s
            .list_active_sessions_with_preview()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.session.id)
            .collect();
        assert_eq!(swapped, vec!["s-3", "s-2", "s-1"]);

        // Unknown id / self-swap: false, nothing moves.
        assert!(!s.swap_session_order("s-1", "s-nope").await.unwrap());
        assert!(!s.swap_session_order("s-1", "s-1").await.unwrap());
        let unchanged: Vec<String> = s
            .list_active_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(unchanged, vec!["s-3", "s-2", "s-1"]);
    }

    /// The two SQL builders carry the SAME order expression — the drift the
    /// scoping review caught (only one of the two old strings was pinned).
    #[test]
    fn both_session_lists_share_the_one_order_expression() {
        assert!(
            list_active_sessions_with_preview_sql().contains(&session_order_by("s.")),
            "the preview SQL orders by the shared expression"
        );
        // The FULL string, tiebreakers included (EYES C1): `created_at ASC,
        // id ASC` is what stops EQUAL sort_keys from reordering between
        // refetches — and equal keys are reachable (create_session computes
        // MAX+1 outside a transaction, so two concurrent creates can share
        // one). Pinning only the prefix would let the fix's own point drift.
        assert_eq!(
            session_order_by(""),
            "ORDER BY (sort_key IS NULL) ASC, sort_key ASC, created_at ASC, id ASC",
            "explicit arrangement first, key-less rows sink to the end, stable tiebreaks"
        );
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

    // `active_sessions_order_by_last_activity` was DELETED here on purpose
    // (1.0.0 Batch 3): it pinned the last-activity-DESC order the user
    // explicitly retired (ideas.md 2026-08-24: "Make the order permanent,
    // first create - first on list"; tray c38a216b picked swap on top). Its
    // successor is `dashboard_order_is_the_users_arrangement_not_activity`,
    // which asserts the SAME trigger (a message in an old session) now moves
    // NOTHING — a sanctioned behavior change with the user's decision cited,
    // not a green-up.

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

    /// Round 12 (migration 0069): a TEMPORARY halt carries its wake instant in
    /// the same slot; an ordinary halt declared over it drops the instant; the
    /// clear drops everything; the boot re-arm sees only open sessions.
    #[tokio::test]
    async fn a_temporary_halt_carries_a_wake_instant_in_the_slot() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s.create_session("s2", "t", None).await.unwrap();
        assert_eq!(s.session_halt_wake_at("s1").await.unwrap(), None);
        s.declare_temporary_session_halt("s1", "hands", "CI on #531", "2026-08-19T13:00:00.000Z")
            .await
            .unwrap();
        assert_eq!(
            s.session_halt_wake_at("s1").await.unwrap().as_deref(),
            Some("2026-08-19T13:00:00.000Z")
        );
        let (by, reason, _) = s.session_halt("s1").await.unwrap().unwrap();
        assert_eq!((by.as_str(), reason.as_str()), ("hands", "CI on #531"));
        // The wake instant is read per session, by the ring that comes up.
        s.declare_temporary_session_halt("s2", "eyes", "deploy", "2026-08-19T13:05:00.000Z")
            .await
            .unwrap();
        assert_eq!(
            s.session_halt_wake_at("s2").await.unwrap().as_deref(),
            Some("2026-08-19T13:05:00.000Z")
        );
        // An ordinary halt over it is one slot: the wake instant goes.
        s.declare_session_halt("s1", "hands", "plain halt").await.unwrap();
        assert_eq!(s.session_halt_wake_at("s1").await.unwrap(), None);
        s.declare_temporary_session_halt("s1", "hands", "again", "2026-08-19T14:00:00.000Z")
            .await
            .unwrap();
        assert!(s.clear_session_halt("s1").await.unwrap());
        assert_eq!(s.session_halt_wake_at("s1").await.unwrap(), None);
        assert!(s.session_halt("s1").await.unwrap().is_none());
    }
}
