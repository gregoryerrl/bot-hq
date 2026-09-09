-- 0002_session_spawn_models.sql — record which model each session's agents
-- were spawned with. Frozen at spawn time; the chat header reads these so
-- agents-as-talking-models is honest even after a config swap.

ALTER TABLE sessions ADD COLUMN brian_model_at_spawn TEXT;
ALTER TABLE sessions ADD COLUMN rain_model_at_spawn TEXT;
