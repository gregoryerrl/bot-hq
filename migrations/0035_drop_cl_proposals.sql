-- The CL proposal review queue is removed (2026-07-21): agents write the
-- Context Library directly via the cl_write_file MCP tool, so the durable
-- proposal rows have nothing to feed. Historical rows are discarded with the
-- table — the queue was rubber-stamp-approved in practice, so there is no
-- audit value worth preserving.
DROP TABLE IF EXISTS cl_proposals;
