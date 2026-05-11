package mvt

import (
	"os"
	"path/filepath"
	"testing"
)

func TestNewTaskState_defaults(t *testing.T) {
	ts := NewTaskState("test-task-1", DecisionMedium)

	if ts.TaskID != "test-task-1" {
		t.Errorf("TaskID = %q, want %q", ts.TaskID, "test-task-1")
	}
	if ts.DecisionClass != DecisionMedium {
		t.Errorf("DecisionClass = %q, want %q", ts.DecisionClass, DecisionMedium)
	}
	if ts.CurrentPhase != StageInvestigate {
		t.Errorf("CurrentPhase = %q, want %q", ts.CurrentPhase, StageInvestigate)
	}
	if ts.PhaseMode != ModeInvestigateSolo {
		t.Errorf("PhaseMode = %q, want %q", ts.PhaseMode, ModeInvestigateSolo)
	}

	budget := ts.PhaseBudget
	if budget.InvestigatePct+budget.PlanPct+budget.ImplementPct+budget.VerifyPct != 100 {
		t.Errorf("PhaseBudget total = %d, want 100",
			budget.InvestigatePct+budget.PlanPct+budget.ImplementPct+budget.VerifyPct)
	}
}

func TestValidatePhaseTransition_valid(t *testing.T) {
	cases := []struct {
		from, to Stage
	}{
		{StageInvestigate, StagePlan},
		{StagePlan, StageImplement},
		{StagePlan, StageInvestigate}, // loop-back
		{StageImplement, StageVerify},
		{StageImplement, StagePlan}, // loop-back
		{StageVerify, StageImplement},
		{StageVerify, StagePlan},
		{StageVerify, StageInvestigate}, // Verify-fail loop-back
	}

	for _, tc := range cases {
		t.Run(string(tc.from)+"->"+string(tc.to), func(t *testing.T) {
			if err := ValidatePhaseTransition(tc.from, tc.to); err != nil {
				t.Errorf("expected valid: %v", err)
			}
		})
	}
}

func TestValidatePhaseTransition_invalid(t *testing.T) {
	cases := []struct {
		from, to Stage
	}{
		{StageInvestigate, StageImplement}, // skip Plan
		{StageInvestigate, StageVerify},    // skip Plan + Implement
		{StagePlan, StageVerify},           // skip Implement
	}

	for _, tc := range cases {
		t.Run(string(tc.from)+"->"+string(tc.to), func(t *testing.T) {
			if err := ValidatePhaseTransition(tc.from, tc.to); err == nil {
				t.Errorf("expected invalid")
			}
		})
	}
}

func TestValidateMode_perStage(t *testing.T) {
	cases := []struct {
		stage   Stage
		mode    Mode
		wantErr bool
	}{
		{StageInvestigate, ModeInvestigateSolo, false},
		{StageInvestigate, ModeInvestigateCollaborative, false},
		{StageInvestigate, ModePlanSolo, true}, // wrong stage
		{StagePlan, ModePlanSolo, false},
		{StagePlan, ModePlanBilateral, false},
		{StagePlan, ModeImplement, true}, // wrong stage
		{StageImplement, ModeImplement, false},
		{StageVerify, ModeImplementVerify, false},
		{StageVerify, ModePlanVerify, false},
	}

	for _, tc := range cases {
		t.Run(string(tc.stage)+"/"+string(tc.mode), func(t *testing.T) {
			err := ValidateMode(tc.stage, tc.mode)
			if (err != nil) != tc.wantErr {
				t.Errorf("err = %v, wantErr = %v", err, tc.wantErr)
			}
		})
	}
}

func TestTransition_bilateralAutoSet(t *testing.T) {
	// Medium-stakes Investigate-Plan should set bilateral-mode
	ts := NewTaskState("test-task-2", DecisionMedium)
	models := map[string]string{"brian": "claude-default", "rain": "deepseek-v4-pro"}

	if err := ts.Transition(StagePlan, models); err != nil {
		t.Fatalf("transition failed: %v", err)
	}
	if ts.PhaseMode != ModePlanBilateral {
		t.Errorf("PhaseMode = %q, want %q (bilateral auto-set for medium-stakes Plan)", ts.PhaseMode, ModePlanBilateral)
	}
	if len(ts.PhaseHistory) != 1 {
		t.Errorf("PhaseHistory entries = %d, want 1", len(ts.PhaseHistory))
	}
	if ts.PhaseHistory[0].BilateralMode != "bilateral" {
		t.Errorf("BilateralMode = %q, want bilateral", ts.PhaseHistory[0].BilateralMode)
	}
}

