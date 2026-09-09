-- 1.0.0 Batch 6 (M7c, tray e7caaf8e): the findings table's approval column was
-- named after the author's reviewer ROLE. The tool is `flag_finding` now
-- (`eyes_flag` accepted as an alias for live sessions); the column follows.
-- Plain RENAME COLUMN — INTEGER NOT NULL DEFAULT 0, no FK, no index on it
-- (verified in plan review).
ALTER TABLE findings RENAME COLUMN eyes_approved TO reviewer_approved;
