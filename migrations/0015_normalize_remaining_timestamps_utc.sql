-- 0015_normalize_remaining_timestamps_utc.sql — finish the UTC backfill.
--
-- 0012 normalized the hot tables (sessions/messages/agent_configs/session_tray).
-- This covers the rest of the zone-less SQLite-default columns (cl_index,
-- cl_folders, cl_reads, plugins, projects) so EVERY stored timestamp is
-- canonical RFC3339-Z. Same idempotent `NOT LIKE '%T%'` guard: rows already in
-- RFC3339 form (chrono-written) are skipped.

UPDATE cl_index
   SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', created_at)
 WHERE created_at IS NOT NULL AND created_at NOT LIKE '%T%';
UPDATE cl_index
   SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', updated_at)
 WHERE updated_at IS NOT NULL AND updated_at NOT LIKE '%T%';

UPDATE cl_folders
   SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', created_at)
 WHERE created_at IS NOT NULL AND created_at NOT LIKE '%T%';
UPDATE cl_folders
   SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', updated_at)
 WHERE updated_at IS NOT NULL AND updated_at NOT LIKE '%T%';

UPDATE cl_reads
   SET read_at = strftime('%Y-%m-%dT%H:%M:%fZ', read_at)
 WHERE read_at IS NOT NULL AND read_at NOT LIKE '%T%';

UPDATE plugins
   SET installed_at = strftime('%Y-%m-%dT%H:%M:%fZ', installed_at)
 WHERE installed_at IS NOT NULL AND installed_at NOT LIKE '%T%';

UPDATE projects
   SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', created_at)
 WHERE created_at IS NOT NULL AND created_at NOT LIKE '%T%';
