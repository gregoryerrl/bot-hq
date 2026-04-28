# Rule Loci Audit (B3a) — Phase J Tier-1 T1.2 Schema-Design Input

**Author:** Rain (BRAIN/EYES, read-only investigation per DISC v2)
**Date:** 2026-04-29
**Driver:** Phase J T1.2 (B3{a,b,c,d}+bonus) — feeds T1.2 schema design (B3d rule-namespace-ratchet)
**Source-substrate:** Brian's T1.2-prep substrate map (msg 5040) + grep-survey on `internal/protocol/disc.go` + `internal/{rain,brian}/{rain,brian}.go` + `internal/gemma/plan_usage.go`

---

## 1. Goal

Feed T1.2 (B3d) schema design with a complete enumeration of every rule in the bot-hq trio prompt surface, including:

- **const_name** — where defined (or `inline-<file>:<line>` if not yet const)
- **test_lock_test_name** — which Go test pins the rule (presence + substance)
- **history_pointer** — where rationale + msg-ID history lives (currently: Go-comment above const; T1.2 may move to `docs/arcs/phase-<id>.md`)
- **payload_mirror** — for runtime-emit rules, the corresponding string in production code (T1.2 B1(iv) ratchet test asserts const ≡ payload)
- **agent_applicability** — Brian-only / Rain-only / both — feeds B2b per-agent-tailoring decisions

This audit is the read-only substrate. T1.2 schema design + impl is Brian-HANDS per DISC v2.

---

## 2. Rule enumeration

### 2.1 Const-shared rules (already in `internal/protocol/disc.go`)

#### Rule: OUTBOUND

| Field                    | Value                                                                         |
| ------------------------ | ----------------------------------------------------------------------------- |
| const_name               | `protocol.DiscV2OutboundRule`                                                 |
| const_loc                | `internal/protocol/disc.go:21-23` (~596B)                                     |
| embedded_in              | `internal/brian/brian.go:260` + `internal/rain/rain.go:246`                   |
| test_lock_test_name      | `TestInitialPromptEmbedsDiscV2OutboundRule` (brian_test.go:83 + rain_test.go:149) — embedding-locks; `TestDiscV2OutboundRule_RatchetLiterals` (disc_test.go:17) — content-shape; `TestDiscV2OutboundRule_HeaderAnchor` (disc_test.go:35) — header anchor |
| history_pointer          | Go-comment disc.go:3-20 (extraction history; DISC v2 audience-driven routing locked at msg 2147; final wording 2325/2327; const extraction 2326/2328) |
| payload_mirror           | none (no runtime-emit equivalent)                                             |
| agent_applicability      | both                                                                          |

#### Rule: PHASE-I PROTOCOL HARDENING (R1–R16)

The const `protocol.PhaseIv1ProtocolHardening` is one Go const containing 16 sub-rules. The ratchet question for T1.2 B3d: treat as **single rule with sub-items** (current shape) OR **enumerate sub-items as 16 separate rules** under the namespace? Recommendation: **enumerate** so rule-namespace-ratchet test can iterate per-sub-rule.

| Field                    | Value                                                                         |
| ------------------------ | ----------------------------------------------------------------------------- |
| const_name               | `protocol.PhaseIv1ProtocolHardening`                                          |
| const_loc                | `internal/protocol/disc.go:63-91` (~11.4KB)                                   |
| embedded_in              | `internal/brian/brian.go:261` + `internal/rain/rain.go:247`                   |
| test_lock_test_name      | `TestInitialPromptEmbedsPhaseIv1ProtocolHardening` (embedding) + `TestPhaseIv1ContentShape` (brian_test.go:106 — 16 rule names locked by string presence) |
| history_pointer          | Go-comment disc.go:25-62 (Phase I cycle msgs 4664/4666/4668/4675/4677/4680/4682; W0 expansion msg 4751-4753; W1a R15-R16 msg 4766-4769) |
| payload_mirror           | partial — see R16 below                                                       |
| agent_applicability      | both                                                                          |

**Sub-rules R1–R16 enumerated:**