func TestTransition_lowStakesSolo(t *testing.T) {
	// Low-stakes Investigate-Plan should default solo
	ts := NewTaskState("test-task-3", DecisionLow)
	models := map[string]string{"brian": "claude-default"}

	if err := ts.Transition(StagePlan, models); err != nil {
		t.Fatalf("transition failed: %v", err)
	}
	if ts.PhaseMode != ModePlanSolo {
		t.Errorf("PhaseMode = %q, want %q (solo for low-stakes Plan)", ts.PhaseMode, ModePlanSolo)
	}
	if ts.PhaseHistory[0].BilateralMode != "solo" {
		t.Errorf("BilateralMode = %q, want solo", ts.PhaseHistory[0].BilateralMode)
	}
}

func TestSaveLoad_roundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "ipav-state.yaml")

	original := NewTaskState("test-task-4", DecisionHigh)
	original.PhaseArtifacts.InvestigationDoc = "/path/to/inv.md"
	original.PhaseArtifacts.FaultTree = "/path/to/ft.json"
	original.PhaseUsed = map[Stage]PhaseUsage{
		StageInvestigate: {
			TokensConsumed: 12345,
			CostPerAgent: map[string]float64{
				"brian": 0.42,
				"rain":  0.38,
			},
		},
	}

	if err := original.Save(path); err != nil {
		t.Fatalf("save: %v", err)
	}

	// Verify file exists + is non-empty
	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if info.Size() == 0 {
		t.Error("saved file is empty")
	}

	loaded, err := Load(path)
	if err != nil {
		t.Fatalf("load: %v", err)
	}

	if loaded.TaskID != original.TaskID {
		t.Errorf("TaskID round-trip: got %q, want %q", loaded.TaskID, original.TaskID)
	}
	if loaded.DecisionClass != original.DecisionClass {
		t.Errorf("DecisionClass round-trip: got %q, want %q", loaded.DecisionClass, original.DecisionClass)
	}
	if loaded.PhaseArtifacts.InvestigationDoc != "/path/to/inv.md" {
		t.Errorf("PhaseArtifacts.InvestigationDoc round-trip: got %q", loaded.PhaseArtifacts.InvestigationDoc)
	}
	if loaded.PhaseUsed[StageInvestigate].CostPerAgent["brian"] != 0.42 {
		t.Errorf("PhaseUsed.CostPerAgent round-trip lost")
	}
}

func TestTaskStatePath_canonical(t *testing.T) {
	got := TaskStatePath("/home/user", "proj-1", "task-abc")
	want := "/home/user/.bot-hq/projects/proj-1/tasks/task-abc/ipav-state.yaml"
	if got != want {
		t.Errorf("TaskStatePath = %q, want %q", got, want)
	}
}

