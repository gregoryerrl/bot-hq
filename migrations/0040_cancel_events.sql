-- One row per Stop, so a cancel that "didn't hold" can be diagnosed afterwards.
--
-- Until now the entire cancel path wrote NOTHING durable: no message, no row, no
-- persisted event. The three lines that identify which branch ran are
-- `tracing::info!`/`warn!` and there is no log sink configured, so they went to a
-- stdout nobody captured. 21 Stops across 13 sessions left zero forensic trace,
-- which is why the user's "agents keep working after Stop" stayed anecdotal for
-- weeks — and why a plausible-but-wrong mechanism survived review.
--
-- Every column here exists to separate one candidate cause from another:
--   deferred_ms/deferral_capped  -> was the interrupt withheld while HANDS was
--                                   mid git-commit/push/migrate? (HANDS-only,
--                                   the current best fit for "mostly brian")
--   *_interrupt_queued           -> was the control_request even delivered, or
--                                   silently dropped by a full channel?
--   both_idle/idled_since_cancel -> did the agent actually stop?
--   cancel_superseded            -> did a user message arrive inside the window?
--   outcome                      -> which branch finally ran.
CREATE TABLE cancel_events (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id             TEXT NOT NULL,
    -- When the user hit Stop (start of the Tauri command), vs when the
    -- escalation decided. The gap is what the user experiences as "it kept
    -- working".
    pressed_at             TEXT NOT NULL,
    settled_at             TEXT NOT NULL,
    -- How long the atomic-op deferral polled before the interrupt was sent, and
    -- whether it gave up at the cap instead of the op finishing.
    deferred_ms            INTEGER NOT NULL DEFAULT 0,
    deferral_capped        INTEGER NOT NULL DEFAULT 0,
    -- 1 = control_request queued, 0 = dropped (full/closed channel),
    -- NULL = no such agent in this session.
    brian_interrupt_queued INTEGER,
    rain_interrupt_queued  INTEGER,
    -- State at the escalation deadline.
    both_idle              INTEGER NOT NULL,
    cancel_superseded      INTEGER NOT NULL,
    idled_since_cancel     INTEGER NOT NULL,
    -- 'honored' | 'superseded' | 'sigkill'
    outcome                TEXT NOT NULL
);

CREATE INDEX idx_cancel_events_session
    ON cancel_events (session_id, id DESC);
