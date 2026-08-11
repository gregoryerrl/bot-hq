# Removing "brian" and "rain" from the core

**Status: planned, not started. Gated on B5 task 14.** User's decision
(2026-08-11): the personal names come out; a participant is identified by its
model or its role. The names may return later as a "Name your agents" plugin.

This is the last piece of the rc3 premise in
[`2026-08-06-session-focused-redesign-design.md`](2026-08-06-session-focused-redesign-design.md)
— "participants, not agents" — that B0–B5 did not reach.

---

## It is not a rename. It is finishing the participants migration.

The names are not merely strings in code. `sessions` carries **11 columns**
named after the two people:

```
brian_model_at_spawn   rain_model_at_spawn
brian_claude_session_id rain_claude_session_id
brian_model_id         rain_model_id
brian_effort           rain_effort
brian_ultracode        rain_ultracode
                       rain_enabled
```

That is a hardcoded two-agent roster in the schema — and `session_participants`
already replaced it. `ensure_session_roster` reads `s.brian_model_id`,
`s.brian_effort`, `s.brian_claude_session_id` to seed the participant rows, so
today the roster is *derived from* the thing it supersedes.

So the task is: **make `session_participants` the sole roster, then drop the
columns.** Most call sites follow mechanically once the columns are gone,
because they are reading a column that no longer exists rather than
independently naming a person. Approaching it as find-and-replace inverts the
work — it edits 996 sites by hand and leaves the duplicate state standing.

## Measured blast radius (2026-08-11, at `99e2aee` + task 11 uncommitted)

| surface | count |
|---|---|
| Rust (`src/`) | **996** |
| — of which `src/core/router.rs` | 71 — **deleted by B5 task 14, for free** |
| — `src/core/session.rs` | 102 |
| — `src/agents/prompts.rs` | 78 — see landmine 2 |
| — `src/agents/spawn.rs` | 59 |
| — `src/signaling/jsonrpc.rs` | 55 |
| — `src/storage/participants.rs` | 51 |
| Frontend (`frontend/src/`) | **183** |
| `messages.author` rows carrying the names | ~207,000 |

Counted with `rg -ci '\b(brian|rain)\b'`. **The word boundaries matter**: a first
pass using `rain\b` returned 1,331 for Rust because it matches `drain`, which
`sequencer.rs` uses constantly, and `brain`. Re-measure rather than carrying
these figures forward — they move with every batch.

## Slug and display name are already separate, which makes the plugin free

`session_participants` carries both:

- **`slug`** — machine addressing, `UNIQUE (session_id, slug)`. What peers use
  to reach each other.
- **`display_name`** — what a human reads.

Current rows set them in lockstep (`'brian'`, `'Brian'`), which is why they look
entangled. They are not. A "Name your agents" plugin writes **only
`display_name`** and can therefore never break peer routing — the personal names
come back as pure config with no code path involved.

### The slug generation rule

Neither candidate the user named is unique on its own: two participants on the
same model, or two in the HANDS role, are both legitimate and would violate
`UNIQUE (session_id, slug)`. So:

> Generate from the role slug, with a numeric suffix on collision (`hands`,
> `hands-2`), user-overridable at invite time.

Identity is already `participant_id`; the slug only has to be stable and unique
within a session.

## Sequencing

**After B5 task 14.** Two reasons:

1. `router.rs` holds 71 of the 996 and is the densest agent-name logic in the
   tree. Task 14 deletes the file. Renaming it first is work thrown away.
2. Task 14 is B5's only irreversible step and wants a clean tree.

## Landmines

1. **`slug == author` is an identity 0044 relied on.** 0044 seeded participants
   on exactly `slug == author`, and `messages.author` is still written and still
   read as the fallback wherever `origin` is NULL. A slug rename silently
   decouples the two unless the `author` drop lands with it. ~207k rows carry the
   old values; the transitional plan in `migrations/0044` already names the
   follow-up migration that drops the column — this is that migration's caller.

2. **`agents/prompts.rs` (78) is agent-visible text, not plumbing.** The names
   appear inside the hardcoded role prompts, so changing them changes how agents
   refer to each other mid-session. That is a behaviour change requiring a live
   check, not a sed. Sessions resumed with `--resume` also carry the old names in
   their existing transcript, so a renamed prompt meets a history that
   contradicts it.

3. **`rain_enabled` has no participant equivalent yet.** It is a session-level
   toggle for whether the second agent exists at all; the participants model
   expresses that as roster membership. Decide which before dropping the column.

4. **Test fixtures bake the slugs.** `ensure_session_roster` +
   `participant_by_slug("s1", "brian")` is the standard setup idiom across
   `storage/` and `core/` tests, including the one task 11 added. These are cheap
   to change but numerous, and they will dominate the diff's line count while
   being the least interesting part of it.

## Open decisions

- Does a session still auto-seed a two-participant roster, or does it start
  empty and require an explicit invite? The columns cannot be dropped until this
  is answered, because `ensure_session_roster` is the only current seeder.
- Do the built-in HANDS/EYES roles keep fixed slugs (`hands`, `eyes`), or do
  they become ordinary user-owned rows with generated slugs like any other?
- Is `display_name` nullable — i.e. does the UI fall back to the slug or the
  model id when the user has named nothing?

## Suggested task order

1. Give `ensure_session_roster` a seeding path that does not read the per-agent
   `sessions` columns (roles + models tables only).
2. Move the remaining readers of those 11 columns onto `session_participants`.
3. Migration: drop the 11 columns. **Irreversible** — same gate discipline as
   B5 task 14.
4. Slug generation + invite-time override.
5. `Author` enum (`{User, Brian, Rain}` in `storage/row_types.rs`) → drop the
   agent variants; it is the type that hardcodes the names into every write.
6. Drop `messages.author` (the migration 0044 already anticipates).
7. Frontend (183) — display names only by this point.
8. Prompts, with a live check.
