-- 0024_cl_atoms.sql — FTS5 atom index for queryable CL retrieval (Phase 3).
--
-- An *atom* is one heading-delimited section of a CL file. This is a standalone
-- FTS5 virtual table: `heading_path` and `body` are tokenized and BM25-searchable;
-- `project_id` / `file_path` / `mtime` / `body_hash` are UNINDEXED metadata (stored,
-- not tokenized). Retrieval ranks matches via bm25(cl_atoms).
--
-- Like `cl_index`, this is a DERIVED, disposable layer: `cl_rescan` repopulates it
-- from disk, so the filesystem stays the source of truth. `body_hash` is a SHA-256
-- of the section body (deterministic + stable across processes/toolchains) for
-- future retrieval-time stale-flagging.
--
-- FTS5 availability: the bundled libsqlite3-sys (sqlx "sqlite" feature) compiles the
-- SQLite amalgamation with SQLITE_ENABLE_FTS5. This CREATE VIRTUAL TABLE is itself
-- the availability probe — were FTS5 absent, this migration (run by every
-- Storage::open / Storage::memory) would fail loudly at boot.
CREATE VIRTUAL TABLE cl_atoms USING fts5(
    project_id  UNINDEXED,
    file_path   UNINDEXED,
    heading_path,
    body,
    mtime       UNINDEXED,
    body_hash   UNINDEXED,
    tokenize = 'porter unicode61'
);
