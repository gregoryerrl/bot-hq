# Phase K Arc — Bot-HQ Context-Integrity-Management Hardening

**Phase opened:** 2026-04-28 (post-Phase-J-close + bcc-ad-manager session 2026-04-29 bilateral-discipline-deviations as proof-of-need)
**Phase closed:** 2026-04-29 (Tier-1 commits all PUSHED to main)
**Backfill authored:** 2026-05-04 (per pre-phase-close-checklist item-5 housekeeping; ratchet-ledger note line 49 reference; phase-l.md§Output-files line 200)
**Origin BRAIN-cycle authorization:** user msg 6396 ("Phase J = nail; Phase K = hammer" + BRAIN-AGREED-as-greenflag bypass authorization for Phase K work-thread)
**Scope-lock:** archived in `~/.bot-hq/ratchets/active.md` Phase K section (lines 57-105)
**Theme:** iterative reinforcement of trio discipline-rhythm against autocompact-induced drift, class-split violations, OUTBOUND-MISS misreads, peer-coord vs user-broadcast confusion, R12 commit-gate skips, per-instance-fire-greenflag drift, force-push elevated-gate skips

---

## Context

Phase K's authorizing motivation was the bcc-ad-manager session on 2026-04-29 where the trio exhibited 4 user-flagged lost-discipline classes (msgs 6326 / 6358 / 6371 / 6372 + 6381 / 6391):
- R12 commit-gate skips (peer-greenflag-msg-id footer not consistently emitted on Brian-side commits)
- per-instance-fire-greenflag drift (HANDS-execute fires without per-instance user verbatim within scope-bypass interpretation)
- force-push elevated-gate skips (force-push without dual peer + user verbatim authorization)
- PM-vs-broadcast authorization confusion (acted on PMs as if user-class authorization)

User msg 6396 authorized Phase K with BRAIN-AGREED-as-greenflag bypass scope (Phase K Tier-1 work-thread; bypass terminates on user explicit withdrawal / phase-close / session-end / work-thread completion).

Phase K shipped 9 Tier-1 enforcement-conversion ratchets in 1 working session — record-shipping cadence — closing the chronic-class loops opened in bcc-ad-manager observation.

---

## Tier-1 deliverables (9/9 SHIPPED, all PUSHED)

| ID    | Commit  | Gloss |
|-------|---------|-------|
| K-12  | f64ec07 | AgentState.DisciplineAnchorSHA256 + Compute/Verify helpers (autocompact-drift detection at conversation-resume) |
| K-16  | fc487fd | PreToolUse class-split gate — `internal/toolgate/` package; rain blocked from HANDS execute (Edit/Write/git commands) |
| K-17  | bcd4ac9 | MsgPeerHalt + MsgPeerHaltAck mutual-halt protocol primitives + R24 MUTUAL-HALT-PROTOCOL discipline rule |
| K-18  | 0ffa97f | MessageClass authorization-input discriminator (PM vs broadcast) + R25 PM-VS-BROADCAST-AUTHORIZATION |
| K-14  | ac906aa | IsOutboundMissNotification helper + R27 OUTBOUND-MISS-SELF-RECOGNITION rule |
| K-13  | 81015ff | R12 pre-commit gate (peer-greenflag-msg-id footer + hub.db verify) + R26 R12-COMMIT-GREENFLAG-FOOTER |
| K-15  | b915bc0 | R28 PER-INSTANCE-FIRE-GREENFLAG rule + BRAIN-AGREED bypass scope codification |
| K-19  | a9a156d | R29 FORCE-PUSH-ELEVATED-GATE (peer + user verbatim, no bypass; dual-cite footer) |
| K-22  | 86521b6 | R30 [HR]-TAG-DISCRIMINATOR (AFK-window untagged-default + use/drop matrix) |

