// Package budget implements T-5 compute-budget allocator + cost-tracking-
// per-agent + cost-budget-per-task circuit-breaker per phase-t.md v5.
//
// Compute-budget tracks token-spend per phase per IPIV percentages
// (Investigate 55-60% / Plan 10-15% / Implement 5% / Verify 20-25%).
// Cost-tracking integrates Brian-Claude OAuth-MAX-flat-noted vs Rain-
// DeepSeek-API-key-metered (per R51 + R52 model-config-aware).
//
// Circuit-breaker: per-task USD budget with warn-at-80% + block-at-100%.
// Provider-cost-comparison-aware routing: future-extension hook (T-5+).

package budget

import (
	"errors"
	"fmt"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

// PhaseBudgetPercent declares the canonical IPIV per-phase budget split
// per phase-t.md v5 (mirrors mvt.DefaultPhaseBudget but typed for clarity).
type PhaseBudgetPercent struct {
	Investigate float64 // typically 55-60
	Plan        float64 // 10-15
	Implement   float64 // 5
	Verify      float64 // 20-25
}

// DefaultPercent returns the canonical 58/12/5/25 split.
func DefaultPercent() PhaseBudgetPercent {
	return PhaseBudgetPercent{
		Investigate: 58,
		Plan:        12,
		Implement:   5,
		Verify:      25,
	}
}

// Allocator distributes a total budget across the IPIV phases per the
// configured percentages.
type Allocator struct {
	totalUSD float64
	percent  PhaseBudgetPercent
}

// NewAllocator constructs an allocator scoped to a per-task USD budget.
func NewAllocator(totalUSD float64, percent PhaseBudgetPercent) (*Allocator, error) {
	if totalUSD <= 0 {
		return nil, errors.New("totalUSD must be positive")
	}
	sum := percent.Investigate + percent.Plan + percent.Implement + percent.Verify
	if sum < 99.5 || sum > 100.5 {
		return nil, fmt.Errorf("percent sum = %.2f, must be ~100", sum)
	}
	return &Allocator{totalUSD: totalUSD, percent: percent}, nil
}

// PerPhase returns the USD budget allocated to a given IPIV stage.
func (a *Allocator) PerPhase(stage mvt.Stage) float64 {
	switch stage {
	case mvt.StageInvestigate:
		return a.totalUSD * a.percent.Investigate / 100
	case mvt.StagePlan:
		return a.totalUSD * a.percent.Plan / 100
	case mvt.StageImplement:
		return a.totalUSD * a.percent.Implement / 100
	case mvt.StageVerify:
		return a.totalUSD * a.percent.Verify / 100
	}
	return 0
}

// Total returns the configured total USD budget.
func (a *Allocator) Total() float64 { return a.totalUSD }

// ====== Cost-tracking ======

// CostBasis declares how an agent's cost is computed.
type CostBasis string

const (
	CostBasisOAuthMax    CostBasis = "oauth-max-subscription-flat-noted" // Claude OAuth MAX (flat subscription)
	CostBasisAPIMetered  CostBasis = "api-key-metered"                   // DeepSeek / OpenAI / etc. (per-token)
	CostBasisEstimated   CostBasis = "estimated-from-token-share"        // fallback when no provider cost-endpoint
)

// AgentCostProfile captures how to charge cost for an agent.
type AgentCostProfile struct {
	AgentID   string
	Basis     CostBasis
	// EstUSDPerKToken is used for CostBasisEstimated to convert token-
	// counts to estimated USD (per-1000-token rate). 0 for OAuth-MAX.
	EstUSDPerKToken float64
}

// DefaultProfiles returns the canonical default cost profiles for the
// bot-hq trio + spawned coders. brian/emma/clive default to OAuth-MAX-flat;
// rain defaults to API-metered.
func DefaultProfiles() map[string]AgentCostProfile {
	return map[string]AgentCostProfile{
		"brian":          {AgentID: "brian", Basis: CostBasisOAuthMax, EstUSDPerKToken: 0},
		"rain":           {AgentID: "rain", Basis: CostBasisAPIMetered, EstUSDPerKToken: 0.014},
		"emma":           {AgentID: "emma", Basis: CostBasisOAuthMax, EstUSDPerKToken: 0},
		"clive":          {AgentID: "clive", Basis: CostBasisOAuthMax, EstUSDPerKToken: 0},
		"coder-template": {AgentID: "coder-template", Basis: CostBasisOAuthMax, EstUSDPerKToken: 0},
	}
}

// Tracker records per-task per-agent cost-spend with circuit-breaker.
type Tracker struct {
	taskID         string
	allocator      *Allocator
	profiles       map[string]AgentCostProfile
	spentPerAgent  map[string]float64
	spentPerPhase  map[mvt.Stage]float64
	warnThreshold  float64 // default 0.8
	blockThreshold float64 // default 1.0
	breachLog      []BreachEvent
	createdAt      time.Time
}

// BreachEvent is one circuit-breaker trip event.
type BreachEvent struct {
	When         time.Time
	Type         string  // "warn" | "block"
	SpentUSD     float64
	BudgetUSD    float64
	PercentSpent float64
	AgentID      string
	Stage        mvt.Stage
}

// NewTracker constructs a per-task cost-tracker.
func NewTracker(taskID string, alloc *Allocator, profiles map[string]AgentCostProfile) (*Tracker, error) {
	if taskID == "" {
		return nil, errors.New("taskID is required")
	}
	if alloc == nil {
		return nil, errors.New("allocator is required")
	}
	if profiles == nil {
		profiles = DefaultProfiles()
	}
	return &Tracker{
		taskID:         taskID,
		allocator:      alloc,
		profiles:       profiles,
		spentPerAgent:  make(map[string]float64),
		spentPerPhase:  make(map[mvt.Stage]float64),
		warnThreshold:  0.8,
		blockThreshold: 1.0,
		createdAt:      time.Now().UTC(),
	}, nil
}

// SetThresholds overrides warn + block thresholds (default 0.8 + 1.0).
func (t *Tracker) SetThresholds(warn, block float64) error {
	if warn <= 0 || warn > 1 {
		return fmt.Errorf("warn threshold %.2f must be in (0, 1]", warn)
	}
	if block <= warn || block > 2 {
		return fmt.Errorf("block threshold %.2f must be > warn %.2f and <= 2", block, warn)
	}
	t.warnThreshold = warn
	t.blockThreshold = block
	return nil
}

// Record adds cost-spend for an agent in a phase. Returns BreachEvent
// describing any circuit-breaker trip + bool indicating whether the
// caller MUST halt (block-trip).
func (t *Tracker) Record(agentID string, stage mvt.Stage, costUSD float64) (event *BreachEvent, mustHalt bool) {
	t.spentPerAgent[agentID] += costUSD
	t.spentPerPhase[stage] += costUSD

	totalSpent := 0.0
	for _, c := range t.spentPerAgent {
		totalSpent += c
	}
	pct := totalSpent / t.allocator.Total()

	if pct >= t.blockThreshold {
		ev := BreachEvent{
			When: time.Now().UTC(), Type: "block",
			SpentUSD: totalSpent, BudgetUSD: t.allocator.Total(),
			PercentSpent: pct * 100, AgentID: agentID, Stage: stage,
		}
		t.breachLog = append(t.breachLog, ev)
		return &ev, true
	}
	if pct >= t.warnThreshold {
		ev := BreachEvent{
			When: time.Now().UTC(), Type: "warn",
			SpentUSD: totalSpent, BudgetUSD: t.allocator.Total(),
			PercentSpent: pct * 100, AgentID: agentID, Stage: stage,
		}
		t.breachLog = append(t.breachLog, ev)
		return &ev, false
	}
	return nil, false
}

// EstimatedCost converts a token-count to USD cost for an agent based on
// their cost-basis profile.
func (t *Tracker) EstimatedCost(agentID string, tokens int) float64 {
	p, ok := t.profiles[agentID]
	if !ok {
		// Default: estimated cost at typical Claude-Sonnet rate
		return float64(tokens) / 1000 * 0.015
	}
	switch p.Basis {
	case CostBasisOAuthMax:
		// Flat subscription; no per-call cost
		return 0
	case CostBasisAPIMetered, CostBasisEstimated:
		return float64(tokens) / 1000 * p.EstUSDPerKToken
	}
	return 0
}

// Spent returns total spend across all agents.
func (t *Tracker) Spent() float64 {
	total := 0.0
	for _, c := range t.spentPerAgent {
		total += c
	}
	return total
}

// PerAgent returns cost-spend per agent.
func (t *Tracker) PerAgent() map[string]float64 {
	out := make(map[string]float64, len(t.spentPerAgent))
	for k, v := range t.spentPerAgent {
		out[k] = v
	}
	return out
}

// PerPhase returns cost-spend per IPIV stage.
func (t *Tracker) PerPhase() map[mvt.Stage]float64 {
	out := make(map[mvt.Stage]float64, len(t.spentPerPhase))
	for k, v := range t.spentPerPhase {
		out[k] = v
	}
	return out
}

// PhaseRemaining returns the USD remaining in the per-phase budget.
// Negative value indicates phase-budget overrun.
func (t *Tracker) PhaseRemaining(stage mvt.Stage) float64 {
	return t.allocator.PerPhase(stage) - t.spentPerPhase[stage]
}

// Breaches returns the circuit-breaker trip log.
func (t *Tracker) Breaches() []BreachEvent {
	out := make([]BreachEvent, len(t.breachLog))
	copy(out, t.breachLog)
	return out
}

// ====== Phase-transition gates ======

// PhaseGateResult reports whether a phase-transition is permitted by
// gate-discipline (per-phase done-criteria).
type PhaseGateResult struct {
	Allowed bool
	Reason  string
}

// PhaseDoneCriteria captures the gate criteria for one IPIV stage.
type PhaseDoneCriteria struct {
	Stage              mvt.Stage
	MinArtifactsPath   []string  // paths that MUST exist (non-empty)
	MaxBudgetUtilized  float64   // max % of phase-budget allowed (default 1.0; >1.0 permits overrun-with-warn)
	RequireConvergence bool      // for Investigate: fault-tree must report IsConvergence
	RequireApproval    bool      // for Plan: Plan-Verify must be approved
}

// CheckTransition validates that current-phase satisfies done-criteria
// before transitioning to next-phase. Returns Allowed=false with reason
// when criteria unmet.
//
// Inputs:
//   - tracker: cost-tracker for budget-utilization check
//   - currentStage: IPIV phase being closed
//   - artifactsPresent: per-artifact-key bool indicating doc/file existence
//   - convergence: result of faulttree.Tree.IsConvergence (Investigate only)
//   - planApproved: result of plan.RunPlanVerify (Plan only)
func CheckTransition(t *Tracker, currentStage mvt.Stage, criteria PhaseDoneCriteria,
	artifactsPresent map[string]bool, convergence bool, planApproved bool,
) PhaseGateResult {
	if criteria.Stage != currentStage {
		return PhaseGateResult{Reason: "criteria stage mismatch"}
	}

	// Required artifacts present?
	for _, path := range criteria.MinArtifactsPath {
		if !artifactsPresent[path] {
			return PhaseGateResult{Reason: fmt.Sprintf("required artifact missing: %s", path)}
		}
	}

	// Budget utilization within limit?
	maxUtil := criteria.MaxBudgetUtilized
	if maxUtil <= 0 {
		maxUtil = 1.0
	}
	if t != nil {
		spent := t.spentPerPhase[currentStage]
		budget := t.allocator.PerPhase(currentStage)
		if budget > 0 && spent/budget > maxUtil {
			return PhaseGateResult{Reason: fmt.Sprintf("phase budget overrun: spent=%.2f budget=%.2f utilized=%.2f > max=%.2f", spent, budget, spent/budget, maxUtil)}
		}
	}

	// Investigate-specific: fault-tree convergence required?
	if criteria.RequireConvergence && !convergence {
		return PhaseGateResult{Reason: "fault-tree has not converged (R44 hypothesis-set criteria not met)"}
	}

	// Plan-specific: Plan-Verify approval required?
	if criteria.RequireApproval && !planApproved {
		return PhaseGateResult{Reason: "Plan-Verify did not approve plan (Rain block-authority)"}
	}

	return PhaseGateResult{Allowed: true, Reason: "all done-criteria satisfied"}
}
