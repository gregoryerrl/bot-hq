//! `roles` / `session_participants` / `participant_cursors` /
//! `participant_deliveries` — the session-focused model's persistence layer.
//!
//! Batch B3 of the redesign. Roles are user-owned templates; a participant is a
//! model plus an **invite-time snapshot** of one, so editing a role never
//! widens a live participant mid-turn. Cursors make delivery an auditable fact
//! instead of a side effect, and deliveries record what a policy withheld —
//! policies gate delivery, never persistence.
//!
//! **Schema note:** these tables ship in migration 0044, applied 2026-08-06
//! (see `docs/plans/2026-08-06-session-participants-runbook.md`). Its backfill
//! was a one-shot over the sessions that existed at apply time, so
//! [`Storage::ensure_session_roster`] is what keeps every session created since
//! from starting life with an empty roster.

use super::*;
use std::sync::Arc;

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

    /// Seed the default roster for a session that has none, returning how many
    /// participants were inserted (0 on the common path).
    ///
    /// 0044 backfilled `session_participants` from the paired `brian_*`/`rain_*`
    /// columns as a **one-shot over the rows that existed when it applied**.
    /// Nothing then created participants for a NEW session, so every message it
    /// wrote resolved `participant_id` to NULL forever — the dual-write in
    /// `insert_message` looks up the roster by slug. Called pre-spawn from
    /// `ensure_session_started`, which every creation path funnels through, so
    /// this both seeds new sessions and heals any left rosterless by that
    /// window.
    ///
    /// The two INSERTs mirror 0044's backfill statement-for-statement, scoped
    /// to one session: a seeded roster is then structurally identical to a
    /// backfilled one, which is what stops the two populations drifting.
    /// `OR IGNORE` rides `UNIQUE (session_id, slug)` for idempotence, so a
    /// healthy respawn pays two no-op inserts and nothing else.
    pub async fn ensure_session_roster(&self, session_id: &str) -> Result<u64> {
        let hands = sqlx::query(
            "INSERT OR IGNORE INTO session_participants \
             (session_id, slug, display_name, role_id, model_id, effort, ultracode, \
              claude_session_id, capabilities, participation_mode, turn_position, joined_at) \
             SELECT s.id, 'brian', 'Brian', \
                    (SELECT id FROM roles WHERE slug = 'hands'), \
                    COALESCE(s.brian_model_id, s.brian_model_at_spawn), \
                    s.brian_effort, s.brian_ultracode, s.brian_claude_session_id, \
                    (SELECT capabilities FROM roles WHERE slug = 'hands'), \
                    (SELECT participation_mode FROM roles WHERE slug = 'hands'), \
                    0, s.created_at \
             FROM sessions s WHERE s.id = ?",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("seeding HANDS participant for {session_id}"))?
        .rows_affected();
        // turn_position 1: the reviewer always sees the executor's work before
        // responding. A solo session keeps the row and disables it, exactly as
        // 0044 did for the 12 solo sessions it backfilled.
        let eyes = sqlx::query(
            "INSERT OR IGNORE INTO session_participants \
             (session_id, slug, display_name, role_id, model_id, effort, ultracode, \
              claude_session_id, capabilities, participation_mode, turn_position, \
              enabled, joined_at) \
             SELECT s.id, 'rain', 'Rain', \
                    (SELECT id FROM roles WHERE slug = 'eyes'), \
                    COALESCE(s.rain_model_id, s.rain_model_at_spawn), \
                    s.rain_effort, s.rain_ultracode, s.rain_claude_session_id, \
                    (SELECT capabilities FROM roles WHERE slug = 'eyes'), \
                    (SELECT participation_mode FROM roles WHERE slug = 'eyes'), \
                    1, s.rain_enabled, s.created_at \
             FROM sessions s WHERE s.id = ?",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("seeding EYES participant for {session_id}"))?
        .rows_affected();

        let inserted = hands + eyes;
        if inserted == 0 {
            return Ok(0);
        }
        // Every participant reads the channel, so every participant has a
        // cursor from birth — same invariant `insert_participant` holds.
        sqlx::query(
            "INSERT OR IGNORE INTO participant_cursors (participant_id) \
             SELECT id FROM session_participants WHERE session_id = ?",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .context("seeding cursors for the seeded roster")?;
        // Repair what the rosterless window wrote. Only reachable when this
        // call actually inserted, so a healthy spawn never runs it. Scoped to
        // `origin = 'participant'`: user/system rows have no participant by
        // design, and pre-0044 rows were already mapped by the migration.
        sqlx::query(
            "UPDATE messages SET participant_id = ( \
                 SELECT p.id FROM session_participants p \
                 WHERE p.session_id = messages.session_id AND p.slug = messages.author) \
             WHERE session_id = ? AND participant_id IS NULL AND origin = 'participant'",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("repairing unmapped messages in {session_id}"))?;
        Ok(inserted)
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

/// A receipt for one channel row, minted by — and only by — the INSERT in
/// [`Storage::post_to_channel`].
///
/// **This is the enabling half, not a closed gate.** Nothing consumes a
/// `PersistedMessage` yet, and all six of the paths that write a string
/// straight to an agent's stdin with no persisted row are still open, so what
/// those agents read remains invisible to the user. The type exists now so that
/// B5 Task 2 can change the send path to take one instead of a `&str`; at that
/// point "wire something that was never recorded" stops being a discipline and
/// becomes a compile error. Until Task 2 lands, that is the plan, not yet a
/// property of the system.
///
/// What IS true today: the value cannot be forged from outside. There is
/// exactly ONE construction site, immediately downstream of that INSERT, and
/// the fields are private to this module — which makes this file, `mod tests`
/// included, the trusted boundary. Keeping it to one construction site is a
/// maintainer's job, not something the compiler checks: a helper added here
/// later could mint a receipt with no row behind it. The claim now covers every
/// write to `messages`, not just this method: B5 Task 1b made
/// [`Storage::insert_message`] — the second live insert path, and the one the
/// duo pump uses on every chunk — a thin wrapper over `post_to_channel`, so
/// there is one INSERT and every row that reaches the table has a receipt
/// behind it.
///
/// `Clone` is deliberate. Fan-out hands one row to N agents by reference, so a
/// clone is never what reaches the wire; consuming by move would instead push
/// callers into re-posting the same text once per recipient.
///
/// The private fields are the enforcement, not a convention — forging one from
/// outside this module is rejected (`E0451`):
///
/// ```compile_fail
/// use bot_hq::storage::PersistedMessage;
///
/// // Nothing was inserted, so there is no row to be proof of. The struct
/// // literal cannot name the fields: they are private to
/// // `storage::participants`, which is the whole point of the type.
/// let forged = PersistedMessage {
///     message_id: 1,
///     session_id: "s1".into(),
///     body: "never persisted".to_string(),
///     envelope: None,
/// };
/// ```
///
/// Two ways that block can rot. It asserts only THAT the snippet fails, never
/// why — stable rustdoc ignores a `compile_fail,E0451` error code (verified: a
/// deliberately wrong code still passes), so re-check the reason by deleting
/// `compile_fail` and reading the real error. And a rename or a moved `pub use`
/// would leave it passing for the wrong reason; the companion doctest below
/// fails loudly if the path stops resolving.
///
/// ```
/// use bot_hq::storage::PersistedMessage;
/// fn _wire(_receipt: &PersistedMessage) {}
/// ```
#[derive(Debug, Clone)]
pub struct PersistedMessage {
    message_id: i64,
    /// The channel this receipt is valid for. A receipt without a scope is
    /// forgeable ACROSS sessions: after Task 2,
    /// `session_a_handle.send_to_all(receipt_from_session_b)` would compile and
    /// wire another session's text into these agents, with the row sitting in
    /// the wrong channel — the exact class of bug this type exists to rule out.
    ///
    /// `Arc<str>` rather than `String` to match `DuoConfig::session_id` and the
    /// `MessagePersisted` / `BatchEmitter` threading this will flow into.
    session_id: Arc<str>,
    body: String,
    envelope: Option<String>,
}

impl PersistedMessage {
    pub fn message_id(&self) -> i64 {
        self.message_id
    }

    /// The channel this receipt authorizes delivery into — see the field.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn envelope(&self) -> Option<&str> {
        self.envelope.as_deref()
    }
}

impl Storage {
    /// Post to the session channel — the write half of "the channel is the
    /// transport". Every wire into a participant goes through here, including
    /// host-authored injections (`origin = "system"`), which today are written
    /// straight to stdin and never recorded at all.
    ///
    /// Returns a [`PersistedMessage`] rather than a bare id. The receipt is
    /// meant to become the permission to wire the text: once B5 Task 2 changes
    /// the send path, it will be able to demand one, after which no caller can
    /// hand it a string that never became a row. Nothing demands one yet — see
    /// the type's own docs for what is and is not true today.
    ///
    /// Writes the legacy `author` column too, so readers that have not migrated
    /// yet keep working (migration revision 3). `system` rows are stored as
    /// `author = 'user'` because that is exactly how today's system notices are
    /// already persisted — legacy readers must not encounter a new author value.
    pub async fn post_to_channel(
        &self,
        session_id: impl Into<Arc<str>>,
        origin: &str,
        participant_slug: Option<&str>,
        kind: &str,
        content: impl Into<String>,
        envelope: Option<String>,
    ) -> Result<PersistedMessage> {
        // `impl Into<Arc<str>>` so a caller already holding one — Task 3's do,
        // from `DuoConfig::session_id` — passes a refcount bump instead of
        // deref-ing to `&str` and re-allocating. To be clear about the size of
        // the win: a ~36-byte copy next to a SQLite INSERT is noise. This is
        // consistency with the ownership argument above, not a perf claim, and
        // it is one line now versus a signature change once six call sites
        // exist.
        let session_id: Arc<str> = session_id.into();
        // Both taken by value and MOVED into the receipt at the bottom. This
        // fires per Text / ToolUse / ToolResult chunk and a tool result can
        // carry a whole file, so copying the body into the receipt would be a
        // full-body heap copy per post. `Arc<str>` would not help —
        // `Arc::from(&str)` allocates and copies exactly as `to_string()` does,
        // and delivery is by reference, so nothing ever clones the body.
        //
        // `content` is `impl Into<String>` so `&str` callers stay ergonomic and
        // `String` callers pay nothing. `envelope` is a plain `Option<String>`
        // on purpose: `Option<impl Into<String>>` cannot infer a bare `None`
        // (E0283), which would force a turbofish at every call site that omits
        // an envelope — and envelopes are small JSON metadata, so the hot-path
        // argument that motivates the generic on `content` does not apply.
        let content: String = content.into();
        let legacy_author = match origin {
            "participant" => participant_slug.unwrap_or("user"),
            // 'system' and 'user' both land as 'user' for legacy readers.
            _ => "user",
        };
        // The participant is resolved INLINE by subquery rather than by a prior
        // awaited SELECT. `insert_message` delegates here and fires on every
        // text/tool_use/tool_result chunk, so a separate round trip per post is
        // a cost worth not paying — and one that would be near-impossible to
        // attribute once the send path starts routing through this method.
        //
        // The `CASE` is the origin guard the old two-step form carried in its
        // `match`: only a participant origin resolves, so a user/system row
        // stays NULL even if handed a live slug, and SQLite skips the subquery
        // entirely for those. A slug with no roster row resolves to NULL rather
        // than erroring, which is correct — `author` still carries the
        // attribution, and an unattributed row beats a lost one.
        let id = sqlx::query(
            "INSERT INTO messages \
             (session_id, participant_id, origin, kind, content, envelope, author, created_at) \
             VALUES (?1, \
                     CASE WHEN ?2 = 'participant' THEN \
                          (SELECT id FROM session_participants \
                           WHERE session_id = ?1 AND slug = ?3) END, \
                     ?2, ?4, ?5, ?6, ?7, ?8)",
        )
        // Bind order is NOT column order — `?3` appears only inside the CASE,
        // so position no longer tells you the target. The binds below are, in
        // order: ?1 session_id, ?2 origin, ?3 participant_slug, ?4 kind,
        // ?5 content, ?6 envelope, ?7 author, ?8 created_at.
        .bind(&*session_id)
        .bind(origin)
        .bind(participant_slug)
        .bind(kind)
        .bind(content.as_str())
        .bind(envelope.as_deref())
        .bind(legacy_author)
        // `now_utc()` (RFC3339-Z), NOT SQLite's `datetime('now')`, which this
        // method used until `insert_message` began delegating here. That is the
        // project baseline every other write already keeps — see `storage::time`
        // — and the difference is not cosmetic. `datetime('now')` emits a
        // zone-less `2026-08-06 11:22:33`, which sorts BEFORE the same instant
        // in RFC3339 (space < 'T'), so `has_message_from_author_since` — a
        // string compare against an RFC3339-Z bound — would have gone
        // permanently false and silently disarmed the findings re-raise
        // turn-evidence guard. The frontend would also have read every new row
        // as local time: the staleness hallucination `now_utc` exists to stop.
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .with_context(|| format!("posting to channel for {session_id}"))?
        .last_insert_rowid();
        // The one and only place a `PersistedMessage` is minted, and it sits
        // downstream of the INSERT — the row is what the value proves. Every
        // field MOVES in; nothing is copied.
        Ok(PersistedMessage {
            message_id: id,
            session_id,
            body: content,
            envelope,
        })
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
            .unwrap()
            .message_id();
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
                                   Some(r#"{"phase":"Apply"}"#.to_string())).await.unwrap();

        // Rain has read nothing yet, so both are unread — including the message
        // she was not "forwarded". Context completeness is structural.
        let unread = s.unread_for_participant(r).await.unwrap();
        assert_eq!(unread.len(), 2);
        assert_eq!(unread[0].id, m1.message_id());
        assert_eq!(unread[0].origin, "user");
        assert_eq!(unread[1].id, m2.message_id());
        assert_eq!(unread[1].participant_id, Some(b), "attributed to its author");
        assert_eq!(
            unread[1].envelope.as_deref(),
            Some(r#"{"phase":"Apply"}"#),
            "the envelope is a visible field, not string mutation"
        );

        // After reading, the cursor advances and the backlog empties.
        s.advance_cursor(r, m2.message_id()).await.unwrap();
        assert!(s.unread_for_participant(r).await.unwrap().is_empty());

        // Brian, who never read, still has both — cursors are per participant.
        assert_eq!(s.unread_for_participant(b).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn post_to_channel_resolves_the_participant_by_slug() {
        // Pins the RESOLUTION RULE itself, so moving it out of a separate
        // awaited `participant_by_slug` SELECT and into the INSERT's own
        // subquery is provably behaviour-preserving: this passes on both sides
        // of that change. A test that only passed afterwards would be pinning
        // the refactor rather than the behaviour.
        //
        // Five cases, because each is a way the resolution can be wrong:
        // a known slug resolves; an unknown one degrades to NULL and STILL
        // writes the row (an agent's output vanishing is far worse than one
        // that is unattributed); resolution is scoped to the session, not
        // global; a non-participant origin never resolves at all; and
        // `("participant", None)` resolves to NULL.
        //
        // That last case is the one whose MECHANISM changed. The old form
        // matched it in Rust — `("participant", Some(slug))` was the only arm
        // that looked anything up, so a bare `None` fell to `_ => None`. The
        // new form rests on SQL three-valued logic instead: `slug = NULL` is
        // NULL, never true, so the subquery matches no row and yields NULL.
        // Same answer, different machinery, and nothing in the signature stops
        // a caller writing it — so it is pinned rather than left to be
        // re-derived.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.create_session("s2", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        s.ensure_session_roster("s2").await.unwrap();
        let brian1 = s.participant_by_slug("s1", "brian").await.unwrap().unwrap().id;
        let brian2 = s.participant_by_slug("s2", "brian").await.unwrap().unwrap().id;
        assert_ne!(brian1, brian2, "precondition: two rosters, two 'brian' rows");

        let known = s
            .post_to_channel("s1", "participant", Some("brian"), "text", "mine", None)
            .await
            .unwrap();
        let unknown = s
            .post_to_channel("s1", "participant", Some("nobody"), "text", "orphan", None)
            .await
            .unwrap();
        let system = s
            .post_to_channel("s1", "system", Some("brian"), "system_notice", "notice", None)
            .await
            .unwrap();
        let slugless = s
            .post_to_channel("s1", "participant", None, "text", "anonymous", None)
            .await
            .unwrap();
        let elsewhere = s
            .post_to_channel("s2", "participant", Some("brian"), "text", "hers", None)
            .await
            .unwrap();

        let rows = s.channel_after("s1", 0).await.unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].id, known.message_id());
        assert_eq!(rows[0].participant_id, Some(brian1), "a known slug resolves");
        assert_eq!(rows[1].id, unknown.message_id());
        assert_eq!(rows[1].participant_id, None, "an unknown slug resolves to NULL");
        assert_eq!(rows[1].content, "orphan", "and the row is written regardless");
        assert_eq!(rows[2].id, system.message_id());
        assert_eq!(
            rows[2].participant_id, None,
            "only a participant origin resolves, even handed a live slug"
        );
        assert_eq!(rows[3].id, slugless.message_id());
        assert_eq!(
            rows[3].participant_id, None,
            "a participant origin with no slug resolves to NULL — `slug = NULL` \
             is NULL, not a match"
        );
        assert_eq!(rows[3].content, "anonymous", "and that row is written too");

        let other = s.channel_after("s2", 0).await.unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].id, elsewhere.message_id());
        assert_eq!(
            other[0].participant_id,
            Some(brian2),
            "resolution is scoped to the posting session, not the slug globally"
        );
    }

    #[tokio::test]
    async fn the_legacy_write_path_now_populates_the_new_columns() {
        // B4a dual-write: `insert_message` keeps its signature and its `author`
        // column, and ALSO fills participant_id/origin. That means the channel
        // is correct from the moment 0044 applies — no backfill pass later, and
        // no flag day where attribution is half-migrated.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let b = s
            .insert_participant("s1", "brian", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();

        s.insert_message("s1", Author::Brian, MessageKind::Text, "work").await.unwrap();
        s.insert_message("s1", Author::User, MessageKind::Text, "reply").await.unwrap();

        let rows = s.channel_after("s1", 0).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].participant_id, Some(b), "resolved to the roster entry");
        assert_eq!(rows[0].origin, "participant");
        assert_eq!(rows[1].participant_id, None, "a user message has no participant");
        assert_eq!(rows[1].origin, "user");
    }

    #[tokio::test]
    async fn a_message_from_an_agent_with_no_roster_entry_still_writes() {
        // A session whose roster does not exist yet (created before its
        // participants are inserted) must not fail to log. The subquery
        // resolves to NULL and `author` still carries the attribution —
        // degraded, not broken. Getting this wrong would mean an agent's output
        // vanishing rather than being mis-attributed, which is far worse.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let id = s
            .insert_message("s1", Author::Rain, MessageKind::Text, "no roster yet")
            .await
            .unwrap()
            .message_id();
        assert!(id > 0, "logging must never depend on the roster existing");
        let rows = s.channel_after("s1", 0).await.unwrap();
        assert_eq!(rows[0].participant_id, None);
        assert_eq!(rows[0].origin, "participant");
        // The legacy path still attributes it correctly.
        let legacy = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(legacy[0].author, "rain");
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
        let pm = s
            .post_to_channel("s1", "system", None, "system_notice",
                             "[System: your previous turn was force-interrupted]", None)
            .await
            .unwrap();
        let rows = s.channel_after("s1", 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, pm.message_id());
        assert_eq!(rows[0].origin, "system");
        assert!(rows[0].participant_id.is_none());
        // And the legacy read path still sees it, unchanged.
        let legacy = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].author, "user", "legacy readers meet no new author value");
    }

    #[tokio::test]
    async fn a_new_session_gets_the_default_roster() {
        // 0044 backfilled only what existed when it applied. Without this,
        // every session created afterwards runs with an empty roster.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.set_session_spawn_config("s1", true, Some("opus"), Some("sonnet")).await.unwrap();

        assert_eq!(s.ensure_session_roster("s1").await.unwrap(), 2);

        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].slug, "brian");
        assert_eq!(roster[0].turn_position, 0, "HANDS acts first");
        assert_eq!(roster[0].model_id.as_deref(), Some("opus"), "model snapshotted off the row");
        assert!(roster[0].capabilities.contains("edit_files"));
        assert!(roster[0].enabled);
        assert_eq!(roster[1].slug, "rain");
        assert_eq!(roster[1].turn_position, 1);
        assert_eq!(roster[1].model_id.as_deref(), Some("sonnet"));
        assert!(!roster[1].capabilities.contains("edit_files"), "EYES stays read-only");
        // A participant without a cursor is invisibly undeliverable.
        for p in &roster {
            assert_eq!(s.cursor_for(p.id).await.unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn seeding_a_roster_twice_is_a_no_op() {
        // It runs pre-spawn on EVERY respawn, so non-idempotence would mean a
        // duplicate roster per restart.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        assert_eq!(s.ensure_session_roster("s1").await.unwrap(), 2);
        assert_eq!(s.ensure_session_roster("s1").await.unwrap(), 0, "second call inserts nothing");
        assert_eq!(s.participants_for_session("s1").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_solo_session_keeps_rain_disabled() {
        // Same shape 0044 gave the 12 solo sessions it backfilled: the row
        // exists (so promoting later is an UPDATE, not an invite) but is off.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.set_session_spawn_config("s1", false, None, None).await.unwrap();
        assert_eq!(s.ensure_session_roster("s1").await.unwrap(), 2);
        let roster = s.participants_for_session("s1").await.unwrap();
        assert!(roster[0].enabled, "HANDS runs");
        assert!(!roster[1].enabled, "EYES present but disabled");
        assert!(s.next_active_participant("s1", None).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn seeding_repairs_messages_written_before_the_roster() {
        // The live defect: a post-0044 session logged 60 messages with
        // participant_id NULL before anything created its roster. Seeding must
        // map them, or that history is permanently unattributed in the channel.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_message("s1", Author::Brian, MessageKind::Text, "work").await.unwrap();
        s.insert_message("s1", Author::Rain, MessageKind::Text, "review").await.unwrap();
        s.insert_message("s1", Author::User, MessageKind::Text, "reply").await.unwrap();
        let before = s.channel_after("s1", 0).await.unwrap();
        assert!(before.iter().all(|m| m.participant_id.is_none()), "precondition: unmapped");

        s.ensure_session_roster("s1").await.unwrap();

        let roster = s.participants_for_session("s1").await.unwrap();
        let after = s.channel_after("s1", 0).await.unwrap();
        assert_eq!(after[0].participant_id, Some(roster[0].id), "brian's row mapped");
        assert_eq!(after[1].participant_id, Some(roster[1].id), "rain's row mapped");
        assert_eq!(after[2].participant_id, None, "a user row has no participant");
        assert_eq!(after[2].origin, "user");
    }

    #[tokio::test]
    async fn insert_message_resolves_the_participant_once_the_roster_exists() {
        // Closes the loop with B4a's dual-write: seeded roster → the inline
        // subquery resolves, so nothing new accumulates unmapped.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        s.insert_message("s1", Author::Brian, MessageKind::Text, "work").await.unwrap();
        let roster = s.participants_for_session("s1").await.unwrap();
        let rows = s.channel_after("s1", 0).await.unwrap();
        assert_eq!(rows[0].participant_id, Some(roster[0].id));
        assert_eq!(rows[0].origin, "participant");
    }

    #[tokio::test]
    async fn a_persisted_message_carries_the_row_it_came_from() {
        // B5 Task 2 renders the wire FROM the receipt and never re-reads the
        // row, so the assertions below compare the receipt against the
        // PERSISTED row rather than against the arguments it was built from.
        // Checking it against the arguments would pass by construction; the
        // property that matters is that the two cannot diverge, because a
        // divergence makes the row a lie — the user's record and the agent's
        // actual input would silently disagree, which is the exact failure
        // this type exists to rule out.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        let pm = s
            .post_to_channel("s1", "participant", Some("brian"), "text", "work",
                             Some(r#"{"phase":"Apply"}"#.to_string()))
            .await
            .unwrap();
        assert!(pm.message_id() > 0, "a PersistedMessage is proof of a row");

        let rows = s.channel_after("s1", 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, pm.message_id());
        // Against the ROW, not the argument: Task 2's cross-session guard rests
        // on the receipt naming the channel the row actually landed in.
        assert_eq!(
            pm.session_id(),
            rows[0].session_id,
            "a receipt is scoped to the channel it was persisted into"
        );
        assert_eq!(pm.body(), rows[0].content, "receipt body IS the persisted body");
        assert_eq!(
            pm.envelope(),
            rows[0].envelope.as_deref(),
            "receipt envelope IS the persisted envelope"
        );
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
