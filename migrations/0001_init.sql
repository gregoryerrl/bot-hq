-- 0001_init.sql — initial schema for bot-hq rebuild
-- ARCHITECTURE.md "bot-hq.db (sqlite)" section is the source of truth.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS sessions (
    id                   TEXT PRIMARY KEY,
    title                TEXT NOT NULL,
    working_repo_path    TEXT,                                  -- NULL allowed for Emma's singleton
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    closed_at            TEXT,                                  -- NULL while session is live
    archived             INTEGER NOT NULL DEFAULT 0             -- bool 0/1
);

CREATE INDEX IF NOT EXISTS idx_sessions_active
    ON sessions (archived, closed_at);

CREATE TABLE IF NOT EXISTS messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    author      TEXT NOT NULL CHECK (author IN ('user', 'emma', 'brian', 'rain')),
    kind        TEXT NOT NULL,                                  -- 'text', 'tool_use', 'tool_result', 'phase_change', …
    content     TEXT NOT NULL,                                  -- raw text or JSON payload depending on `kind`
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_messages_session_time
    ON messages (session_id, created_at);

CREATE INDEX IF NOT EXISTS idx_messages_session_id
    ON messages (session_id, id);

CREATE TABLE IF NOT EXISTS agent_configs (
    agent_name   TEXT PRIMARY KEY CHECK (agent_name IN ('emma', 'brian', 'rain')),
    provider     TEXT NOT NULL DEFAULT 'anthropic',
    model_name   TEXT NOT NULL,
    base_url     TEXT,
    auth_token   TEXT,                                          -- plaintext for v1; see docs/decisions.md#auth-storage
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Emma singleton session row. Idempotent.
INSERT OR IGNORE INTO sessions (id, title, working_repo_path, created_at)
    VALUES ('emma', 'Emma', NULL, datetime('now'));

-- Default agent_configs rows so a brand-new install has sensible placeholders.
INSERT OR IGNORE INTO agent_configs (agent_name, provider, model_name)
    VALUES
        ('emma',  'anthropic', 'claude-haiku-4-5'),
        ('brian', 'anthropic', 'claude-opus-4-7'),
        ('rain',  'anthropic', 'claude-sonnet-4-6');
