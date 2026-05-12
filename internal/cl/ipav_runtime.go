// Package cl — ipav_runtime.go: T-2.1 IPAV state machine runtime integration.
//
// Wraps the MVT IPAV state-machine (internal/mvt) + T-1.7 CL persistence
// + R47 decision-class-tagging + R44 bilateral-mode auto-set into a
// runtime-callable interface for daemon + agent consumption.
//
// Public API:
//
//	IPAVRuntime — daemon-callable handle wrapping CL + emit-hooks
//	IPAVRuntime.OpenTask      — create new IPAV task with decision-class
//	IPAVRuntime.GetTask        — load existing task by id
//	IPAVRuntime.TransitionPhase — phase-transition with auto-bilateral-mode
//	IPAVRuntime.RecordPhaseUsage — track tokens + cost-per-agent per phase
//	IPAVRuntime.CompleteTask  — mark task done (Verify pass/fail/escalated)
//
// Future extensions (T-2.6 IPAV-first-task validation; T-2.7 cost-tracking):
//   - Hook wiring for R44 anti-cross + R45 mode-tag + R49 pre-seal-audit
//   - LLM-call cost-tracking integration

package cl

import (
	"errors"
	"fmt"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/mvt"
	"github.com/google/uuid"
)

// IPAVRuntime is the daemon-callable handle for IPAV state machine.
type IPAVRuntime struct {
	cl      *CL
	project string
}

// NewIPAVRuntime constructs a runtime handle scoped to the given project.
// Project corresponds to ~/.bot-hq/projects/<project>/tasks/ subtree.
func NewIPAVRuntime(c *CL, project string) (*IPAVRuntime, error) {
	if c == nil {
		return nil, errors.New("CL handle is nil")
	}
	if project == "" {
		return nil, errors.New("project is required")
	}
	return &IPAVRuntime{cl: c, project: project}, nil
}

// OpenTask creates a new IPAV per-task state with a generated UUID-based
// task-id + the given decision-class. Returns the task-id + persisted state.
//
// sessionID binds the task to its containing session-cluster — populates
// TaskState.SessionID so bot_hq_ipav_complete(verify=pass) can fire
// hub_session_finalize on the right session per session-lifecycle-cleanup.
// Empty sessionID is permitted (degraded mode; no auto-finalize wires —
// agent must call hub_session_finalize manually).
//
// Per R44 expanded + R47 revised: medium/high decision-class triggers
// bilateral-mode in subsequent Investigate + Plan transitions.
func (r *IPAVRuntime) OpenTask(sessionID string, decisionClass mvt.DecisionClass) (string, *mvt.TaskState, error) {
	taskID := uuid.New().String()
	ts := mvt.NewTaskState(taskID, sessionID, decisionClass)
	if err := r.cl.SaveIPAVState(r.project, ts); err != nil {
		return "", nil, fmt.Errorf("save new task: %w", err)
	}
	return taskID, ts, nil
}

// GetTask loads an existing IPAV per-task state. Returns ErrNotFound when
// the task does not exist.
func (r *IPAVRuntime) GetTask(taskID string) (*mvt.TaskState, error) {
	return r.cl.IPAVState(r.project, taskID)
}

// TransitionPhase advances a task to the next IPAV phase + persists.
// agentModels maps agent-id → active-model-name (e.g. brian → claude-default,
// rain → deepseek-v4-pro) for phase-history attribution.
//
// Bilateral-mode auto-set per mvt.TaskState.Transition logic (R44 expanded:
// medium/high decision-class triggers bilateral on Investigate + Plan).
func (r *IPAVRuntime) TransitionPhase(taskID string, to mvt.Stage, agentModels map[string]string) (*mvt.TaskState, error) {
	ts, err := r.GetTask(taskID)
	if err != nil {
		return nil, fmt.Errorf("load task %s: %w", taskID, err)
	}
	if err := ts.Transition(to, agentModels); err != nil {
		return nil, fmt.Errorf("transition %s→%s: %w", ts.CurrentPhase, to, err)
	}
	if err := r.cl.SaveIPAVState(r.project, ts); err != nil {
		return nil, fmt.Errorf("save after transition: %w", err)
	}
	return ts, nil
}

