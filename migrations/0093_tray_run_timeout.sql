-- 0093: the bound an approved gate's command runs under, in seconds, when the
-- agent asked for one (feedback #66b): `action_gate(timeout_secs)`, at most
-- 600. NULL means the default bound (`tool_gate::DEFAULT_TIMEOUT`, 120 s; a
-- late push re-run keeps its own 600 s). Read at approval time by the run,
-- so a gate parked with a bound keeps it however long the user takes.
ALTER TABLE session_tray ADD COLUMN run_timeout_secs INTEGER;
