-- Explicit delegated application control. No existing/default role is granted it.
CREATE TABLE application_control_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_session TEXT NOT NULL,
    source_participant INTEGER NOT NULL,
    source_slug TEXT NOT NULL,
    target_session TEXT,
    operation TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    outcome TEXT NOT NULL CHECK(outcome IN ('started', 'ok', 'failed')),
    detail TEXT
);
CREATE INDEX application_control_source ON application_control_audit(source_session, id DESC);
CREATE INDEX application_control_target ON application_control_audit(target_session, id DESC);
