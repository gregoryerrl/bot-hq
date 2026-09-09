-- 0022_findings_escalation.sql — soft auto-escalation for the EYES-sign-off gate.
--
-- raise_count: how many times EYES has raised this OPEN finding. EYES re-files
-- via eyes_flag; a same-summary OPEN finding is deduped (its raise_count bumps)
-- instead of inserting a duplicate — but ONLY when HANDS has had a turn since the
-- last raise, so buffer/turn latency can't false-escalate. raise_count >= 2 =
-- "escalated": the frontend banner emphasizes it. NO halt (soft-notify).
--
-- eyes_approved: set by the EYES-only `approve_finding` tool. After HANDS
-- resolves an escalated finding (disposition_finding clears the commit gate
-- immediately — no deadlock), the escalation banner shows "fixed, awaiting EYES
-- confirm" until EYES approves, flipping this to 1 and clearing the escalation
-- signal. Orthogonal to `status` (a flag, not a lifecycle state).

ALTER TABLE findings ADD COLUMN raise_count INTEGER NOT NULL DEFAULT 1;
ALTER TABLE findings ADD COLUMN eyes_approved INTEGER NOT NULL DEFAULT 0;
