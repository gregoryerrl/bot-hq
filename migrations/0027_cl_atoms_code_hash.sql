-- Add a `code_hash` column to the derived FTS5 atom index for retrieval-time
-- stale-flagging (CL v2 brief, failure-mode #1). An atom that cites repo source
-- paths (e.g. `src/foo.rs`) stores a hash of THAT code as it was when the atom
-- was indexed; retrieval recomputes it and flags the atom (⚠) when the code has
-- drifted since. (`body_hash` hashes the atom's own text; `code_hash` hashes the
-- code it describes — a different question, and the reason body_hash alone can't
-- answer it.)
--
-- FTS5 has no `ALTER TABLE ADD COLUMN`, so this standalone vtable is dropped and
-- recreated — the same empty-then-backfill path migrations 0024/0026 used. Safe
-- because `cl_atoms` is a DERIVED, DISPOSABLE layer rebuilt from disk by
-- `cl_rescan`: after this migration the table is empty, and the per-project boot
-- rescan (main.rs) plus the zero-atom backfill branch (bridge/cl_facade.rs)
-- repopulate every file's atoms — now carrying `code_hash` — on the next launch.
-- No data is lost that the next scan can't re-derive.
DROP TABLE cl_atoms;
CREATE VIRTUAL TABLE cl_atoms USING fts5(
    project_id  UNINDEXED,
    file_path   UNINDEXED,
    kind        UNINDEXED,
    heading_path,
    body,
    mtime       UNINDEXED,
    body_hash   UNINDEXED,
    code_hash   UNINDEXED,
    tokenize = 'porter unicode61'
);
