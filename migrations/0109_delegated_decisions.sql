-- Set by the same pending->answered claim as the decision. NULL is human.
ALTER TABLE session_tray ADD COLUMN decision_actor_json TEXT
    CHECK (decision_actor_json IS NULL OR json_valid(decision_actor_json));
