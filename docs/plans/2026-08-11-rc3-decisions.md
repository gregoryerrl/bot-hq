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

### D13 — `rain_disabled_default` is deleted, not renamed

The user: *"There's no 'disable rain by default' on rc3, thats moot. Just don't
add the role to your session creation."*

Right — the setting is a pre-rc3 answer to a question rc3 deletes. Solo-vs-duo
was a toggle only because the roster was fixed at two; now the New Session dialog
picks the roster, so "start solo" is just not adding a second participant.

Remove: the toggle in Settings → Policy → Session defaults, the
`app_settings.rain_disabled_default` key, `Storage::default_rain_enabled`
(`src/storage/models.rs`), and its two readers at `src/tauri_cmd/sessions.rs`
and `src/core/state.rs`.

**Consequence that needs an answer, not a silent default.** Those two readers are
the create paths with NO dialog — the Maintain-CL button and plugin-created
sessions. They have no user to pick a roster, so deleting the setting leaves them
needing one. Per design §1 ("how many agents, **default 1**") they should seed a
single participant rather than the historical pair. Whatever is chosen must be
stated where `ensure_session_roster` seeds it, because it is now a product
default with no UI behind it.

### D14 — `AgentEvent::Error` is deleted

The user, on the variant D9 left with no emitter: *"Delete it. I'll think about
the plugin later."*

Remove the variant, its handler in `src/core/duo.rs` and its two tests. The
native loop was its only producer; a future connector plugin can reintroduce the
rendering path it needs rather than inheriting a dead one kept on speculation.

### D15 — the Maintain CL button goes; sessions write their own learnings on close

The user: *"should we remove the maintain CL button and just have the sessions
automatically do a short maintenance on close session? User can still have a
dedicated session into maintaining it but they have to do it manually."*

Agreed on removing the button, with one correction to the second half, made after
reading what the button actually dispatches.

**Remove the button.** It exists only because there was no other way to start a
scoped, pre-prompted session. The New Session dialog now picks roles and takes a
prompt, so "start a session and tell it to maintain the CL" *is* the button minus
a modal, a hardcoded prompt, a bespoke command and a dialog-less create path.
Removing it also resolves half of [D13] — one of the two pathless creators
disappears.

Deletes: `frontend/src/app/MaintainCLModal.tsx`,
`frontend/src/lib/maintainClPrompt.ts` (+ its test), the button and modal wiring
in `frontend/src/app/ContextManager.tsx`, and the `dispatch_session` command in
`src/tauri_cmd/sessions.rs` if the plugin arm does not still need it.

**But close-time maintenance is NOT the Maintain-CL job.** Two facts settle this:

1. `maintainClPrompt` is **project-wide housekeeping over a full IPAV cycle** —
   ground-truth the whole library against the real repo, prune as much as you
   add, rescan. "Short" and that job are incompatible: a shallow version cannot
   ground-truth, and a real one on every close is expensive.
2. **Agents already write CL entries mid-session.** The library's own log is
   `cl: bcc-ad-manager/conventions.md (brian)`,
   `cl: bot-hq/learnings-…-b5-channel-batch.md (brian)`. The per-session capture
   is largely happening already; what is missing is a session that ends before
   anyone writes anything down.

And a hazard that rules out the naive version: **the Context Library is a git
repo and the user runs concurrent sessions.** Project-wide maintenance fired on
every close means two sessions closing near each other rewrite the same
project's library at once — generating merge conflicts in the knowledge base as
a side effect of closing a tab.

**So, split by scope:**

- **On close** — the session writes *its own* learnings, if it has any worth
  writing. Scoped to what happened in that session, so no cross-session
  contention. This formalises what agents already do ad hoc.
- **Library-wide maintenance** — a manual session the user starts and instructs.
  No button, no special path, no hardcoded prompt.

**Close must not become blocking.** Closing is the user saying stop. The
learnings write is fire-and-forget with a visible row: a failed or slow write
never delays the close and never leaves the session un-closable. If the session
has no participant holding `write_context_library`, it is skipped silently —
that is a capability answer, not a special case.

**SETTLED — an empty-handed session writes NOTHING to the Context Library.**
Not a marker, not a stub, not a "nothing worth keeping" line. The user's reason,
which is stronger than the bloat argument it replaces: *"empty handed sessions
might risk corrupting the CL."*

That is the right frame. An agent with nothing to say, prompted at close to write
its learnings, does not return empty — it produces plausible filler. That filler
lands in a layer whose entire purpose is that future sessions **orient from it
instead of re-reading the codebase**, so invented content is not noise to be
pruned later, it is fabricated knowledge presented as experience, and it
compounds: the next session reads it as fact and builds on it.

Implementation consequence: the close-time write must be genuinely optional, and
the prompt must make "write nothing" an expected, blameless outcome rather than
a failure to complete a task. A close-time instruction phrased as *"write what
you learned"* produces filler by construction; it has to permit, and expect,
silence.

Still open, and smaller: a fire-and-forget write that FAILS looks identical to
one that correctly declined. Record the decision where session state already
lives — a visible row, as the capped halt posts one — so the two are
distinguishable without the Context Library carrying anything.

### D16 — `close_session` becomes a real capability, and the user is the fallback

Decided 2026-08-13, closing the `PARITY_HOLD` question rc3 left open.
**SHIPPED 2026-08-13** (`6cc2b9a`) — `PARITY_HOLD` is empty, the UI Close path is
pinned as ungated across both its files, and the parity oracle names
`close_session` as its one sanctioned divergence rather than skipping it.

