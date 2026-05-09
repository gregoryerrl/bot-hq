package cl

import (
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/faulttree"
	"github.com/gregoryerrl/bot-hq/internal/hypothesis"
	"github.com/gregoryerrl/bot-hq/internal/mvt"
	"github.com/gregoryerrl/bot-hq/internal/toolgate"
)

// TestIPIVFirstTask_endToEnd is the T-2.6 IPIV-first-task validation per
// phase-t.md v5: at least one full IPIV cycle (I→P→Implement→V) executed
// successfully on representative task with all hooks firing + cross-model
// bilateral active for at least Investigate phase.
//
// This test wires together every T-2 deliverable + verifies they integrate:
//   - T-2.1 IPIVRuntime (OpenTask + TransitionPhase + RecordPhaseUsage + CompleteTask)
//   - T-2.4 cl_fault_tree (faulttree.Tree)
//   - T-2.4 cl_hypothesis_loop (hypothesis.Loop)
//   - T-2.4 cl_strong_style_assign (hypothesis.AssignDriver)
//   - T-2.5 R44 anti-cross check (toolgate.R44AntiCrossCheck)
//   - T-2.5 R47 decision-class-tag check (toolgate.R47DecisionClassTagCheck)
//   - T-1.7 IPIV-state-tracking schema in CL (CL.SaveIPIVState/IPIVState)
//
// Cross-model bilateral simulated via agentModels map (Brian-claude + Rain-deepseek).
// Real cross-model live execution requires DeepSeek subprocess (T-0.5 capability-
// parity test scaffolding); this test validates orchestration logic.
func TestIPIVFirstTask_endToEnd(t *testing.T) {
	cl := newTestCL(t)
	rt, err := NewIPIVRuntime(cl, "bot-hq")
	if err != nil {
		t.Fatalf("NewIPIVRuntime: %v", err)
	}

	agentModels := map[string]string{
		"brian": "claude-default",
		"rain":  "deepseek-v4-pro",
	}

	// === Phase: Open task with high-stakes (triggers R47 bilateral routing) ===
	taskID, ts, err := rt.OpenTask(mvt.DecisionHigh)
	if err != nil {
		t.Fatalf("OpenTask: %v", err)
	}
	if ts.CurrentPhase != mvt.StageInvestigate {
		t.Fatalf("expected initial phase Investigate; got %s", ts.CurrentPhase)
	}
	t.Logf("STEP 1: opened task %s (decision_class=high)", taskID)

	// === T-2.5 R47 decision-class-tag verification ===
	v47 := toolgate.R47DecisionClassTagCheck("scope-lock-write", "high")
	if v47.ShouldBlock {
		t.Errorf("R47 should allow tagged high-stakes; got: %s", v47.Reason)
	}
	t.Logf("STEP 2: R47 high-stakes tagging passed")

	// === Phase: Investigate — build fault-tree + assign drivers ===
	tree := faulttree.NewTree(taskID)
	rootID, err := tree.AddNode(&faulttree.Node{
		Type:        faulttree.NodeAction,
		Title:       "R31 cite-drift recurs without mechanical-validation",
		Description: "Hypothesis: manual peer-cross-check is non-terminal at recursion-depth-N.",
		Owner:       "brian", // Brian proposes this hypothesis
	})
	if err != nil {
		t.Fatalf("AddNode root: %v", err)
	}
	leafID, err := tree.AddNode(&faulttree.Node{
		Type:     faulttree.NodeCondition,
		Title:    "Phase-T BRAIN-cycle exhibits cite-drift instances",
		Owner:    "brian",
		ParentID: rootID,
	})
	if err != nil {
		t.Fatalf("AddNode leaf: %v", err)
	}

	// === T-2.5 R44 anti-cross enforcement: Brian (owner) cannot drive own hypothesis ===
	v44 := toolgate.R44AntiCrossCheck("brian", "brian", "investigate")
	if !v44.ShouldBlock {
		t.Errorf("R44 should block owner=driver; got: %s", v44.Reason)
	}
	t.Logf("STEP 3: R44 anti-cross correctly blocked self-investigation")

	// === T-2.4 cl_strong_style_assign: auto-assign peer ===
	driver, err := hypothesis.AssignDriver("brian", []string{"brian", "rain"})
	if err != nil {
		t.Fatalf("AssignDriver: %v", err)
	}
	if driver != "rain" {
		t.Errorf("driver = %q, want rain", driver)
	}
	t.Logf("STEP 4: cl_strong_style_assign assigned driver=%s (peer of owner=brian)", driver)

	// Wire driver into fault-tree
	if err := tree.AssignInvestigator(leafID, driver); err != nil {
		t.Fatalf("AssignInvestigator: %v", err)
	}

	// Persist fault-tree alongside IPIV state
	ftPath := faulttree.CanonicalPath(cl.Root(), "bot-hq", taskID)
	if err := tree.Save(ftPath); err != nil {
		t.Fatalf("Save fault-tree: %v", err)
	}
	rt.SetPhaseArtifact(taskID, "fault_tree", ftPath)
	t.Logf("STEP 5: fault-tree persisted at %s", ftPath)

	// === T-2.4 cl_hypothesis_loop: driver runs Zeller loop on assigned leaf ===
	loop, err := hypothesis.NewLoop(taskID, leafID, driver,
		"Cite-drift recurs unless mechanical-validation hook auto-fires at every cite-bearing emit.")
	if err != nil {
		t.Fatalf("NewLoop: %v", err)
	}
	if err := loop.SetPrediction("Deploying R49 + cite-anchor validation reduces drift catch-rate to ≥90%."); err != nil {
		t.Fatalf("SetPrediction: %v", err)
	}
	if err := loop.SetExperimentObservation(
		"Run cite-validate against phase-t.md v5 with hub.DB.MessageExists wired (T-2 hub wiring).",
		"Validator extracted 166 anchors; 126 valid (76%); previously without msg-id wiring 113 were skipped.",
	); err != nil {
		t.Fatalf("SetExperimentObservation: %v", err)
	}
	loop.AddCiteAnchor("docs/research/perf-baseline-pre-phase-t.md")
	loop.AddCiteAnchor("internal/citeanchor/validator.go")
	if err := loop.Conclude(hypothesis.ConclusionConfirmed); err != nil {
		t.Fatalf("Conclude: %v", err)
	}
	if !loop.IsComplete() {
		t.Error("loop should be complete")
	}

	loopPath := hypothesis.CanonicalPath(cl.Root(), "bot-hq", taskID, loop.ID)
	if err := loop.Save(loopPath); err != nil {
		t.Fatalf("Save loop: %v", err)
	}
	t.Logf("STEP 6: hypothesis-loop CONFIRMED + persisted at %s", loopPath)

	// Update fault-tree node status based on conclusion
	tree.SetStatus(leafID, faulttree.StatusConfirmed)
	tree.AddCiteAnchor(leafID, "loop:"+loop.ID)
	tree.AddCiteAnchor(leafID, "msg 17162") // T-2 partial surface msg
	tree.SetStatus(rootID, faulttree.StatusConfirmed)
	tree.AddCiteAnchor(rootID, "loop:"+loop.ID)
	tree.AddCiteAnchor(rootID, "phase-t.md v5 R31 evidence")
	tree.Save(ftPath)

	if !tree.IsConvergence() {
		t.Error("fault-tree should converge after both nodes confirmed with ≥2 cite-anchors")
	}
	t.Logf("STEP 7: fault-tree CONVERGED")

	// Record phase usage (cost-tracking T-2.7 partial)
	rt.RecordPhaseUsage(taskID, "brian", 8000, 0.32)
	rt.RecordPhaseUsage(taskID, "rain", 6500, 0.10)

	// === Phase: I → P transition ===
	ts, err = rt.TransitionPhase(taskID, mvt.StagePlan, agentModels)
	if err != nil {
		t.Fatalf("TransitionPhase I→P: %v", err)
	}
	if ts.PhaseMode != mvt.ModePlanBilateral {
		t.Errorf("PhaseMode after I→P = %q, want plan-bilateral (high-stakes auto)", ts.PhaseMode)
	}
	t.Logf("STEP 8: I→P transition; mode=%s (bilateral auto-set)", ts.PhaseMode)

	rt.SetPhaseArtifact(taskID, "plan_doc", filepath.Join(cl.Root(), "phase", "plan.md"))
	rt.RecordPhaseUsage(taskID, "brian", 2000, 0.08)
	rt.RecordPhaseUsage(taskID, "rain", 1500, 0.04)

	// === Phase: P → Implement transition ===
	ts, err = rt.TransitionPhase(taskID, mvt.StageImplement, agentModels)
	if err != nil {
		t.Fatalf("TransitionPhase P→Implement: %v", err)
	}
	if ts.PhaseMode != mvt.ModeImplement {
		t.Errorf("PhaseMode = %q, want implement", ts.PhaseMode)
	}
	t.Logf("STEP 9: P→Implement transition; Brian solo HANDS")

	rt.AddImplementCommit(taskID, "abc123-cite-anchor-validation-hook-deployment")
	rt.AddImplementCommit(taskID, "def456-r49-pre-seal-audit-implementation")
	rt.RecordPhaseUsage(taskID, "brian", 1200, 0.05)

	// === Phase: Implement → Verify transition ===
	ts, err = rt.TransitionPhase(taskID, mvt.StageVerify, agentModels)
	if err != nil {
		t.Fatalf("TransitionPhase Implement→Verify: %v", err)
	}
	if ts.PhaseMode != mvt.ModeImplementVerify {
		t.Errorf("PhaseMode = %q, want implement-verify", ts.PhaseMode)
	}
	t.Logf("STEP 10: Implement→Verify transition; Rain solo adversarial")

	rt.SetPhaseArtifact(taskID, "verify_report", filepath.Join(cl.Root(), "phase", "verify.md"))
	rt.RecordPhaseUsage(taskID, "rain", 4000, 0.12)

	// === Verify result: PASS ===
	ts, err = rt.CompleteTask(taskID, mvt.VerifyPass)
	if err != nil {
		t.Fatalf("CompleteTask: %v", err)
	}
	t.Logf("STEP 11: Verify PASS — task complete")

	// === Final state validation ===
	loaded, err := rt.GetTask(taskID)
	if err != nil {
		t.Fatalf("Final reload: %v", err)
	}
	if loaded.VerifyResult != mvt.VerifyPass {
		t.Errorf("final VerifyResult = %q, want pass", loaded.VerifyResult)
	}
	if loaded.CurrentPhase != mvt.StageVerify {
		t.Errorf("final CurrentPhase = %q, want Verify", loaded.CurrentPhase)
	}
	if len(loaded.PhaseArtifacts.ImplementCommits) != 2 {
		t.Errorf("commits = %d, want 2", len(loaded.PhaseArtifacts.ImplementCommits))
	}

	// Sum cost across all phases / agents
	totalCost := 0.0
	totalTokens := 0
	for _, usage := range loaded.PhaseUsed {
		totalTokens += usage.TokensConsumed
		for _, c := range usage.CostPerAgent {
			totalCost += c
		}
	}
	if totalCost < 0.5 || totalCost > 2.0 {
		t.Errorf("totalCost = $%.2f outside expected range $0.5-$2.0", totalCost)
	}
	if totalTokens < 20000 {
		t.Errorf("totalTokens = %d, want >= 20000", totalTokens)
	}

	t.Logf("=== T-2.6 IPIV-FIRST-TASK VALIDATION PASS ===")
	t.Logf("Final: phase=%s / verify=%s / commits=%d / total-tokens=%d / total-cost=$%.2f",
		loaded.CurrentPhase, loaded.VerifyResult, len(loaded.PhaseArtifacts.ImplementCommits),
		totalTokens, totalCost)
	t.Logf("Cross-model bilateral: brian=%s / rain=%s", agentModels["brian"], agentModels["rain"])
	t.Logf("Wired components VERIFIED: IPIVRuntime + faulttree + hypothesis + cl_strong_style_assign + R44 + R47 + IPIV-state-tracking-in-CL")
}