// RecordPhaseUsage updates per-phase token + cost tracking. Called after
// each LLM round-trip in the active phase (T-5 cost-tracking-per-agent
// will integrate at LLM-call subprocess sites).
func (r *IPAVRuntime) RecordPhaseUsage(taskID string, agentID string, tokens int, costUSD float64) (*mvt.TaskState, error) {
	ts, err := r.GetTask(taskID)
	if err != nil {
		return nil, fmt.Errorf("load task %s: %w", taskID, err)
	}
	if ts.PhaseUsed == nil {
		ts.PhaseUsed = make(map[mvt.Stage]mvt.PhaseUsage)
	}
	usage := ts.PhaseUsed[ts.CurrentPhase]
	usage.TokensConsumed += tokens
	if usage.CostPerAgent == nil {
		usage.CostPerAgent = make(map[string]float64)
	}
	usage.CostPerAgent[agentID] += costUSD
	ts.PhaseUsed[ts.CurrentPhase] = usage

	if err := r.cl.SaveIPAVState(r.project, ts); err != nil {
		return nil, fmt.Errorf("save after usage record: %w", err)
	}
	return ts, nil
}

// SetPhaseArtifact records an artifact path for the current phase. Common
// keys: investigation_doc, fault_tree, plan_doc, plan_bilateral_a/b,
// plan_merge_log, verify_report. Implement commits accumulate via
// AddImplementCommit.
func (r *IPAVRuntime) SetPhaseArtifact(taskID, key, path string) (*mvt.TaskState, error) {
	ts, err := r.GetTask(taskID)
	if err != nil {
		return nil, fmt.Errorf("load task %s: %w", taskID, err)
	}
	switch key {
	case "investigation_doc":
		ts.PhaseArtifacts.InvestigationDoc = path
	case "fault_tree":
		ts.PhaseArtifacts.FaultTree = path
	case "plan_doc":
		ts.PhaseArtifacts.PlanDoc = path
	case "plan_bilateral_a":
		ts.PhaseArtifacts.PlanBilateralA = path
	case "plan_bilateral_b":
		ts.PhaseArtifacts.PlanBilateralB = path
	case "plan_merge_log":
		ts.PhaseArtifacts.PlanMergeLog = path
	case "verify_report":
		ts.PhaseArtifacts.VerifyReport = path
	default:
		return nil, fmt.Errorf("unknown artifact key: %s", key)
	}
	if err := r.cl.SaveIPAVState(r.project, ts); err != nil {
		return nil, fmt.Errorf("save after artifact set: %w", err)
	}
	return ts, nil
}

// AddImplementCommit appends a git commit SHA to the task's implement-
// commits list (T-2 + T-5 git-class operations).
func (r *IPAVRuntime) AddImplementCommit(taskID, commitSHA string) (*mvt.TaskState, error) {
	ts, err := r.GetTask(taskID)
	if err != nil {
		return nil, err
	}
	ts.PhaseArtifacts.ImplementCommits = append(ts.PhaseArtifacts.ImplementCommits, commitSHA)
	if err := r.cl.SaveIPAVState(r.project, ts); err != nil {
		return nil, fmt.Errorf("save after commit-append: %w", err)
	}
	return ts, nil
}

// CompleteTask marks the task with a Verify result (pass/fail/escalated).
// Increments verify_loop_count when result is "fail" (per R-T-4 max-3-retry
// circuit-breaker). Sets ClosedAt when result is terminal (pass/escalated)
// — fail leaves the task open for the V→P loop-back.
func (r *IPAVRuntime) CompleteTask(taskID string, result mvt.VerifyResult) (*mvt.TaskState, error) {
	ts, err := r.GetTask(taskID)
	if err != nil {
		return nil, err
	}
	ts.VerifyResult = result
	switch result {
	case mvt.VerifyFail:
		ts.VerifyLoopCount++
	case mvt.VerifyPass, mvt.VerifyEscalated:
		ts.ClosedAt = time.Now().UTC()
	}
	if err := r.cl.SaveIPAVState(r.project, ts); err != nil {
		return nil, fmt.Errorf("save after complete: %w", err)
	}
	return ts, nil
}

// ListTasks returns all open + closed task-states for the runtime's project.
func (r *IPAVRuntime) ListTasks() ([]*mvt.TaskState, error) {
	return r.cl.ListIPAVStates(r.project)
}

// TaskAge returns how long since the task was opened. Useful for stale-
// task detection in daemon health-checks.
func TaskAge(ts *mvt.TaskState) time.Duration {
	return time.Since(ts.OpenedAt)
}
