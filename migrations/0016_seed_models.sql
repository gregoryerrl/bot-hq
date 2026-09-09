-- 0016_seed_models.sql — seed the model registry.
--
-- The Agents tab is now dropdown-only (no per-agent free-text), so the registry
-- must be populated for agents to have a model to pick. Seed every distinct
-- claude-code Claude model id, plus migrate Rain's DeepSeek-V4-Pro credentials
-- out of her agent_config into a saved model.
--
-- created_at / updated_at fall to the table DEFAULT (RFC3339-Z, migration 0013).
-- INSERT OR IGNORE keeps it idempotent and never clobbers a user-edited row.

INSERT OR IGNORE INTO models (id, display_name, provider, model_name, base_url, auth_token)
VALUES
  ('claude-opus-4-8',   'Claude Opus 4.8',   'anthropic', 'claude-opus-4-8',   NULL, NULL),
  ('claude-opus-4-7',   'Claude Opus 4.7',   'anthropic', 'claude-opus-4-7',   NULL, NULL),
  ('claude-opus-4-6',   'Claude Opus 4.6',   'anthropic', 'claude-opus-4-6',   NULL, NULL),
  ('claude-opus-4-5',   'Claude Opus 4.5',   'anthropic', 'claude-opus-4-5',   NULL, NULL),
  ('claude-opus-4-1',   'Claude Opus 4.1',   'anthropic', 'claude-opus-4-1',   NULL, NULL),
  ('claude-opus-4',     'Claude Opus 4',     'anthropic', 'claude-opus-4',     NULL, NULL),
  ('claude-sonnet-4-6', 'Claude Sonnet 4.6', 'anthropic', 'claude-sonnet-4-6', NULL, NULL),
  ('claude-sonnet-4-5', 'Claude Sonnet 4.5', 'anthropic', 'claude-sonnet-4-5', NULL, NULL),
  ('claude-sonnet-4',   'Claude Sonnet 4',   'anthropic', 'claude-sonnet-4',   NULL, NULL),
  ('claude-sonnet-3-7', 'Claude Sonnet 3.7', 'anthropic', 'claude-sonnet-3-7', NULL, NULL),
  ('claude-haiku-4-5',  'Claude Haiku 4.5',  'anthropic', 'claude-haiku-4-5',  NULL, NULL),
  ('claude-haiku-4',    'Claude Haiku 4',    'anthropic', 'claude-haiku-4',    NULL, NULL),
  ('claude-haiku-3-5',  'Claude Haiku 3.5',  'anthropic', 'claude-haiku-3-5',  NULL, NULL);

-- Migrate Rain's DeepSeek credentials: copy model_name/base_url/auth_token from
-- her live agent_config (so the token is NOT hardcoded in this SQL). Only fires
-- when Rain has a custom gateway (base_url set) — a no-op on fresh installs.
INSERT OR IGNORE INTO models (id, display_name, provider, model_name, base_url, auth_token)
SELECT 'deepseek-v4-pro', 'DeepSeek V4 Pro', 'deepseek', model_name, base_url, auth_token
  FROM agent_configs
 WHERE agent_name = 'rain' AND base_url IS NOT NULL AND base_url != '';

-- Align Rain's provider label with the seeded model so the Agents-tab dropdown
-- pre-selects DeepSeek. provider is a label only (spawn uses model_name /
-- base_url / auth_token), so this is match-enabling, not a behavior change.
UPDATE agent_configs
   SET provider = 'deepseek'
 WHERE agent_name = 'rain' AND base_url IS NOT NULL AND base_url != '';
