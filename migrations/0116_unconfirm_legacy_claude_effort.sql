-- Earlier Claude knobs were resolved requests, not process observations. Keep
-- those values for history; only a new actual readback may confirm them.
UPDATE session_participants SET spawn_knobs_recorded = 0 WHERE runtime = 'claude_code';
