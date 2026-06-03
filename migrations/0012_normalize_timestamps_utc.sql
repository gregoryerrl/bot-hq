-- 0012_normalize_timestamps_utc.sql — backfill zone-less timestamps to UTC.
--
-- Existing rows wrote timestamps via SQLite `datetime('now')` /
-- `CURRENT_TIMESTAMP`, which emit a ZONE-LESS string (`2026-06-03 07:40:00`).
-- The frontend's `new Date(iso)` parses a zone-less string as LOCAL time, so
-- "x ago" was inflated by the viewer's UTC offset (the "stale 8h" hallucination).
-- Rewrite each stored zone-less value to canonical RFC3339-Z, interpreting the
-- existing value as UTC (which it always was — SQLite `'now'` is UTC).
--
-- The `NOT LIKE '%T%'` guard skips rows already in RFC3339 form (chrono-written,
-- e.g. session_documents), so this is idempotent and safe to re-run.

UPDATE sessions
   SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', created_at)
 WHERE created_at IS NOT NULL AND created_at NOT LIKE '%T%';

UPDATE sessions
   SET closed_at = strftime('%Y-%m-%dT%H:%M:%fZ', closed_at)
 WHERE closed_at IS NOT NULL AND closed_at NOT LIKE '%T%';

UPDATE messages
   SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', created_at)
 WHERE created_at IS NOT NULL AND created_at NOT LIKE '%T%';

UPDATE agent_configs
   SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', updated_at)
 WHERE updated_at IS NOT NULL AND updated_at NOT LIKE '%T%';

UPDATE session_tray
   SET asked_at = strftime('%Y-%m-%dT%H:%M:%fZ', asked_at)
 WHERE asked_at IS NOT NULL AND asked_at NOT LIKE '%T%';

UPDATE session_tray
   SET answered_at = strftime('%Y-%m-%dT%H:%M:%fZ', answered_at)
 WHERE answered_at IS NOT NULL AND answered_at NOT LIKE '%T%';
