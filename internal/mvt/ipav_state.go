// Package mvt implements the Phase T-0.5 MVT prototype for the IPAV pipeline
// state machine (Investigate / Plan / Implement / Verify).
//
// This is an MVT (Minimum-Viable-Test) prototype per phase-t.md v5 T-0.5
// sub-phase: minimal IPAV state-machine + 1-2 investigator tools demonstrating
// end-to-end cycle on a real task. Full implementation is T-2 sub-phase scope.
//
// Storage: per-task state YAML at
//   ~/.bot-hq/projects/<project>/tasks/<task-id>/ipav-state.yaml
//
// MVT scope (per phase-t.md v5):
//   - IPAV state schema types (Stage / Mode / TaskState)
//   - Load/save with atomic-write
//   - Phase-transition validator
//   - Mode-transition validator
//
// NOT in MVT scope (T-2 expansion):
//   - Per-rule mechanical-enforcement hooks (R44/R45/R46/R47/R49/R50)
//   - Bilateral-Investigate primitives (cross-model)
//   - Full investigator-toolset (clusters A-E + new tools 6-16)
//   - Cost-tracking instrumentation (R53; T-5 expansion)
package mvt

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"gopkg.in/yaml.v3"
)

// Stage represents the current IPAV pipeline stage.
type Stage string

const (
	StageInvestigate Stage = "I"
	StagePlan        Stage = "P"
	StageImplement   Stage = "Implement"
	StageVerify      Stage = "V"
)

// Mode represents the per-stage agent mode per R45 EXTENDED.
type Mode string

const (
	ModeInvestigateSolo          Mode = "investigate-solo"
	ModeInvestigateCollaborative Mode = "investigate-collaborative"
	ModeInvestigateNavigator     Mode = "investigate-navigator"
	ModeInvestigateDriver        Mode = "investigate-driver"
	ModePlanSolo                 Mode = "plan-solo"
	ModePlanBilateral            Mode = "plan-bilateral"
	ModePlanVerify               Mode = "plan-verify"
	ModeImplement                Mode = "implement"
	ModeImplementVerify          Mode = "implement-verify"
)

// DecisionClass represents the R47 REVISED decision-class tag.
type DecisionClass string

const (
	DecisionLow    DecisionClass = "low"
	DecisionMedium DecisionClass = "medium"
	DecisionHigh   DecisionClass = "high"
)

// VerifyResult captures the Verify-phase outcome.
type VerifyResult string

const (
	VerifyPending    VerifyResult = ""
	VerifyPass       VerifyResult = "pass"
	VerifyFail       VerifyResult = "fail"
	VerifyEscalated  VerifyResult = "escalated"
)

// TaskState is the per-task IPAV state schema per phase-t.md v5.
//
// ClosedAt distinguishes open vs closed tasks: zero = still in flight
// (including post-Verify-fail loop-backs); non-zero = terminal closure
// (pass / escalated). Set by IPAVRuntime.CompleteTask on terminal result.
type TaskState struct {
	TaskID    string `yaml:"task_id"`
	// SessionID binds the task to the session-cluster it was opened
	// within. Populated by IPAVRuntime.OpenTask. Empty for legacy tasks
	// opened pre-session-lifecycle-cleanup OR when no active session
	// existed at open-time (the auto-finalize on verify-pass path then
	// silently no-ops; nothing to close).
	SessionID       string               `yaml:"session_id,omitempty"`
	OpenedAt        time.Time            `yaml:"opened_at"`
	ClosedAt        time.Time            `yaml:"closed_at,omitempty"`
	DecisionClass   DecisionClass        `yaml:"decision_class"`
	CurrentPhase    Stage                `yaml:"current_phase"`
	PhaseMode       Mode                 `yaml:"phase_mode"`
	PhaseBudget     PhaseBudget          `yaml:"phase_budget"`
	PhaseUsed       map[Stage]PhaseUsage `yaml:"phase_used"`
	PhaseArtifacts  PhaseArtifacts       `yaml:"phase_artifacts"`
	PhaseHistory    []PhaseHistoryEntry  `yaml:"phase_history"`
	VerifyResult    VerifyResult         `yaml:"verify_result"`
	VerifyLoopCount int                  `yaml:"verify_loop_count"`
}

