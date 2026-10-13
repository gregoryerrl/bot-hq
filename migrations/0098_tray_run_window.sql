-- 0098: a gated command's RUN WINDOW (feedback #66a, the user's pick): the
-- user approves whenever they see the card; bot-hq runs the command at
-- `run_not_before` (or at once when approved inside the window), unattended,
-- and refuses it after `run_not_after`. The end is required and at most 24 h
-- after the park; a Cancel sits on the Tray tab until the scheduled run.
-- RFC3339-Z stamps, compared as text like every other cutoff. NULL = no
-- window: the command runs at approval, as it always did.
ALTER TABLE session_tray ADD COLUMN run_not_before TEXT;
ALTER TABLE session_tray ADD COLUMN run_not_after TEXT;
