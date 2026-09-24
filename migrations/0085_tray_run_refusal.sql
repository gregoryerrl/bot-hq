-- 0085_tray_run_refusal.sql — an APPROVED gate that bot-hq then refused to run
-- (EYES finding 16629ec7, 2026-09-24).
--
-- 0084's approval-time re-check can refuse a user-approved command (its body
-- file changed or vanished after review). The row still reads `answered` +
-- Approve, so `gate_status` reported it as executed — "do not re-run it" — the
-- opposite of the truth. This column holds the refusal, so `gate_status` can
-- say "approved but NOT RUN". NULL on every row that ran (or was rejected).
ALTER TABLE session_tray ADD COLUMN run_refusal TEXT;
