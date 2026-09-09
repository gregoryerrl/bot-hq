-- 0011_withdraw_orphaned_pending.sql — one-time backfill.
--
-- close_session used to leave a session's pending tray rows as `pending`
-- forever, so already-closed sessions accumulated dead pending rows. They're
-- harmless (the notifier's open-session filter hides them) but are cruft.
-- Withdraw them. Going forward, close_session withdraws a session's pending at
-- close time, so this only clears the pre-existing backlog (and on a fresh DB
-- it matches nothing).

UPDATE session_tray
   SET status = 'withdrawn'
 WHERE status = 'pending'
   AND session_id IN (SELECT id FROM sessions WHERE closed_at IS NOT NULL);
