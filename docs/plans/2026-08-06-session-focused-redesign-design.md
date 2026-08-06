# bot-hq redesign — session-focused architecture

**Status: DESIGN IN PROGRESS.** Sections are validated one at a time with the
user; open decisions are marked. Nothing here is implemented, and the B1
migration draft (`2026-08-06-session-participants-migration-DRAFT.sql`) is
deliberately not armed.

Origin: the user's observation that bot-hq's *doing* is correct but the *design
of the doing* is problematic — agent-focus makes agent plugins hard, and hidden
machinery means "sometimes I have no idea what's happening under the hood."

Supporting evidence lives in this session's IPAV docs: blast radius (1333 Rust +
312 frontend agent-name occurrences), the 21-subsystem inventory, and the six
invisible injection points.

---

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

**What this deletes:** the volley-breakers (order is fixed, not emergent), the
wake-rule configuration layer, per-message addressing, and `core/router.rs`'s
bilateral routing — there is no routing, only a ring.

### ⛔ OPEN — Q2: round termination

When a full round completes, does the cycle (A) yield to the user, (B) loop
until a participant declares done, or (C) loop with a visible round budget?
Parked with the user. Recommendation: **C** — B's autonomy without B's runaway,
and unlike A it does not make the user the pump for ordinary multi-step work.

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

### ⛔ OPEN — queued product decisions
- Designated vs derived reviewer (does the gate care WHICH participant filed?)
- Roles fixed at create vs reassignable mid-session
- Does a role carry a default model?
- Upper bound on N

---

## Section 3 — Channel transport ⏳ DRAFTED, NOT YET VALIDATED

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

---

## Section 4 — Migration ⏳ DRAFTED, NOT YET VALIDATED

Big-bang schema (user-picked), code in batches. See the B1 draft + runbook.

Revision needed before arming: `session_participants.preset TEXT` becomes
`role_id` against a new user-owned `roles` table, which supersedes
`agent_configs` (whose `agent_name` PK is CHECK-constrained to emma/brian/rain —
the same closed-enum problem one layer down). The `messages`-rebuild half is
already dry-run-proven and its guards proven to abort; that evidence survives.

**Cutover gate:** a grep audit proving every read of `sessions.{brian,rain}_*`
is dead after B1. If any path still reads them, that is split brain.

---

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
