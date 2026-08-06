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
