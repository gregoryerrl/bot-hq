-- 0087_tray_run_result.sql — what an approved gate's run produced
-- (feedback #56 / #83 / #93, 2026-10-03).
--
-- `gate_status` could say "approved — executed, the output was delivered as an
-- out-of-band message" and nothing more: no exit code, no output, no time. The
-- output reaches the issuing agent only at its next turn, and a reviewer asked
-- to check a result had no way to fetch it. These three columns link a gate to
-- its run:
--
--   result_row_id  the `messages.id` of the delivery row that carries the
--                  output. The output is NOT copied here: that row is already
--                  redacted (F10), and a second copy would be a second place
--                  for a secret to sit.
--   exit_code      the command's exit code (124 when bot-hq timed it out).
--   ran_ms         how long the run took.
--
-- All NULL on a gate that has not run, was rejected, or ran before this
-- migration; `gate_status` says so for the last case.
ALTER TABLE session_tray ADD COLUMN result_row_id INTEGER;
ALTER TABLE session_tray ADD COLUMN exit_code INTEGER;
ALTER TABLE session_tray ADD COLUMN ran_ms INTEGER;
