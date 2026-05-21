-- Per-session free-form document store. Isolated from CL (cl_index) so
-- plans, investigation findings, and scratch notes don't pollute the
-- shared knowledge base. Agents upsert by (session_id, slug); reads /
-- searches are scoped to the session. Promotion to CL is just the agent
-- writing the body to a CL path via Write/Bash + cl_rescan — no separate
-- mechanism. Rows persist with the session row (sessions archive rather
-- than delete; the CASCADE here only matters if/when a session is hard-
-- deleted, which the app doesn't do today).

CREATE TABLE IF NOT EXISTS session_documents (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    slug        TEXT NOT NULL,
    body        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE (session_id, slug),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS session_documents_session_idx
    ON session_documents(session_id);
