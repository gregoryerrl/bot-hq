-- Durable requests for session roster/model changes. Runtime execution is
-- separate: accepting or claiming a request does not mutate the live roster.
ALTER TABLE sessions ADD COLUMN roster_revision INTEGER NOT NULL DEFAULT 0
    CHECK (roster_revision >= 0);

CREATE TRIGGER participant_roster_insert AFTER INSERT ON session_participants
BEGIN
    UPDATE sessions SET roster_revision = roster_revision + 1 WHERE id = NEW.session_id;
END;
CREATE TRIGGER participant_roster_delete AFTER DELETE ON session_participants
BEGIN
    UPDATE sessions SET roster_revision = roster_revision + 1 WHERE id = OLD.session_id;
END;
CREATE TRIGGER participant_roster_update
AFTER UPDATE OF model_id, role_id, enabled, capabilities, participation_mode,
    turn_position, label, effort, ultracode, color ON session_participants
WHEN OLD.model_id IS NOT NEW.model_id OR OLD.role_id IS NOT NEW.role_id
    OR OLD.enabled IS NOT NEW.enabled OR OLD.capabilities IS NOT NEW.capabilities
    OR OLD.participation_mode IS NOT NEW.participation_mode
    OR OLD.turn_position IS NOT NEW.turn_position OR OLD.label IS NOT NEW.label
    OR OLD.effort IS NOT NEW.effort OR OLD.ultracode IS NOT NEW.ultracode
    OR OLD.color IS NOT NEW.color
BEGIN
    UPDATE sessions SET roster_revision = roster_revision + 1 WHERE id = NEW.session_id;
END;

CREATE TABLE session_participant_changes (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    expected_revision INTEGER NOT NULL CHECK (expected_revision >= 0),
    selection_fingerprint TEXT NOT NULL,
    request_json TEXT NOT NULL CHECK (json_valid(request_json)),
    origin_json TEXT NOT NULL CHECK (json_valid(origin_json)),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'applying', 'applied', 'failed', 'cancelled')),
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    error TEXT,
    -- A runtime worker must persist recovery data before changing a roster.
    recovery_json TEXT CHECK (recovery_json IS NULL OR json_valid(recovery_json))
);
CREATE UNIQUE INDEX one_open_participant_change ON session_participant_changes(session_id)
    WHERE status IN ('pending', 'applying');
CREATE INDEX participant_change_history ON session_participant_changes(session_id, created_at DESC, id);
