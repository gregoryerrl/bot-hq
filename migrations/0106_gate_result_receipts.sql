-- A gate's latest recorded answer/output is independent of handoff age.
ALTER TABLE session_tray ADD COLUMN resolution_row_id INTEGER;
UPDATE session_tray SET resolution_row_id = result_row_id
 WHERE result_row_id IS NOT NULL;
CREATE INDEX idx_tray_resolution ON session_tray(session_id, resolution_row_id)
 WHERE resolution_row_id IS NOT NULL;

-- A tool-result notice is not a channel delivery. Separate columns preserve
-- that distinction; gate delivery proof outlives telemetry retention.
CREATE TABLE gate_result_notices (
    participant_id INTEGER NOT NULL REFERENCES session_participants(id) ON DELETE CASCADE,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    noticed_at TEXT,
    delivered_at TEXT,
    CHECK (noticed_at IS NOT NULL OR delivered_at IS NOT NULL),
    PRIMARY KEY (participant_id, message_id)
);
INSERT OR IGNORE INTO gate_result_notices (participant_id, message_id, delivered_at)
 SELECT d.participant_id, d.message_id, d.delivered_at
 FROM participant_deliveries d JOIN session_tray t ON t.resolution_row_id = d.message_id
 JOIN session_participants p ON p.id = d.participant_id AND p.session_id = t.session_id
 JOIN messages m ON m.id = d.message_id AND m.session_id = t.session_id
 WHERE d.delivered_at IS NOT NULL;
