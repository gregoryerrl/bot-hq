# DB reset runbook — clear session data, keep every configuration

**Decided 2026-08-12.** The user is starting the database over as part of the
rc3 reframe: *"I accept losing session data, they've written their learnings to
CL anyways."* Configuration is explicitly NOT in scope for the reset —
*"yes please carry my config forward those are really important especially the
projects."*

**Run this only AFTER the D10 schema work has landed and the app has been
launched once on the new code**, so the schema is current before the reset. The
whole point of resetting in place is that the schema is already migrated.

## Why reset-in-place, not fresh-DB-plus-import

The obvious approach — create a fresh database and copy configuration into it —
requires enumerating what to carry, and anything not on the list is silently
lost. Deleting session data from a copy keeps everything else **automatically**,
including configuration nobody thought to enumerate, on a schema that is already
correct. Strictly less risk for the same outcome.

## What survives the reset, verified

The recipe below was dry-run against the legacy archive on 2026-08-12. Result:

| kept | rows | | cleared | rows |
|---|---|---|---|---|
| `projects` | 14 | | `sessions` | 0 |
| `models` | 5 | | `messages` | 0 |
| `app_settings` | 1 | | `session_participants` | 0 |
| `roles` | 2 | | `participant_deliveries` | 0 |
| `cl_index` | 125 | | `findings` | 0 |
| `cl_folders` | 9 | | `session_tray` | 0 |
| `cl_atoms` | 2086 | | every `*_events` table | 0 |

`PRAGMA integrity_check` → `ok`. `PRAGMA foreign_key_check` → no dangling rows.
Context Library full-text search still returns (435 hits for `session`), so the
FTS index survives rather than needing a rescan. File size 511 MB → 5.1 MB.

`agent_configs` keeps its 2 rows and is expected to be dead after the Agents tab
is retired (D8) — it is left alone here rather than special-cased.

## The independent safety net

Context Library **content** does not live in the database at all. It is
`~/.bot-hq/library/` — 1,254 files, 8.9 MB, **a git repository with a clean
working tree**, committed as the agents wrote. The database holds only the
search index over it, which a rescan can rebuild from disk. This is what makes
losing session history cheap: the learnings were written to the CL, and the CL
is versioned on disk.

⚠️ That repo has **no remote**. It is one disk failure away from gone. Worth a
backup independent of this runbook.

## The recipe

Archive first — the legacy copy already exists at
`~/.bot-hq/.local/legacy/bot-hq-legacy-2026-08-12.db` (511 MB, `integrity_check
ok`, 392 sessions / 213,704 messages / 784 participants). It stays queryable so
an agent can answer *"what did they work on in July"* against it directly.

Stop bot-hq first — `pkill -f "target/debug/bot-hq"` — and take a second archive
of the CURRENT database before deleting anything from it.

Deletes run with `foreign_keys=OFF` in child-before-parent order; `deliveries`
before `cursors` before `session_participants` before `sessions` is what the
constraints require. `PRAGMA foreign_key_check` afterwards is what proves the
result rather than assuming it.

```sql
PRAGMA foreign_keys=OFF;
BEGIN;
DELETE FROM participant_deliveries;
DELETE FROM participant_cursors;
DELETE FROM session_participants;
DELETE FROM session_documents;
DELETE FROM session_tray;
DELETE FROM activity_events;
DELETE FROM cancel_events;
DELETE FROM forward_events;
DELETE FROM retrieval_events;
DELETE FROM findings;
DELETE FROM cl_reads;
DELETE FROM agent_feedback;
DELETE FROM messages;
DELETE FROM sessions;
COMMIT;
VACUUM;
```

Then verify, and do not skip this — the checks are the deliverable:

```sql
PRAGMA integrity_check;      -- expect: ok
PRAGMA foreign_key_check;    -- expect: no rows
SELECT COUNT(*) FROM projects;   -- expect: 14
SELECT COUNT(*) FROM models;     -- expect: 5
SELECT COUNT(*) FROM cl_atoms WHERE cl_atoms MATCH 'session';  -- expect: > 0
SELECT COUNT(*) FROM sessions;   -- expect: 0
```

Also delete `.local/lock` if bot-hq did not shut down cleanly, and — once the
native loop is gone (D9) — `.local/native-accounting.jsonl` and
`.local/native-history/`, which no longer have a writer.
