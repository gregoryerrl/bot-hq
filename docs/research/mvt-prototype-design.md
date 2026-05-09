# MVT Prototype Design (Phase T-0.5)

**Captured:** 2026-05-09 (Phase T-0.5 MVT prototype)
**Purpose:** De-risk substantial Phase T rebuild via small-validation-first per phase-t.md v5 T-0.5 sub-phase. Demonstrate IPIV cycle on representative task end-to-end with minimal IPIV state-machine + investigator-tool wrapper.

## Scope

**MVT (Minimum-Viable-Test) deliverables:**
1. ✓ IPIV state-machine schema types (Stage / Mode / TaskState / PhaseBudget / PhaseUsage / PhaseArtifacts / PhaseHistoryEntry)
2. ✓ Phase-transition validator (sequential I → P → Implement → V + Verify-fail loop-back)
3. ✓ Mode-transition validator (R45 EXTENDED per-stage modes)
4. ✓ Bilateral-mode auto-set per R44 expanded (medium/high-stakes Investigate + Plan → bilateral)
5. ✓ Atomic save/load YAML round-trip
6. ✓ Simulated end-to-end cycle test (I → P → Implement → V with cost-tracking captured)

**NOT in MVT scope (T-2 expansion):**
- Per-rule mechanical-enforcement hooks (R44/R45/R46/R47/R49/R50)
- Bilateral-Investigate primitives (cross-model live execution)
- Full investigator-toolset (clusters A-E + new tools 6-16)
- Cost-tracking instrumentation (R53; T-5 expansion)

## Implementation

**Package:** `github.com/gregoryerrl/bot-hq/internal/mvt`
**File:** `internal/mvt/ipiv_state.go` (~250L Go)
**Tests:** `internal/mvt/ipiv_state_test.go` (~280L Go; 9 test functions)

**Test results:** ALL PASS
```
PASS: TestNewTaskState_defaults
PASS: TestValidatePhaseTransition_valid (8 sub-cases)
PASS: TestValidatePhaseTransition_invalid (3 sub-cases)
PASS: TestValidateMode_perStage (9 sub-cases)
PASS: TestTransition_bilateralAutoSet
PASS: TestTransition_lowStakesSolo
PASS: TestSaveLoad_roundTrip
PASS: TestTaskStatePath_canonical
PASS: TestEndToEndCycle_simulated
```

## Key API surface

```go
// Create new task with decision-class
ts := mvt.NewTaskState("task-uuid", mvt.DecisionHigh)

// Phase-transition with auto-bilateral-mode for medium/high-stakes
err := ts.Transition(mvt.StagePlan, agentModelMap)
// → ts.PhaseMode = mvt.ModePlanBilateral (auto-set per R44 expanded)

// Atomic save
err := ts.Save("/path/to/ipiv-state.yaml")

// Load
ts, err := mvt.Load("/path/to/ipiv-state.yaml")

// Canonical storage path
path := mvt.TaskStatePath(homeDir, projectID, taskID)
```

## End-to-end simulated cycle (TestEndToEndCycle_simulated)

Demonstrates full IPIV cycle on representative high-stakes task:

```
1. NewTaskState("e2e-cycle-1", DecisionHigh)
   → CurrentPhase=I, PhaseMode=investigate-solo (default)

2. [bilateral Investigate per R44 expanded]
   PhaseMode=investigate-collaborative
   PhaseHistory[0]={Phase=I, SubPhase=pre-hypothesis, BilateralMode=bilateral, AgentModelAtPhase={brian:claude, rain:deepseek}}
   PhaseUsed[I]={Tokens=50000, Cost={brian:$0.50, rain:$0.30}}

3. Transition(StagePlan)
   → PhaseMode=plan-bilateral (auto per R44 expanded for high-stakes)
   PhaseUsed[P]={Tokens=15000, Cost={brian:$0.15, rain:$0.10}}

4. Transition(StageImplement)
   → PhaseMode=implement (Brian solo per R46 partial-supersede)
   PhaseUsed[Implement]={Tokens=8000, Cost={brian:$0.08}}

5. Transition(StageVerify)
   → PhaseMode=implement-verify (Rain solo adversarial)
   VerifyResult=pass
   PhaseUsed[V]={Tokens=22000, Cost={rain:$0.22}}

Final state:
- 4 phase-transitions
- Total tokens: 95,000
- Total cost: $1.35
- VerifyResult: PASS
```