// PhaseBudget represents per-phase compute-budget allocation per phase-t.md v5.
type PhaseBudget struct {
	InvestigatePct int `yaml:"investigate"` // 55-60
	PlanPct        int `yaml:"plan"`        // 10-15
	ImplementPct   int `yaml:"implement"`   // 5
	VerifyPct      int `yaml:"verify"`      // 20-25
}

// DefaultPhaseBudget returns the default IPAV budget per phase-t.md v5.
func DefaultPhaseBudget() PhaseBudget {
	return PhaseBudget{
		InvestigatePct: 58,
		PlanPct:        12,
		ImplementPct:   5,
		VerifyPct:      25,
	}
}

// PhaseUsage tracks per-phase token + cost spend.
type PhaseUsage struct {
	TokensConsumed int                 `yaml:"tokens_consumed"`
	CostPerAgent   map[string]float64  `yaml:"cost_per_agent"` // agent_id -> usd
}

// PhaseArtifacts tracks per-phase output paths.
type PhaseArtifacts struct {
	InvestigationDoc string   `yaml:"investigation_doc,omitempty"`
	FaultTree        string   `yaml:"fault_tree,omitempty"`
	PlanDoc          string   `yaml:"plan_doc,omitempty"`
	PlanBilateralA   string   `yaml:"plan_bilateral_a,omitempty"` // Brian-Claude variant
	PlanBilateralB   string   `yaml:"plan_bilateral_b,omitempty"` // Rain-DeepSeek variant
	PlanMergeLog     string   `yaml:"plan_merge_log,omitempty"`
	ImplementCommits []string `yaml:"implement_commits,omitempty"`
	VerifyReport     string   `yaml:"verify_report,omitempty"`
}

// PhaseHistoryEntry tracks a phase-transition event.
type PhaseHistoryEntry struct {
	Phase           Stage             `yaml:"phase"`
	SubPhase        string            `yaml:"sub_phase,omitempty"` // e.g. pre-hypothesis | hypothesis-investigation | solo
	StartedAt       time.Time         `yaml:"started_at"`
	EndedAt         time.Time         `yaml:"ended_at,omitempty"`
	SyncPoints      []int             `yaml:"sync_points,omitempty"` // [25, 50, 75, 90]
	BilateralMode   string            `yaml:"bilateral_mode,omitempty"` // solo | bilateral
	AgentModelAtPhase map[string]string `yaml:"agent_model_at_phase,omitempty"` // agent_id -> model_name
}

// validPhaseTransitions enforces I → P → Implement → V sequential pipeline.
var validPhaseTransitions = map[Stage][]Stage{
	StageInvestigate: {StagePlan},
	StagePlan:        {StageImplement, StageInvestigate}, // can loop-back to Investigate
	StageImplement:   {StageVerify, StagePlan},           // can loop-back to Plan
	StageVerify:      {StageImplement, StagePlan, StageInvestigate}, // Verify-fail loop-back
}

// ValidatePhaseTransition checks if a phase-transition is permitted per IPAV.
func ValidatePhaseTransition(from, to Stage) error {
	allowedTos, ok := validPhaseTransitions[from]
	if !ok {
		return fmt.Errorf("invalid from-phase: %s", from)
	}
	for _, allowed := range allowedTos {
		if allowed == to {
			return nil
		}
	}
	return fmt.Errorf("invalid phase transition: %s -> %s", from, to)
}

// validModesForStage enforces mode-stage compatibility per R45.
var validModesForStage = map[Stage][]Mode{
	StageInvestigate: {
		ModeInvestigateSolo,
		ModeInvestigateCollaborative,
		ModeInvestigateNavigator,
		ModeInvestigateDriver,
	},
	StagePlan: {
		ModePlanSolo,
		ModePlanBilateral,
		ModePlanVerify,
	},
	StageImplement: {
		ModeImplement,
	},
	StageVerify: {
		ModePlanVerify,
		ModeImplementVerify,
	},
}

