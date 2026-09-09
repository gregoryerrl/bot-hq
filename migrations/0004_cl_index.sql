-- Context library (CL) index. Agents query this BEFORE reading whole CL
-- files so they can decide what's relevant by description, saving context.
-- Mirrors the on-disk layout under ~/.bot-hq/projects/<project>/ + the
-- bot-hq root for globals.

-- Registered projects. The session row carries working_repo_path for live
-- sessions; this table is the durable directory of all projects the user
-- has registered through bot-hq UI (Emma's project-registration flow).
-- The special row name='_globals' represents the bot-hq root itself — it
-- houses general-rules.md and other cross-project files. Agents must NOT
-- treat _globals as a real working project; it's a bucket for shared CL.
CREATE TABLE projects (
    name              TEXT PRIMARY KEY,
    display_name      TEXT NOT NULL,
    working_repo_path TEXT,
    description       TEXT,
    created_at        TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Bootstrap row for cross-project / bot-hq-system files.
INSERT INTO projects (name, display_name, working_repo_path, description)
VALUES ('_globals', 'Global rules', NULL,
        'System-level CL — general-rules.md, agent custom-instructions, etc. Not a real working project.');

-- CL file index. One row per .md (or other content file) under the
-- project's directory. file_path is relative to the project root inside
-- ~/.bot-hq/ (for the _globals project that root is ~/.bot-hq/ itself).
CREATE TABLE cl_index (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id  TEXT NOT NULL REFERENCES projects(name) ON DELETE CASCADE,
    file_path   TEXT NOT NULL,
    description TEXT NOT NULL,
    -- Optional user-authored search hooks (comma-separated terms). Free-form;
    -- agents match via LIKE when searching by query. Empty/NULL is fine.
    tags        TEXT,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(project_id, file_path)
);

CREATE INDEX idx_cl_index_project ON cl_index(project_id);
CREATE INDEX idx_cl_index_updated ON cl_index(updated_at);

-- Audit / lineage table. One row per (agent, file) READ event. Bot-hq
-- doesn't surface this in v1 but the data lets us answer "what context
-- did Brian have before he made decision X?" later. Cheap to write.
CREATE TABLE cl_reads (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    cl_index_id INTEGER NOT NULL REFERENCES cl_index(id) ON DELETE CASCADE,
    session_id TEXT,
    agent      TEXT NOT NULL,
    read_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_cl_reads_index ON cl_reads(cl_index_id);
CREATE INDEX idx_cl_reads_session ON cl_reads(session_id);
