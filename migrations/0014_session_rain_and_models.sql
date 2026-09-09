-- 0014_session_rain_and_models.sql — per-session Rain toggle + model selection.
--
-- `rain_enabled` lets a session run solo-Brian (MVP-2 is enabled by default but
-- can be disabled to save credits). Default 1 = duo, the existing behavior, so
-- every pre-existing session keeps spawning Rain. A dedicated column (not
-- "rain_model_id IS NULL") so "Rain off" is never confused with "Rain on, model
-- unset / resume failed".
--
-- `brian_model_id` / `rain_model_id` reference `models.id` (the saved-model
-- registry). NULL = fall back to the per-agent config (legacy sessions). The
-- create dialog defaults each to `app_settings.default_model_id`.

ALTER TABLE sessions ADD COLUMN rain_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE sessions ADD COLUMN brian_model_id TEXT;
ALTER TABLE sessions ADD COLUMN rain_model_id TEXT;
