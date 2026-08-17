//! `messages` table: append-only per-session turn log.

use super::*;

/// Column list for a `Message` row — shared across the `messages_for_session`
/// query branches so the projection can't drift between them.
const MESSAGE_COLUMNS: &str = "id, session_id, author, kind, content, created_at";

/// The watermark read (`messages_for_session`) — one statement for both the
/// "everything" and the "since id" shapes. NOT the `?2 IS NULL OR id > ?2`
/// idiom `channel_page` uses for its optional filter: SQLite cannot push a
/// disjunction into an index range, so that form dropped the `id > ?` seek and
/// walked the session's whole history on every emitter flush (review, round
/// 8). `COALESCE(?2, -1)` keeps the seek — `-1` is below every id (`INTEGER
/// PRIMARY KEY`, positive). A test EXPLAINs this exact string.
fn messages_since_sql() -> String {
    format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages \
         WHERE session_id = ?1 AND id > COALESCE(?2, -1) ORDER BY id ASC"
    )
}

impl Storage {
    /// The user-row write path — a `MessageKind` + content shape — expressed as
    /// a thin wrapper over
    /// [`Storage::post_to_channel`]. It owns no SQL of its own, so there is
    /// exactly ONE insert into `messages` and exactly one place a
    /// [`PersistedMessage`] can be minted.
    ///
    /// **Why the two paths converged (B5 Task 1b).** They were coherent while
    /// this recorded an agent's *output* and delivery was a separate act: an
    /// output row needed no receipt. B5 collapses that — an agent's output row
    /// IS the row its peer reads through its cursor, one row, and delivery is
    /// receipt-gated. A second receipt-less insert would have forced the send
    /// path to either write a *second* row for one logical message (two rows in
    /// the channel for one utterance) or re-read the row to synthesise a
    /// receipt — a SELECT per chunk, and forgery-by-reconstruction back on the
    /// table.
    ///
    /// **It writes `origin = "user"` unconditionally, and that is the whole
    /// contract now — which is why the name says USER.** Two host rows (the
    /// idle watchdog's chat notice, every phase-change notice) once came
    /// through here under the old name `insert_message` and reached every
    /// participant tagged `[user]`; host rows go through
    /// `core::post_system_notice`, and this writer is for text the user typed
    /// (or answered — the out-of-band tray replay is the user's own words).
    ///
    /// **No production caller today, on purpose** — the user's typed rows and
    /// the tray replay both go through `post_to_channel` directly because they
    /// carry an envelope. It stays as the NAMED user-row writer so a host row
    /// cannot borrow it by picking the obvious-looking helper (which is exactly
    /// how the two rows above went wrong); a dead-code sweep that reads
    /// "zero production callers" as "delete" would remove the rail, not the
    /// dead weight. It used to take an `Author` and map it to
    /// `(origin, participant_slug)`; every production caller passed
    /// `Author::User`, so the participant arm was reachable only from tests, and
    /// the enum could not name a single participant this app creates. Deleted in
    /// the D10 retirement. Participant rows go through
    /// [`Storage::post_to_channel`] with their own slug — which is what the
    /// production paths already did.
    ///
    /// **What changed and what did not.** `author`, `origin` and the RFC3339-Z
    /// timestamp are written with the same values as before. `participant_id`
    /// is written with the same values but by a DIFFERENT SHAPE, and the shape
    /// changed for every user row, not just a hypothetical one: the old SQL ran
    /// its subquery unconditionally, so an `Author::User` row looked up
    /// `slug = 'user'` on every insert and got NULL back because no such row
    /// existed. The new form does not look at all — `origin = 'user'` fails the
    /// CASE guard. The two therefore agree on every database reachable today,
    /// and would disagree only if some `session_participants` row carried
    /// `slug = 'user'`: the old form would attribute the user's own message to
    /// it, the new form still writes NULL. Nothing creates such a row and no
    /// CHECK forbids one, so this is a real if currently unreachable
    /// difference, and the new behaviour is the defensible one.
    ///
    /// `every_legacy_message_query_still_works_after_0044` and
    /// `the_legacy_write_path_now_populates_the_new_columns` are the pins.
    ///
    /// `content` and `session_id` mirror [`Storage::post_to_channel`]'s own
    /// generics rather than narrowing them: this fires per chunk, the layer
    /// below can take ownership, and a narrower wrapper would force a body copy
    /// and a session-id allocation at a boundary where both sides could move.
    pub async fn insert_user_message(
        &self,
        session_id: impl Into<std::sync::Arc<str>>,
        kind: MessageKind,
        content: impl Into<String>,
    ) -> Result<PersistedMessage> {
        self.post_to_channel(session_id, "user", None, kind.as_str(), content, None)
            .await
    }