| Sub | Name                              | Test-lock substring | payload_mirror_in_code | agent |
| --- | --------------------------------- | ------------------- | ---------------------- | ----- |
| R1  | HANDSHAKE-TERMINATOR              | rule-name presence  | none                   | both  |
| R2  | CROSS-TIMING-DEDUP                | rule-name presence  | none                   | both  |
| R3  | QUOTE-TRIM                        | rule-name presence  | none                   | both  |
| R4  | SNAP-GATING                       | rule-name presence  | none                   | both  |
| R5  | BRAIN-CYCLE-RESPONSE-SHAPE        | rule-name presence  | none                   | both  |
| R6  | TOOL-RESULT-DISCIPLINE            | rule-name presence  | none                   | both  |
| R7  | SUBAGENT-DISPATCH                 | rule-name presence  | none                   | both  |
| R8  | COMPACT-COMMIT-FORMAT             | rule-name presence  | none                   | both  |
| R9  | AUDIENCE-CLASS-DISCRIMINATOR      | rule-name presence  | none                   | both  |
| R10 | SCOPE-LOCK-BEFORE-IMPL            | rule-name presence  | none                   | both  |
| R11 | HALT-DISCIPLINE                   | rule-name presence  | none                   | both  |
| R12 | GATE-PROTOCOL                     | rule-name presence  | none                   | both  |
| R13 | SCOPE-VERIFY-PRE-DRAFT            | rule-name presence  | none                   | both  |
| R14 | HALT-95%-SNAP                     | rule-name presence  | none                   | both  |
| R15 | AGENT-AUTHORITY-MATRIX            | rule-name presence  | none                   | both  |
| R16 | CROSS-RESTART-RESUME-OPERATIONAL  | rule-name presence  | **`planCapResumeFmt` (plan_usage.go:59) + emitter sites :213, :331** — bootstrap-order (a)/(b)/(c)/(d) | both  |

