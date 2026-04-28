# Hub Message Type Taxonomy Audit (B7 / T1.7) — Phase J

**Author:** Rain (BRAIN/EYES, audit per Phase J Q1 markdown-design-doc HANDS scope)
**Date:** 2026-04-29
**Driver:** Phase J T1.7 (B7) — codify routing rules for MessageType + tie to AUDIENCE-CLASS-DISCRIMINATOR (R9). Brian noted Phase I drift: "update for status, response for BRAIN replies, command for directives — inconsistently."
**Source-substrate:** `internal/protocol/types.go:40-58` (MessageType constants) + this-session's hub.db traffic patterns

---

## 1. Constants enumeration (types.go:43-50)

| # | Constant     | Wire value    | Semantic intent (current docs none — derived)         |
| - | ------------ | ------------- | ------------------------------------------------------ |
| 1 | MsgHandshake | "handshake"   | initial agent-to-agent introduction / session-bind     |
| 2 | MsgQuestion  | "question"    | request requiring response                             |
| 3 | MsgResponse  | "response"    | reply to a question or BRAIN-cycle item                |
| 4 | MsgCommand   | "command"     | directive (must do action) — flow vs request           |
| 5 | MsgUpdate    | "update"      | status / progress / state-change broadcast             |
| 6 | MsgResult    | "result"      | outcome of an action / task                            |
| 7 | MsgError     | "error"       | exception / failure report                             |
| 8 | MsgFlag      | "flag"        | escalation requiring user attention (hub_flag)         |

`Valid()` method (types.go:53-58) accepts all 8.

---

## 2. Observed-usage taxonomy (this-session sample, msgs ~5008-5075)

### MsgUpdate (most common — drift surface)
**Used for:**
- Peer status reports ("brian|tip-confirmed|...")
- Self-state acks ("rain|online|replay-cutoff:4959|...")
- Peer-coord progress notes ("rain|investigation-#5-PB1-CLAUDE.md-cwd-verify|complete|...")
- Handshake terminators (`.` single-period)
- Janitorial echoes (Emma stale-coder PMs)

**Drift:** "update" carries everything from "I'm alive" to "investigation findings" to "single-dot terminator" — semantic-overload.

### MsgResponse
**Used for:**
- BRAIN-cycle replies to peer-asks (Rain BRAIN-2nd-doc, Brian BRAIN-1st-counter)
- Direct answers to user-asks ("rain|substring-'plan usage reset'|trigger:YES|...")
- Investigation-result hub_sends with substantive findings

**Drift:** overlaps with MsgUpdate when content is "report progress + answer implicit ask."

### MsgCommand
**Used for:**
- Directives from agent to agent
- Emma's RESUME-flag emit (per H-31/H-33 protocol)
- (Rare in this session — observed only in Emma RESUME emit pattern)

**Cleaner:** more directive-vs-status-vs-reply distinct. Reserved for "do this action."

### MsgFlag
**Used for:**
- hub_flag elevations (escalation to user)
- Sentinel always-flag hits from Emma
- Emma's HALT/RESUME elevations

**Cleanest:** singular semantic — "user-attention-needed."

### MsgError
**Used for:**
- (None observed this session.)

**Underused:** could replace some MsgUpdate-class "error/failure" reports.

### MsgResult
**Used for:**
- (None observed this session.)

**Underused:** specific "task outcome" semantic could replace MsgResponse for action-completed reports (e.g., "commit:done|sha:abc123|...").

### MsgHandshake
**Used for:**
- (None observed this session.)

**Underused/legacy:** not part of current trio protocol.

### MsgQuestion
**Used for:**
- (None observed this session — agents emit asks inside MsgResponse content.)

**Underused:** agents prefer "ask-embedded-in-response" over discrete MsgQuestion.

---

## 3. Drift findings

### F1: MsgUpdate is semantic catch-all
- ~70% of this-session's traffic is MsgUpdate-typed regardless of actual semantic class.
- Routing receivers (hub processMessageQueue, agent poll loops) cannot discriminate by Type alone — must content-parse.

