-- 0017_remove_emma.sql — remove the Emma singleton agent from bot-hq.
--
-- Emma (the solo helper) was deleted from the codebase; this purges her
-- residual data so both fresh installs (which still run 0001's
-- `INSERT OR IGNORE` emma seed) and existing DBs end up Emma-free.
--
-- Order matters: `session_tray.session_id` carries a NO-ACTION FK to
-- `sessions(id)` (it survived the 0003→0010 rename), so its rows must go first
-- or the session delete would fail the FK. `messages` and `session_documents`
-- both cascade `ON DELETE`, so deleting the session row takes them with it.
-- `agent_configs` is independent.
--
-- The `author` / `agent_name` CHECK constraints still list 'emma' — left as-is.
-- They only validate INSERTs; nothing inserts 'emma' anymore, and dropping a
-- value from a SQLite CHECK needs a full table rebuild that isn't worth it.

DELETE FROM session_tray   WHERE session_id = 'emma';
DELETE FROM sessions       WHERE id = 'emma';
DELETE FROM agent_configs  WHERE agent_name = 'emma';
