-- Mirror `models.native` / `models.context_window` onto the per-agent row.
--
-- 0036/0037 put both columns on `models`, which covers a session created through
-- the dialog: it stores a `*_model_id` and `resolve_spawn_config` reads the model
-- row. Every OTHER path leaves those ids NULL on purpose — "Maintain CL"
-- (`dispatch_session`), the plugin proxy's `spawn_session`, and any driver
-- `create_session` that doesn't name a model — and falls back to `agent_configs`.
--
-- That fallback had no way to say "native", so assigning a native model to Rain on
-- the Agents tab produced a claude-code Rain forever, and the only working route
-- was re-picking the model in the create dialog every single session.
--
-- `agent_configs` is already a denormalized snapshot of a model's spawn-relevant
-- fields (provider / model_name / base_url / auth_token), so this completes an
-- existing pattern rather than introducing one. It inherits that pattern's
-- drift too: editing the model row later does not update the snapshot.
--
-- Defaults keep every existing row on claude-code, which is the safe direction.
ALTER TABLE agent_configs ADD COLUMN native INTEGER NOT NULL DEFAULT 0;

-- NULL = unknown, exactly as on `models`: an unknown window renders as a visible
-- gap and leaves the native loop's context ceiling dark. Never a guessed number.
ALTER TABLE agent_configs ADD COLUMN context_window INTEGER;
