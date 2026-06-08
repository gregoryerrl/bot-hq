-- 0018_session_effort_ultracode.sql — per-session effort + ultracode overrides.
--
-- Per-session effort/ultracode picks from the create dialog. NULL = inherit the
-- Settings → Claude Config defaults (persistent claude-overrides.json), then the
-- global effortLevel/env, then claude-code's own default. The effort columns
-- hold the level string (low/medium/high/xhigh/max); the ultracode columns are
-- nullable bool (NULL = inherit, 1 = on, 0 = explicitly off). Mirrors 0014's
-- per-session model-override pattern. ultracode only takes effect for Brian
-- (EYES gets no --settings, so its toggle is moot there).

ALTER TABLE sessions ADD COLUMN brian_effort TEXT;
ALTER TABLE sessions ADD COLUMN rain_effort TEXT;
ALTER TABLE sessions ADD COLUMN brian_ultracode INTEGER;
ALTER TABLE sessions ADD COLUMN rain_ultracode INTEGER;
