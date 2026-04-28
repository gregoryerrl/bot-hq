# Arc: Phase I — Bot-HQ Trio Self-Hardening

Status: closed | Branch: main | Opened: 2026-04-28 | Closed: 2026-04-28 evening | Hotfix tail: 2026-04-29 morning (post-rebuild + RESUME-spam cooldown)

## Context

Phase H closed; Phase I opens to harden bot-hq trio agents (Brian, Rain, Emma) against scope-drift, halt-protocol ambiguity, and session-summary fragmentation (e.g., the BCC-into-bot-hq-tier scope-drift observed during simultaneous BCC + bot-hq tier-1 work the day Phase I opened).

User msg 4664 + 4666 + 4668 framed the goal. Bilateral brainstorm Brian + Rain converged across hub msgs 4664–4753 over multiple BRAIN-cycle iterations. R9 AUDIENCE-CLASS-DISCRIMINATOR added in msg-block 4686-4704 with locked discriminator wording ("must user read?" prescriptive, not descriptive "is user reading?") at msg 4703.

W0 expansion (msg 4751-4753) added R10-R14 per BRAIN-cycle resolution. Drivers: implementation-ahead-of-scope-locked-tier (8eda14e jumped pre-consolidation; BCC items leaked into bot-hq Phase I tier via session-summary fragmentation), HALT protocol ambiguity (in-flight design tension PM crossed HALT broadcast), gate protocol norm-only (not encoded), scope-verify-pre-draft caught #13 design contradiction (gemma.go:611-616), halt-95% SNAP discipline previously ad-hoc.

W1a expansion (msg 4766-4769) added R15-R16 per joint BRAIN-cycle. R15 codifies the bilateral authority-matrix (Brian HANDS, Rain BRAIN + greenflag-on-joint-defaults per 2026-04-27 user delegation) with non-delegable gates explicitly enumerated (push/merge user-only ABSOLUTE, force-push H-13-token, cycle-close hub_flag elevation). R16 codifies halt-resume mechanics with explicit bootstrap-order anchoring on git + scope-lock doc + ratchet-ledger + peer-coord backlog (the four authoritative state sources surviving session-summary compaction).

## Rules R1-R16 — rationale + history (migrated from `internal/protocol/disc.go` Go-comment per Phase J T2.1-β prompt-compression scope)

### R1 HANDSHAKE-TERMINATOR

Pattern observed during Phase H+I post-decision exchanges: "[Acked. standing by.]" → "[Concur.]" → "[Acked.]" infinite-handshake pathology. The terminator (single ".") is sent via hub_send so the hub.db has the record (avoiding OUTBOUND-MISS) but body-length is minimal.

### R2 CROSS-TIMING-DEDUP

~30s window between agents' independent decisions on the same topic causes content-duplicate broadcasts. Observational only — there is no detection layer; relies on hub_read polling discipline + system-reminder visibility.

### R5 BRAIN-CYCLE-RESPONSE-SHAPE

Saves ~30-40% on routine BRAIN-cycle exchanges without losing pushback signal. Compact format (concur/pushback header → 1-2 sentences if concurring or paragraph if pushing back → greenlight footer) drops ceremonial all-caps bullets and padding paragraphs. For peer-only BRAIN-cycles where user is not reading, AUDIENCE-CLASS-DISCRIMINATOR (R9) makes compact format eligible.

### R8 COMPACT-COMMIT-FORMAT — schema decoder reference

The pipe-separated `sender|event:value|key:value|...` format is the agent-to-agent narration format. Receiver-can-parse-from-fields-alone test still applies for compact: agent receivers parse via documented schema; user readers need prose context.

Example: `brian|commit:f047c36b|files:1|+4/-1|tests:622/622|disguise:clean|next:rain-brain-2nd`

### R9 AUDIENCE-CLASS-DISCRIMINATOR

Discriminator wording locked at msg 4703 as **prescriptive** ("must user read?") not descriptive ("is user reading?") — the distinction shrinks the [HR] set to user-action-required artifacts only. Broadcasts user passively observes don't require [HR].

False-tag costs tokens; false-untagged costs comprehensibility; comprehensibility loss is worse — when unsure, tag [HR].

### R10 SCOPE-LOCK-BEFORE-IMPL