// ValidateMode checks if a mode is valid for the given stage per R45.
func ValidateMode(stage Stage, mode Mode) error {
	allowedModes, ok := validModesForStage[stage]
	if !ok {
		return fmt.Errorf("invalid stage: %s", stage)
	}
	for _, allowed := range allowedModes {
		if allowed == mode {
			return nil
		}
	}
	return fmt.Errorf("invalid mode %s for stage %s", mode, stage)
}

// NewTaskState creates a new task-state with defaults per phase-t.md v5.
// sessionID binds the task to its session-cluster for auto-finalize on
// verify-pass per session-lifecycle-cleanup. Empty sessionID is permitted
// (degraded mode; no auto-finalize wires).
func NewTaskState(taskID, sessionID string, decisionClass DecisionClass) *TaskState {
	return &TaskState{
		TaskID:        taskID,
		SessionID:     sessionID,
		OpenedAt:      time.Now().UTC(),
		DecisionClass: decisionClass,
		CurrentPhase:  StageInvestigate,
		PhaseMode:     ModeInvestigateSolo, // default; bilateral-fires per R47 if medium/high-stakes
		PhaseBudget:   DefaultPhaseBudget(),
		PhaseUsed:     make(map[Stage]PhaseUsage),
		PhaseHistory:  []PhaseHistoryEntry{},
		VerifyResult:  VerifyPending,
	}
}

// Transition advances to the next phase + records phase-history.
// Per R44 expanded: bilateral-mode auto-set if decision-class is medium/high.
func (ts *TaskState) Transition(to Stage, agentModels map[string]string) error {
	if err := ValidatePhaseTransition(ts.CurrentPhase, to); err != nil {
		return err
	}

	// Close current phase-history entry
	if len(ts.PhaseHistory) > 0 {
		ts.PhaseHistory[len(ts.PhaseHistory)-1].EndedAt = time.Now().UTC()
	}

	// Open new phase-history entry
	bilateralMode := "solo"
	if (to == StageInvestigate || to == StagePlan) && ts.DecisionClass != DecisionLow {
		bilateralMode = "bilateral"
	}

	ts.CurrentPhase = to
	ts.PhaseMode = defaultModeForStage(to, bilateralMode)
	ts.PhaseHistory = append(ts.PhaseHistory, PhaseHistoryEntry{
		Phase:             to,
		StartedAt:         time.Now().UTC(),
		BilateralMode:     bilateralMode,
		AgentModelAtPhase: agentModels,
	})

	return nil
}

// defaultModeForStage returns the default mode for a stage given bilateral-mode.
func defaultModeForStage(stage Stage, bilateralMode string) Mode {
	switch stage {
	case StageInvestigate:
		if bilateralMode == "bilateral" {
			return ModeInvestigateCollaborative
		}
		return ModeInvestigateSolo
	case StagePlan:
		if bilateralMode == "bilateral" {
			return ModePlanBilateral
		}
		return ModePlanSolo
	case StageImplement:
		return ModeImplement
	case StageVerify:
		return ModeImplementVerify // default; PlanVerify if Plan→Verify-only checkpoint
	}
	return ""
}

// Save writes task-state to YAML at given path with atomic-write.
func (ts *TaskState) Save(path string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}

	data, err := yaml.Marshal(ts)
	if err != nil {
		return fmt.Errorf("yaml marshal: %w", err)
	}

	tmpPath := path + ".tmp"
	if err := os.WriteFile(tmpPath, data, 0o644); err != nil {
		return fmt.Errorf("write tmp: %w", err)
	}

	if err := os.Rename(tmpPath, path); err != nil {
		return fmt.Errorf("rename: %w", err)
	}

	return nil
}

// Load reads task-state from YAML at given path.
func Load(path string) (*TaskState, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read: %w", err)
	}

	var ts TaskState
	if err := yaml.Unmarshal(data, &ts); err != nil {
		return nil, fmt.Errorf("yaml unmarshal: %w", err)
	}

	return &ts, nil
}

// TaskStatePath returns the canonical YAML storage path for a task.
func TaskStatePath(homeDir, projectID, taskID string) string {
	return filepath.Join(homeDir, ".bot-hq", "projects", projectID, "tasks", taskID, "ipav-state.yaml")
}

// ErrInvalidTransition is returned when a phase or mode transition is rejected.
var ErrInvalidTransition = errors.New("invalid transition")
