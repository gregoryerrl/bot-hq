# B1 migration runbook — session participants

Companion to `2026-08-06-session-participants-migration-DRAFT.sql`. Written
**before** the migration runs, because that migration is irreversible by design:
sqlx checksums applied migrations at runtime and bot-hq's immutable-artifact
pre-commit gate blocks edits to `migrations/*.sql`, so there is no second
attempt. The only rollback is restore-from-backup.

## Why the draft is not in `migrations/`

sqlx applies every file in `migrations/` automatically at app start. Creating
the file there **is** arming the destructive step — there is no separate "run
it" action. So the draft lives in `docs/plans/` until explicitly approved, then
moves to `migrations/0044_session_participants.sql` (0043 is the current head).

## STATUS: ARMED 2026-08-06

`migrations/0044_session_participants.sql` is in place. **It applies to the live
database on the next app start**, not before — the running instance holds the
old schema until it restarts.

What was done before arming:
- **Online backup taken and verified:** `~/.bot-hq/.local/bot-hq.db.pre0044`,
  `PRAGMA integrity_check = ok`, 201,034 messages / 384 sessions, matching live
  exactly. Taken with `sqlite3 .backup`, which is consistent on a live DB.
- **Final dry-run against the exact current live state:** exit 0, 201,034 rows
  with `author` preserved on every one, max id 201,164 intact, 0 unmapped, 768
  participants across 384 sessions, 768 cursors, integrity + FK clean.
- **Test suite re-run with 0044 embedded:** all 10 participant tests now pass
  against the migration applied by sqlx's own migrator on a fresh DB, not the
  hand-applied draft. The `storage_with_0044()` scaffold is gone.
- Gates green: cargo 1079, vitest 199, tsc + both builds.

**What is NOT yet verified:** the live application of 0044 to the real database.
That happens on your next app restart. If it fails, the migration is not
recorded as applied (sqlite DDL is transactional), the app will report the
error, and the restore path below is intact.

### ⚠ Precondition discovered while arming: FREE DISK SPACE

The rebuild copies `messages` before dropping the original, so **the migration
transiently needs roughly 2× the database size** — about 450 MB here. This was
found the hard way: a dry-run copy failed with `No space left on device` because
the volume was at 100% (accumulated dry-run copies had themselves consumed
8.8 GB). Check free space before restarting the app:

```
df -h /System/Volumes/Data
ls -la ~/.bot-hq/.local/*.db*
```

Note the pre-existing `bot-hq.db.bak-20260728-194915` (350 MB) and the new
`bot-hq.db.pre0044` (452 MB) both sit on the same volume. Keep `pre0044` until
the live migration is verified; the July backup is prunable.

## Pre-flight

1. **Stop bot-hq.** A live app holds the single-instance lock and will apply
   migrations on next start; the copy below must be taken from a quiesced DB.
2. **Back up, verify the backup opens:**
   ```
   cp ~/.bot-hq/.local/bot-hq.db ~/.bot-hq/.local/bot-hq.db.pre0044
   sqlite3 ~/.bot-hq/.local/bot-hq.db.pre0044 "PRAGMA integrity_check;"
   ```
   `integrity_check` must print `ok`. A backup nobody opened is not a backup.
3. **Record the baseline** (these are the numbers the guards assert against):
   ```
   sqlite3 bot-hq.db "SELECT count(*), max(id) FROM messages;"
   sqlite3 bot-hq.db "SELECT author, count(*) FROM messages GROUP BY author;"
   sqlite3 bot-hq.db "SELECT count(*) FROM sessions;"
   ```
   Expected at time of writing: **199,607 / brian 132,037 / rain 62,548 /
   user 5,022 / 382 sessions**. Re-measure — the numbers move every session.

## Dry run (mandatory — this is where it gets caught)

Run against a **copy**, never the live DB:

```
cp bot-hq.db /tmp/dryrun.db
sqlite3 /tmp/dryrun.db < docs/plans/2026-08-06-session-participants-migration-DRAFT.sql
```

Then verify on `/tmp/dryrun.db`:

