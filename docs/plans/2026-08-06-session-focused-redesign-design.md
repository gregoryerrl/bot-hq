# bot-hq redesign — session-focused architecture

**Status: DESIGN COMPLETE — no open architectural questions.** Every section was
validated with the user one at a time. Nothing here is implemented, and the B1
migration draft (`2026-08-06-session-participants-migration-DRAFT.sql`) is
deliberately not armed.

**Decisions and who made them** — the four forks were the user's, not the
agent's:

| # | question | answer |
|---|---|---|
| Q1 | what governs waking | **turn-based cycle** (user's own model; beat all three options offered) |
| Q2 | round termination | **consensus halt**, derived from vision.md's AI-car |
| Q3 | review authority | **derived + existing blocking/advisory severity** |
| Q4 | parity vs serialisation | **accept serialisation** — "the staleness was the bug, not the speed" |

Next step is an implementation plan, not code: the B1 draft needs its `roles`
table before it can be armed.

Origin: the user's observation that bot-hq's *doing* is correct but the *design
of the doing* is problematic — agent-focus makes agent plugins hard, and hidden
machinery means "sometimes I have no idea what's happening under the hood."

Supporting evidence lives in this session's IPAV docs: blast radius (1333 Rust +
312 frontend agent-name occurrences), the 21-subsystem inventory, and the six
invisible injection points.

---

## Constraint 0 — behavioural parity (user, 2026-08-06)

> *"This shouldn't change what works today client-side. We're redesigning how
> the flow works, not the flow itself. We're migrating HANDS and EYES into
> roles, so nothing should change client-side."*

This is an **acceptance criterion**, not an aspiration. A session created with
the seeded defaults must behave as today's duo does:

- HANDS capability set maps 1:1 onto today's `HANDS_ONLY_TOOLS` + write access;
  EYES onto `EYES_ONLY_TOOLS` + read-only.
- The commit gate behaves identically: a reviewer files → executor dispositions →
  commits blocked until resolved.
- `eyes_flag` / `disposition_finding` / `approve_finding` survive as the same
  tools, gated by capability instead of name equality.
- IPAV, the tray, push/Tool gates, CL, terminal, worktrees: unchanged.
- The composed prompt delivers the same contract the agents read today.
- The Roles tab exists but requires no interaction — HANDS and EYES are seeded.

**Verification:** the full gate suite green, plus a live smoke comparing a
default session against today's behaviour — not just "it compiles".

### ⚠ The one place parity and the turn model genuinely conflict

**Today's duo is CONCURRENT.** `broadcast_user_message` delivers the user's
message to *both* agents' input channels and marks both busy
(`state.rs:762-765`, `session.rs:111-118`). Evidence, not inference: this
session's `activity_events` carries repeated `busy | 1 | 1` rows.

**A turn cycle is serial by definition.** The user's message wakes participant 1;
participant 2 acts when its turn comes. Two client-visible consequences:

1. wall-clock for a "both respond" round becomes sequential, not parallel;
2. the reviewer can no longer comment on the user's message *before* the
   executor works.

**RESOLVED — serialisation accepted** (user, Q4): *"the staleness was the bug,
not the speed."* Concurrency was never a chosen feature; it is an artifact of
the duo pump, and it is the direct cause of the staleness investigated earlier
the same session (EYES believing the session was still in Investigate after four
commits had landed). Serialising costs wall-clock per round and removes an
entire class of wrong-premise work.

So parity is scoped precisely: **capabilities, gates, tools, prompts and
surfaces are unchanged; concurrency is deliberately dropped.** That exception is
recorded here so it is never mistaken for a regression during implementation.

## Pillars

1. **Participants, not agents.** A session owns N participants. Cardinality and
   capability are rows, not code. Adding an agent is an invite, not a refactor.
2. **The channel is the transport.** Every wire into a participant is a channel
   message. What is delivered == what is recorded == what is displayed.
3. **The roster is the transparency surface.** Per-participant status,
   capabilities, turn position and read cursor are all visible.