The user: *"close session tick on **role** capabilities. if no agents are
ticked, then user must be the one to manually click the close button if they
want to close."* (corrected from "agent capabilities" in the same breath —
capabilities are a property of the ROLE, snapshotted onto the participant at
invite time. `session_participants.capabilities` is what the gate reads; the
role is where the user ticks them.)

**Remove `close_session` from `jsonrpc::PARITY_HOLD`** and let it gate on
`Capability::CloseSession` like every other tool — read from the participant's
invite-time snapshot of its role's ticks, which is how every other capability is
already resolved. Today the hold makes it
ungated, so every participant can close a session regardless of what its role
ticks — while the generated prompt already tells a participant without the
capability that it may not. The prompt has been the honest half all along; this
makes the runtime agree with it.

**Consequences to build deliberately, not to discover:**

1. **A roster where nobody holds `close_session` is a LEGAL configuration**, not
   an error. It means the session ends when the user says so — the Close button
   in the UI, which must work regardless of the roster and must never be gated
   on a participant's capability. Verify the button path does not route through
   the same gate before assuming it is fine.
2. **The seeded `eyes` role does not hold `CloseSession`.** So after this lands, a
   HANDS + EYES session where only HANDS holds it behaves as pre-rc3 did; a
   session of EYES alone can no longer close itself. That is the intended
   behaviour change and the reason this was held for a decision.
3. **The refusal must be legible.** A participant that tries to close without the
   capability should get the standard capability refusal — and, since rc3 P2, a
   visible row saying so. A session that silently will not close looks like a
   hang.
4. `parity::UNGATED` and `PARITY_HOLD` both carry `close_session` today, and the
   parity oracle asserts the pre-rc3 answer for it. Both move together; the
   oracle's expectation for this one tool changes from "admitted" to "gated", and
   that edit is the point rather than a test getting in the way.

**Definition of done:** `PARITY_HOLD` is empty or gone; a participant without
`CloseSession` is refused and the refusal is a row; a participant with it closes
as before; the UI Close button closes a session whose roster holds the capability
nowhere; and the parity oracle states plainly that this one tool deliberately
diverges from pre-rc3 behaviour, with this decision as the reason.

### D17 — `on_mention`: a participant you summon, for exactly one turn

Decided 2026-08-13. **SHIPPED 2026-08-13** (`5c17818`). This closes **A3** in
[`2026-08-11-design-drift-audit.md`](2026-08-11-design-drift-audit.md), the last
of the three decisions that audit found existing "in neither the code nor any
batch" — the other two shipped as the round cap and PASS.

**Why it was stuck.** Design §1 defined the mode as *"not in rotation; reads;
posts only when addressed"* — and the same section **deleted per-message
addressing** as one of the things the ring replaces. Both halves were written in
one document. The rotation half works (`next_active_participant` already skips
the mode); the addressed half was specified in terms of a mechanism being
removed. So the mode has been, in the audit's words, *"a way to build a
participant that cannot participate"*.

**The mechanism, from the user:** *"if mentioned the next turn automatically goes
to that agent, then after his turn, the agent will be omitted from the ring until
he gets mentioned again."*

This is a **wake target, not addressing** — which is exactly what D1 already
settled. The ring still hands out one turn at a time; a mention only chooses who
holds the next one. It fits the existing machinery: `SequencerCommand::UserMessage`
already resolves the next holder by passing `None` (= reset to the front of the
rotation), so a mention is a third input to a decision the ring already makes —
neither "resume" nor "reset" but "hand it to this one".

**Rename the mode `on_demand` → `on_mention`** (display: "On mention"). The name
should say how to trigger it, and now that the trigger is literally `@`, it does.
Free to do: zero `roles` and zero `session_participants` rows use the old value,
so it is the constant, `PARTICIPATION_MODES`, any CHECK constraint, and the
picker — no backfill.

**Settled sub-decisions** (all four confirmed by the user, so an implementer does
not re-derive them):

1. **Only USER messages may mention.** Enforce it in the parser, not in role
   prose. Participant-to-participant mentions stay forbidden (D1), and the reason
   is concrete: HANDS mentions the advisor → the advisor speaks and mentions the
   reviewer → the reviewer mentions HANDS, an unbounded summon loop in which
   **every turn is substantive**, so the consensus tally cannot fire (each
   `Spoke` clears it), spin detection cannot fire (the content differs each
   time), and no pass is ever cast. Only the 500-lap round cap would end it, at
   one real model call per lap on the most expensive role in the session. This is
   the same shape as the pass volley: individually-correct behaviours composing
   into a spin nothing catches.
2. **The mention UI is a picker, not free text.** Typing `@` opens a list of THIS
   session's participants; arrow keys or click to choose (Discord-style). That
   makes mentioning a non-participant **impossible to express** rather than an
   error to report — prevention over detection, and it removes the "mentioned
   someone not in the roster" case entirely.
3. **Multiple mentions queue**, in the order written. `@advisor @security` gives
   the advisor the next turn and the security role the one after.
4. **After the summoned turn, the rotation resumes where it was.** A mention is
   an INSERTION, not a reset — otherwise summoning someone silently restarts the
   cycle at participant 1. The summoned participant then drops back out until
   mentioned again.