| check | expected |
|---|---|
| `SELECT count(*) FROM messages;` | identical to baseline |
| `SELECT max(id) FROM messages;` | identical to baseline |
| `SELECT count(*) FROM messages WHERE origin='participant' AND participant_id IS NULL;` | **0** |
| `SELECT count(*) FROM session_participants;` | `382 + 382` = 764 rows (solo sessions keep a disabled `rain` row) |
| `SELECT count(*) FROM session_participants WHERE slug='rain' AND enabled=0;` | **12** |
| `SELECT count(*) FROM participant_cursors;` | equals participant count |
| per-participant message counts | match the old per-author counts exactly |
| `PRAGMA integrity_check;` | `ok` |
| `PRAGMA foreign_key_check;` | empty |

Also confirm the app **boots** against the dry-run DB before touching the real
one (`BOT_HQ_DATA_DIR` pointed at a copied data dir) — a schema that satisfies
SQL guards can still fail sqlx's compile-time query checks.

## Dry-run results — 2026-08-06 (already performed)

Run against `/tmp/dryrun_participants.db`, a copy of the live DB.

**First attempt FAILED and this is why the dry run exists:** the guards used
`SELECT RAISE(ABORT, …)`, which is legal **only inside a trigger program**. As a
bare statement it is a parse error — and `sqlite3` continues past parse errors,
exiting **0**. All three guards silently no-op'd while the rebuild proceeded
unguarded. In `migrations/` that would have applied, checksummed, and been
unrevisable. Rewritten as a `CHECK (failure IS NULL)` guard table whose
conditional INSERT raises a constraint error.

**Second attempt, with `sqlite3 -bail`, EXIT=0:**

| check | result |
|---|---|
| messages count / max id | 199,637 / 199,767 (the live DB grows during a session; the in-migration guards assert equality at run time) |
| `session_participants` | **764** = 382 sessions × 2 |
| `rain` rows with `enabled=0` | **12** — matches the 12 solo sessions exactly |
| `participant_cursors` | **764** |
| unmapped participant rows | **0** |
| per-participant counts | brian 132,063 · rain 62,551 · user 5,023 |
| `PRAGMA integrity_check` | `ok` |
| `PRAGMA foreign_key_check` | empty |

**Guard-firing test (a guard that only passes proves nothing).** Injected an
unmappable `author='emma'` row into a second copy and re-ran:

```
Runtime error near line 178: CHECK constraint failed: failure IS NULL (19)
EXIT=1
```

and critically, the original `messages` table was **left intact** (old schema,
full row count) — the guards run *before* `DROP TABLE messages`, so a failed
migration is a no-op, not a half-rebuild.

**Still outstanding before cutover:** the app-boot check against a copied data
dir. SQL guards cannot catch a sqlx compile-time query mismatch.

## Revision 2 dry-run — 2026-08-06 (roles + turn state added)

The draft grew a `roles` table (roles became first-class and user-owned) and the
turn-model columns (`turn_position`, `participation_mode`, `done_vote`, per-
session round state). Re-run against a fresh copy of the live DB,
`sqlite3 -bail`, **true exit 0**:

| check | result |
|---|---|
| `roles` seeded | **2** (`hands`, `eyes`) |
| `session_participants` | **764** = 382 × 2 |
| participants with a resolved `role_id` | **764** — none orphaned |
| sessions with a position-0 participant | **382** = every session has a defined cycle start |
| `rain` rows disabled | **12** — matches the solo sessions |
| `participant_cursors` | **764** |
| unmapped messages | **0** |
| `PRAGMA integrity_check` / `foreign_key_check` | `ok` / empty |

### Guard-firing tests for the NEW guards

Two deliberately-broken variants of the migration were generated and run against
fresh copies:

| broken variant | result | original `messages` table |
|---|---|---|
| rain seeded at `turn_position = 0` (two cycle starts per session) | **GUARD 5 fired** — `CHECK constraint failed: failure IS NULL`, true exit **1** | **intact, old schema** |
| role seeding removed (participants would resolve no role) | aborted at seeding — `NOT NULL constraint failed: session_participants.capabilities`, true exit **1** | **intact, old schema** |

