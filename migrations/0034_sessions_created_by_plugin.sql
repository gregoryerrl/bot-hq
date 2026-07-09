-- Record which plugin created a session, if any. NULL = user/agent/driver-
-- created (the default and the overwhelming majority). Set only by the
-- `plugin_sessions` catalog capability's create arm, and read by the
-- ownership fence (`require_owned_session`) so a plugin can drive ONLY the
-- sessions it itself created — never the user's own sessions, never another
-- plugin's. Rows created before this migration stay NULL (unowned by any
-- plugin), which is correct.
ALTER TABLE sessions ADD COLUMN created_by_plugin TEXT;