5. **A summoned substantive turn clears the done tally**, like any other. That is
   correct rather than incidental: summoning an advisor into a converged session
   should un-converge it — nobody calls one in to rubber-stamp an arrival.

**Already handled, listed so it is not re-litigated:** an `on_mention`
participant never votes, because `all_active_voted_done` filters on
`participation_mode == "active"`. So it cannot block a halt by existing, and —
because it holds the turn for exactly one turn — it cannot hold one open by being
mid-thought either. That resolves the open `on_demand`-and-the-halt question
raised alongside P8.

**Definition of done:** a role can be set to `on_mention` in the Roles tab; `@`
in the chat input offers only this session's participants; a mention hands that
participant the next turn and no more; the rotation resumes where it left off; a
mention typed by an agent does nothing; and the whole path is pinned by a test
that would fail if a participant's mention were honoured.

**Settled while building it** (2026-08-13), so they are not re-derived:

1. **An `on_mention` participant IS spawned.** It was excluded from
   `resolve_spawn_roster` while nothing could wake one. A summons cannot reach a
   process that does not exist, and spawning lazily on the first mention would be
   a SECOND way into the rotation — the shape D19 spent a day deleting. Cost: one
   idle subprocess, no tokens until it is fed. It does not trip the
   reviewer-down commit gate: the stall verdict requires `busy`, and a
   participant that has never held a turn is not.
2. **The ring steps from an ANCHOR, not from the holder.** The anchor is the last
   participant to hold a RING turn, and a summoned turn does not move it. That
   one rule is the whole of "the rotation resumes where it was". Stepping from
   the holder passes every other test in the file — mutation-verified.
3. **A user message clears the done tally whether or not it names anyone.** The
   clear used to ride the restart-to-the-front, which a mention deliberately does
   not do. Left bound to the restart, a summons after a converged halt leaves the
   old votes standing, the summoned participant passes, and the session halts
   again with the actives never having read the message that restarted it.
4. **An unresolvable mention never reaches the ring.** `resolve_mentions` drops a
   slug that names nobody, so the message arrives as a plain reset — which is
   what "an unknown slug is ordinary prose" means operationally. The sequencer's
   own drop path covers only the race where a participant leaves between the
   mention and the turn.
5. **The picker matches on word prefixes, not substrings.** The label carries the
   model (`EYES · Claude Opus 5`), so `@e` under substring matching offered every
   participant running Claud**e** and Enter inserted the wrong one.

### D18 — delete `observer`; two participation modes, both of which do something

Decided 2026-08-13, alongside [D17]. **SHIPPED 2026-08-13** (`4c431b4`). The user, on being shown what an observer
actually does: *"delete it, so we have two role types that are actually useful?
Active and On Mention"* — yes.

**`PARTICIPATION_MODES` becomes `["active", "on_mention"]`.**

**Why it goes.** Design §1 specified `observer` as *"not in rotation; **reads**;
doesn't post."* The reads half was never implemented, and the mode is worse than
inert — it is inert **and** expensive:

| | |
|---|---|
| Spawned? | **Yes.** `resolve_spawn_roster` filters `enabled && participation_mode != "on_demand"`, so an observer gets a full claude-code subprocess, its own context window and its own bill |
| Handed a turn? | No — `next_active_participant` filters on `active` |
| Delivered anything? | **No.** A turn is a PULL: "a participant's cursor is offered to it when its turn comes". No turn, no delivery, the cursor never moves |
| Posts? | No |
| Votes? | No — `all_active_voted_done` filters on `active` |

So it starts a subprocess that reads nothing, says nothing, and bills for
existing. **This is the same defect as `on_demand`'s** — a mode specified with a
capability the ring does not grant it — and it hid longer only because an inert
participant does not spin, it just quietly costs a process.

**Why not fix it instead.** Once `on_mention` exists, ask what "reads but
structurally can never speak" is for. The candidate answers do not survive:
*accumulates context for later* — it is delivered nothing; *promote it to active
mid-session* — participation mode is not editable on a live session; *a silent
auditor* — with no delivery it audits nothing, and with delivery it is
`on_mention` that you never mention. Two modes that both do something beats
three where one is a trap.

**Scope — no data migration.** Zero `roles` rows and zero `session_participants`
rows use `observer`, and the column has no CHECK constraint (only
`NOT NULL DEFAULT 'active'`), so this is code plus the picker:

- `PARTICIPATION_MODES` in `src/storage/participants.rs`.
- The Roles-tab picker (which already offers only `active`, since it omitted
  `on_demand` under D1 — after D17 it offers `active` and `on_mention`).
- `RoleDraftInput` validation, which reads the constant and needs no edit.
- **Retarget, do not delete, `an_observer_is_skipped_not_given_a_no_op_turn`**
  (`core/sequencer.rs`). Its subject — *a non-active participant must be skipped
  rather than handed a wake it cannot use* — survives as the property that keeps
  `on_mention` out of the rotation, and it is the pin that stops a future change
  putting it back. Rename it for `on_mention` and keep the assertion.

**Definition of done:** the picker offers exactly `active` and `on_mention`;
storage refuses any other value; the skip property is still pinned by a named
test under its new subject; and no code path spawns a participant that cannot
take a turn.

### D19 — the ring is the only delivery path

Decided and shipped 2026-08-13, after three live sessions diagnosed it.

