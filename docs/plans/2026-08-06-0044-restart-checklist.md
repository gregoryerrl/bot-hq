# 0044 restart checklist — do this, in this order

Standalone, copy-pasteable. Full reasoning lives in
[`2026-08-06-session-participants-runbook.md`](2026-08-06-session-participants-runbook.md);
this is the operational sequence only.

**What is about to happen:** `migrations/0044_session_participants.sql` is
committed but has **not run**. It applies to the live database during the next
app start. The running instance holds the old schema until then.

**Verified before this checklist was written** (HEAD `e08dfaf`): four dry runs —
the last against the exact current database — 201,470 messages with `author`
preserved on every row, 0 unmapped, 768 participants across 384 sessions,
integrity + FK clean; three guard-firing tests; a legacy-query boot check; all
five gates green.

---

## 1. Fresh backup (do NOT skip — the existing one goes stale)

```bash
sqlite3 ~/.bot-hq/.local/bot-hq.db ".backup '$HOME/.bot-hq/.local/bot-hq.db.pre0044'"
sqlite3 ~/.bot-hq/.local/bot-hq.db.pre0044 "PRAGMA integrity_check;"
```

Expect `ok`. `.backup` is consistent on a live database, so the app can stay
running for this step. A backup nobody opened is not a backup — the
`integrity_check` is the point.

## 2. Check disk headroom

```bash
df -h /System/Volumes/Data
```

The rebuild copies the largest table before dropping the original, so it
transiently needs **≈2× the database size** (~1 GB for a 452 MB DB). This is not
theoretical — a dry run failed with `No space left on device` while this was
being built.

## 3. Rebuild — frontend FIRST

```bash
cd ~/Projects/bot-hq/frontend && npm run build
cd ~/Projects/bot-hq && cargo build --release
```

Order matters: the binary embeds `frontend/dist`, so building the binary first
bakes a stale frontend (a documented past incident — unstyled app, 5 KB CSS
instead of 30 KB).

## 4. Restart

Quit bot-hq **fully**, then start it. 0044 applies during startup.

## 5. Verify it landed

```bash
sqlite3 ~/.bot-hq/.local/bot-hq.db "
  SELECT 'applied='||count(*) FROM _sqlx_migrations WHERE version=44;
  SELECT 'messages='||count(*)||' max='||max(id) FROM messages;
  SELECT 'author_preserved='||count(*) FROM messages WHERE author IS NOT NULL;
  SELECT 'unmapped='||count(*) FROM messages WHERE origin='participant' AND participant_id IS NULL;
  SELECT 'roles='||count(*) FROM roles;
  SELECT 'participants='||count(*) FROM session_participants;
  PRAGMA integrity_check;"
```

| field | expected |
|---|---|
| `applied` | **1** |
| `messages` | ≈ the pre-restart count (grows as you use the app) |
| `author_preserved` | **equal to `messages`** |
| `unmapped` | **0** |
| `roles` | **2** |
| `participants` | **2 × session count** |
| `integrity_check` | `ok` |

Also worth an eyeball: open a session and confirm the chat history renders. The
message table was rebuilt; this is the visible proof it survived.

## 6. If the app does NOT start

It fails **loudly**, not silently: a migration error propagates through
`Storage::open` and `main.rs`'s `?`, so the app refuses to start rather than
starting degraded.

```bash
cp ~/.bot-hq/.local/bot-hq.db.pre0044 ~/.bot-hq/.local/bot-hq.db
rm ~/Projects/bot-hq/migrations/0044_session_participants.sql
# or: git revert d7b910c
cd ~/Projects/bot-hq && cargo build --release
```

**Deleting the migration before restarting is essential** — otherwise sqlx
re-applies it to the restored database and you are back where you started.

Check the log for the failure reason before retrying:
`~/.bot-hq/.local/logs/` (rolling daily files).

---

## 7. FOLLOW-UP (after verification): reclaim ~400 MB with VACUUM

The rebuild leaves the dropped table's pages on the freelist — SQLite does not
return them to the filesystem on its own. Measured immediately after 0044
applied:

```
page_count=221382  freelist=102511  page_size=4096   →  400.4 MB reclaimable
```

The database file grew from 452 MB to **864.8 MB**, i.e. **46% of it is empty
space**. `VACUUM` rewrites the file compactly and gives that back.

**Requires the app STOPPED** — `VACUUM` needs an exclusive lock and will fail or
block against a live connection. It also needs free disk roughly equal to the
final size while it runs.

```bash
# with bot-hq quit:
sqlite3 ~/.bot-hq/.local/bot-hq.db "VACUUM;"
sqlite3 ~/.bot-hq/.local/bot-hq.db "PRAGMA integrity_check;"   # expect: ok
ls -la ~/.bot-hq/.local/bot-hq.db
```

Not urgent, but worth doing before the next table rebuild — that one will need
≈2× the *file* size, and a bloated file makes the requirement 2× larger than it
needs to be.

## What this session survives

- **Session row, all 1,474 messages, and the IPAV docs** — preserved by the
  migration (proven in every dry run).
- **Brian's context** — `brian_claude_session_id` is present, so claude-code
  restores the conversation via `--resume`.
- **Rain's context** — she runs on the native connector, which persists to
  `~/.bot-hq/.local/native-history/s-310657f6-rain.json` (862 KB). The NULL
  `rain_claude_session_id` is a claude-code field that does not apply to her.

Agents respawn lazily when the session is reopened, not automatically at app
start.

## After verification

Next batch is **B4b** — the structural rekey (`SessionHandle` → participant map,
`ActivityTracker` per participant). It is the first batch with no additive
escape hatch. Plan:
[`2026-08-06-session-focused-redesign-implementation.md`](2026-08-06-session-focused-redesign-implementation.md).