4. **Roles are user-owned.** The Roles tab defines capabilities, participation
   mode and description. HANDS and EYES become *the user's* configurations, not
   product primitives.

---

## Section 1 — Turn model ✅ VALIDATED (user-authored)

A session has a **turn cycle** over its *active* participants in a fixed order.
Exactly one participant holds the turn.

- When a participant's turn ends, the cycle advances and the next participant
  **wakes**, receiving every channel row after its cursor — everything said
  since it last acted, in order. **Context completeness is structural**, not a
  forwarding discipline.
- A *turn* is one participant's entire turn (many tool calls), not one message —
  same granularity as today.
- **Cost is O(1) wakes per turn regardless of N.** Broadcast would be O(N);
  addressing is unpredictable. This is the only model where the user always
  knows who acts next.
- **A user message resets the cycle to participant 1.** A new instruction starts
  at the top of the pipeline.
- **Turn order is fixed at session create.** (YAGNI: reordering is additive
  later; it costs UI now for an undemonstrated need.)
- A participant may **PASS** rather than burn a turn. The pass is recorded in
  the channel so it is visible.

**Participation modes** (declared by the role):

| mode | in rotation? | reads channel? | posts? |
|---|---|---|---|
| active | yes | yes | yes |
| observer | **no — skipped entirely** | yes | no |
| on-demand | no | yes | only when addressed |

Observers are skipped rather than given a no-op turn: a wake that cannot produce
output by construction is pure waste.

**What this deletes:** the wake-rule configuration layer, per-message addressing,
`core/router.rs`'s bilateral routing (there is no routing, only a ring), and the
**L2 hard-cap** — that breaker exists for *emergent ping-pong ordering*, which a
fixed ring makes impossible.

**What survives, contrary to an earlier claim in this doc:** repetition
detection. A single participant can still spin — producing near-identical output
round after round — which a fixed order does nothing to prevent. The existing
convergence detector (Jaccard similarity) is repurposed as **spin detection over
one participant's output across rounds**. A round cap alone would be the wrong
instrument: it punishes long-but-productive work as a false positive.

## Section 1b — Round termination ✅ VALIDATED (user-authored; derived from vision.md)

**Consensus halt.** The cycle continues until every active participant agrees
there is nothing left to do.

- Each participant, at turn end, either produces substantive output or declares
  **done**.
- When ALL active participants have declared done **consecutively**, the session
  halts and yields to the user.
- **Any substantive output resets the tally** — a stale "done" cannot accumulate
  into a false arrival. (Mirrors the existing `convergence_reset` flag.)
- Observers and on-demand participants are skipped in rotation, so **they do not
  vote**. Otherwise 1 active + 3 observers would need 4 yields to halt.
- **A parked question halts immediately and unilaterally** — it is a yield, not
  a vote. A participant blocking on the user stops the cycle regardless of what
  the others would have done.

**Why this and not a round budget** (the recommendation this replaces): vision.md
says *"the first prompt sets the destination, not the arrival"* and *"in
principle the user can tell the agents to drive to the destination alone."* A
budget is the car pulling over every N miles to ask permission to continue —
exactly the "user becomes the pump" failure the vision rejects. The AI-car has
two stopping conditions and the consensus model reproduces both:

| vision concept | mechanism |
|---|---|
| **arrival** | all active participants declare done |
| **obstacle / junction** | a participant parks a question → immediate halt, surfaced in the tray |
| **driving** | anyone still producing work |

**Backstops** (safety nets, not checkpoints):
1. **Spin detection** (primary) — repetition across rounds by one participant is
   an obstacle, surfaced like any other.
2. **Round cap** (crude second net) — high enough to be invisible in normal use;
   visible and user-overridable per session.

---

## Section 2 — Roles ✅ VALIDATED (user-authored)

The **Agents tab becomes a Roles tab.** A role is a user-owned template:
capabilities, participation mode, description, desired runtime. Session creation
asks how many agents (**default 1**) and assigns a role to each.

**Prompt composition — three layers, one of them free text:**

1. **Core rules** — always injected, NOT editable (evidence discipline, question
   discipline, never-fabricate-authorization). Each was written after a real
   incident; a user-authored role must not be able to omit them.