**The defect.** `broadcast_user_message` fanned the user's text into EVERY
participant's stdin, and three other paths did the same: the session-start
CL-opener nudge, the paused-wake replay, and the tray-answer receipt. A
participant woken that way begins a turn the ring never handed it, so the pump
snapshots its epoch before one has been published and carries `0` forever. Every
completion it sends is then discarded and the cycle cannot step past slot 0.

Measured in `s-cc30fc19`: slot 0 carried epoch 3, slots 1 and 2 carried 0, and
the ring advanced exactly one place per user message. This also explains the
2026-08-12 stall attributed at the time to the idle watchdog — the ring was not
idle, it could not step.

**The rule: the ROW is the delivery.** A caller posts the row and tells the ring
(`notify_ring_user_message`); the ring hands the turn to the front of the
rotation and each participant reads the row off its own cursor when its turn
comes. Ordering matters — notify AFTER the row is persisted, or the woken
participant drains an empty backlog and the message lands a turn late.

This is the argument the router deletion already made, with `broadcast` in the
router's place: **two paths delivering into one stdin, only one of which the ring
can reason about.**

### D19a — a participant reads prose, never a peer's tool plumbing

`channel_page` had no `kind` filter, so the drain handed each participant every
peer's raw `tool_use` / `tool_result` JSON. The router had forwarded a turn's
buffered PROSE; the ring drains rows, and tool calls are rows.

Observed in `s-0d063183`: a participant was delivered
`{"input":{"project":"cognotify"},"name":"…cl_index_search"}` and spent a turn
correctly objecting that it was an envelope, not a message.

Not merely noise. `tool_result` bodies are file reads, git output and CL dumps,
so every participant was paying to read every peer's plumbing on every turn —
the most plausible cause of the `Prompt is too long` that killed a participant on
a 1M-token model.

Fixed: reads FOR a participant exclude `tool_use` / `tool_result`; the UI read
stays unfiltered so the transcript still shows what agents did.

### D19b — the ring records who holds the turn

`sessions.current_turn_participant_id` shipped in 0044 and **nothing ever wrote
it**. So a participant that had simply not been reached yet was
indistinguishable from a dead one — reported from a live N=3 session where only
the first participant acted for two minutes. `hand_over` now records the holder.
Best-effort: a failed write costs a UI hint, never a turn.

### D20 — a participant's label is the user's, and colour rotates

Deferred to a session, 2026-08-13. **FIRST HALF SHIPPED 2026-08-13** (`f3f4809`):
the ordinal. The user-set label and its editor are still open.

**What shipped, and why it was enough to close the complaint.** The ordinal is
read off the SLUG — `first_free_slug` already assigns `eyes`, `eyes-2`, `eyes-3`
at invite time — so the visible name and the internal key agree by construction.
A count over the roster would be a SECOND numbering, and two numberings of one
thing disagree the first time a participant is disabled. The first of a role
takes no suffix, so a one-reviewer session is unchanged.

**Colour needed no separate mechanism** — ❌ **WRONG, and disproved within the
hour.** The claim was that `authorColor` hashes the LABEL, so distinct labels
give distinct hues and the rotation falls out. Distinct STRINGS were never the
constraint; distinct OUTPUTS were, and the palette held exactly **two** hues
against a roster that caps at four. At N=3 a collision is pigeonhole, not
chance. The user reported it from the next live session: *"HANDS and EYES-2 have
the same color."*