### F2: MsgResult + MsgResult are underused vs MsgUpdate/MsgResponse
- Action-completion reports (commits, test runs, investigations shipped) get MsgUpdate/MsgResponse instead of MsgResult.
- Failure reports get MsgUpdate instead of MsgError.

### F3: MsgHandshake + MsgQuestion are dead-letters in trio protocol
- Trio agents don't use these. MsgHandshake had a role in earlier slice (per types.go:43) but trio protocol matured past it.

### F4: AUDIENCE-CLASS-DISCRIMINATOR (R9) operates orthogonally to MessageType
- R9 distinguishes `[HR]` vs untagged via content-prefix + audience-discriminator
- MessageType is a separate axis (intent of the msg)
- Currently NO codified mapping — agents pick MessageType ad-hoc, R9 separately
- Result: BRAIN-cycle reply could be MsgResponse [HR] or MsgUpdate untagged or MsgResponse untagged — all are "valid"

### F5: hub.go routing only special-cases MsgFlag (hub.go:153, 228)
- `if msg.ToAgent == "" || msg.Type == protocol.MsgFlag` — broadcast OR flag get fanout-handling
- Other types treated identically by hub-level dispatch
- Type is decorative for hub but semantic for agent-receivers (and audit replay)

---

## 4. Recommended taxonomy (T1.7 codification)

### 4.1 Active types (keep, with explicit semantic)

| Type         | Semantic                                                                | Routing rule                                |
| ------------ | ----------------------------------------------------------------------- | ------------------------------------------- |
| MsgUpdate    | **Status / state / progress** — "I am here, I am doing X, X is done"   | broadcast or peer; receiver may skip-silent |
| MsgResponse  | **BRAIN-cycle reply / direct-answer-to-ask** — content addresses a prior question/proposal | peer-route default; broadcast if user in audience |
| MsgCommand   | **Directive** — recipient must do action                                | peer-route required (commands need addressee) |
| MsgResult    | **Action-outcome** — commit landed, test result, task completed         | broadcast for state-change visibility       |
| MsgError     | **Failure report** — "X attempted, X failed"                            | broadcast or peer; pairs with hub_flag for serious |
| MsgFlag      | **User-attention escalation** — hub_flag mechanism                      | broadcast; harness sends Discord notification |

### 4.2 Deprecated (legacy, tests preserve but trio agents avoid)

| Type         | Status         | Migration                                            |
| ------------ | -------------- | ---------------------------------------------------- |
| MsgHandshake | Deprecated     | use MsgUpdate ("agent online") instead              |
| MsgQuestion  | Deprecated     | embed question inside MsgResponse content (current pattern) |

### 4.3 R9 AUDIENCE-CLASS-DISCRIMINATOR cross-mapping

| Type × Audience-class       | `[HR]` typical?      | Compact-format eligible?         |
| --------------------------- | -------------------- | -------------------------------- |
| MsgUpdate untagged (default)| no                   | yes (single-dot, status-pipe)    |
| MsgUpdate `[HR]`            | yes                  | no (verbose required)            |
| MsgResponse untagged        | no                   | yes (concur-header + brief)      |
| MsgResponse `[HR]`          | yes                  | no                               |
| MsgCommand                  | yes (action-required)| no                               |
| MsgResult untagged          | no                   | yes (`commit:sha\|tests:N/N`)    |
| MsgResult `[HR]`            | yes                  | partial (commit msg verbose, summary compact) |
| MsgError                    | yes (typically)      | no                               |
| MsgFlag                     | yes (always — user attn) | no                          |

**Rule cross-mapping codification (proposed addition to PhaseIv1 R8/R9):**

> Type-vs-Tag-vs-Format Discriminator:
> - **MsgFlag + `[HR]`** is mandatory for hub_flag (already R9 implicit).
> - **MsgCommand** is `[HR]` by default (directive needs explicit reading).
> - **MsgUpdate** defaults untagged + compact unless content is user-decision-required.
> - **MsgResponse** defaults untagged + compact for peer BRAIN-cycle, `[HR]` + verbose when user is in audience for the reply content.
> - **MsgResult** defaults untagged + compact (commit-pipe format) for agent receivers; switches to `[HR]` + verbose for commit messages user reviews on GitHub.
> - **MsgError** defaults `[HR]` (failures matter) unless trivial-recoverable.

