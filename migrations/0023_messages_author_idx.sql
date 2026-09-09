-- Covers has_message_from_author_since(session_id, author, created_at) — the
-- eyes_flag re-raise turn guard (signaling/bridge/findings.rs). The existing
-- idx_messages_session_time(session_id, created_at) seeks session_id and ranges
-- created_at but leaves `author` a residual, row-by-row filter; on `messages` (the
-- one table that grows unbounded per session) this index makes all three terms
-- index-resolved. No IF NOT EXISTS / CONCURRENTLY: single-user SQLite, applied once.
CREATE INDEX idx_messages_session_author_time
    ON messages (session_id, author, created_at);
