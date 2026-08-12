# rc3 is an architecture reframe, not a redesign — the contract

**Authored from the user's own words, 2026-08-12.** This document exists because
the last time a direction was stated only in conversation, it was lost for five
days: prose-only decisions never became inventory rows, so the plan never
scheduled them and the acceptance gate structurally could not notice they were
missing. This one is a checklist, and it is binding.

## The user's framing, verbatim

> "this whole rc3 thing is not a redesign, its an architecture reframe. What
> worked previously before rc3 must also work on rc3 - for example the agents
> must work the same way - HANDS and EYES will work adversarially, (on rc3 they
> should be my 2 personal role configurations, not something that comes with
> bot-hq when new users install it.) But still just the same with my previous
> workflows, if not better or faster or more optimized. I accept that this comes
> with changes here and there (like the Agents -> Roles subtab, and the Create
> New Session dialog)"

## What that means operationally

Three obligations, in priority order. Every rc3 change is measured against them.

### 1. Behaviour parity — the reframe test

For every behaviour that worked before rc3, name **where it lives now**. Not
"it should still work" — an actual location, and a green test at that location.

A reframe moves the *source* of a behaviour without changing the behaviour. A
redesign changes the behaviour. The moment an rc3 change makes something the
user relied on work *differently* rather than work *from somewhere else*, it has
stopped being a reframe and needs to be argued for on its own merits.

This is the same instrument as
[`2026-08-06-router-behaviour-inventory.md`](2026-08-06-router-behaviour-inventory.md),
which walks 20 router behaviours and assigns each PRESERVED / DISSOLVED /
DROPPED. That inventory worked. Extend the method, do not invent a new one.

**HANDS and EYES working adversarially is the flagship case.** It must survive
byte-for-byte in effect: HANDS edits and executes, EYES reviews and blocks, and
neither can do the other's job. Faster or more optimised is welcome. Different
is not.

### 2. The roles are the user's, not the product's

> "on rc3 they should be my 2 personal role configurations, not something that
> comes with bot-hq when new users install it"

This is the load-bearing distinction, and it is currently **not honoured**:

- `migrations/0044_session_participants.sql:71-83` seeds `hands` and `eyes`
  unconditionally with `builtin = 1`. Every fresh install ships with HANDS and
  EYES. They are product defaults today, not personal configuration.
- The behaviour that makes them adversarial does **not** come from those rows at
  all. It comes from hardcoded agent names — `caller.agent == "rain"`
  (`src/signaling/jsonrpc.rs:214`) and `cfg.agent_name == "rain"`
  (`src/agents/spawn.rs:1139`). Verified 2026-08-12: outside tests,
  `CapabilitySet` is read only by the Roles tab's own form validation.

The consequence is precise and worth stating plainly: **a role's capability
checkboxes change nothing at spawn.** The Roles tab can save a set, the prompt
can describe it, and the runtime still gates on whether the agent is literally
named `rain`. So HANDS and EYES cannot yet be "just my two roles" — the product
hardcodes them — and any *third* role the user adds is ungated.

**Therefore the reframe is not complete until the gate reads capabilities.**
That is the item that converts HANDS/EYES from product internals into user
configuration, which is exactly what the user asked for. It outranks everything
else remaining in the queue.

### 3. Surface changes are accepted, behaviour changes are not

Explicitly blessed by the user: Agents tab → Roles subtab, and the New Session
dialog growing to pick participants. Anything of that kind is in scope without
further argument. What is *not* in scope is a change to what the agents do.

## The seeding question, and why it is not one migration

Removing the seeded rows outright would break session creation today:
`ensure_session_roster` resolves the roster through two literal
`(SELECT id FROM roles WHERE slug = 'hands' / 'eyes')` subqueries
(`src/storage/participants.rs:645`, `667`). A fresh install with no roles would
create every session with `role_id IS NULL` and no error.

So the answer splits along that dependency:

| | What | When |
|---|---|---|
| **now** | The seeded rows become *the user's* — `builtin = 0`, so nothing in the product claims to own them. Correct the stray `route_gated_command` grant the seed wrote, which is not a `Capability` and which the Roles tab has to work around. | migration 0048 |
| **with N-participant session create** | Session creation picks roles instead of assuming two literal slugs; only then can a fresh install ship with no roles at all. | queue item 5 |

`builtin` drives a UI badge and nothing else (verified), so flipping it is
zero-risk and it is the honest statement: bot-hq ships no roles.

## Acceptance

An rc3 change is done when:

1. Every pre-rc3 behaviour it touches is named, with a green test at its new
   location — or is explicitly DISSOLVED/DROPPED with a written reason.
2. Nothing it ships asserts an enforcement that is not wired. A prompt or a
   UI string that describes a rule the runtime does not apply is a defect, not
   documentation. Two such assertions were caught in review on 2026-08-12 and
   are the reason this line exists.
3. It moves a source, not a behaviour — or it says out loud that it is changing
   a behaviour, and why.
