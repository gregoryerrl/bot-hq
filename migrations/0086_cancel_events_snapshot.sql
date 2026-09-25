-- 0086_cancel_events_snapshot.sql — what the turn was doing when Pause was
-- pressed (feedback #44(2), 2026-09-25).
--
-- A user pressed Pause three times in one session because work had visibly
-- stopped, and afterwards nothing could say whether the turn was hung in a
-- tool, waiting on the model, or stuck in the ring. These columns are read AT
-- THE PRESS, before the interrupt: once the interrupted turn ends, its clock
-- and tool record are cleared.
--
-- NULL on every row written before this migration, and on a press with no
-- participant mid-turn.
ALTER TABLE cancel_events ADD COLUMN holder TEXT;            -- the busy participant's slug
ALTER TABLE cancel_events ADD COLUMN turn_age_ms INTEGER;    -- how long its turn had run
ALTER TABLE cancel_events ADD COLUMN tools_in_flight INTEGER; -- tool calls not yet returned
ALTER TABLE cancel_events ADD COLUMN last_tool TEXT;         -- the turn's latest tool call
ALTER TABLE cancel_events ADD COLUMN last_tool_age_ms INTEGER; -- since that call started
ALTER TABLE cancel_events ADD COLUMN last_event_age_ms INTEGER; -- since the agent's last event
