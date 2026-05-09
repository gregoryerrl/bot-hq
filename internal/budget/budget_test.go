package budget

import (
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

func TestNewAllocator_validates(t *testing.T) {
	if _, err := NewAllocator(0, DefaultPercent()); err == nil {
		t.Error("expected error for zero budget")
	}
	if _, err := NewAllocator(-1, DefaultPercent()); err == nil {
		t.Error("expected error for negative budget")
	}
	bad := PhaseBudgetPercent{Investigate: 50, Plan: 10, Implement: 5, Verify: 25} // sums to 90
	if _, err := NewAllocator(10, bad); err == nil {
		t.Error("expected error for non-100 percent")
	}
}

func TestPerPhase_distribution(t *testing.T) {
	a, _ := NewAllocator(10.0, DefaultPercent())
	if a.PerPhase(mvt.StageInvestigate) != 5.8 {
		t.Errorf("Investigate = %f, want 5.8", a.PerPhase(mvt.StageInvestigate))
	}
	if a.PerPhase(mvt.StagePlan) != 1.2 {
		t.Errorf("Plan = %f, want 1.2", a.PerPhase(mvt.StagePlan))
	}
	if a.PerPhase(mvt.StageImplement) != 0.5 {
		t.Errorf("Implement = %f, want 0.5", a.PerPhase(mvt.StageImplement))
	}
	if a.PerPhase(mvt.StageVerify) != 2.5 {
		t.Errorf("Verify = %f, want 2.5", a.PerPhase(mvt.StageVerify))
	}
}

func TestNewTracker_validates(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	if _, err := NewTracker("", a, nil); err == nil {
		t.Error("expected error for empty taskID")
	}
	if _, err := NewTracker("t", nil, nil); err == nil {
		t.Error("expected error for nil allocator")
	}
}

func TestRecord_warnAt80Percent(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)

	// Spend $7 (70%) — no warn
	if ev, halt := tr.Record("brian", mvt.StageImplement, 7.0); ev != nil {
		t.Errorf("70%% spend should not breach; got %+v", ev)
	} else if halt {
		t.Error("70%% should not halt")
	}

	// Spend additional $1.50 (now $8.50 = 85%) — warn
	ev, halt := tr.Record("rain", mvt.StageVerify, 1.5)
	if ev == nil {
		t.Error("85%% should breach warn-threshold")
	} else if ev.Type != "warn" {
		t.Errorf("breach type = %q, want warn", ev.Type)
	}
	if halt {
		t.Error("warn should not force halt")
	}
}

func TestRecord_blockAt100Percent(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)

	ev, halt := tr.Record("brian", mvt.StageImplement, 11.0) // > 100%
	if ev == nil || ev.Type != "block" {
		t.Errorf("100%%+ should breach block; got %+v", ev)
	}
	if !halt {
		t.Error("block-trip should force halt")
	}
}

func TestSetThresholds_validation(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)
	if err := tr.SetThresholds(0, 1); err == nil {
		t.Error("warn=0 should error")
	}
	if err := tr.SetThresholds(0.5, 0.5); err == nil {
		t.Error("block <= warn should error")
	}
	if err := tr.SetThresholds(0.7, 0.95); err != nil {
		t.Errorf("valid thresholds errored: %v", err)
	}
}

func TestEstimatedCost_oauthMaxFlatZero(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)
	cost := tr.EstimatedCost("brian", 100000)
	if cost != 0 {
		t.Errorf("OAuth-MAX should be flat-zero; got $%.2f", cost)
	}
}

func TestEstimatedCost_apiMeteredScales(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)
	cost := tr.EstimatedCost("rain", 1000) // 1k tokens
	expected := 0.014                       // per default DeepSeek profile
	if cost < expected*0.99 || cost > expected*1.01 {
		t.Errorf("rain cost = $%.4f, want ~$%.4f", cost, expected)
	}
}

func TestEstimatedCost_unknownAgentDefault(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)
	cost := tr.EstimatedCost("nonexistent", 1000)
	if cost <= 0 {
		t.Error("unknown agent should have non-zero default")
	}
}

