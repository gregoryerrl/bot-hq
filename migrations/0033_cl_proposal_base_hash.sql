-- 0033_cl_proposal_base_hash.sql — optimistic-concurrency snapshot for CL proposals.
--
-- sha256 (lowercase hex) of the target file's content at propose time, captured
-- for kind='correct'/'delete' proposals. NULL for 'add' (no base file) and for
-- rows filed before this migration — approval treats NULL as "no drift check
-- possible" and skips staleness detection for them.

ALTER TABLE cl_proposals ADD COLUMN base_hash TEXT;