2. **Capability-derived rules** — generated FROM the capability set. Holds
   `GatedBash` → the gate contract; lacks it → the text is absent rather than
   misleading. **Prompt and guard cannot drift, because one is derived from the
   other.**
3. **Role description** — user free text: identity, voice, priorities. The ONLY
   stored prose.

Rationale for layer 2 being generated rather than authored: CL
`learnings-2026-08-04` records *"a prompt can order a call a guard refuses"* —
the Apply-entry nudge told HANDS to `mark_awaiting_user`, which the guard
hard-refuses, and the refusal said to do the opposite. Three fires in one
session. Free-text rules reproduce that class at scale.

**Capabilities are grants only.** Deny-lists and spawn flags are *derived* from
absent capabilities; storing "prohibitions" separately allows contradiction and
forces a precedence rule. "Can / cannot" is a UI rendering, not two sources of
truth.

**Capabilities carry dependencies.** `GatedBash` requires `RunBash`;
`CloseSession` requires `ReadChannel`. The tab models `requires` / `conflicts`,
not a flat checklist.

**Session policy is the ceiling:**
`effective_capabilities = role_capabilities ∩ session_policy`. A role may be
more restricted than the session permits, never less.

**Roles snapshot at invite.** Editing a role must not mutate running sessions —
that would widen a live participant's permissions mid-turn. The participant
carries a copy; the role is the template. (Consistent with `*_model_at_spawn`
and `session-policies/<sid>.yaml`.)

**Two things cannot live on a role:**
- *Model properties* — native eligibility is a credential constraint
  (subscription OAuth is CLI-bound). Resolve as role-desired ∩ model-supported.
- *Relationship constraints* — `disposition_finding` requires "not the author";
  `approve_finding` requires "is the author". A role grants the verb; the
  relation is enforced per-object at the tool boundary.

**Roles-tab guards:**
1. **Completeness** — a session where nobody holds `FileFinding` (or nobody
   holds `DispositionFinding`) has a silently disabled review gate. Say so at
   create time.
2. **Self-review** — one role holding BOTH `FileFinding` and
   `DispositionFinding` can file against itself and clear it; the commit gate
   becomes ceremonial. Flag it.
3. **Explain configurations** — no execution capability = observer; execution
   without `AskUser` = a silent worker that can never surface a question.

**Decided without needing a user call:**
- A role **may carry an optional default model**, overridable at invite. Cheap,
  and it makes a role a complete recipe rather than half of one.
- **No hard cap on N.** The create flow warns past ~5 rather than forbidding it;
  an arbitrary limit is a guess, a warning is information.
- **Turn order and role assignment are fixed at session create.** Reordering and
  reassignment are additive later; building them now costs UI for an
  undemonstrated need.

### Section 2b — Review authority ✅ VALIDATED

**Derived, with the existing severity distinction.** Any participant holding
`FileFinding` may file; a finding is `blocking` or `advisory`; only `blocking`
gates the commit. No designation concept enters the schema.

This is the smallest possible generalisation: `eyes_flag` already carries a
blocking flag and the gate already enforces it, so N-way needs **no new
concept** — only the gate's query changes from "Rain's findings" to "any
participant's blocking findings, unresolved."

A participant that files noise is a **role-configuration** problem (don't grant
`FileFinding`), not a gate problem. The one case this deliberately does not
express: letting an experimental participant file *blocking* findings while
denying it the power to block. If that need ever appears, a designation concept
can be added — but it is not built on speculation.

Relationship constraints are unchanged and are enforced per-object, not by role:
`disposition_finding` requires "not the author"; `approve_finding` requires "is
the author".

---

## Section 3 — Channel transport ✅ VALIDATED

- One session channel. Participants read it via `participant_cursors`; delivery
  is a cursor advance, making it an auditable fact.
- **Delivery is a projection, not polling:** append row → notify → the turn
  sequencer wakes the next participant → its unread rows are rendered
  (envelope → text) → written to stdin → cursor advances.