func TestPerAgentAndPerPhase_aggregation(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)

	tr.Record("brian", mvt.StageInvestigate, 1.0)
	tr.Record("rain", mvt.StageInvestigate, 0.5)
	tr.Record("brian", mvt.StagePlan, 0.3)

	pa := tr.PerAgent()
	if pa["brian"] != 1.3 {
		t.Errorf("brian = %f, want 1.3", pa["brian"])
	}
	if pa["rain"] != 0.5 {
		t.Errorf("rain = %f, want 0.5", pa["rain"])
	}

	pp := tr.PerPhase()
	if pp[mvt.StageInvestigate] != 1.5 {
		t.Errorf("Investigate = %f, want 1.5", pp[mvt.StageInvestigate])
	}
}

func TestPhaseRemaining(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)
	tr.Record("brian", mvt.StageInvestigate, 2.0)
	rem := tr.PhaseRemaining(mvt.StageInvestigate)
	expected := 5.8 - 2.0
	if rem < expected*0.99 || rem > expected*1.01 {
		t.Errorf("Investigate remaining = %f, want ~%f", rem, expected)
	}
}

// ====== Phase-transition gate tests ======

func TestCheckTransition_allowsWithAllCriteriaSatisfied(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)
	tr.Record("brian", mvt.StageInvestigate, 1.0) // well within 5.8

	criteria := PhaseDoneCriteria{
		Stage:              mvt.StageInvestigate,
		MinArtifactsPath:   []string{"investigation_doc"},
		MaxBudgetUtilized:  1.0,
		RequireConvergence: true,
	}
	artifacts := map[string]bool{"investigation_doc": true}

	res := CheckTransition(tr, mvt.StageInvestigate, criteria, artifacts, true, false)
	if !res.Allowed {
		t.Errorf("expected allowed; got: %s", res.Reason)
	}
}

func TestCheckTransition_blocksOnMissingArtifact(t *testing.T) {
	criteria := PhaseDoneCriteria{
		Stage:            mvt.StageInvestigate,
		MinArtifactsPath: []string{"investigation_doc"},
	}
	artifacts := map[string]bool{} // missing

	res := CheckTransition(nil, mvt.StageInvestigate, criteria, artifacts, true, false)
	if res.Allowed {
		t.Error("missing required artifact should block")
	}
}

func TestCheckTransition_blocksOnBudgetOverrun(t *testing.T) {
	a, _ := NewAllocator(10, DefaultPercent())
	tr, _ := NewTracker("t", a, nil)
	tr.Record("brian", mvt.StageInvestigate, 7.0) // > 5.8 budget

	criteria := PhaseDoneCriteria{
		Stage:             mvt.StageInvestigate,
		MaxBudgetUtilized: 1.0,
	}

	res := CheckTransition(tr, mvt.StageInvestigate, criteria, nil, true, false)
	if res.Allowed {
		t.Error("phase-budget overrun should block")
	}
}

func TestCheckTransition_blocksOnConvergenceFalse(t *testing.T) {
	criteria := PhaseDoneCriteria{
		Stage:              mvt.StageInvestigate,
		RequireConvergence: true,
	}
	res := CheckTransition(nil, mvt.StageInvestigate, criteria, nil, false, false)
	if res.Allowed {
		t.Error("convergence=false should block")
	}
}

func TestCheckTransition_blocksOnPlanApprovalFalse(t *testing.T) {
	criteria := PhaseDoneCriteria{
		Stage:           mvt.StagePlan,
		RequireApproval: true,
	}
	res := CheckTransition(nil, mvt.StagePlan, criteria, nil, true, false)
	if res.Allowed {
		t.Error("plan-approval=false should block")
	}
}

func TestCheckTransition_stageMismatch(t *testing.T) {
	criteria := PhaseDoneCriteria{Stage: mvt.StagePlan}
	res := CheckTransition(nil, mvt.StageInvestigate, criteria, nil, true, false)
	if res.Allowed {
		t.Error("stage mismatch should not allow")
	}
}
