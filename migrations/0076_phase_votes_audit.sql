-- 1.0.0 Batch 9 (T6c, dissect #8): phase_votes held ZERO rows for a session
-- that cast 11 votes across 3 advances — every row was deleted on epoch bump
-- (documented as hygiene: a stale epoch can never match a live tally) and on
-- pass-retraction, so no post-hoc record existed of who voted for what. Votes
-- are audit rows now: retraction MARKS instead of deleting, the epoch bump
-- keeps history (growth = participants x transitions, negligible), and every
-- tally filters on retracted_at IS NULL — which stale-epoch rows already fail
-- on the epoch key, exactly as the pinned test says.
ALTER TABLE phase_votes ADD COLUMN retracted_at TEXT;
