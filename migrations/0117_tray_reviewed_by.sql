-- The reviewer's read as data on the card (feedback #143): who read the queued
-- publish, beside when (`reviewed_at`). Set with `reviewed_at` on a completed
-- read, cleared with it on every revision; NULL for a skip and for rows that
-- predate this column — those are not called unreviewed, their provenance is
-- simply not recorded.
ALTER TABLE session_tray ADD COLUMN reviewed_by TEXT;
