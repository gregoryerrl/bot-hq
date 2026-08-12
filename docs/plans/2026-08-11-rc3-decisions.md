# rc3 decisions — user calls, 2026-08-11

Answers to every question raised by the drift audit and the brief-mapping round.
Written down because the audit's finding was that **prose decisions which never
became a durable artifact get lost** — three validated ones went missing for five
days exactly that way. A decision recorded only in chat is not recorded.

Cross-references: [`design`](2026-08-06-session-focused-redesign-design.md),
[`drift audit`](2026-08-11-design-drift-audit.md),
[`inventory`](2026-08-06-router-behaviour-inventory.md).

---

## D1 — `on_demand` is reachable via user `@mention` ✅ RESOLVED (user)

The audit framed this as "drop it or defer it". **Both were wrong** — the user
supplied a third answer that dissolves the objection.

Design §1 rejects addressing because *"addressing is unpredictable. This is the
only model where the user always knows who acts next."* The rejection is about
**unpredictable ordering**, not addressing as such. And the very next bullet
already grants the user the power to redirect the ring: *"a user message resets
the cycle to participant 1."*

So a **user** `@mention` is not a new mechanism — it parameterises a sanctioned
one, changing the reset target from "participant 1" to "the named participant".
The user chose, so they still know who acts next.

| who mentions | ordering | verdict |
|---|---|---|
| **user** | predictable — they picked | **allowed** |
| **participant** | emergent | **forbidden** — this is what §1 deleted, and inventory row #1 dissolves the L2 hard cap *on the grounds that a fixed ring cannot ping-pong*. Participant mentions would falsify that. |

**There is no per-message addressing, and none is needed.** The mention selects
**who wakes**, not **what they receive**: §1 already guarantees a woken
participant reads every row after its cursor — *"context completeness is
structural, not a forwarding discipline"*. Routing one message to a specialist
would give it LESS context, not more.

Mechanics: parse `@slug` (already `UNIQUE (session_id, slug)`); `UserMessage`
carries an optional target; `advance_turn`'s reset passes that participant
instead of `None`. An addressed `on_demand` participant takes exactly one turn —
`next_active_participant` already skips it afterwards, so nothing puts it back to
sleep. Unknown slug = ordinary prose, never an error. Works on `active`
participants too (`@brian look at this` jumps the queue).

**Consequences:**
- An `on_demand` participant's cursor never advances while dormant, so a
  first-time wake delivers the WHOLE session. Correct, and heavy.
- Therefore **C1 is a hard prerequisite**: the live run measured the native loop
  turning one row into one API turn (87 rows → 82 turns, 7.5M prompt tokens).
  Shipping `@mention` before the fold fix would be a footgun on exactly the roles
  it serves. Order is forced: **C1 → @mention/on_demand → B8 offers the mode.**
- An addressed `on_demand` participant does NOT vote (§1b excludes non-actives)
  but its substantive output DOES reset the tally — the actives re-agree after a
  specialist speaks.

## D2 — Round cap default: **500**, `0` = off ✅ RESOLVED (user + measurement)

Both proposed defaults were wrong, and the user's own data said so. Measured over
**3,561** uninterrupted stretches in the live database (2026-08-11):

| p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|
| 7 | 24 | 84 | 167 | **294** |

A cap of 40 fires on >5% of real work; **100 would have killed the top four
stretches** (294 / 184 / 170 / 167) — legitimate overnight runs. A backstop bounds
a runaway, which is unbounded; it only has to sit above legitimate work.

`0` = the backstop is off, for deliberate unattended runs. NULL = inherit the
default. **The per-session override ships in the slice, not deferred to B8** —
otherwise the default IS the policy for every user, and the user explicitly noted
other users will differ.

⚠ **Unit caveat, unresolved:** the measurement counts agent *text messages*; a
ring "round" is one pass over N participants, so 294 messages ≈ 147 rounds at
N=2. The implementer must define the unit precisely and re-derive from this same
dataset rather than inheriting 500 as a number.

## D3 — Capability deny-text is GENERATED, not deleted and not hand-written

Design §2 says layer 2 is derived from the capability set and *"lacks it → the
text is absent rather than misleading"*. Read literally that deletes EYES's
~1,100 chars of explicit refusals.

**Decision: generate the denials from the ABSENT capabilities**, same source as
the grants. Drift is still impossible — both sides derive from one set — and the
agent keeps the "don't try X" guidance that saves a failed call. Deleting obeys
the letter and loses real value; hand-writing is the drift §2 exists to prevent.

## D4 — Peer names in prompts are generated from the live roster

`"Rain (EYES) can file BLOCKING findings on your work"`, `"don't rely on Brian
reading chat"` are hardcoded. Under N user-named roles these are **roster facts,
not constants** — generated at spawn from the actual roster. Same job as the
[name removal](2026-08-11-agent-name-removal.md), done once.

## D5 — Migration parity: line-multiset identical, with a declared heading diff