**Phase K Tier-1 commit log (PUSHED to main since be53f9d Phase J close):**
```
f64ec07 → fc487fd → bcd4ac9 → 0ffa97f → ac906aa → 81015ff → b915bc0 → a9a156d → 86521b6
```

Plus housekeeping commit `c02f2be` post-K-22 (remove weekly-window halt trigger; halt + pre-snap gate on five_hour only).

---

## R-rules added (R24-R30, 7 new)

| Rule | Origin | Class |
|------|--------|-------|
| R24 MUTUAL-HALT-PROTOCOL | K-17 (commit bcd4ac9) | Bilateral peer-halt authority + 7-class drift-trigger taxonomy |
| R25 PM-VS-BROADCAST-AUTHORIZATION | K-18 (commit 0ffa97f) | MessageClass authorization-input discriminator (only user-class authorizes HANDS-execute) |
| R26 R12-COMMIT-GREENFLAG-FOOTER | K-13 (commit 81015ff) | Pre-commit footer cite + hub.db verify (toolgate enforcement-conversion of R12) |
| R27 OUTBOUND-MISS-SELF-RECOGNITION | K-14 (commit ac906aa) | Self-recognition discriminator for `[OUTBOUND-MISS]` notifications (avoid misread-as-own-output) |
| R28 PER-INSTANCE-FIRE-GREENFLAG | K-15 (commit b915bc0) | HANDS-execute per-instance user-verbatim + BRAIN-AGREED bypass scope |
| R29 FORCE-PUSH-ELEVATED-GATE | K-19 (commit a9a156d) | Force-push dual-authority (peer + user verbatim, no bypass) — stacks above R26+R28 |
| R30 HR-TAG-DISCRIMINATOR | K-22 (commit 86521b6) | AFK-window untagged-default + 8-class [HR] use-criterion matrix |

---

## New packages / helpers (Phase K)

- **`internal/protocol/peerhalt.go`** — PeerHaltPayload + PeerHaltAckPayload + Build/Parse helpers + 7 PeerHaltTrigger constants + 3 PeerHaltAckResult constants (K-17)
- **`internal/protocol/messageclass.go`** (in types.go) — MessageClass type + 6 constants + IsAuthorizationEligible method + AllMessageClasses slice (K-18)
- **`internal/protocol/outboundmiss.go`** — IsOutboundMissNotification helper + outboundMissPrefix constant (K-14)
- **`internal/protocol/agentstate.go`** — DisciplineAnchorSHA256 field + Compute/Verify helpers + disciplineAnchorPath helper (K-12)
- **`internal/toolgate/`** — NEW package: gate.go (IsHANDSExecutePattern + IsCommitPattern + IsForcePushPattern + tokenize) + hook.go (RunHook entry) + install.go (InstallTrioHook) + r12.go (VerifyCommit + R12Verdict + extractCommitMessage) + ~33 tests (K-16, K-13)
- **`internal/hub/db.go`** — GetMessageByID(int64) helper for K-13 commit-gate verification (K-13)
- **`cmd/bot-hq/main.go`** — 2 new subcommands: tool-permission-hook + install-toolgate-hook (K-16)

---

## Tier-2 holds (9 deferred, re-evaluated at Phase L close)

Per discipline-log §Phase J/K Tier-2 holds sweep + Phase L L-4 ratchet (`~/.bot-hq/discipline-log.md`):

| ID | Phase L disposition |
|----|---------------------|
| K-1-bis-deeper-tail-5-soak | DEPRECATED (closed-no-impl + no Phase L recurrence) |
| K-2 R20-mcp-tool | Re-defer Phase M (R20 prompt-following sufficient) |
| K-5 B5-probe-enum | Re-defer Phase M |
| K-6 §5.3-MsgUpdate→Result | Re-defer Phase M |
| K-7 B5c-context-telemetry | Re-defer Phase M |
| K-8 B5e-compact-focus | Re-defer Phase M |
| K-9 LastSeen-advance-root | Re-defer Phase M |
| K-10 T2.1-(d)-skills-trim | FOLDED into L-3a (achieved via L-3a/L-3b ship-list + skill-extend Phase L L-3b c2) |
| K-11 emma-offline-at-rollover | Re-defer Phase M (Axis-C pre-req) |