**Honest note on guard 4.** The second test aborted at the seeding INSERT rather
than reaching guard 4, because the `capabilities` NOT NULL constraint fires
first when the role subselect returns NULL. So guard 4 ("participant with no
role") is belt-and-braces rather than the primary catch for that failure — the
failure mode IS caught, just one layer earlier. Recorded rather than glossed:
the guard has not been proven to fire on its own path.

Both aborts left the original table untouched, confirming the ordering property
still holds after the revision: **every guard runs before `DROP TABLE
messages`,** so a failed migration is a no-op rather than a half-rebuild.

## Revision 3 — the transitional `author` column (2026-08-06)

**This revision removes the migration's dependency on the code migration.**

Revision 2 dropped `author`, which meant the app could not boot until all 153
`Author::` references and 31 `insert_message` call sites moved in one
unreviewable commit. Revision 3 keeps `author` as a populated legacy column —
the CHECK constraint (the only reason the table needed rebuilding) is still
gone, but every existing query keeps working the moment the migration applies.

Consequences:
- **Arming 0044 no longer waits on B3b/B4.** The schema change stays big-bang
  (one migration); the code migrates in reviewable slices afterwards.
- A follow-up migration drops `author` and makes `origin` NOT NULL once the
  cutover grep audit proves nothing reads it.
- `idx_messages_session_author_time` is retained for as long as the column is —
  dropping it while queries still use `author` would silently turn the message
  pane's per-agent reads into a table scan over ~200k rows.

**A defect this revision caught, which would have stopped the app booting:**
`origin` was `NOT NULL` with no default. Every legacy `insert_message` — which
knows nothing about `origin` — would have failed on insert. It is now
transitional-nullable, backfilled for existing rows, and made NOT NULL by the
follow-up migration. Found by writing
`every_legacy_message_query_still_works_after_0044`, not by reading the SQL.

**Revision 3 dry-run** (fresh copy of the live DB, `-bail`, exit 0):

| check | result |
|---|---|
| messages | **200,726** |
| `author` preserved | **200,726** — all rows |
| `origin` backfilled | **200,726** — all rows |
| unmapped participant rows | **0** |
| `PRAGMA integrity_check` | `ok` |
| guard-firing (injected `emma` row) | **aborts**, original table intact |

**The app-boot unknown is now partly closed:** `every_legacy_message_query_still_works_after_0044`
exercises `insert_message`, `count_user_messages` and `messages_for_session`
against the real post-migration schema. A full app boot against a copied data
dir is still worth doing, but the query-shape failure mode it was meant to catch
is now covered by a test.

## Go / no-go

Proceed only if every row above passes. Any mismatch → stop, fix the draft, dry
run again. The draft is editable **only while it is still in `docs/plans/`**.

## Cutover

1. `git mv docs/plans/2026-08-06-session-participants-migration-DRAFT.sql
   migrations/0044_session_participants.sql` and strip the DRAFT banner.
2. Start bot-hq; sqlx applies it once and records the checksum.
3. Re-run the verification table against the live DB.

## If it goes wrong

```
# stop the app first
cp ~/.bot-hq/.local/bot-hq.db.pre0044 ~/.bot-hq/.local/bot-hq.db
```
Then delete `migrations/0044_*.sql` **before** restarting, or sqlx re-applies it
to the restored DB. If 0044 was already committed, revert that commit too —
the immutable-artifact gate will otherwise refuse the deletion.

## Known non-goals of this migration

Deliberately deferred so this one stays restore-revertible rather than
revertible-by-nothing:

- dropping `sessions.{brian,rain}_*` + `rain_enabled` (a later migration, after
  the code stops reading them);
- `agent_configs.agent_name` CHECK (low traffic, no urgency);
- `retrieval_events.agent`, `cancel_events`, `activity_events` paired columns
  (batch B3 rekeys them).

## Residual risk, stated plainly

The rebuild rewrites the highest-traffic table in the system with foreign keys
disabled. The three in-migration guards (`row count`, `unmapped authors`,
`max id`) abort on the failures that are detectable in SQL. They cannot catch a
semantic error — e.g. a wrong `origin` classification — which is what the dry-run
verification table and the app-boot check are for. Nothing here makes the
migration safe to run unreviewed.
