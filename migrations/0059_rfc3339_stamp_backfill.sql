-- Round-2 audit R5: backfill the two columns holding SQLite's zone-less
-- `datetime('now')` shape into the RFC3339-Z everything else in this database
-- uses.
--
-- Why it matters, and why a backfill rather than only a writer fix: a zone-less
-- `2026-08-12 15:48:26` sorts BEFORE any same-day RFC3339 `2026-08-12T…Z`,
-- because ' ' (0x20) < 'T' (0x54). Every lexicographic window over these
-- columns therefore reads a zone-less row as EARLIER than midnight of its own
-- day. The frontend separately parses a zone-less string as LOCAL time, which
-- is the staleness hallucination `storage::time` exists to prevent.
--
-- `participant_deliveries.delivered_at` — `commit_delivery` wrote SQLite's
-- `datetime('now')` until 1a575e8 fixed the WRITER and shipped a guard. No
-- backfill went with it, and the guard only ever inspects a row it just wrote,
-- so it could not see the population: 4011 of 4025 rows were still wrong.
--
-- `participant_cursors.updated_at` — found by this round's pre-flight, which
-- the reviewer insisted on running BEFORE any migration. Different cause: the
-- column carries a `datetime('now')` DEFAULT and the seeding INSERT omitted it,
-- so the default fired on every cursor ever created. 85 of 90 rows; only the
-- ones a later UPDATE had touched were right. Both INSERT sites now bind
-- `now_utc()` explicitly.
--
-- Shape-guarded, not blanket: `LIKE '____-__-__ __:__:__'` matches only the
-- exact zone-less form, so a row already RFC3339 (with or without millis) is
-- untouched and re-running this changes nothing. The `.000Z` millis keep the
-- output byte-comparable with `now_utc()`, which always emits them.
--
-- The other 15 `datetime('now')` / `CURRENT_TIMESTAMP` defaults in the schema
-- are deliberately left. The pre-flight measured every one of them against the
-- live database and found no zone-less data: `sessions.created_at`,
-- `messages.created_at`, `session_tray.asked_at`, `session_participants.joined_at`,
-- `projects.created_at`, `cl_index.*`, `cl_folders.*`, `agent_feedback.*`,
-- `roles.*`, `agent_configs.updated_at`, `cl_reads.read_at`, `plugins.installed_at`
-- — every prod insert binds them, so those defaults never fire. Dropping them
-- would mean rebuilding 13 tables to remove a hazard that has not materialised
-- in any of them; the evidence says fix the one writer that was actually
-- omitting a bind, which is what this migration's code half does.

UPDATE participant_deliveries
SET delivered_at = replace(delivered_at, ' ', 'T') || '.000Z'
WHERE delivered_at LIKE '____-__-__ __:__:__';

UPDATE participant_cursors
SET updated_at = replace(updated_at, ' ', 'T') || '.000Z'
WHERE updated_at LIKE '____-__-__ __:__:__';
