-- 0007_session_claude_ids.sql — persist claude-code session UUIDs per agent
-- so a bot-hq session can be resumed across app restarts via
-- `claude --resume <uuid>`. One UUID per spawned subprocess, captured from
-- the stream-json `init` event in `core/duo.rs::pump_agent`.

ALTER TABLE sessions ADD COLUMN brian_claude_session_id TEXT;
ALTER TABLE sessions ADD COLUMN rain_claude_session_id TEXT;