func TestEndToEndCycle_simulated(t *testing.T) {
	// Simulated end-to-end IPAV cycle on a representative task per phase-t.md v5
	// T-0.5 MVT done-criteria: "MVT prototype demonstrates IPAV cycle on real task end-to-end".
	dir := t.TempDir()
	path := filepath.Join(dir, "ipav-state.yaml")
	models := map[string]string{"brian": "claude-default", "rain": "deepseek-v4-pro"}

	// 1. Open task with high-stakes (triggers bilateral per R47 revised)
	ts := NewTaskState("e2e-cycle-1", DecisionHigh)
	if ts.CurrentPhase != StageInvestigate || ts.PhaseMode != ModeInvestigateSolo {
		t.Fatalf("init state: phase=%s mode=%s", ts.CurrentPhase, ts.PhaseMode)
	}

	// Manually set bilateral Investigate per R44 expanded
	ts.PhaseMode = ModeInvestigateCollaborative
	ts.PhaseHistory = append(ts.PhaseHistory, PhaseHistoryEntry{
		Phase: StageInvestigate, StartedAt: ts.OpenedAt,
		SubPhase: "pre-hypothesis", BilateralMode: "bilateral",
		AgentModelAtPhase: models,
		SyncPoints:        []int{25, 50, 75, 90},
	})
	ts.PhaseArtifacts.InvestigationDoc = "/sim/inv.md"
	ts.PhaseArtifacts.FaultTree = "/sim/ft.json"
	ts.PhaseUsed[StageInvestigate] = PhaseUsage{
		TokensConsumed: 50000,
		CostPerAgent: map[string]float64{"brian": 0.50, "rain": 0.30},
	}

	if err := ts.Save(path); err != nil {
		t.Fatalf("save after Investigate: %v", err)
	}

	// 2. Transition I → P (bilateral-Plan auto-set per R44 expanded for high-stakes)
	if err := ts.Transition(StagePlan, models); err != nil {
		t.Fatalf("transition I->P: %v", err)
	}
	if ts.PhaseMode != ModePlanBilateral {
		t.Errorf("expected ModePlanBilateral, got %s", ts.PhaseMode)
	}
	ts.PhaseArtifacts.PlanBilateralA = "/sim/plan-a.md"
	ts.PhaseArtifacts.PlanBilateralB = "/sim/plan-b.md"
	ts.PhaseArtifacts.PlanMergeLog = "/sim/plan-merge.log"
	ts.PhaseUsed[StagePlan] = PhaseUsage{
		TokensConsumed: 15000,
		CostPerAgent: map[string]float64{"brian": 0.15, "rain": 0.10},
	}

	if err := ts.Save(path); err != nil {
		t.Fatalf("save after Plan: %v", err)
	}

	// 3. Transition P → Implement (Brian solo per R46 partial-supersede)
	if err := ts.Transition(StageImplement, models); err != nil {
		t.Fatalf("transition P->Implement: %v", err)
	}
	if ts.PhaseMode != ModeImplement {
		t.Errorf("expected ModeImplement, got %s", ts.PhaseMode)
	}
	ts.PhaseArtifacts.ImplementCommits = []string{"abc123", "def456"}
	ts.PhaseUsed[StageImplement] = PhaseUsage{
		TokensConsumed: 8000,
		CostPerAgent: map[string]float64{"brian": 0.08},
	}

	if err := ts.Save(path); err != nil {
		t.Fatalf("save after Implement: %v", err)
	}

	// 4. Transition Implement → Verify (Rain solo adversarial)
	if err := ts.Transition(StageVerify, models); err != nil {
		t.Fatalf("transition Implement->Verify: %v", err)
	}
	if ts.PhaseMode != ModeImplementVerify {
		t.Errorf("expected ModeImplementVerify, got %s", ts.PhaseMode)
	}
	ts.PhaseArtifacts.VerifyReport = "/sim/verify.md"
	ts.VerifyResult = VerifyPass
	ts.PhaseUsed[StageVerify] = PhaseUsage{
		TokensConsumed: 22000,
		CostPerAgent: map[string]float64{"rain": 0.22},
	}

	if err := ts.Save(path); err != nil {
		t.Fatalf("save after Verify: %v", err)
	}

	// 5. Verify final state via load
	loaded, err := Load(path)
	if err != nil {
		t.Fatalf("load final: %v", err)
	}
	if loaded.VerifyResult != VerifyPass {
		t.Errorf("final VerifyResult = %q, want %q", loaded.VerifyResult, VerifyPass)
	}
	if loaded.CurrentPhase != StageVerify {
		t.Errorf("final CurrentPhase = %q, want %q", loaded.CurrentPhase, StageVerify)
	}

	// Phase-history length: 4 (one per stage transition) + 1 initial Investigate = ?
	// Note: NewTaskState doesn't add an Investigate entry; first entry added when we manually appended above.
	// Then Transition adds entries for P, Implement, Verify = total 4 entries
	if len(loaded.PhaseHistory) != 4 {
		t.Errorf("PhaseHistory length = %d, want 4 (Investigate-init + P + Implement + V)", len(loaded.PhaseHistory))
	}

	// Verify per-phase cost-tracking captured
	totalCost := 0.0
	for _, usage := range loaded.PhaseUsed {
		for _, cost := range usage.CostPerAgent {
			totalCost += cost
		}
	}
	if totalCost < 1.0 || totalCost > 2.0 {
		t.Errorf("totalCost = $%.2f, want roughly $1.35", totalCost)
	}

	t.Logf("E2E IPAV cycle SIMULATED PASS: 4 phase-transitions / total-cost $%.2f / final %s", totalCost, loaded.VerifyResult)
}