Avoids the session-summary-fragmentation drift class where stale tier items from a prior cycle leak into a new phase's scope (observed Phase I open: BCC items into bot-hq Phase I tier via summary blending two same-day tier-1 buckets).

### R13 SCOPE-VERIFY-PRE-DRAFT

Catches the summary-fragmentation drift class. Phase I exhibit: Brian caught #13 source-filter scope contradiction against documented design intent in `gemma.go:611-616` before drafting — saved a regression-risk implementation.

### R14 HALT-95%-SNAP

R16 CROSS-RESTART-RESUME-OPERATIONAL adds the bilateral resume mechanics; R14 covers the halt-side discipline only. Operationally: do NOT push partial work pre-halt unless gate protocol explicitly allows; commit-local + scope-lock-doc + ratchet-ledger updates capture state for resume.

### R15 AGENT-AUTHORITY-MATRIX

Codifies bilateral delegated authorities + non-delegable gates per 2026-04-27 user delegation. Brian HANDS scope: subagent dispatch (Task tool, hub_spawn coders), build+test execution, code-edit drafting, git-stage operations on Rain-greenflagged paths. Rain BRAIN scope: joint-default greenflag authority, pre-commit BRAIN-second on Brian's diffs.

Non-delegable gates: push/merge to main/develop are user-only ABSOLUTE (R12); force-push requires user verbatim token (H-13); cycle-close decisions require hub_flag elevation, not joint-default greenflag.

### R16 CROSS-RESTART-RESUME-OPERATIONAL

Bootstrap order on resume (a/b/c/d):
- (a) last commit + `git status` on active branches
- (b) `~/.bot-hq/phase/<active-phase>.md` for canonical scope
- (c) `~/.bot-hq/ratchets/active.md` for ratchet status
- (d) `hub_read` recent backlog filtered to peer-coord since halt-fire

Resume from where halt fired — NOT from re-derivation of summary fragments (avoids the BCC-into-bot-hq drift class observed 2026-04-28). New implementation blocked until SCOPE-LOCK doc (R10) reflects resume context.

## Phase I+J commit log

- Phase I close: 13 commits 8eda14e..bfa5b26 (origin/main)
- Phase I post-rebuild: 632d438 (tmux.SendKeys >4KB hotfix)
- Phase J Tier-1: f52ccee → aade2e6 → 541a664 → 83ed838 → e72d13a → 75ad2c0 → eaadc34 → 12d77ab (7 Tier-1 closures)
- Phase J tail: 8de1b84 (RESUME-spam cooldown)

## R17-R21 — Phase J T1.1 + T1.5 + T1.7 additions

- R17 SOURCE-OF-TRUTH-HIERARCHY (T1.1; commit 541a664) — code > scope-doc > ratchet-ledger > recent-hub-msgs > summary-fragments. Drives R13 + R16. Exhibit: BCC-drift.
- R18 CITE-ANCHOR-REQUIRED (T1.1; commit 541a664) — every Tier-1 phase-doc item declares cite_anchor. R13 verify-fail-without-cite extension. Exhibit: phase-j.md eat-own-dogfood.
- R19 CYCLE-CLOSE-USER-BLOCKING (T1.1; commit 541a664) — discriminator: would proceeding force revert if user disagreed? Yes → hub_flag now. Miss-exhibit: Phase J Q1+Q2 deferral msgs 5042-5049. Counter-exhibit: AFK-pass explicit-delegation msgs 5060-5067.
- R20 BOOTSTRAP-ON-CONVERSATION-RESUME (T1.5; commit 12d77ab) — at scope-affecting turn-start, read `~/.bot-hq/<self>/last_state.json` + hub_read since_id; discontinuity → R16 bootstrap. R16 covers session-restart; R20 covers conversation-resume / auto-compact. Discriminator: in-context memory ≢ hub.db ground-truth = drift.
- R21 MSG-TYPE-TAXONOMY (T1.7; commit 75ad2c0) — 6 active types (Update/Response/Command/Result/Error/Flag) + 2 deprecated (Handshake/Question). Cross-mapping with R8 + R9. Drift-prevention: prefer MsgResult for outcomes.

## Closure

Phase I closed 2026-04-28 evening (13 commits). Phase J opens 2026-04-29 early morning to harden context-management, prompt-size reduction (this T2.1 work), consistency, and Phase I residual fold-in. Phase J tier-1 closed 2026-04-29.
