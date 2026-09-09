-- 0010_session_tray.sql — rename session_questions → session_tray + add command_text.
--
-- The table outgrew "questions": it durably mirrors every awaiting-input tray
-- item — questions, approvals, action_gate gated commands, mark_awaiting_user
-- halts. Rename it to match the UI "tray".
--
-- `command_text` persists the command for an action_gate (ToolBlocklist)
-- approval so it can execute on approve at resolve time — even hours/days later
-- or after a restart, when the in-memory oneshot channel is gone. NULL for
-- plain questions / halts.
--
-- SQLite >= 3.25 rewrites the self-referencing `supersedes_id` foreign key to
-- point at the renamed table automatically. The partial pending-index is
-- dropped and recreated under the new name.

ALTER TABLE session_questions RENAME TO session_tray;
ALTER TABLE session_tray ADD COLUMN command_text TEXT;

DROP INDEX IF EXISTS idx_session_questions_pending;
CREATE INDEX idx_session_tray_pending
    ON session_tray(session_id, status)
    WHERE status = 'pending';
