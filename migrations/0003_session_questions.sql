-- 0003_session_questions.sql — durable per-session question cache.
--
-- One row per question an agent surfaces to the user. Lifecycle:
--   pending  → asked, not yet answered
--   answered → user picked an option (or wrote a reply for open-ask)
--   withdrawn → agent abandoned the question (figured it out / no longer needed)
--   superseded → agent rephrased; supersedes_id points at the replacement
--
-- `kind` distinguishes the three surface types:
--   choice    → has options_json (JSON array of strings)
--   open_ask  → no options, free-text answer expected
--   halt      → mark_awaiting_user — informational halt, no user response needed
--
-- This table is the source of truth for the in-chat questions tray + the
-- dashboard card's `[Need User Input] · N` counter. The bridge's in-memory
-- `pending` map is the live oneshot::Sender channel for blocking-style
-- ask_user_choice; the table mirrors it durably so restarts don't lose state.

CREATE TABLE session_questions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT NOT NULL REFERENCES sessions(id),
    choice_id       TEXT NOT NULL UNIQUE,
    agent           TEXT NOT NULL,
    kind            TEXT NOT NULL,
    prompt          TEXT NOT NULL,
    options_json    TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    picked_option   TEXT,
    asked_at        TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    answered_at     TEXT,
    supersedes_id   INTEGER REFERENCES session_questions(id)
);

CREATE INDEX idx_session_questions_pending
    ON session_questions(session_id, status)
    WHERE status = 'pending';
