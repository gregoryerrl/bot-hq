-- Add a `kind` column to the derived FTS5 atom index so retrieval can pin (and
-- later decay) BY KIND (convention|gotcha|decision|policy|issue|handoff|idea|
-- note) instead of by hardcoded filenames. FTS5 has no `ALTER TABLE ADD COLUMN`,
-- so this standalone vtable is dropped and recreated.
--
-- Safe because `cl_atoms` is a DERIVED, DISPOSABLE layer rebuilt from disk by
-- `cl_rescan`: after this migration the table is empty, and the per-project boot
-- rescan (main.rs) plus the zero-atom backfill branch (bridge/cl_facade.rs)
-- repopulate every file's atoms — now carrying `kind` — on the next launch. This
-- is the exact empty-then-backfill path migration 0024 relied on; no data is
-- lost that the next scan can't re-derive.
DROP TABLE cl_atoms;
CREATE VIRTUAL TABLE cl_atoms USING fts5(
    project_id  UNINDEXED,
    file_path   UNINDEXED,
    kind        UNINDEXED,
    heading_path,
    body,
    mtime       UNINDEXED,
    body_hash   UNINDEXED,
    tokenize = 'porter unicode61'
);