    /// True if any participant OTHER than `author` posted in `session_id` after
    /// `since` (an RFC3339-Z timestamp).
    ///
    /// What the findings re-raise guard asks: a reviewer only escalates once its peer
    /// has actually had a turn since the last raise. Scoped to
    /// `origin = 'participant'` so a host `system_notice` or the user's own reply
    /// cannot stand in for a peer turn.
    pub async fn has_message_from_other_participant_since(
        &self,
        session_id: &str,
        author: &str,
        since: &str,
    ) -> Result<bool> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages \
             WHERE session_id = ? AND origin = 'participant' AND author <> ? \
               AND created_at > ?)",
        )
        .bind(session_id)
        .bind(author)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("checking non-{author} participant messages since {since}"))?;
        Ok(exists != 0)
    }

    /// Count of user-authored TEXT rows for a session. Seeds the in-memory
    /// `SessionHandle.user_broadcasts` counter at spawn, so an app restart
    /// mid-task doesn't disarm the idle-unflagged watchdog until the next
    /// typed message (found by the d61d277 live smoke). Text-only on purpose:
    /// `phase_change` / `system_notice` rows are persisted as synthetic
    /// `author=user` and are host artifacts, not user engagement.
    pub async fn count_user_messages(&self, session_id: &str) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM messages \
             WHERE session_id = ? AND author = 'user' AND kind = 'text'",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("counting user messages for {session_id}"))?;
        Ok(n as u64)
    }

    /// All messages for the session, oldest first.
    /// If `since_id` is provided, returns only messages with id > since_id.
    pub async fn messages_for_session(
        &self,
        session_id: &str,
        since_id: Option<i64>,
    ) -> Result<Vec<Message>> {
        let rows = sqlx::query_as::<_, Message>(&messages_since_sql())
        .bind(session_id)
        .bind(since_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// The prose one participant wrote after `since_id`, joined oldest-first,
    /// with the id of the newest row read.
    ///
    /// Written for spin detection (B5 task 11), which needs "what did this
    /// participant just say" and gets no text on
    /// [`SequencerCommand::TurnComplete`](crate::core::sequencer::SequencerCommand).
    ///
    /// **`kind = 'text'` is the load-bearing filter, not a tidy-up.** A
    /// participant's turn also writes `tool_use` and `tool_result` rows — on the
    /// live database they outnumber its prose four to one — and those are
    /// near-identical across rounds *by construction*: the same tool called
    /// twice differs only in its arguments, so a token-set similarity over them
    /// reads as repetition on a participant that is working normally. The
    /// router's breaker compared forwarded prose; this is that same material.
    ///
    /// **Newest-capped, returned oldest-first.** `insert_user_message` fires per
    /// chunk, so one turn is many rows and an unbounded read would grow with the
    /// session. Taking the newest `limit` rather than the oldest keeps the
    /// returned id the participant's true high-water mark, so a turn longer than
    /// the cap cannot smear its remainder into the next comparison — the cost is
    /// that only the tail of such a turn is compared, which is the part a
    /// repeating participant repeats.
    ///
    /// `Ok(None)` means the participant has written no prose since `since_id`.
    /// That is not the same as "wrote something empty", and the caller leans on
    /// the difference: a turn that produced no text must leave a repetition
    /// streak standing rather than reset it (router inventory #13).
    pub async fn participant_text_since(
        &self,
        participant_id: i64,
        since_id: i64,
        limit: i64,
    ) -> Result<Option<(String, i64)>> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, content FROM messages \
             WHERE participant_id = ? AND kind = 'text' AND id > ? \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(participant_id)
        .bind(since_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("reading prose for participant {participant_id}"))?;

        let Some(&(newest, _)) = rows.first() else {
            return Ok(None);
        };
        let joined = rows
            .iter()
            .rev()
            .map(|(_, c)| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Some((joined, newest)))
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::{MessageKind, Storage};

    /// **The watermark read SEEKS** (round 8, review). `messages_for_session`
    /// is the emitter's per-flush read (50 ms / N=20 cadence) and the plugin
    /// polling read; a refactor to `(?2 IS NULL OR id > ?2)` kept one query
    /// string and silently dropped the `id > ?` range from the index seek —
    /// SQLite cannot push a disjunction into a range — so every flush walked
    /// the session's whole history. This pins the PLAN, not the rows: the
    /// query must use the session index with BOTH the equality and the range.
    #[tokio::test]
    async fn the_watermark_read_seeks_past_the_watermark() {
        let s = Storage::memory().await.unwrap();
        // The production string itself, not a copy of it.
        let sql = format!("EXPLAIN QUERY PLAN {}", super::messages_since_sql());
        let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(&sql)
            .bind("s1")
            .bind(Some(10_i64))
            .fetch_all(s.pool())
            .await
            .unwrap();
        let plan = rows.iter().map(|r| r.3.as_str()).collect::<Vec<_>>().join(" | ");
        assert!(
            plan.contains("session_id=?") && plan.contains("id>?"),
            "the read must seek on (session_id, id), got: {plan}"
        );
    }

    /// Pins the three decisions in [`Storage::participant_text_since`] that the
    /// sequencer's own spin tests cannot fail: the `kind` filter, the watermark
    /// being the newest PROSE row, and the cap taking the newest rows rather
    /// than the oldest. Both callers there post only prose, so deleting the
    /// filter leaves them green — measured, not assumed.
    ///
    /// Tool traffic is what the filter excludes, and on the live database it
    /// outnumbers participant prose four to one. `tool_use` and `tool_result`
    /// rows repeat almost exactly across rounds by construction — the same tool
    /// called twice differs only in its arguments — so a similarity test fed
    /// them reads as repetition on a participant that is working normally, and
    /// spin detection would halt the healthiest cycles first.
    #[tokio::test]
    async fn participant_text_since_reads_prose_and_ignores_tool_traffic() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "S", None).await.unwrap();
        // `create_session` seeds no roster — 0044 backfilled the sessions that
        // already existed, and every one created since is seeded here, pre-spawn.
        s.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
        let pid = s
            .participant_by_slug("s1", "hands")
            .await
            .unwrap()
            .expect("ensure_session_roster seeds brian")
            .id;

        let post = |kind: MessageKind, body: &'static str| {
            let s = &s;
            async move {
                s.post_to_channel("s1", "participant", Some("hands"), kind.as_str(), body, None)
                    .await
                    .unwrap();
            }
        };

        post(MessageKind::ToolUse, r#"{"name":"read","args":{}}"#).await;
        post(MessageKind::Text, "found the breaker").await;
        post(MessageKind::ToolResult, r#"{"output":"ok"}"#).await;
        post(MessageKind::Text, "and it keys off a token set").await;
        // AFTER the last prose row, so the watermark below cannot simply be
        // "the newest row this participant wrote".
        post(MessageKind::ToolUse, r#"{"name":"read","args":{}}"#).await;

        let (text, newest) = s
            .participant_text_since(pid, 0, 64)
            .await
            .unwrap()
            .expect("the participant wrote prose");
        assert_eq!(
            text, "found the breaker\nand it keys off a token set",
            "prose only, joined oldest-first"
        );

        assert!(
            s.participant_text_since(pid, newest, 64).await.unwrap().is_none(),
            "the watermark is the newest PROSE row — the tool call past it is not new prose"
        );

        let (tail, _) = s
            .participant_text_since(pid, 0, 1)
            .await
            .unwrap()
            .expect("capped, not empty");
        assert_eq!(
            tail, "and it keys off a token set",
            "the cap keeps the NEWEST rows: taking the oldest instead would leave a long \
             turn's remainder to smear into the next comparison"
        );
    }

    #[tokio::test]
    async fn count_user_messages_counts_text_only_and_per_session() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-1", "S", None).await.unwrap();
        s.create_session("s-2", "Other", None).await.unwrap();

        // Two real user prompts (incl. an OOB answer, stored the same way)…
        s.insert_user_message("s-1", MessageKind::Text, "task")
            .await
            .unwrap();
        s.insert_user_message("s-1", MessageKind::Text, "oob answer")
            .await
            .unwrap();
        // …plus host rows (`origin = "system"`, kind phase_change /
        // system_notice) that must NOT count…
        s.post_to_channel("s-1", "system", None, MessageKind::PhaseChange.as_str(), "Plan", None)
            .await
            .unwrap();
        s.post_to_channel("s-1", "system", None, MessageKind::SystemNotice.as_str(), "nudged", None)
            .await
            .unwrap();
        // …plus agent text and another session's user text (both excluded).
        s.post_to_channel("s-1", "participant", Some("hands"), MessageKind::Text.as_str(), "ack", None)
            .await
            .unwrap();
        s.insert_user_message("s-2", MessageKind::Text, "elsewhere")
            .await
            .unwrap();

        assert_eq!(s.count_user_messages("s-1").await.unwrap(), 2);
        assert_eq!(s.count_user_messages("s-2").await.unwrap(), 1);
        assert_eq!(s.count_user_messages("s-none").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn insert_message_still_writes_an_rfc3339_z_timestamp() {
        // Delegating to `post_to_channel` moved where `created_at` comes from,
        // and that method wrote SQLite's `datetime('now')` — a ZONE-LESS
        // `2026-08-06 11:22:33`. The regression would have been silent in both
        // directions: the frontend parses a zone-less string as LOCAL time (the
        // staleness hallucination `storage::time` exists to prevent), and
        // `has_message_from_other_participant_since` is a STRING compare
        // against an RFC3339-Z bound, so it would have gone permanently false
        // for a same-day bound and quietly disarmed the findings re-raise
        // turn-evidence guard.
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "S", None).await.unwrap();
        s.post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "work", None)
            .await
            .unwrap();

        let rows = s.messages_for_session("s1", None).await.unwrap();
        let ts = rows[0].created_at.clone();
        assert!(ts.contains('T') && ts.ends_with('Z'), "expected RFC3339-Z, got {ts}");
        chrono::DateTime::parse_from_rfc3339(&ts).expect("created_at must parse as RFC3339");

        // The comparison that actually breaks, derived from the row itself so
        // it needs no wall clock: midnight of the row's own day. ' ' (0x20)
        // sorts BEFORE 'T' (0x54), so a zone-less `created_at` reads as EARLIER
        // than midnight of the same day and this goes false.
        let midnight = format!("{}T00:00:00.000Z", &ts[..10]);
        assert!(
            s.has_message_from_other_participant_since("s1", "eyes", &midnight)
                .await
                .unwrap(),
            "a row written today must read as after today's midnight ({ts} vs {midnight})"
        );
    }

    #[tokio::test]
    async fn insert_message_returns_a_receipt_for_the_row_it_wrote() {
        // The point of the convergence: the duo pump's per-chunk write is now
        // receipt-bearing, so B5's receipt-gated send needs neither a second
        // row nor a re-read of the row to obtain one. Asserted against the
        // PERSISTED row rather than the arguments — a receipt that can diverge
        // from its row is exactly the lie the type exists to rule out.
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "S", None).await.unwrap();
        let pm = s
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "work", None)
            .await
            .unwrap();

        let rows = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(pm.message_id(), rows[0].id);
        assert_eq!(pm.session_id(), rows[0].session_id);
        assert_eq!(pm.body(), rows[0].content);
        assert_eq!(pm.envelope(), None, "the legacy shape carries no envelope");

        // A user row is the only arm now — `insert_user_message` writes
        // `origin = "user"` unconditionally — and it still returns
        // a receipt for its own row — there is no receipt-less variant to fall
        // through to, which is the point of deleting the id-only shim: it took
        // the same argument list, so swapping it in would have compiled
        // silently and dropped the receipt with no diagnostic.
        let user = s
            .insert_user_message("s1", MessageKind::Text, "reply")
            .await
            .unwrap();
        let rows = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(rows[1].id, user.message_id());
        assert_eq!(rows[1].author, "user");
        assert_eq!(user.body(), "reply");
    }
}