Byte-identity is achievable for HANDS but not EYES without a per-role ordering
manifest. Prove no line was lost, allow reordering, record the new headings.
Byte-identity buys ceremony, not safety.

## D6 — Restore `## Observations only` to native EYES (a latent bug)

`strip_claude_code_tool_inventory` currently strips it from the NATIVE prompt and
no test notices — native Rain has been running without an instruction CLI Rain
receives. Under the new structure it is layer 3 and survives.
**Rain's behaviour will change.** It was lost by accident, not by decision.

## D7 — A capped halt posts a visible row

Otherwise a capped session is indistinguishable from a completed one from inside
the app. Accepted against the known cost: the `system_notice` lane already
carries five injections at one-line sizing (B5 task 16).

## D8 — NO Agents tab. The Roles tab owns the default model ✅ (user)

The earlier recommendation — keep the Agents tab temporarily as "Model
assignment" — was **rejected**. Instead:

- **Roles tab** gets a **Default Model** select per role → `roles.default_model_id`.
- **New Session dialog** can override it per agent when adding one →
  `session_participants.model_id`.

**Both columns already exist** (0044). No new schema, no transition tab, no
window where models cannot be changed. `src/tauri_cmd/agent_configs.rs` and the
Agents tab are deleted outright in B7/B8 rather than renamed.

## Locked without controversy

`pass_turn` as the tool name · pass rows `origin='participant'` · no new roster
column for passes · no envelope tag for an overridden pass · role prose in
`roles.description_prompt`, NOT the Context Library · role deletion is archival ·
implicit turn order (row order) · slugified role slugs with numeric suffix ·
migration-literal seeding with a `#[cfg(test)]` Rust oracle · the native fold is
unconditional, never gated on `BOT_HQ_SEQUENCER` (the tested path must be the
shipped path).

---

## Briefs invalidated by these decisions

- **A3** — mapped against the obsolete "drop vs defer" fork. Re-run against D1.
- **B8** — mapped assuming the Agents tab survives. Re-run against D8.

The other four are unaffected.

---

## D9–D12 — decided 2026-08-12, after the first live rc3 test sessions

Recorded the same day they were made. The five-day drift happened because
decisions stayed in conversation; these are rows, not prose.

### D9 — the claude CLI is the connector. The native loop is DELETED.

The user: *"I actually want to commit using the claude cli as the model
connector, defer the native loop/connector as a plugin I'll build in the future.
The reason is uniformity."*

`src/agents/native/` (6,290 lines) and its 119 call sites come out, along with
the `native` model flag, `may_run_native`, the native spawn branch and the
native toggles in the UI. **Git history is the archive** — the future plugin
starts from `git show <sha>:src/agents/native/`. Chosen over dormant-code and
feature-flag options because a second runtime that nobody builds still costs
every reviewer a re-read and every refactor a second case.

Consequence to keep in view: this retires D6 (`## Observations only` on the
native EYES prompt) and every "the native loop cannot keep this promise"
carve-out, because there is no native loop.

### D10 — Brian and Rain are gone, data included. History stays legacy.

The user: *"I already said multiple times that I'm dropping the names, only the
Role + Model Name"*, and *"brian and rain's history can be legacy data. I don't
want to see brian and rain anymore moving forward."*

- A participant is displayed as **Role + model name**, nowhere as a person's name.
- `session_participants.slug` becomes role-derived; `spawn_session_handle` stops
  binding rows with literal `"brian"` / `"rain"` lookups.
- The agents' own prompts stop saying `You are **Brian**`.
- **Existing `messages.author = 'brian' | 'rain'` rows are NOT backfilled.** They
  are legacy data and must keep rendering. Migration 0049 covers new structure,
  not history.
- The user notes the display name remains theirs anyway: *"I can technically
  still name them brian and rain by changing model name (model id is what
  chooses the model anyway)."*

**This is what lifts the 2-participant cap.** The cap exists only because spawn
binds by literal slug.

### D11 — the New Session dialog warns from CAPABILITIES, never from role meaning

The user, correcting a framing that assumed the product knows what a reviewer
is: *"how would bot-hq know EYES are reviewers? Maybe warn that no participant
can edit files (participant list has no write capabilities ticked)."*

Right, and it generalises. bot-hq must not encode what any role MEANS — that is
the user's to define. It knows only the ticked boxes. So the dialog computes the
union of the picked participants' capabilities and names what the set cannot do
— e.g. **no participant holds `edit_files`, so this session can review but not
act**. Duplicate roles are NOT blocked and NOT special-cased; two of the same
role is simply one case that can produce that warning.

### D12 — effort level is per participant

The Create New Session dialog's effort section is currently two fixed
Brian/Rain blocks. It becomes one effort select per participant row, alongside
that row's role and model. Follows D10: nothing in that dialog is keyed on an
agent name.

### Confirmed working in the 2026-08-12 live tests

Session `s-15090a`: gates and questions parked on the tray correctly, and
`close_session` worked when asked. Session `s-bc50f2` surfaced D10 and D11.
