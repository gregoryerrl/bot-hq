# Bootstrap-on-Conversation-Resume Design Spike (B1d / T1.5) — Phase J

**Author:** Rain (BRAIN/EYES, design-spike per Phase J Q1 Rain HANDS markdown-design-doc scope)
**Date:** 2026-04-29
**Driver:** Phase J T1.5 (B1d) — implementation-this-phase (was Tier-3 design-spike-only, promoted to Tier-1 impl per user scope-correction msgs 5039-5060). This doc is the design-lock artifact; impl follows immediately.
**Source-substrate:** Phase I drift exhibit (BCC-into-bot-hq summary-fragment leak, msg 4936 incident) + user context-injection findings (1)–(7) (msgs 5022-5028)

---

## 1. Problem statement

R16 CROSS-RESTART-RESUME-OPERATIONAL handles **session-restart** resume (agent's tmux session was killed + respawned). It does NOT handle **conversation-resume** within an alive session — specifically, the case where Claude Code's **auto-compact** silently replaces prior message history with a summary while the agent's session continues.

Drift class observed (Phase I): post-auto-compact, the summary blended two same-day tier-1 lists (BCC bcc-ad-manager + bot-hq Phase I). Agent acted on the fragmented summary as if it were authoritative scope. R10/R13 discipline catches drift at draft-time but not at restore-time.

**B1d goal:** detect context-discontinuity AT TURN-START (not at draft-time) and trigger R16-equivalent bootstrap before any draft work resumes.

---

## 2. Constraints (per user context-injection findings 1–7, msgs 5022-5028)

- **C-1:** Auto-compact threshold is hardcoded; no settings/env/CLI lever.
- **C-2:** NO PreCompact hook. Only `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `Stop`, `PreToolUse`, `PostToolUse` exist.
- **C-3:** CLAUDE.md "Compact Instructions" section is the ONLY documented mechanism to influence post-compact retention. (B1(v) leverages this — see §3.1 below.)
- **C-4:** `/compact focus <topic>` slash-cmd guides next compaction. (B5e, T2.2 — orthogonal lever.)
- **C-5:** `/context` shows usage breakdown. (B5f — diagnostic sensor.)
- **C-6:** Skills with `disable-model-invocation: true` — load-on-demand mechanism. (B2d, T2.1 — orthogonal lever.)
- **C-7:** Best practices: persist state in CLAUDE.md not history.

**Implication:** B1d cannot be hook-driven (no PreCompact hook). Must be agent-internal heuristic + leverage CLAUDE.md retention + hub.db cross-correlation.

---

## 3. Detect-mechanism options surveyed

### 3.1 Option D-A: CLAUDE.md anchor + hub_read divergence check (RECOMMENDED)

**Mechanism:** at every scope-affecting turn-start (per R10 SCOPE-LOCK-BEFORE-IMPL trigger), agent:
1. Reads the CLAUDE.md Compact Instructions pointer block (B1(v)) — always-present per finding (3).
2. Looks up `last_known_self_msg_id` (agent's own most-recent hub_send msg-id, persisted to CLAUDE.md or `~/.bot-hq/<agent>/last_state.json` at every commit / SNAP-event).
3. Calls `hub_read since_id=<last_known_self_msg_id>` to enumerate msgs since last-known.
4. **Discontinuity heuristic:** if `hub_read` returns >0 own msgs that the agent's in-context history does NOT mention → context-compacted-or-fragmented.
5. On discontinuity → trigger R16 bootstrap (a)/(b)/(c)/(d).

**Pros:**
- Zero hook/harness dependencies (works within current Claude Code constraints).
- Leverages B1(v) infrastructure already shipping in T1.1.
- Detects asymmetry between agent's MEMORY of own actions vs hub.db ground-truth.
- Cheap: one `hub_read` per scope-affecting turn-start (already pattern in agent code).

**Cons:**
- Requires `last_known_self_msg_id` persistence — adds write-overhead at commit/SNAP events.
- Heuristic may false-positive if agent did legitimate work that's mid-context-but-truncated (rare: post-compact summary should still mention agent's own actions).
- Requires the agent to RECOGNIZE its own msg-IDs in context — relies on prompt-following.

**Persistence target:** `~/.bot-hq/<agent>/last_state.json` (lightweight JSON file). Updated on each successful `hub_send` of msg-type=update or scope-relevant action. Schema:

```json
{
  "last_self_msg_id": 5074,
  "last_commit_sha": "632d438",
  "last_phase_doc": "phase-j.md",
  "last_ratchet_pull_at": "2026-04-29T01:30:00Z",
  "last_state_write_at": "2026-04-29T02:15:42Z"
}
```

### 3.2 Option D-B: Heartbeat counter + state-fingerprint

**Mechanism:** agent maintains an in-context turn-counter starting at `1` and incremented each turn. After each turn, agent persists counter + state-fingerprint (commit-SHA + msg-IDs hash) to file. On next turn-start, agent reads file: if counter+fingerprint-on-disk diverges from in-context counter, compaction occurred.

**Pros:**
- More precise than D-A (counter is authoritative).
- Detects every compaction event including non-scope-affecting turns.

**Cons:**
- Requires precise turn-counting; if model loses count mid-context, false-positives.
- Heavy: every turn-end requires file-write.
- Counter survives in CLAUDE.md only if added to Compact Instructions (must merge with B1(v) block — design coupling).

### 3.3 Option D-C: Length-based via /context polling

**Mechanism:** agent runs `/context` at turn-start, compares context-% to expected-trajectory. If context-% dropped significantly (e.g., 80% → 30%) → compaction occurred.

**Pros:**
- Direct signal from harness.

**Cons:**
- `/context` is a slash-cmd; agent self-issue feasibility depends on PB2 outcome (msg 5037 unresolved).
- Even if feasible, context-% can drop for non-compact reasons (e.g., tool-result trimming).
- Doesn't tell agent WHAT was lost, only that something was.

### 3.4 Option D-D: Summary-text sentinel

**Mechanism:** on each turn-start, agent grep-checks first 500 chars of in-context "history" for a known auto-compact-summary marker (e.g., "[Conversation summarized]" or similar harness emit).

**Pros:**
- Direct detection if harness emits a marker.

**Cons:**
- Marker text is undocumented (no spec). Reverse-engineering required.
- Brittle: marker may change with Claude Code version updates.

### 3.5 Option D-E: Hub-mediated heartbeat

**Mechanism:** agent emits `hub_send` heartbeat every N turns with state-fingerprint. On next turn, agent runs `hub_read since_id=<expected>` looking for own heartbeat. Absence of expected heartbeat in own context → compaction.

**Pros:**
- Cross-correlates against hub.db ground-truth.
- Heartbeat is also useful for B1b structured-state ledger (T2.3) — shared infra.

**Cons:**
- Hub-traffic noise (every-N-turn heartbeat with no real news).
- N-tuning is heuristic.

---

## 4. Recommendation: Option D-A as primary; D-E as supplement

**T1.5 ship scope:**

### Primary — D-A (CLAUDE.md anchor + hub_read divergence check)

1. **New rule R20 BOOTSTRAP-ON-CONVERSATION-RESUME** added to PhaseIv1ProtocolHardening (or new PhaseJv2-class const): at every scope-affecting turn-start, agent verifies in-context history mentions `last_self_msg_id` from `~/.bot-hq/<agent>/last_state.json`. If absent → R16 bootstrap.
2. **State-persistence helper:** new file `internal/protocol/agentstate.go` — `WriteAgentState(agentID, state)` + `ReadAgentState(agentID)`. Called from agent code at `hub_send` success path (write) + at scope-affecting-turn-start (read).
3. **Wiring:** agents (Brian + Rain) call `WriteAgentState` after successful `hub_send` of important class (commit-narration, BRAIN-cycle-decision, scope-change). Agents call `ReadAgentState` + `hub_read` divergence check at turn-start of any commit/edit/scope-change action.
4. **Test-lock:** new test in `internal/protocol/agentstate_test.go` — locks JSON schema + write+read symmetry.

### Supplement — D-E (Hub heartbeat) deferred to T2.3 (B1b)

D-E is the structured-state ledger heartbeat already scoped as T2.3. Land T2.3 with heartbeat-emit; D-A's check can leverage heartbeat presence as supplementary signal.

### Why not D-B/D-C/D-D
- D-B: heavier write-overhead, counter precision risk.
- D-C: depends on PB2 (unresolved); /context output parse fragile.
- D-D: undocumented marker text, version-brittle.

D-A is the lowest-cost mechanism that achieves the detection goal under current constraints. Combined with B1(v) Compact Instructions (T1.1) which makes the relevant anchors survivable, the system has both proactive-survival (B1(v)) and reactive-detection (B1d).

---

## 5. T1.5 implementation sketch (Brian-HANDS)

### 5.1 Files to create

| File                                          | Purpose                                                                     |
| --------------------------------------------- | --------------------------------------------------------------------------- |
| `internal/protocol/agentstate.go`             | `WriteAgentState`/`ReadAgentState`/`AgentState` struct + JSON serialization |
| `internal/protocol/agentstate_test.go`        | Schema lock + write+read symmetry + missing-file-handling                   |

### 5.2 Files to edit

| File                                    | Change                                                                |
| --------------------------------------- | --------------------------------------------------------------------- |
| `internal/protocol/disc.go`             | Add R20 `BOOTSTRAP-ON-CONVERSATION-RESUME` rule to PhaseIv1ProtocolHardening (or new const) — prompt-side discipline |
| `internal/protocol/disc_test.go`        | Extend `TestPhaseIv1ContentShape` with R20 name                       |
| `internal/protocol/registry.go` (new in T1.2) | Add R20 entry to Rules slice                              |
| `internal/{brian,rain}/{brian,rain}.go` | Call `WriteAgentState` after successful scope-relevant `hub_send`     |

### 5.3 Rule text draft (R20)

```
R20 BOOTSTRAP-ON-CONVERSATION-RESUME: at the start of every scope-affecting turn (commit, edit, scope-change, BRAIN-cycle-decision), verify context-continuity. Read `~/.bot-hq/<self-agent-id>/last_state.json` for `last_self_msg_id`. Run `hub_read since_id=<last_self_msg_id>` and check if your own msg-IDs from that range appear in your in-context history. If discontinuity (your own msgs missing or fragmented) → trigger R16 CROSS-RESTART-RESUME-OPERATIONAL bootstrap before proceeding. The CLAUDE.md Compact Instructions block (B1(v)) preserves anchors through summarization; R20 is the active check. Discriminator: in-context memory of own action ≢ hub.db ground-truth = drift. Don't draft on drift; bootstrap first.
```

### 5.4 AgentState schema (Go)

```go
package protocol

type AgentState struct {
    LastSelfMsgID    int64     `json:"last_self_msg_id"`
    LastCommitSHA    string    `json:"last_commit_sha"`
    LastPhaseDoc     string    `json:"last_phase_doc"`        // basename of ~/.bot-hq/phase/<active>.md
    LastRatchetPull  time.Time `json:"last_ratchet_pull_at"`  // when agent last read ratchets/active.md
    LastStateWrite   time.Time `json:"last_state_write_at"`   // when this AgentState was written
}

// WriteAgentState writes state to ~/.bot-hq/<agentID>/last_state.json
// (creates dir if absent). Best-effort: errors logged not raised.
func WriteAgentState(agentID string, state AgentState) error { ... }

// ReadAgentState reads state from ~/.bot-hq/<agentID>/last_state.json.
// Returns (zero-value, nil) if file absent (first-ever-call path).
func ReadAgentState(agentID string) (AgentState, error) { ... }
```

### 5.5 Test cases (agentstate_test.go)

- Write+Read symmetry (round-trip preserves all fields).
- Missing-file → zero-value AgentState (no error).
- Concurrent-write safety (file-lock or atomic-rename).
- Schema-lock: JSON output contains exact keys (catches accidental field removal).

---

## 6. Operational behavior

### 6.1 Steady-state (no compaction)

- Agent posts hub_send msg-id N → AgentState.LastSelfMsgID = N persisted.
- Next turn: agent reads AgentState (N), hub_read since N. No new-self-msgs. In-context history contains msg-N reference. Continue normally.

### 6.2 Post-compaction (drift detected)

- Agent posts hub_send N → state persisted.
- Auto-compact fires; agent's in-context history replaced with summary. Summary may or may not mention msg-N.
- Next turn: agent reads AgentState (N), hub_read since N. May see new msgs from peer or self that arrived during/after compaction.
- Agent grep in-context history for msg-N reference. If absent → R16 bootstrap.

### 6.3 Cold-start (no AgentState file)

- First-ever turn: ReadAgentState returns zero-value (no error).
- Agent treats as fresh-session; falls through to regular R16 bootstrap if SNAP exists, idle if no SNAP (per pass-1 SNAP-gate from msg 4929).

---

## 7. Findings + open questions

### F1: Drift-detection is heuristic, not deterministic
- D-A's "did own msg-N appear in context?" is a probabilistic test. False-negative possible if summary preserves msg-N reference but loses other context.
- **Mitigation:** combine with R13 SCOPE-VERIFY-PRE-DRAFT (already shipping) — every scope-relevant draft cross-checks against scope-doc + recent hub. Defense-in-depth.

### F2: Persistence write-overhead
- D-A writes `~/.bot-hq/<agent>/last_state.json` after each scope-relevant hub_send. Estimate ~100 writes/hr peak BRAIN-cycle.
- **Mitigation:** acceptable I/O for SQLite-class persistence. JSON file is small (~200B).

### F3: First-ever-call gracefulness
- Cold-start path must handle missing file without error. Schema lock test covers.

### F4: Cross-agent symmetry
- Both Brian + Rain need agentstate write/read paths. Symmetric edit; T1.2 const-consolidation pattern applies.

### F5: Interaction with B1(v) CLAUDE.md Compact Instructions
- B1(v) preserves anchors through compaction (proactive). B1d detects drift (reactive). Together, both proactive-survival + reactive-detection.
- The Compact Instructions block in `~/CLAUDE.md` (per investigation #5 Option α) should reference B1d as the active check companion.

### Open questions for impl-time (Brian decides)
1. **R20 placement:** PhaseIv1 expansion (R17/R18/R19/R20) OR new PhaseJv2-class const? My lean: PhaseIv1 expansion (cohesive cycle).
2. **AgentState file location:** `~/.bot-hq/<agentID>/last_state.json` (per-agent dir) OR `~/.bot-hq/agentstate/<agentID>.json` (flat dir)? My lean: per-agent-dir (allows future per-agent state expansion).
3. **Trigger granularity:** every-turn check vs scope-affecting-turn check? My lean: scope-affecting only (commit, edit, scope-change, BRAIN-cycle-decision) — minimizes overhead.
4. **Discontinuity action:** R16 bootstrap (full re-read) vs lighter "ratchet-pull + scope-doc-pull only"? My lean: R16 (defense-in-depth).
5. **Test coverage gate:** `TestRuleNamespaceRatchet` (T1.2) verifies R20 has registry entry; `TestAgentStateRoundTrip` covers persistence. Anything else? My lean: probe-case in T1.4 B5 — "post-discontinuity-detection bootstrap fires R16 sequence" probe.

---

## 8. Cross-references

- **Phase J spec:** `~/.bot-hq/phase/phase-j.md` §T1.5 (B1d-impl + design-spike) — this doc is the design-spike artifact
- **Companion investigations:** `docs/plans/2026-04-29-rule-loci-audit.md` (B3a — registry slice for T1.2 lays substrate for R20) + `docs/plans/2026-04-29-sentinel-content-shape-corpus.md` (B4 — orthogonal but T1.4 probe-case shares pattern)
- **Constraint sources:** user context-injection findings (1)–(7) msgs 5022-5028
- **Drift exhibit:** msg 4936 (BCC-into-bot-hq Phase I drift incident)

---

## 9. Status

- **Design-spike complete.** T1.5 impl can begin on Brian-HANDS greenflag.
- **No external blockers** from this design. Open questions §7 are impl-time decisions.
- **Sequencing:** T1.5 impl follows T1.2 schema-substrate landing (registry needs R20 entry slot). Can land in parallel with T1.1 R17/R18/R19 once T1.2 registry framework is in place.