## Mapping to T-2 expansion

**MVT validates the design assumptions for T-2 implementation:**

| MVT validates | T-2 implements |
|---|---|
| IPIV schema + types | Production-grade schema migration + corruption-recovery + concurrent-write-handling |
| Phase-transition validator | Phase-transition gates with hook-firing (R45 mode-tag / R47 decision-class / R49 pre-seal-audit / R50 bare-dot-block) |
| Bilateral-mode auto-set per R44 expanded | Bilateral-Investigate primitives (cross-model live execution; symmetric Pattern A+B+D pre-hypothesis; asymmetric strong-style hypothesis-investigation) |
| Atomic save/load YAML | Per-task state-machine + phase-transition gates + concurrent-write-handling |
| Simulated cost-tracking | Cost-tracking instrumentation (T-5 hookin) |
| Single-agent simulated cycle | IPIV-first-task validation (real cross-model bilateral on representative task) |

## Risks identified during MVT

1. **YAML field-tag verbosity:** TaskState struct has many YAML tags; consider code-generation OR simpler struct+gob if YAML overhead becomes issue. **Mitigation:** keep YAML for human-readability; benchmark at T-2 if hot-path.

2. **Phase-history append pattern memory growth:** unbounded PhaseHistory slice; long-running tasks could accumulate entries. **Mitigation:** T-2 implementation includes max-entries-cap with rolling-archive.

3. **Mode-defaulting logic in Transition()** doesn't account for sub-phase modes (e.g., navigator vs driver during Investigate hypothesis-investigation). **Mitigation:** T-2 implementation extends with explicit mode-set API for sub-phase transitions.

4. **No concurrent-write handling** in MVT save: file rename race possible if two writers. **Mitigation:** T-2 adds file-lock or use SQLite-backed state.

5. **Cost-tracking is manual** (test sets PhaseUsed.CostPerAgent directly). **Mitigation:** T-5 cost-tracking-per-agent hooks into agent-spawn-routine + LLM-call subprocess output parsing.

## Recommendations for T-2 implementation

1. **Adopt MVT schema as foundation;** extend with hook-firing integration points
2. **Add per-stage sub-phase tracking** (pre-hypothesis vs hypothesis-investigation for Investigate; solo vs bilateral for Plan)
3. **Add per-rule mechanical-enforcement hooks** at phase-transition + mode-transition (R44 anti-cross / R45 mode-tag / R47 decision-class / R49 pre-seal-audit / R50 bare-dot-block)
4. **Add SQLite-backed state option** (alongside YAML for human-readability) for high-concurrency scenarios
5. **Add cost-tracking-per-agent integration** at LLM-call subprocess invocation (T-5 dep)
6. **Add fault-tree integration** (PhaseArtifacts.FaultTree path → cl_fault_tree tool consumes/produces)
7. **Add bilateral-Plan merge-algorithm primitive** (PlanBilateralA + PlanBilateralB → merge → consolidated PlanDoc)

## Conclusion

**MVT prototype VALIDATES IPIV state-machine design assumptions for Phase T T-2 implementation.** Schema + transition logic + bilateral-mode auto-set + atomic save/load + simulated end-to-end cycle all working correctly. T-2 expansion can build on this foundation with full mechanical-enforcement + bilateral primitives + cost-tracking + production-grade hardening.

---

**Cite-anchors:** phase-t.md v5 T-0.5 sub-phase + R44 EXPANDED + R45 EXTENDED + R46 PARTIALLY-SUPERSEDED + R47 REVISED + Phase T BRAIN-cycle + IPIV pipeline architecture section.
