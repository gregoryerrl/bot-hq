package cl

import (
	"errors"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

func newTestRuntime(t *testing.T) *IPAVRuntime {
	t.Helper()
	cl := newTestCL(t)
	rt, err := NewIPAVRuntime(cl, "bot-hq")
	if err != nil {
		t.Fatalf("NewIPAVRuntime: %v", err)
	}
	return rt
}

func TestNewIPAVRuntime_validation(t *testing.T) {
	cl := newTestCL(t)

	if _, err := NewIPAVRuntime(nil, "bot-hq"); err == nil {
		t.Error("expected error for nil CL")
	}
	if _, err := NewIPAVRuntime(cl, ""); err == nil {
		t.Error("expected error for empty project")
	}
}

func TestOpenTask_generatesUUIDAndPersists(t *testing.T) {
	rt := newTestRuntime(t)

	taskID, ts, err := rt.OpenTask(mvt.DecisionHigh)
	if err != nil {
		t.Fatalf("OpenTask: %v", err)
	}
	if taskID == "" {
		t.Error("taskID is empty")
	}
	if ts.TaskID != taskID {
		t.Errorf("ts.TaskID = %q, want %q", ts.TaskID, taskID)
	}
	if ts.DecisionClass != mvt.DecisionHigh {
		t.Errorf("DecisionClass = %q, want high", ts.DecisionClass)
	}
	if ts.CurrentPhase != mvt.StageInvestigate {
		t.Errorf("CurrentPhase = %q, want Investigate", ts.CurrentPhase)
	}

	// Verify persisted
	loaded, err := rt.GetTask(taskID)
	if err != nil {
		t.Fatalf("GetTask: %v", err)
	}
	if loaded.TaskID != taskID {
		t.Errorf("loaded TaskID mismatch")
	}
}

func TestGetTask_missing_returnsErrNotFound(t *testing.T) {
	rt := newTestRuntime(t)
	_, err := rt.GetTask("nonexistent")
	if !errors.Is(err, ErrNotFound) {
		t.Errorf("err = %v, want wrap of ErrNotFound", err)
	}
}

func TestTransitionPhase_bilateralAutoSetForHighStakes(t *testing.T) {
	rt := newTestRuntime(t)
	taskID, _, err := rt.OpenTask(mvt.DecisionHigh)
	if err != nil {
		t.Fatalf("OpenTask: %v", err)
	}

	models := map[string]string{
		"brian": "claude-default",
		"rain":  "deepseek-v4-pro",
	}
	ts, err := rt.TransitionPhase(taskID, mvt.StagePlan, models)
	if err != nil {
		t.Fatalf("TransitionPhase: %v", err)
	}
	if ts.CurrentPhase != mvt.StagePlan {
		t.Errorf("CurrentPhase = %q, want Plan", ts.CurrentPhase)
	}
	if ts.PhaseMode != mvt.ModePlanBilateral {
		t.Errorf("PhaseMode = %q, want plan-bilateral (auto-set for high-stakes)", ts.PhaseMode)
	}

	// Verify persisted by reload
	loaded, err := rt.GetTask(taskID)
	if err != nil {
		t.Fatalf("reload: %v", err)
	}
	if loaded.PhaseMode != mvt.ModePlanBilateral {
		t.Errorf("persisted PhaseMode = %q, want plan-bilateral", loaded.PhaseMode)
	}
}

func TestTransitionPhase_lowStakesSolo(t *testing.T) {
	rt := newTestRuntime(t)
	taskID, _, err := rt.OpenTask(mvt.DecisionLow)
	if err != nil {
		t.Fatalf("OpenTask: %v", err)
	}

	ts, err := rt.TransitionPhase(taskID, mvt.StagePlan, nil)
	if err != nil {
		t.Fatalf("TransitionPhase: %v", err)
	}
	if ts.PhaseMode != mvt.ModePlanSolo {
		t.Errorf("PhaseMode = %q, want plan-solo (low-stakes)", ts.PhaseMode)
	}
}

func TestRecordPhaseUsage_aggregatesAcrossAgents(t *testing.T) {
	rt := newTestRuntime(t)
	taskID, _, err := rt.OpenTask(mvt.DecisionMedium)
	if err != nil {
		t.Fatalf("OpenTask: %v", err)
	}

	// Record usage for brian + rain in Investigate
	if _, err := rt.RecordPhaseUsage(taskID, "brian", 5000, 0.20); err != nil {
		t.Fatalf("RecordPhaseUsage brian: %v", err)
	}
	if _, err := rt.RecordPhaseUsage(taskID, "rain", 4000, 0.15); err != nil {
		t.Fatalf("RecordPhaseUsage rain: %v", err)
	}

	ts, err := rt.GetTask(taskID)
	if err != nil {
		t.Fatalf("reload: %v", err)
	}
	usage := ts.PhaseUsed[mvt.StageInvestigate]
	if usage.TokensConsumed != 9000 {
		t.Errorf("tokens = %d, want 9000", usage.TokensConsumed)
	}
	if usage.CostPerAgent["brian"] != 0.20 {
		t.Errorf("brian cost = %f, want 0.20", usage.CostPerAgent["brian"])
	}
	if usage.CostPerAgent["rain"] != 0.15 {
		t.Errorf("rain cost = %f, want 0.15", usage.CostPerAgent["rain"])
	}
}

func TestSetPhaseArtifact_validKeys(t *testing.T) {
	rt := newTestRuntime(t)
	taskID, _, _ := rt.OpenTask(mvt.DecisionMedium)

	cases := []struct {
		key, path string
	}{
		{"investigation_doc", "/path/inv.md"},
		{"fault_tree", "/path/ft.json"},
		{"plan_doc", "/path/plan.md"},
		{"plan_bilateral_a", "/path/plan-a.md"},
		{"plan_bilateral_b", "/path/plan-b.md"},
		{"plan_merge_log", "/path/merge.log"},
		{"verify_report", "/path/verify.md"},
	}
	for _, tc := range cases {
		if _, err := rt.SetPhaseArtifact(taskID, tc.key, tc.path); err != nil {
			t.Errorf("SetPhaseArtifact(%s): %v", tc.key, err)
		}
	}
}

func TestSetPhaseArtifact_unknownKey_errors(t *testing.T) {
	rt := newTestRuntime(t)
	taskID, _, _ := rt.OpenTask(mvt.DecisionMedium)
	if _, err := rt.SetPhaseArtifact(taskID, "nonsense_key", "/path"); err == nil {
		t.Error("expected error for unknown key")
	}
}

func TestAddImplementCommit_appends(t *testing.T) {
	rt := newTestRuntime(t)
	taskID, _, _ := rt.OpenTask(mvt.DecisionMedium)

	if _, err := rt.AddImplementCommit(taskID, "abc123"); err != nil {
		t.Fatalf("AddImplementCommit: %v", err)
	}
	if _, err := rt.AddImplementCommit(taskID, "def456"); err != nil {
		t.Fatalf("AddImplementCommit 2: %v", err)
	}

	ts, _ := rt.GetTask(taskID)
	if len(ts.PhaseArtifacts.ImplementCommits) != 2 {
		t.Errorf("commits = %d, want 2", len(ts.PhaseArtifacts.ImplementCommits))
	}
	if ts.PhaseArtifacts.ImplementCommits[0] != "abc123" {
		t.Errorf("commit[0] = %q", ts.PhaseArtifacts.ImplementCommits[0])
	}
}

func TestCompleteTask_passResult(t *testing.T) {
	rt := newTestRuntime(t)
	taskID, _, _ := rt.OpenTask(mvt.DecisionHigh)

	ts, err := rt.CompleteTask(taskID, mvt.VerifyPass)
	if err != nil {
		t.Fatalf("CompleteTask: %v", err)
	}
	if ts.VerifyResult != mvt.VerifyPass {
		t.Errorf("VerifyResult = %q, want pass", ts.VerifyResult)
	}
	if ts.VerifyLoopCount != 0 {
		t.Errorf("loop count should be 0 on pass; got %d", ts.VerifyLoopCount)
	}
}

func TestCompleteTask_failIncrementsLoopCount(t *testing.T) {
	rt := newTestRuntime(t)
	taskID, _, _ := rt.OpenTask(mvt.DecisionHigh)

	for i := 1; i <= 3; i++ {
		ts, err := rt.CompleteTask(taskID, mvt.VerifyFail)
		if err != nil {
			t.Fatalf("CompleteTask iter %d: %v", i, err)
		}
		if ts.VerifyLoopCount != i {
			t.Errorf("after %d fails: loop count = %d, want %d", i, ts.VerifyLoopCount, i)
		}
	}
}

func TestListTasks_multipleOpen(t *testing.T) {
	rt := newTestRuntime(t)

	for i := 0; i < 3; i++ {
		if _, _, err := rt.OpenTask(mvt.DecisionMedium); err != nil {
			t.Fatalf("OpenTask iter %d: %v", i, err)
		}
	}

	all, err := rt.ListTasks()
	if err != nil {
		t.Fatalf("ListTasks: %v", err)
	}
	if len(all) != 3 {
		t.Errorf("count = %d, want 3", len(all))
	}
}

func TestTaskAge_returnsSinceOpen(t *testing.T) {
	rt := newTestRuntime(t)
	_, ts, _ := rt.OpenTask(mvt.DecisionMedium)
	age := TaskAge(ts)
	if age < 0 {
		t.Errorf("age should be non-negative; got %v", age)
	}
}