---

## 5. T1.7 implementation surface (Brian-HANDS)

### 5.1 Documentation-only changes (lightest scope)

- Add semantic-comment block to `internal/protocol/types.go:40-58` documenting each MessageType's intended use + routing rule.
- Add cross-mapping table to const PhaseIv1ProtocolHardening (or new const PhaseJv2MsgTypeTaxonomy) — codifies §4.3 rules into prompt-side discipline.

### 5.2 Test-coverage additions

- Extend `TestMessageTypeValid` (types_test.go:17) — keep allowing all 8.
- New test `TestMessageTypeSemanticCovered` — registry of (Type, semantic-doc-string) pairs; ratchet-fails if Type added without semantic doc.
- New test (B5 probe-case extension) — sentinel-checked agent-emit conformance: e.g., commits emit MsgResult not MsgUpdate; hub_flag emits MsgFlag.

### 5.3 Code refactor (medium scope, optional)

- Audit existing emit sites to migrate MsgUpdate→MsgResult where action-completion semantic applies (e.g., commit-narration emits).
- Migrate failure-reports MsgUpdate→MsgError.
- Hub routing: optionally treat MsgError + MsgCommand specially (e.g., MsgError auto-promotes log-level; MsgCommand requires non-empty ToAgent).

**My lean: T1.7 ships §5.1 + §5.2 only. §5.3 code refactor deferred to Phase K target if churn-warranted.** Rationale: doc-codification + test-locks are R10-aligned (codify discipline first), code refactor without exhibit is scope-creep.

---

## 6. Open questions for Brian-HANDS impl-time

1. **Const placement:** add R21 MSG-TYPE-TAXONOMY to PhaseIv1 (cohesive cycle) OR new PhaseJv2MsgTypeTaxonomy const? My lean: PhaseIv1-expansion (matches R17/R18/R19/R20).
2. **Deprecated-types treatment:** keep MsgHandshake + MsgQuestion as Valid() OR remove? My lean: KEEP (existing tests + future-use possible; deprecation is doc-only this phase).
3. **§5.3 code refactor scope:** included this phase or Phase K? My lean: Phase K (no exhibit, scope-creep).
4. **Cross-mapping test depth:** lock the Type-vs-Tag-vs-Format table from §4.3 in const + ratchet-test? My lean: yes (load-bearing for B5 probe-cases).
5. **Documentation doc-arcs migration:** §5.1 semantic-comment block stays in types.go OR moves to docs/arcs/? My lean: stays in types.go (close to source-of-truth; matches B2a only-after-T1.4 sequencing).

---

## 7. Findings summary

- **F1-F5 catalogued.** MsgUpdate semantic-overload is primary drift class.
- **Recommended fix:** §4.1 explicit semantics + §4.3 cross-mapping table + §5.1+§5.2 doc-codification + test-locks.
- **Defer:** §5.3 code refactor (no exhibit, Phase K target).
- **Hub-level routing change:** optional, not required; current routing works regardless of Type.

---

## 8. Cross-references

- **Phase J spec:** `~/.bot-hq/phase/phase-j.md` §T1.7 (B7)
- **Companion investigations:** `docs/plans/2026-04-29-rule-loci-audit.md` (B3a registry includes R9 + new R21) + `docs/plans/2026-04-29-bootstrap-resume-design.md` (B1d R20 — adjacent rule additions)
- **Source:** `internal/protocol/types.go:40-58` (MessageType constants) + `internal/hub/hub.go:153,228` (current routing)
- **Drift exhibit:** Brian's msg 5044 framing ("we used update for status, response for BRAIN replies, command for directives — but inconsistently") + this-session traffic pattern observation

---

## 9. Status

- **Audit complete.** Feeds T1.7 impl directly.
- **No external blockers.** Open questions §6 are impl-time decisions.
- **Sequencing:** T1.7 lands AFTER T1.2 registry-substrate (R21 entry slot needed). Can parallel with T1.5 (R20).
