-- 0025_cl_proposals.sql — durable project-scoped CL edit proposals.
--
-- Agents file proposals via non-mutating MCP tools. The rows live beyond any
-- one session and are scoped to a CL project; session_id is audit-only.
-- Host-mediated approval performs supported write-back and then rescans the CL.

CREATE TABLE cl_proposals (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    proposal_uid    TEXT NOT NULL UNIQUE,
    project_id      TEXT NOT NULL,
    file_path       TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK(kind IN ('add', 'correct', 'delete')),
    target_excerpt  TEXT,
    proposed_body   TEXT NOT NULL,
    evidence        TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open', 'approved', 'rejected')),
    proposed_by     TEXT NOT NULL,
    session_id      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(name) ON DELETE CASCADE,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

-- Project review queue, oldest-first within a status.
CREATE INDEX idx_cl_proposals_project_status
    ON cl_proposals(project_id, status, created_at);

-- File-level queue for showing proposals against an individual CL file.
CREATE INDEX idx_cl_proposals_file_status
    ON cl_proposals(project_id, file_path, status);
