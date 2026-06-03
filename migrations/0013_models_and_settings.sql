-- 0013_models_and_settings.sql — saved-model registry + app settings KV store.
--
-- `models` is a user-managed registry of LLM endpoints: a display name plus the
-- provider / model id / optional base_url + auth_token an agent spawns with.
-- New sessions pick a model for Brian and Rain (the create dialog defaults each
-- to `app_settings.default_model_id`, overridable per agent).
--
-- `app_settings` is a small key/value store. First keys:
--   default_model_id  — model new sessions use for Brian + Rain by default
--   rain_disabled_default — "1" if the create dialog pre-checks "disable Rain"
--
-- Timestamps are RFC3339-Z (see migration 0012 + storage::time::now_utc).

CREATE TABLE IF NOT EXISTS models (
    id           TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    provider     TEXT NOT NULL DEFAULT 'anthropic',
    model_name   TEXT NOT NULL,
    base_url     TEXT,
    auth_token   TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS app_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
