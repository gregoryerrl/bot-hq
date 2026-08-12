-- ============================================================================
-- 0051 — context_readings: what every `result` event said about the window.
--
-- rc3 **P7** (docs/plans/2026-08-13-dogfood-queue.md). On 2026-08-12 a
-- participant died mid-session with `Prompt is too long` — the provider error
-- arrived as that participant's own next "message" — and there is NO record of
-- what its context meter showed beforehand, because `ContextUsage` was
-- forwarded to the UI and never written down. The failure could be watched
-- live and not diagnosed afterwards.
--
-- The table therefore records the RAW OPERANDS of every reading, including the
-- readings that are not usable:
--   * `used_tokens`      — point-in-time prompt size (`usage.*_tokens` summed),
--                          NULL when the event carried no `usage` object.
--   * `reported_window`  — `modelUsage[<model>].contextWindow` EXACTLY as the
--                          provider reported it, NULL when it reported none.
--   * `verdict`          — why the meter did or did not move.
--
-- A row is written for every `result` event, not only the usable ones, and
-- that is the point: with only usable rows, "the provider never sent a window"
-- and "the agent never finished a turn" are the same empty result. The open
-- question P7 asks to settle by MEASUREMENT — does `contextWindow` arrive at
-- all through a gateway model — is answerable only if the absences are rows.
--
-- Nothing is derived or filled in here: a missing operand stays NULL rather
-- than falling back to the model's configured `context_window`, which is a
-- number the user typed and which nothing in the runtime reads today. Whether
-- it SHOULD become a fallback is the decision this data exists to inform, and
-- writing it in as though it were measured would destroy the evidence.
--
-- Append-only. `ON DELETE CASCADE` so a session that is genuinely deleted does
-- not leave orphans; closing a session archives it and keeps its rows.
-- ============================================================================

CREATE TABLE IF NOT EXISTS context_readings (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    -- The participant's roster slug (`session_participants.slug`), which is
    -- also its `messages.author` string.
    participant_slug TEXT NOT NULL,
    -- The `modelUsage` key the operands were read from. NULL when no entry
    -- carried a usable window.
    model            TEXT,
    used_tokens      INTEGER,
    reported_window  INTEGER,
    -- 'usable' | 'no_window' | 'no_usage' | 'implausible_window'
    verdict          TEXT NOT NULL,
    created_at       TEXT NOT NULL
);

-- The two reads this table exists for: one participant's history, and a whole
-- session's, both newest-last.
CREATE INDEX IF NOT EXISTS idx_context_readings_session
    ON context_readings (session_id, id);
CREATE INDEX IF NOT EXISTS idx_context_readings_participant
    ON context_readings (session_id, participant_slug, id);
