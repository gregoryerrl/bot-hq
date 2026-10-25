CREATE TABLE session_helper_batches (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    participant_id INTEGER NOT NULL REFERENCES session_participants(id),
    request_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    owner_incarnation TEXT NOT NULL,
    owner_epoch INTEGER NOT NULL,
    request_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('starting','running','cancelling','completed','cancelled','failed')),
    result_json TEXT,
    created_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE(session_id, participant_id, request_id)
);
CREATE UNIQUE INDEX one_active_helper_batch_per_parent
ON session_helper_batches(session_id,participant_id)
WHERE status IN ('starting','running','cancelling');
