-- 0091: a usage-limit mark per Claude account AND model, so the next session
-- to spawn onto an account that just reported a limit is told, in advance.
--
-- Keyed by the config dir, the organisation id the CLI reported for it at
-- spawn, and the model the participant ran: a dir is a credential slot, not
-- an account — `/login` swaps the account behind the same dir — so a mark
-- left by one account must not warn the next (`org_id` is '' when the CLI
-- could not say); and claude.ai limits are per MODEL (the user, 2026-10-06:
-- "I ran out of fable usage on account 1 but I still have other model usage
-- there"), so a Fable limit must not warn the same account's Opus rows, and
-- a clean Opus turn must not clear it.
--
-- ADVISORY, never a refusal and never a re-route (spec §8): after the Oct 1
-- weekly limit the same account kept working for ~3.5 days before its stated
-- reset (why was not measured; extra usage is the likely reason), so a spawn
-- that refused until `limited_until` would have blocked real work. The mark
-- is cleared by a completed non-error turn from any participant on that
-- account + model that STARTED after `marked_at`, or by the user.
-- `limited_until` is the parsed reset when the line carried one, else a
-- conservative fallback; `limited_text` keeps the CLI's own line.
CREATE TABLE account_marks (
    config_dir    TEXT NOT NULL,
    org_id        TEXT NOT NULL DEFAULT '',
    model_name    TEXT NOT NULL DEFAULT '',
    email         TEXT,
    limited_until TEXT,
    limited_text  TEXT NOT NULL,
    marked_at     TEXT NOT NULL,
    PRIMARY KEY (config_dir, org_id, model_name)
);
