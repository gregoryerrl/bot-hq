-- 1.0.0 Batch 1 (T7, dissect s-43567984 #11): the Apply-tab diff anchor was
-- re-captured on every roster spawn — a reopen (or a plain restart) silently
-- rebaselined "diff since session start" to whatever HEAD was at that moment
-- (the live specimen: a mid-session staging merge became the anchor, and the
-- session's own earlier commits vanished from the diff). Persist the anchor on
-- the row, write-once: the spawn path reads it first and only captures+writes
-- when it is NULL.
ALTER TABLE sessions ADD COLUMN session_start_sha TEXT;