**R16 payload-mirror detail (B1(iv) target):**
- Const text in PhaseIv1: lines 89-90 — bootstrap order is `(a) last commit + git status on active branches, (b) ~/.bot-hq/phase/<active-phase>.md for canonical scope, (c) ~/.bot-hq/ratchets/active.md for ratchet status, (d) hub_read recent backlog filtered to peer-coord since halt-fire`
- Payload text in plan_usage.go (per Brian's substrate map msg 5040): "plan usage reset to %d%%, resume work via R16 cross-restart-resume protocol bootstrap (a) git status (b) ~/.bot-hq/phase/<active-phase>.md (c) ~/.bot-hq/ratchets/active.md (d) hub_read backlog since halt-fire"
- **Asymmetry observed (Brian's surface, msg 5040):**
  - separator: const uses ", " between items; payload uses spaces
  - (a) wording: const = "last commit + git status on active branches"; payload = "git status" (omits "last commit + on active branches")
  - (d) wording: const = "hub_read recent backlog filtered to peer-coord since halt-fire"; payload = "hub_read backlog since halt-fire" (omits "recent" + "filtered to peer-coord")
- **B1(iv) test-shape decision** (defer to Brian as HANDS):
  - Option A: exact-match — const literal substring must appear in payload literal. Risk: brittle, locks current asymmetry.
  - Option B: shared-substring-set — both must contain `(a)`, `(b)`, `(c)`, `(d)` markers + each marker followed by a token-set including (`git status`/`commit`), (`phase/<active-phase>.md`), (`ratchets/active.md`), (`hub_read`/`backlog`/`halt-fire`). Robust, but allows wording drift.
  - Option C: structured-extraction — parse both into `{(a): step, (b): step, ...}` maps, assert key-set equality + per-key substring intersection ≥ N tokens. Most robust, most code.
  - **My lean: B (shared-substring-set)** — preserves R16 semantic shape (4-step bootstrap, in-order) without locking literal wording. Allows future const updates without payload sync (or payload updates without const sync) provided the 4-step shape holds. If wording drifts past the substring-intersection threshold, test fires.

#### Rule: H-13 FORCE-PUSH-TOKEN-PROTOCOL

| Field                    | Value                                                                         |
| ------------------------ | ----------------------------------------------------------------------------- |
| const_name               | `protocol.H13ForcePushProtocol`                                               |
| const_loc                | `internal/protocol/disc.go:105-111` (~910B)                                   |
| embedded_in              | `internal/brian/brian.go:287` (Brian-only)                                    |
| test_lock_test_name      | `TestInitialPromptContainsH13ForcePushProtocol` (brian_test.go:367); coder-side mirror in `internal/mcp/preamble_test.go:135` |
| history_pointer          | Go-comment disc.go:93-104 (Phase H slice 1; rules.ForcePushBlocked + buildCoderPreamble coupling) |
| payload_mirror           | none in agent prompts; protocol governs interaction with `internal/mcp/tools.go buildCoderPreamble` (coder-side) |
| agent_applicability      | **Brian-only** (Rain does not dispatch coders, does not relay force-push tokens)  |

### 2.2 Inline rules (T1.2 B3b extract targets — not yet const)

These rules currently live as multi-line strings inside `initialPrompt()` of each agent file. Symmetric edits are required for both files; this is the drift-surface T1.2 B3b is fixing by promoting to a shared const.

#### Rule: H-31 HALT-ALL-WORK

| Field                    | Value                                                                         |
| ------------------------ | ----------------------------------------------------------------------------- |
| const_name               | **none yet** — inline at brian.go:284 + rain.go:265                           |
| proposed const_name (B3b)| `protocol.PhaseJv1HaltResumeProtocol` (one const containing both H-31 + RESUME-FROM-HALT) — OR split into `PhaseJv1HaltAllWorkRule` + `PhaseJv1ResumeFromHaltRule` — **schema-design decision** |
| embedded_in              | `internal/brian/brian.go:284` + `internal/rain/rain.go:265`                   |
| test_lock_test_name      | `TestBrianPromptContainsHaltAllWork` (brian_test.go:275) + `TestRainPromptContainsHaltAllWork` (rain_test.go:222) — substring-locked |
| history_pointer          | currently inline string content carries history (msg 4929 SNAP-gate refinement reference); proposed: Go-comment above new const, mirror PhaseIv1 pattern |
| payload_mirror           | partial — H-31 trigger-substrings `"agent <id> at <N>%, halt"` and `"plan usage at <N>%, halt"` are emitted by Emma (`internal/gemma/context_cap.go` for context-cap; `internal/gemma/plan_usage.go:firePlanCapHalt` line ~239 for plan-cap). Symmetry test target: const trigger-substring = emitter format-string. |
| agent_applicability      | both                                                                          |

#### Rule: RESUME-FROM-HALT

| Field                    | Value                                                                         |
| ------------------------ | ----------------------------------------------------------------------------- |
| const_name               | **none yet** — inline at brian.go:285 + rain.go:266                           |
| proposed const_name (B3b)| see H-31 above (paired or split with H-31)                                    |
| embedded_in              | `internal/brian/brian.go:285` + `internal/rain/rain.go:266`                   |
| test_lock_test_name      | `TestBrianPromptContainsResumeFromHalt` (brian_test.go:303) + `TestRainPromptContainsResumeFromHalt` (rain_test.go:250) — substring-locked |
| history_pointer          | currently inline (msg 4929 SNAP-gate refinement); proposed: Go-comment above new const |
| payload_mirror           | **`planCapResumeFmt` const at plan_usage.go:59** (Brian substrate map msg 5040) — emitted by `emitPlanCapResume` (line ~213) + `wakePayload` (line ~331). Trigger-substring `"plan usage reset"` must match prompt-rule recognition. **B1(iv) ratchet test target — see R16 payload-mirror detail above for asymmetry analysis.** |
| agent_applicability      | both                                                                          |

---

## 3. Schema design input (T1.2 B3d feed)

### 3.1 Schema fields per rule (proposed, decision deferred to Brian-HANDS)

```
type Rule struct {
    ID                  string   // canonical identifier (e.g., "R1", "H-13", "H-31")
    Name                string   // rule-name (e.g., "HANDSHAKE-TERMINATOR")
    ConstName           string   // Go-symbol path (e.g., "protocol.PhaseIv1ProtocolHardening" — for R1-R16, this points at parent; sub-rules denoted via SubID)
    SubID               string   // optional — for sub-rules within bundled const (e.g., "R1" within PhaseIv1)
    EmbeddedIn          []string // file:line locations agent-prompts embed at
    TestLockTestNames   []string // Go test functions that pin the rule
    HistoryPointer      string   // Go-comment loc OR docs/arcs/<id>.md path
    PayloadMirror       string   // optional — file:line of runtime-emit equivalent (e.g., "plan_usage.go:59:planCapResumeFmt" for R16)
    AgentApplicability  []string // ["brian"], ["rain"], or ["brian", "rain"]
}
```

### 3.2 Ratchet-test enumerator (T1.2 B3d) implementation sketch

```go
var rules = []Rule{
    {ID: "OUTBOUND",  ConstName: "protocol.DiscV2OutboundRule", ...},
    {ID: "R1",        ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R1", Name: "HANDSHAKE-TERMINATOR", ...},
    // ...R2..R16
    {ID: "H-13",      ConstName: "protocol.H13ForcePushProtocol", AgentApplicability: []string{"brian"}, ...},
    {ID: "H-31",      ConstName: "protocol.PhaseJv1HaltResumeProtocol", SubID: "HALT-ALL-WORK", PayloadMirror: "...", ...},
    {ID: "RESUME",    ConstName: "protocol.PhaseJv1HaltResumeProtocol", SubID: "RESUME-FROM-HALT", PayloadMirror: "plan_usage.go:59:planCapResumeFmt", ...},
}

func TestRuleNamespaceRatchet(t *testing.T) {
    for _, r := range rules {
        // assert const_name resolves
        // assert test_lock_test_names exist (via reflection or registered-test list)
        // assert history_pointer non-empty
        // if payload_mirror non-empty: load runtime-emit string, run shared-substring-set assertion
        // assert agent_applicability matches actual embed sites in brian.go / rain.go
    }
}
```

### 3.3 Per-agent-tailoring (B2b) feed

Currently:
- **Both agents:** OUTBOUND + R1–R16 (PhaseIv1) + H-31 + RESUME-FROM-HALT
- **Brian-only:** H-13 (already correctly Brian-only — no Rain pollution)
- **Rain-only:** none

T1.2 B3b extraction does NOT change per-agent applicability. B2b per-agent tailoring scope question: **are there sub-rules within PhaseIv1 R1–R16 that could be Brian-only or Rain-only?** Candidates surveyed:

| Sub | Could be agent-tailored?                                                                   |
| --- | ------------------------------------------------------------------------------------------ |
| R7  | SUBAGENT-DISPATCH — Brian uses Task tool + hub_spawn (HANDS); Rain uses hub_spawn_gemma for analyze: queries (EYES). Both relevant; **keep both**. |
| R12 | GATE-PROTOCOL — references "force-push H-13-token-gated" which is Brian-only. **But Rain BRAIN-2nd needs to know the gate exists** to BRAIN on diffs. **Keep both**.  |
| R15 | AGENT-AUTHORITY-MATRIX — Brian-side + Rain-side authority enumerated. **Keep both** (mutual-authority awareness is load-bearing).                                  |

**Conclusion: no per-agent-tailoring savings within R1–R16.** B2b savings come from (a) trim prose / move rationale to docs/arcs (B2a), (b) skills lazy-load (B2d), (c) test-locked compression (B2c).

---

## 4. Findings + recommendations

### F1: Asymmetry in R16 const ↔ plan_usage.go payload (B1(iv) blocker)
- Const text (disc.go:89-90) vs payload text (plan_usage.go:59) drifted in (a) and (d) wording (see §2.1 R16 detail).
- **Recommendation:** B1(iv) ratchet test uses Option B (shared-substring-set assertion) per my lean above. Brian decides at impl-time.

### F2: H-31 trigger-substring symmetry (additional B1(iv)-class target)
- Const inline at rain.go:265 + brian.go:284 references trigger-substrings `"agent <id> at <N>%, halt"` and `"plan usage at <N>%, halt"`.
- Emitters: Emma's `firePlanCapHalt` (plan_usage.go:239) emits `"[CRITICAL] plan usage at <N>%, halt + checkpoint via H-15 + idle for fresh session"` (per gemma/plan_usage.go content shipped Phase I Fix-1).
- **Substring `"plan usage at"` + `"halt"` present in both — substring-set test passes.**
- **But: const says "checkpoint via H-15" / "idle for fresh session"** ⚠️ — H-15 is now obsolete (Phase I Fix-3 replaced "fresh session" with "idle in pane"). The trigger-substring const-text in PhaseJv1HaltResumeProtocol must be checked: does H-31 trigger-substring still match emitter wording? If emitter still says "fresh session" and prompt says "idle in pane", recognition still works (substring is "halt"), but the post-halt-action text drifted.
- **Recommendation:** B3b extraction also revisits H-31 prompt-rule wording vs current emitter format-string. May surface a Phase I residual cleanup.

### F3: Bundled-const PhaseIv1 — sub-rule enumeration via parse vs registry
- PhaseIv1 is one big multi-line string. Sub-rule enumeration could be: (a) parse the const at test-time via line-prefix matching (`- <RULE-NAME>:`), (b) maintain a separate Go slice/map registry alongside the const.
- (a) lower-maintenance but couples test to const formatting; (b) explicit registry but two sources to keep in sync.
- **Recommendation:** (b) registry — explicit, doesn't depend on prose-formatting stability; the registry IS the schema enumerator T1.2 B3d builds.

### F4: history_pointer current-state vs T1.2 target
- All current rule history-pointers are Go-comments above the const definition (see §2.1).
- Phase J Out-of-scope §289 says "deferred to Phase K: B1d bootstrap-on-conversation-resume impl" — does T1.2 history_pointer move rationale OUT of Go-comments INTO `docs/arcs/phase-i.md`? B2a in T2.1 says yes (move rationale + history to docs/arcs/).
- **Recommendation:** T1.2 schema design treats history_pointer as a string field — value can be either Go-comment-loc OR docs/arcs/-path. T2.1 (B2a, gated on T1.4) migrates Go-comments → docs/arcs/ files. T1.2 ratchet enumerator works with either form.

### F5: B5e + B5f payload-mirror class — future runtime-emit rule additions
- B5e (`/compact focus <topic>` self-issue) — when shipped in T2.2, the agent emits a slash-command. Const-side rule will reference `/compact focus`; payload-side: agent's text-emit (or ScheduleWakeup payload). Same payload_mirror discipline applies.
- B5f (`/context` 2nd sensor) — agent reads /context output. The "rule" is operational pattern, not a hub-emit, so payload_mirror = none.
- **Recommendation:** schema field `PayloadMirror` accommodates future runtime-emit rules. T2.2 shipping adds these to the rules registry.

---

## 5. Open questions for T1.2 schema design (Brian decides)

1. **Bundled vs split:** PhaseJv1HaltResumeProtocol = one const with H-31 + RESUME-FROM-HALT, OR two consts? My lean: one const (matches PhaseIv1 pattern, single-extract operation, paired by halt-cycle).
2. **PhaseIv1 sub-rule enumeration mechanism:** parse-from-const vs registry-slice (my lean: registry, see F3).
3. **Agent_applicability enforcement:** ratchet test cross-checks Rule.AgentApplicability against actual embed sites in brian.go + rain.go. Should test-fail if mismatch (e.g., H-13 accidentally embedded in rain.go)? My lean: yes, prevents pollution.
4. **R16 payload-mirror test shape:** A (exact-match) / B (shared-substring-set) / C (structured-extraction). My lean: B.
5. **History_pointer field type:** string (Go-comment-loc OR docs/arcs/-path), with T2.1 migration path. My lean: keep flexible, T2.1 migrates.
6. **F2 H-31 trigger-substring vs emitter post-halt-action drift:** address in B3b or surface as Phase I residual issue separate from T1.2? My lean: include in B3b — small scope creep but eliminates a stale-rule-text class.

---

## 6. Cross-references

- **Phase J scope-lock:** `~/.bot-hq/phase/phase-j.md` (T1.2 B3a feed-source for this audit)
- **Brian substrate map:** msg 5040 (provided const loci + inline loci + plan_usage.go payload-mirror loci)
- **Source files audited:** `internal/protocol/disc.go`, `internal/brian/brian.go`, `internal/rain/rain.go`, `internal/{brian,rain}/*_test.go`, `internal/protocol/disc_test.go`
- **Source files referenced (not opened yet):** `internal/gemma/plan_usage.go` (B1(iv) target — Brian's substrate map sufficient for audit; full read at T1.2 impl time)
- **Phase I const history:** disc.go Go-comments (lines 3-20 OUTBOUND; lines 25-62 PhaseIv1; lines 93-104 H-13)

---

## 7. Status

- **Audit complete.** Feeds T1.2 B3d schema design.
- **Next Rain investigations (per priority lean):** #5 PB1 CLAUDE.md cwd-verify, then parallel: #1 B4 corpus, #4 B1d design-spike, #6 PB2 slash-cmd-feasibility, #7 PB3 skill-mechanism.
- **Blocker for T1.2:** none from this audit. Brian's open-questions §5 are schema-design choices, made at T1.2 impl time.
