# rc3 restart + VACUUM — do this, in this order

> **CLOSED — the restart happened and B4a.1/B4b have been live since 2026-08-07; the VACUUM note stands as a next-restart chore, recorded in the CL (`notes.md`).** Nothing below is pending. Banner added in round 12 (2026-08-19).

Standalone, copy-pasteable. Sibling of
[`2026-08-06-0044-restart-checklist.md`](2026-08-06-0044-restart-checklist.md),
which covered the migration restart; this one covers the restart that puts
B4a.1 + B4b into the running app and compacts the database afterwards.

**What is about to happen:** five commits (`e06fd21` → `eed9a15`) are built and
green but **not running** — the live app is still the pre-B4a.1 binary. Until it
restarts, every message this session writes lands with `participant_id` NULL.
No migration runs on this restart; 0044 is already applied.

**Verified before this checklist was written** (HEAD `eed9a15`, 2026-08-06
~08:35Z):

- `cargo build --release` re-runs in **0.56 s with zero `Compiling` lines** →
  the binary is current at HEAD. `frontend/dist` is current too — CSS is
  **36,882 bytes** (the stale-dist tell-tale is ~5 KB).
- Five gates green: cargo **1092**, frontend 199 vitest / tsc / build, release.
- **589** unmapped rows, all in **one** session (`s-1b310b17`), still climbing.
- `target/debug` **148 GB**; free space **1.6 GiB**; DB **865 MB** with
  **399 MB** on the freelist.

---

## 0. Reclaim the disk FIRST (recommended, not required)

```bash
cd ~/Projects/bot-hq && cargo clean --profile dev
```

VACUUM gives back 399 MB. `target/debug` is holding **148 GB**, and the volume
is at **1.6 GiB free**. This is the fix; VACUUM is the rounding error.

`--profile dev` leaves `target/release` (2.6 GB) intact, so the binary you are
about to restart into survives and the restart stays instant. **Cost:** the next
`cargo test` recompiles from scratch (~2–3 min) — which B5 needs anyway, so this
is the cheap moment to pay it.

Not theoretical: a 0044 dry run on this machine failed with `No space left on
device`, and B5 is the largest batch in the arc.

## 1. No rebuild needed

Skip it. The frontend-first rebuild the 0044 checklist opens with is **already
done** (verified above). Building again would only burn disk you do not have.

## 2. Quit bot-hq fully

`VACUUM` needs an exclusive lock and will block or fail against a live
connection.

## 3. Compact the database

```bash
sqlite3 ~/.bot-hq/.local/bot-hq.db "VACUUM;"
sqlite3 ~/.bot-hq/.local/bot-hq.db "PRAGMA integrity_check;"   # expect: ok
ls -la ~/.bot-hq/.local/bot-hq.db
```

| field | expected |
|---|---|
| `integrity_check` | `ok` |
| file size | **~466 MB** (119,214 live pages × 4096), down from 865 MB |

The freelist is what 0044's table rebuild left behind — SQLite never returns
those pages to the filesystem on its own.

## 4. Relaunch, then **reopen the session**

This step is the one that is easy to get wrong.

`ensure_session_roster` runs inside `spawn_session_handle`, and **agents respawn
lazily when a session is opened** — not at app start. So the repair fires per
session, on open. Relaunching and stopping there heals nothing; that is the
lazy-spawn behaviour, not a failed fix.

Every unmapped row is in `s-1b310b17`, which is also the only rosterless
session — so reopening that one session heals all of it.

## 5. Verify

```bash
sqlite3 ~/.bot-hq/.local/bot-hq.db "
  SELECT 'unmapped='||count(*) FROM messages
    WHERE origin='participant' AND participant_id IS NULL;
  SELECT 'sessions_without_roster='||count(*) FROM sessions s
    WHERE NOT EXISTS (SELECT 1 FROM session_participants p WHERE p.session_id=s.id);
  SELECT 'participants='||count(*) FROM session_participants;
  SELECT 'author_preserved='||count(*) FROM messages WHERE author IS NOT NULL;"
```

| field | before | expected after |
|---|---|---|
| `unmapped` | 589 (climbing) | **0** |
| `sessions_without_roster` | 1 | **0** |
| `participants` | 768 | **770** (the healed session's two) |
| `author_preserved` | == `messages` | unchanged — the legacy column is still written |

Also worth an eyeball: open the session and confirm the chat history renders.

## 6. If something looks wrong

Nothing in this checklist is irreversible.

- **`VACUUM` failed or was interrupted** — SQLite rolls back; the original file
  is intact. Most likely cause here is disk, so do step 0 first and retry.
- **`integrity_check` is not `ok`** — stop and report it. Do not restart the app
  into it.
- **The app will not start** — it fails loud, not degraded (`Storage::open`
  errors propagate through `main.rs`'s `?`). Check
  `~/.bot-hq/.local/logs/` for the reason. No migration runs on this restart, so
  a migration error is not an expected failure mode.
- **`cargo clean --profile dev` regret** — it is recovered by a rebuild, nothing
  else.

## What this restart puts into the running app

| commit | what |
|---|---|
| `e06fd21` | B4a.1 — `ensure_session_roster`: seeds the roster pre-spawn and repairs rows written before it existed |
| `51c9f29` | B4b.1 — `ActivityTracker` rekeyed to per-participant maps |
| `492218a` | B4b.2 — `SessionHandle` → `Vec<SessionAgent>`; roster seed moved to the shared spawn choke point |
| `a16af64` | B4b.3/.4 — `DuoConfig.participant_id` + the B4b parity checkpoint |
| `eed9a15` | the PROGRESS entry |

No client-visible behaviour change. B0's parity oracle is byte-identical across
all of it and the frontend diff is empty — serialisation arrives in **B5**, not
here.

## After verification

Next batch is **B5** — channel transport + turn sequencer, the one that deletes
`core/router.rs`. The implementation plan
([`2026-08-06-session-focused-redesign-implementation.md`](2026-08-06-session-focused-redesign-implementation.md))
asks for a fresh `superpowers:writing-plans` pass per batch now that B4 has
landed.