Shipped properly in `f78d58e` + `93eed52`: eight named hues, assigned by roster
position, with a per-participant override (migration 0052 stores the palette
entry's NAME). **Verified live in `s-991f7416`** — three participants, three
distinct colours, confirmed on screen.

The lesson worth keeping is the shape of the error rather than the fix: the
comment beside the two-hue palette said a repeat was *"a shared colour, not a
wrong one"* — true when a session held two participants, false the moment the cap
moved to four, and nobody revisited it. A claim that was correct under an old
constraint reads exactly like a claim that is correct.

**Four frontend fixtures already modelled the exact case** (role `EYES`, slug
`eyes-2`) and asserted the collision as correct. Their expectations were the bug.

**Still open:** the user-set label that overrides the ordinal, and its editor in
the New Session dialog. That half needs a column and UI; the ordinal is what the
reported complaint was about. Reported from a live N=3 run: *"for the 2
reviewers, i don't know which is which."* Two participants of one role on one
model render identically — `EYES · DeepSeek V4 Pro`, character for character —
because the display rule has no ordinal and colour is keyed to the role.

The user's spec, settled:

- **A participant carries an optional user-set LABEL**, editable where the
  participant is chosen (New Session dialog) and shown wherever it is named.
- **Empty falls back to an ordinal**: `EYES`, `EYES-2`, `EYES-3` — matching the
  slug scheme (`eyes`, `eyes-2`) so the visible name and the internal key agree.
  The first of a role takes no suffix.
- **Colour ROTATES** so two participants of one role never collide. The user may
  override per participant; rotation is the default, not a fallback.

Notes for whoever builds it: `authorColor.ts` currently maps the two legacy
slugs only, so every role-derived slug already falls through to neutral —
rotation replaces that, it does not have to preserve it. Colour is keyed to turn
slot rather than to role (see ARCHITECTURE's design-system section), which is
already the right hook for a rotation.

### D21 — a BOOT phase, so participants orient in parallel

Proposed 2026-08-13 by the user after watching a three-participant session where
only the first acted for two minutes: *"how can we make it so all agents can boot
at the same time? … an 'Opening' or 'Boot' phase where they load CL and process
initial instructions."*

**The problem is real and the ring causes it.** Orientation — reading the CL,
the conventions, the task — is per-participant work that depends on nothing and
contends for nothing. Serialising it through the ring costs N × the orientation
time before any work starts, and a participant that has not been reached yet is
invisible from the outside (D19b helps, but latency is latency).

**Agreed, with one refinement that decides whether it works: BOOT IS
ORIENTATION, NOT WORK.**

Every participant may read in parallel. No participant may *act* in parallel —
that is the free-for-all D19 just removed, and it is what produced three agents
editing blind in `s-be58fdf0`. Concretely:

1. Spawn every participant, then feed each its own primer (CL index, conventions,
   the task text) directly — in parallel, since nothing is contested.
2. **Boot output is persisted and shown to the USER, but not delivered to
   peers.** Three near-identical "CL loaded for cognotify" rows are exactly the
   noise the channel does not need, and a peer reading them learns nothing. This
   is why D19a lands first: the kind filter is the mechanism a boot-kind row
   rides on.
3. **No participant sends a completion during boot.** There is no holder, so
   there is no epoch — a completion here is precisely the discarded-forever case
   D19 fixed. The pump has to know boot from a turn.
4. When every participant has finished orienting — or a timeout fires, because
   one slow agent must not hold the session — the ring starts and hands turn one
   to the front of the rotation.

**The hard part is (3), and it is where this will break if rushed.** The pump
currently learns a turn started from its first event, which is exactly the
assumption that made the broadcast fan-out fatal. Boot needs an explicit signal
that a participant is orienting rather than holding a turn, and the ring must not
be startable until that phase is over.

**Open question for the implementer, worth answering with a measurement:** does
the task text belong in boot, or only the CL? Putting it in boot means every
participant has read the task before anyone acts, which is the point. But it also
means three agents have *opinions* ready and the first turn arrives into a room
where everyone already decided. Try it both ways on a real session before
settling it.

### D22 — a parked question finishes the lap before it halts

Decided and shipped 2026-08-13, from `s-e8a20797`. The user, shown the three
options: **"Finish the lap, then halt."**

**The defect, which is a COMPOSITION rather than a bug.** Every mechanism
involved was individually correct:

1. A participant ends its turn with `ask_user_choice`.
2. That parks a question, which halted the ring where it stood.
3. The user answers; the answer is a user message, which restarts the cycle at
   the FRONT of the rotation.
4. Which is the same participant. Go to 1.

So a participant that asks the user something at the end of each turn makes its
peers **structurally unreachable**. Measured in `s-e8a20797`: HANDS held every
turn of a seven-minute session, four deliveries to slot 0 and **zero** to slots 1
and 2, with both EYES subprocesses alive and their MCP connections initialised.
The session before it (`s-81057bde`) diagnosed the same shape from inside and
called it "EYES has never been handed a turn" without naming the halt as the
cause.

**This is the rc3 reframe's own requirement failing.** Before rc3 the bilateral
router forwarded the executor's output to its peer regardless of any halt, so the
reviewer read everything and reviewed it. The ring turned that forward into a
turn, and a halt stops turns. The user's brief was explicit: *"What worked
previously before rc3 must also work on rc3 — HANDS and EYES will work
adversarially."* A configuration where the reviewer cannot speak does not.

**The rule.** A park ends the ASKER's turn instead of stopping the ring. The
rotation carries on; the cycle halts when it comes back around to a participant
that is blocked. Bounded at **N-1 extra turns** — one each for the participants
waiting on nothing — which is exactly the adversarial pass the roster was built
for. The reviewer sees the work while the user decides, which also makes the
answer better informed.

**Mechanics:**

- `QuestionParked` carries `participant_id: Option<i64>`, resolved from the
  caller's slug by the bridge — the layer that holds both the session and the
  roster. `None` falls back to halting outright, which is the safe direction and
  what every park used to do.
- The sequencer keeps a `blocked` set in its own frame. A user message clears it
  BEFORE the ring is stepped, or the release would re-halt on the spot.
- Only a park naming the HOLDER ends a turn. One naming anybody else records the
  block and leaves the live turn alone; stepping there would put two participants
  on a turn at once.
- The drain stops for a park only when it names the participant being fed. While
  a park stopped the cycle outright, cutting any drain short was right; it now
  leaves other turns live, and stopping would hand the holder a partial backlog
  for nothing.

**What this deliberately gives up.** "One participant blocking on the user stops
the cycle regardless of what the others would have done" was the documented
meaning of *unilateral*, and it is now narrower: unilateral in that no vote is
cast and the cycle does stop, but not immediate. The cost is at most N-1 model
calls per parked question while the user is away.

**Verified by mutation**, three ways: halt at the park instead of stepping, never
halt on reaching a blocked participant, or leave the block set uncleared by a
user message — each reddens the tests written for it, and the suite is green with
all three restored.

### D23 — a delivered row says who wrote it

Shipped 2026-08-13 (`411ee95`), from three sessions' worth of confusion that all
trace to one gap: **the wire carried no author at all.**

`render_wire` rendered the envelope (phase, findings banner, system prefix) and
the body. Nothing said who the body was from. A participant handed four rows
received four anonymous strings and had to infer which was the user's task, which
was a peer's aside, and which was the host talking.

**The evidence, in order of how badly it read:**

| session | what it looked like | what it was |
|---|---|---|
| `s-81057bde` | a reviewer reporting "no task from the user and no HANDS output" | it had been delivered eight rows and could not tell what they were |
| `s-534b8761` | HANDS describing three messages "injected alongside a tool result instead of arriving as its own turn" | its own turn's opening backlog, delivered as three unlabelled stdin writes |
| `s-be58fdf0` | the user: *"for the 2 reviewers, i don't know which is which"* | the same problem one layer up (D20) |

**The rule.** Every wire leads with `[speaker]` — the participant slug for a peer,
`user`, or `system`. The slug rather than the display name (`ROLE · Model`)
because it is ON the row, so labelling costs no lookup and cannot go stale
mid-session, and because it is the handle `@mention` parses: what a peer reads is
the string the user would type to summon it. D20's user-set label supersedes it
when it lands — the label peers read and the label the user reads should be one
string.

**`user` and `system` are deliberately distinct.** A system notice is bot-hq
talking. An agent that reads one as the user has been handed a fabricated
instruction, which is the failure the general rules are built around.

**On the forty-five tests that did not change.** The sequencer's tests assert
ROUTING and name rows by content only to identify them. Threading the speaker
through all of them would state it forty-five times and test it nowhere — worse,
an expectation written from observed output accepts a WRONG name as readily as a
right one. The shared helper strips the prefix; the label is pinned where it is
the subject.

**Still open, and deliberately not bundled:** `deliver_backlog` writes one stdin
message per row, so a turn's backlog is N writes and rows 2..N land inside the
turn row 1 opened. With the speaker on every row this is much less confusing than
it was, which is exactly why it should be measured before being changed.

### D24 — a straggler must not bind the next turn's epoch

Shipped 2026-08-13 (`d392f05`), after `s-206e8921` stopped dead for nineteen
minutes with a live reviewer holding a turn it could never hand back.

**The defect.** `pump_agent` binds a turn to the epoch cell on the first event
after the previous completion. *"The first event after a completion"* is not the
same thing as *"the first event of the next turn"*, and the gap between them is
the bug: a participant that emits anything before the ring has handed it another
turn reads the cell as it stands — still the epoch it just completed with. The
real turn arrives, the guard sees `turn_epoch` already set, and every completion
from then on carries a number the ring retired. All discarded. The ring cannot
step past a participant it is waiting on, and nothing in the loop recovers it.

**The trigger is not rare — it is the user typing while a participant is
mid-turn.** The ring resets to the front, the preempt interrupt ends that
participant's turn, its completion arrives behind the reset and is correctly
discarded, and whatever it emits next binds the retired epoch. Both reviewers in
that session died this way, two minutes apart; one spoke exactly once in
twenty-nine minutes. The user's message that triggered it was, verbatim, a note
asking what happens if they type while the agents are working.

Measured: completed 03:56:01 carrying epoch 9 → handed epoch 11 at 03:56:28 →
completed 04:01:51 **still carrying 9**.

**The fix.** A cell that still reads what this pump last completed with means no
new turn has been handed out, so the event is a straggler and opens nothing. The
epoch strictly increases at every handover, which makes "unchanged" an exact test
rather than a heuristic.

**The test needed fixing before it could fail.** `send` only queues, so storing
the new epoch before the pump had processed the straggler meant the race never
happened — the first version passed with the guard deleted. It now waits for the
row the straggler persists, which is the only barrier that proves the pump ran the
binding code. Bypass the guard and it reports `left: 9, right: 11`.

**Recovery, for the record.** What unwedged that session was not a ring reset: the
user pressed Stop, the reviewer did not honour the interrupt (`cancel: interrupt
not honored in time — SIGKILL fallback`), and the next broadcast found the session
stale and rebuilt it — new sequencer, epochs from 1, all three participants
respawned. Worth knowing because the log's `sequencer: started` followed 20µs
later by `control channel closed; exiting` reads as a task dying instantly, and is
actually two different sequencers: the new one starting and the old wedged one
finally noticing its senders were dropped.

### D25 — a turn carries at most one pass

Shipped 2026-08-13 (`049e58c`), from a runaway caught live in `s-a4e9a1b4` — a
real work session, not a test.

**What happened.** `pass_turn` was called **141 times in the eight minutes after
the last handover**, one every two seconds, one real model call each. The user
noticed it on screen; nothing in bot-hq did.

The chain, and every link is a bug already known:

1. The executor parked a question. D22 worked — the ring handed the turn on
   rather than halting.
2. The reviewer finished its turn and its completion was **discarded** on a
   retired epoch (D24). From the ring's view it still held the turn, and no turn
   was handed out again.
3. The reviewer had nothing to review while a human was blocking, so it passed —
   and the tool answered *"pass noted"*. Nothing it could see said the pass had
   already been recorded, so it passed again. And again.

**The round cap could not have caught it, and that is the finding.** The cap
counts LAPS of the ring, and the ring was not moving at all — one turn that never
ended. The single backstop bot-hq has for runaway loops is structurally blind to
a participant looping *inside* a turn, because it measures the ring rather than
the spend. rc3 P8 anticipated a pass volley ACROSS turns, bounded by the cap at
500 laps; this is the same shape one level down, bounded by nothing.

**The rule.** A pass is the turn ending, so a turn carries at most one. The second
is refused with a message that names the attempt number and says to end the turn.

- **Counted bridge-side**, not in the pump: the refusal has to reach the AGENT,
  and the tool result is the only thing it reads synchronously.
- **Cleared by the ring** when it starts a turn for that participant —
  deliberately not by anything the participant does, because a turn that never
  ends is precisely the state this bounds.

**Two guards, not one.** D24 stops the loop from starting; D25 stops it running
away when something else wedges a turn. They are independent on purpose: D24 was
also once thought sufficient.

### The delivery-order problem, measured

Not a decision yet — evidence, recorded so the decision has something to stand on.

`s-a4e9a1b4` at 05:26:44 delivered NINE rows to the reviewer as **nine separate
stdin writes**:

```
1  [system]  Session idled with no question or halt parked — nudged the executor
2  [system]  [System: this session went idle…]
3-8 hands    six rows narrating a test run
9  [user]    prepare to close — commit what needs committing…
```

claude-code opens the turn on row 1 — bot-hq's own idle nudge — and rows 2..9
arrive DURING that turn as interruptions. So the user's actual instruction was
last in the queue, behind six rows of a peer's narration, and the reviewer spent
the turn reviewing the peer instead. The user: *"why does it feel like its not
addressing my current message?"*

Two fixes point at this and only one of them has shipped:

- **D23** (shipped) labels each row, so row 9 is identifiable as the user's at
  all. On the build that session ran, all nine were anonymous.
- **Coalescing** the backlog into ONE write is what fixes the ORDERING: in a
  single prompt the last line is the most recent instruction, which is the normal
  conversational shape. As nine writes, position 1 frames the turn and position 9
  is an interruption. The label does not fix this on its own.

An earlier note in this file said coalescing should be measured before being
built. This is the measurement.

### Live verification of D22–D25 (`s-d8773b42`, `s-991f7416`)

Two sessions on the fixed build, measured with `scripts/turn-latency.py` so the
before/after is the same query rather than a re-derivation.

| | `s-a4e9a1b4` (before) | `s-d8773b42` | `s-991f7416` |
|---|---|---|---|
| `pass_turn` calls | 209 (**45.4%** of all tool calls) | 6 | 5 (**8.6%**) |
| pass bursts | 9, the largest 178 calls over 10.5 min | 0 | 0 |
| turn end → handover | 9.5s | 1.0s | 5.0s |
| handover → first output | 9.5s | 10.3s | 9.3s |
| model pace | 0.6s | 0.4s | 0.6s |

**D25's refusal has teeth, and it takes one or two rather than none.** This was
the open question — a unit test can prove the tool returns an error, not that a
model reads it. In `s-991f7416` EYES passed, retried once, was refused, stopped;
EYES-2 passed, retried twice, was refused twice, stopped. Bounded at 2–3
attempts against 141, and structurally incapable of a runaway.

**The wedge is gone.** Every discard in both sessions is the ordinary supersede
shape (the ring had already moved on); no epoch-0 carrier, and no participant
carrying the same epoch twice.

**The latency split is confirmed rather than hypothesised.** The "turn ending"
half was the wedge and has halved or better; the "starting" half is prefill,
unchanged in every build, and is what the delivery-order work would address.

**Still true, and now the largest open item:** the user's message arrives buried
3 times in 4, including row 8 of 8. D23's label makes it identifiable; only
coalescing makes it last.

### D27–D30 — the session that force-closed, and everything it exposed

Shipped 2026-08-14 after `s-8ac0d2d0` was force-closed four minutes in. One
report — *"they volley on boot, that's why I had to stop"* — turned out to sit on
top of four separate defects.

**What actually happened, in order:** boot completed with no task given, so the
ring dealt turn one into a session with nothing to do. A participant handed a
turn with nothing to do can only pass — **and its pass is a row**, so the next
participant's turn delivers it, and that one passes too. Every pass generates the
input for the next, so the ring never runs out of something to hand over and
never converges. 23 provider calls in 77 seconds, ~240 KB each, producing
`(passed — nothing to add this round)`. The only floor was the 500-lap round cap:
over five hours.

The user pressed Stop, which SIGKILLed the agents, which made the session stale,
so the next message respawned it — and a respawn re-ran boot. Three boots in four
minutes. *"what, its still on boot phase?"*

| | |
|---|---|
| **D27** | a lap of nothing but passes yields to the user |
| **D28** | responding is ONE event: release the ring and clear the halt row |
| **D29** | boot ends by yielding, not by kicking; no boot on a respawn; input locked while orienting; the duplicate CL opener dropped |
| **D30** | the halt renders above the input box, as a recap |

**D28 is the one with the longest tail.** Three paths mean "the user responded" —
typed message, answered tray card, phase advance — and each did a different
subset. Answering a tray card released the ring and never cleared the halt row.
**52 occasions in the archive** where a second tray row opened while the first was
unanswered; the worst, one row sitting under six more for 53 minutes. The user
had reported it (*"answering a question didn't clear a halt, so they parked
another"*); I checked and said it had never happened, because I queried what was
pending NOW and every resolved pile-up was invisible to that. The report was
right and the method was wrong.

It is the third bug of this exact shape: a halt shipped with no release (D19), a
health verdict that reached the UI but no record (D26), a release with no clear.
**One event, two halves, N call sites, nothing making them travel together.** The
test does not check that each path clears the halt — each path looked fine alone.
It checks that exactly one place *can*.

**What the tests said when the condition was wrong.** D27's first cut yielded on
"nobody produced substantive output", which broke six tests: it also caught laps
containing `Done` votes and would have pre-empted the arrival the consensus tally
exists to reach. Narrowed to nothing-but-passes, two remained, and both changed
SUBJECT rather than expectation — an all-pass ring does still never halt *by
consensus*, and an identical pass is still not a spin, though that one now needs
N=2 because at N=1 every turn is a whole lap.

**And one assertion of mine was wrong in a way worth keeping:** a dropped kick
CLOSES its channel, so `recv` returns `None` immediately rather than timing out.
`is_err()` passes for a merely-slow sender and fails for the behaviour being
pinned.

**Still open.** The general mid-turn input lock (`c13fcdb`) remains a band-aid:
it closes the box whenever a turn is in flight, which is why Stop was the only
way to speak. Buffering — hold the message, deliver it at the next turn boundary
— gives the box back and removes the reason to press Stop at all. D29 removes the
boot loop that made it painful; it does not make the lock right.

### D31–D33 — Pause is the only real interrupt

Shipped 2026-08-14. D31 was a loose end from D28's afternoon; D32 and D33 came
out of two screenshots the user sent of a session claiming one thing while
visibly doing another.

**D31 — a refused handover takes its busy flag back.** The ring sets the next
participant busy, then discovers it is blocked and halts. The flag stayed set,
so the UI showed a participant working that the ring had already declined to
hand a turn to. Fourth instance of the same shape: *one event, two halves,
nothing making them travel together.* It was invisible because every sequencer
test but one passes `activity: None` — a fixture (`ring_with_activity`) had to
exist before the defect could be seen at all.

**D32 — HALT is a claim about the session, not about the tray.** The banner said
HALT whenever any row was pending, so parking a question printed "HALT" over a
session whose own status line, one line below, correctly named two participants
mid-turn. The user: *"parking a question in tray toggles the halt (it should
not), its asynchronous."* They were right, and it was the semantics rather than
the wording: `ask_user_choice` is non-blocking BY DESIGN. Only `halt` /
`mark_awaiting_user` is a participant saying it stopped.

#### D33 — the rule, and what it closes

The user set the destination — *"i want to build towards → Pause button is the
only real interrupt"* — and then corrected the shape I was building toward it:

> *"users are never allowed to type while agents are working, no halt = no type
> (except for pause button which is the real interrupt)"*

**This closes the item D27–D30 left open, in the opposite direction from the one
it recommended.** That entry called the mid-turn input lock (`c13fcdb`) a
band-aid and proposed buffering — hold the user's message, deliver it at the
next turn boundary. Buffering was never a decision, only my recommendation, and
the user's rule is strictly simpler: the lock is not a band-aid, it is the
design. No queue, no delivery point, no "it will land later" affordance, no
answer needed for what happens when the session halts before the boundary. And
no message can arrive mid-turn, which is the corruption the lock existed to
prevent.

The cost is chosen rather than discovered: **arriving at a working session costs
one extra click.** You open a session to fire a prompt and leave; if participants
are mid-turn you press Pause first.

**Locked ⟺ somebody is working**, and that is read from the per-participant busy
MAP, not the session enum. `SessionActivity::derive` ranks `awaiting` ABOVE
`busy`, so a parked question reported `awaiting_user` while two participants ran
— which is precisely the screenshot: an open textarea over a working session.
The collapsed enum cannot answer "is anyone working"; the map can, and the
backend emits it on every activity event whatever the derived state. `paused` is
the one exception, because taking the box back is what the button is for.

**Approvals are not parkable — they take the input slot.** Something is
synchronously blocked on the answer: a pre-push hook holding a push open, a
gated command that has not run. The tray treated it as one more card in a list,
with a Send button of its own, which is how the user came to answer a row and
watch nothing move. The gate replaces the input box, is answered on the spot,
and keeps Pause reachable — a gate you cannot escape is how a harness loses a
user's trust. The tray now reports approvals as a count and says where they are
answered; it does not offer a second way to answer them, because two paths into
one row is the defect, not the fix. Discard went with it: for a gate the
explicit no is **Reject**, which tells the hook, where discarding just walked
away from a held-open command.

**A discriminator that was wrong by a third.** `isApproval` first asked
`command_text !== null`, which `ask_user_choice_inner` sets for ToolBlocklist
(action-gate) rows ALONE. A parked `request_approval` — the push gate — carries
none, so **10 of 31 approvals in the archive** were classified as ordinary
questions while a pre-push hook blocked on each one. Both gate kinds ask exactly
`Approve`/`Reject`; no ordinary question in any session ever recorded uses those
two options. The lesson is the one from the two-hue palette: a discriminator
that holds on the cases in front of you reads exactly like one that holds.

**What the ring does NOT do.** Approvals still do not freeze it.
`set_session_awaiting` passes `halt_ring = !blocking` deliberately — the tool
call is in flight, the holder still holds the turn, and the ring is already
waiting on its completion. Freezing peers would undo D22's review lap. The asker
is blocked; everyone else keeps working. The gate is a UI claim about where the
answer goes, not a new way to stop the ring.

**Tests.** `isLocked` had no unit test until it became load-bearing — every case
had to be expressed as a render. It has one now. Three edits changed SUBJECT and
say so in place: the input no longer opens for a parked question, the tray no
longer answers an approval, and Stop is now called Pause because that is what it
does (it parks; Resume picks the ring up where it left off). Both new rules were
mutation-verified — drop `anyBusy` from `isLocked`, or restore the
`command_text` discriminator, and the tests that exist for them go red.