- **Policies gate DELIVERY, never PERSISTENCE.** A withheld message is still a
  visible row with a reason (`participant_deliveries.withheld_reason`).
  Suppressing the post is what makes today's losses invisible.
- **Envelopes become metadata**, not string mutation: phase, sender role,
  findings banner, ack tags render beside the body — the user sees the object
  the participant reads.
- **A `system` participant** posts host-authored injections (nudges, reconcile
  directives, phase notices), so all six invisible wires become rows.
- **Enforcement, not discipline:** the participant's input sender is private and
  takes a `PersistedMessage` newtype whose constructor is private to storage and
  produced only by the row insert. "Wire without a row" becomes a compile error.
- Ordering is free: cursors order by `id`; per-participant tasks read one
  ordered stream at different positions and cannot race.
- **Perf to measure:** today's delivery path deliberately never touches storage
  (`open_blocking` is a lock-free cache for exactly that). A write per delivery
  must be benchmarked.

**The turn sequencer replaces per-participant reactive tasks.** Because exactly
one participant acts at a time, there is no fan-out: when a turn completes, the
sequencer picks the next active participant, hands it every unread row, and
waits. One loop, not N reactive tasks — simpler than the model drafted before
the turn cycle was settled.

**What the user sees.** The roster shows, per participant: turn position, whether
it holds the turn, cursor lag (how far behind the channel it is), its capability
set, and its last vote (done / working). "Waiting on participant 3" becomes a
rendered state rather than a gap between events — which was the original
complaint.

---

## Section 4 — Migration ✅ VALIDATED

Big-bang schema (user-picked), code in batches. See the B1 draft + runbook.

Revision needed before arming: `session_participants.preset TEXT` becomes
`role_id` against a new user-owned `roles` table, which supersedes
`agent_configs` (whose `agent_name` PK is CHECK-constrained to emma/brian/rain —
the same closed-enum problem one layer down). The `messages`-rebuild half is
already dry-run-proven and its guards proven to abort; that evidence survives.

**Cutover gate:** a grep audit proving every read of `sessions.{brian,rain}_*`
is dead after B1. If any path still reads them, that is split brain.

---

## Section 5 — Re-triage of the deferred bundle

PLAN.md defers three items **behind the Rain-plugin migration**. That gate no
longer exists (roles delete the migration outright), so each needs re-filing:

| item | verdict |
|---|---|
| **#26** held-forward flush (`held_late` ×12/day) | **DISSOLVES.** The channel model has no held forwards — cursors either advance or visibly do not. The bug class cannot be expressed. |
| **#30** duplicate spawn warmup | **SURVIVES, reshaped.** Becomes "invite creates a duplicate participant"; re-file against the participant model rather than the spawn path. |
| peer-wait watchdog misclassification | **DISSOLVES.** A participant awaiting its turn is *waiting* — a visible roster state, not silence to be inferred from. The watchdog reads turn position instead of guessing. |

## Issues this redesign closes as a side effect

- **#5 — "EYES has no user channel", flagged needs-user-decision.** Resolves
  without a decision: addressing the user becomes the `AskUser` capability, so
  it is a per-role toggle rather than an architectural fork.
- **`close_session` ungated** (the live gap found this session): becomes the
  `CloseSession` capability.
- **The idle watchdog's false NEEDS DIRECTION**: superseded by turn position.
- **Six invisible injection points**: closed by the `system` participant plus
  the `PersistedMessage` newtype, which makes "wire without a row" a compile
  error.

## Consequences beyond the redesign

- **The Rain-plugin migration is deleted, not rescheduled.** It existed only to
  make Rain optional; under roles there is no "Rain" to make optional.
- PLAN.md defers `#26`, `#30` and the peer-wait watchdog classification *behind*
  that migration. `#26` (held forwards) dissolves outright — the channel model
  has no held forwards. The others need re-filing.
- `close_session`'s missing gate (CL issues #5) is fixed as a side effect: it
  becomes a capability.
- The idle watchdog gains a first-class peer-wait: a participant awaiting its
  turn is *waiting*, not stalled — retiring the false NEEDS DIRECTION.
