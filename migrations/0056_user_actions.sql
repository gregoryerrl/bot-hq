-- The "waiting on you" ledger (s-761704e8 dissection, 2026-08-15).
--
-- The session recorded the user's own action items perfectly — "Merge all 5
-- myself now", the EOD send, the PR-create for the pushed follow-up branch —
-- and every one of them vanished from every SURFACE the moment its tray row
-- was answered and the session closed. tasks.md held the prose; nothing
-- showed it. It took a forensic dissection to reconstruct the user's own
-- checklist.
--
-- One row per action the USER owes, written by close_session's optional
-- user_actions argument. Surfaced on the dashboard until checked off.
-- UNIQUE(session_id, action): the close-out staleness sweep refuses the
-- FIRST close_session call, and the retry passes the same list — the
-- second insert must be a no-op, not a duplicate.
CREATE TABLE user_actions (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    action     TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    done_at    TEXT,
    UNIQUE (session_id, action)
);

CREATE INDEX idx_user_actions_open ON user_actions(done_at) WHERE done_at IS NULL;
