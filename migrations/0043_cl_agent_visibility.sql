-- Per-file agent visibility (2026-08-05). 1 = agents see the file in CL
-- search/retrieval (the default); 0 = user-only (personal notes / diary):
-- hidden from agent-facing cl_index_search / cl_retrieve and refused by
-- agent cl_write_file. The Library UI always shows every file, and rescan
-- upserts never touch the flag (ON CONFLICT updates description/tags only).
ALTER TABLE cl_index ADD COLUMN agent_visible INTEGER NOT NULL DEFAULT 1;
