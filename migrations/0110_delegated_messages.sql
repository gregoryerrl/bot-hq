CREATE TABLE delegated_messages (
    source_session TEXT NOT NULL,
    source_participant INTEGER NOT NULL,
    request_id TEXT NOT NULL,
    target_session TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    -- Preserve the retry tombstone without blocking normal message retention.
    message_id INTEGER NOT NULL,
    PRIMARY KEY (source_session, source_participant, request_id)
);
