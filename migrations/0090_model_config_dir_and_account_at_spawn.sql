-- 0090: a saved model may name the Claude config dir its subscription is
-- signed in under, and a participant records the dir it was spawned with.
--
-- `models.claude_config_dir` is passed to the child as `CLAUDE_CONFIG_DIR`.
-- claude-code keeps one credential per config dir (the default `~/.claude`
-- holds one account at a time), so two subscriptions can only run side by
-- side when each has its own dir and every participant is spawned into the
-- one its model row names. NULL or '' = the default dir, which is what every
-- participant was spawned with before this migration.
--
-- The three `session_participants` columns are a spawn-time snapshot, like
-- `effort_at_spawn` (0061): the dir is RECORDED at a participant's first spawn
-- and reused on every later spawn, because claude-code's session store lives
-- inside the config dir — a `--resume` in a different dir starts blank. The
-- model row is not consulted again, so editing a row's dir cannot restart a
-- live participant with no context. The email and organisation id are the
-- signed-in identity the CLI reported at spawn (`claude auth status --json`):
-- a dir is a credential slot, not an account — `/login` changes the account
-- behind the same dir and a resume crosses over without complaint (seen
-- 2026-10-06T13:07Z) — so the identity is kept to say when that happened.
--
-- Backfill: every row that has already spawned (`claude_session_id` set) was
-- spawned with no `CLAUDE_CONFIG_DIR`, so its dir is the default. Without
-- this a live session on a model row later pointed at another dir would
-- `--resume` there and lose its context. The identity stays NULL — the next
-- spawn records what the CLI reports, without a mismatch notice.
--
-- Nullable / defaulted so every existing INSERT keeps working (0044 lesson).
ALTER TABLE models ADD COLUMN claude_config_dir TEXT;
ALTER TABLE session_participants ADD COLUMN account_dir_at_spawn TEXT;
ALTER TABLE session_participants ADD COLUMN account_email_at_spawn TEXT;
ALTER TABLE session_participants ADD COLUMN account_org_at_spawn TEXT;
UPDATE session_participants SET account_dir_at_spawn = '' WHERE claude_session_id IS NOT NULL;
