-- 0021_findings.sql — EYES-sign-off gate: per-session review findings.
--
-- EYES (rain) files findings via the `eyes_flag` MCP tool; HANDS (brian)
-- resolves them via `disposition_finding`. A finding that is `blocking` AND
-- still `open` gates `git commit` — enforced by the MCP `check_open_findings`
-- tool (prompted primary) PLUS the pre-commit/pre-push git hooks (mechanical
-- backstop, read-only DB query). `advisory` findings never gate. This mirrors
-- the `check_commit_message` disguise gate's two-layer model.
--
-- Lifecycle (status):
--   open     → filed, not yet resolved (gates when severity='blocking')
--   fixed    → HANDS fixed it (disposition_reason references the fix)
--   rebutted → HANDS disagrees, with a reason (no EYES agreement needed —
--              prevents deadlock; surfaces to the user)
--   stale    → code the finding referenced is gone, or user-cleared
--
-- `finding_uid` is the public handle the agent passes to `disposition_finding`
-- (like session_tray.choice_id) — stable, decoupled from the autoincrement id.
-- Timestamps are written RFC3339-Z by the Rust layer (storage::now_utc), so the
-- columns carry no CURRENT_TIMESTAMP default (that would be the pre-0012 shape).

CREATE TABLE findings (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id          TEXT NOT NULL,
    finding_uid         TEXT NOT NULL UNIQUE,
    agent               TEXT NOT NULL,
    severity            TEXT NOT NULL,
    summary             TEXT NOT NULL,
    code_ref            TEXT,
    status              TEXT NOT NULL DEFAULT 'open',
    disposition_reason  TEXT,
    disposed_by         TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

-- Gate hot path: count / list open blocking findings for a session.
CREATE INDEX idx_findings_open
    ON findings(session_id, status)
    WHERE status = 'open';