---

## Retrospective insights

### What worked

1. **Toolgate-conversion as the load-bearing primitive (K-13 + K-16 + K-19):** PreToolUse hook design landing during Phase K opened the architectural pattern that Phase L extended (R33 toolgate gate-CHECK at L-5). Phase K's `internal/toolgate/` package is the foundation for Phase L's gate-file SHA verification.
2. **MsgPeerHalt + R24 MUTUAL-HALT-PROTOCOL (K-17):** bilateral peer-halt authority codified as discrete msg-type + helper API. Clean separation of detection (drift-trigger constants) from action (halt-ack with recovery_summary).
3. **Discipline-anchor SHA256 (K-12):** autocompact-drift detection via AgentState.DisciplineAnchorSHA256. Anchors agent prompts against tampering / silent drift across compaction events.
4. **Class-split gate at toolgate-time (K-16):** PreToolUse hook blocks Rain from HANDS-execute (Edit/Write/git commands) BEFORE tool-call fires. Mechanical enforcement-conversion of EYES role from prompt-rule to runtime-block.
5. **R26 R12 commit-footer cite + hub.db verify (K-13):** mechanical proof-of-peer-greenflag at git-commit time. First Phase-K toolgate enforcement-conversion of a Phase J/I rule.
6. **R30 HR-tag discriminator (K-22):** AFK-window-aware tagging matrix. Reduces token-cost during user-AFK windows while preserving must-read criterion for user-action-required artifacts.

### What carried forward to Phase L

1. **K-13 brian-side hook enforcement gap (latent bug discovered Phase L L-5 commit-2 #27):** R12 PreToolUse hook only fired for rain (gated by agentID early-return); R33 paths placed AFTER early-return. Compounding K-13 gap on brian-side. Phase L L-5 c2 fix-pattern (split rain-only / non-rain branches) becomes precedent for Phase M K-13 brian-side fix.
2. **K-10 T2.1-(d) skills-trim:** Phase J→K deferred per PB3 deeper-refactor scope; Phase L L-3a/L-3b absorbed via prompt-shrink ship-list + skill-extend at `~/.claude/skills/phase-rules-detail/SKILL.md`.

---

## Closure

Phase K closed via 9-commit Tier-1 bundle PUSHED to main on 2026-04-29. Combined with Phase J close (18-commit bundle f52ccee..be53f9d), total 27 commits since c02f2be (post-Phase-K halt-trigger housekeeping) provide the discipline-rhythm foundation that Phase L extended.

**Phase K empirical headline:** mechanical enforcement-conversion at toolgate-time (PreToolUse hooks) is the durable cure for chronic discipline-class drift. R26 (commit-footer cite) + R29 (force-push dual-gate) + K-16 (class-split gate) all instantiate the pattern. Phase L extended via R33 (gate-file SHA verification at execute-time).

---

## Cross-references

- **Phase K active ratchet ledger snapshot:** `~/.bot-hq/ratchets/active.md` Phase K section (lines 57-105)
- **Phase J archived snapshot:** `~/.bot-hq/ratchets/active.md` Phase J section (lines 109-152) + `docs/arcs/phase-i.md` (Phase I+J commit log)
- **Phase L extension:** `docs/arcs/phase-l.md` (extends K toolgate pattern via R33 gate-file SHA verification)
- **Origin authorization:** user msg 6396 (Phase K open + BRAIN-AGREED bypass scope grant)
- **Bcc-ad-manager 2026-04-29 session (proof-of-need source):** msgs 6326 / 6358 / 6371 / 6372 / 6381 / 6391 (4 user-flagged lost-discipline classes)
