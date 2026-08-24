-- 1.0.0 Batch 3 (ideas.md 2026-08-24, tray c38a216b): the dashboard ordered by
-- last activity, so tiles swapped places whenever any session spoke — the user:
-- "i don't like the cards switching all over the place. Make the order
-- permanent, first create - first on list", plus drag-to-SWAP on top. The
-- explicit key is the order; creation order seeds it.
--
-- The backfill is a correlated subquery ON PURPOSE: a bare
-- `SET sort_key = row_number() OVER (…)` is a window-function misuse error in
-- SQLite (verified on 3.51 during plan review).
ALTER TABLE sessions ADD COLUMN sort_key INTEGER;
UPDATE sessions SET sort_key = (
    SELECT rn FROM (
        SELECT id, row_number() OVER (ORDER BY created_at ASC, id ASC) AS rn
        FROM sessions
    ) t
    WHERE t.id = sessions.id
);
