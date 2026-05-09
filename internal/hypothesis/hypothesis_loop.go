// Package hypothesis implements cl_hypothesis_loop (tool 8) +
// cl_strong_style_assign (tool 9) per phase-t.md v5 T-2.4.
//
// cl_hypothesis_loop enforces Zeller scientific-debugging structured-output:
//
//	Hypothesis → Prediction → Experiment → Conclusion
//
// Each driver runs this loop on a fault-tree leaf assigned by navigator.
// Structured-output persists per-leaf-investigation cycle for cite-anchored
// evidence + cross-verify gate (R44 expanded).
//
// cl_strong_style_assign auto-assigns investigators per R44 anti-cross-
// confirmation-bias: navigator who proposes hypothesis H cannot drive
// investigation of H. Mechanically enforced via faulttree.Tree.AssignInvestigator
// + this auto-assignment helper.

package hypothesis

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

// LoopStatus indicates the current Zeller loop phase.
type LoopStatus string

const (
	StatusHypothesisFormed LoopStatus = "hypothesis"
	StatusPredictionMade   LoopStatus = "prediction"
	StatusExperimentRun    LoopStatus = "experiment"
	StatusConcluded        LoopStatus = "concluded"
)

// Conclusion enumerates the loop outcome.
type Conclusion string

const (
	ConclusionConfirmed Conclusion = "confirmed"
	ConclusionRefuted   Conclusion = "refuted"
	ConclusionWeak      Conclusion = "weak" // partial-evidence; needs continuation
	ConclusionUnknown   Conclusion = "unknown"
)

// Loop is one Zeller hypothesis-investigation cycle.
type Loop struct {
	ID            string     `json:"id"`              // generated UUID
	TaskID        string     `json:"task_id"`
	NodeID        string     `json:"node_id"`         // fault-tree node being investigated
	Driver        string     `json:"driver"`          // agent-id running the loop
	Status        LoopStatus `json:"status"`
	Hypothesis    string     `json:"hypothesis"`
	Prediction    string     `json:"prediction,omitempty"`
	Experiment    string     `json:"experiment,omitempty"`
	Observation   string     `json:"observation,omitempty"`
	Conclusion    Conclusion `json:"conclusion,omitempty"`
	CiteAnchors   []string   `json:"cite_anchors,omitempty"`
	CreatedAt     time.Time  `json:"created_at"`
	UpdatedAt     time.Time  `json:"updated_at"`
}

// NewLoop initializes a new hypothesis loop. driver MUST not be the
// node's owner (R44 anti-cross — caller validates via faulttree.AssignInvestigator
// or cl_strong_style_assign before constructing the loop).
func NewLoop(taskID, nodeID, driver, hypothesis string) (*Loop, error) {
	if hypothesis == "" {
		return nil, errors.New("hypothesis is required")
	}
	if driver == "" {
		return nil, errors.New("driver is required")
	}
	now := time.Now().UTC()
	return &Loop{
		ID:         genID(),
		TaskID:     taskID,
		NodeID:     nodeID,
		Driver:     driver,
		Status:     StatusHypothesisFormed,
		Hypothesis: hypothesis,
		CreatedAt:  now,
		UpdatedAt:  now,
	}, nil
}

// SetPrediction advances loop to Prediction phase. Must be called from
// StatusHypothesisFormed.
func (l *Loop) SetPrediction(prediction string) error {
	if l.Status != StatusHypothesisFormed {
		return fmt.Errorf("invalid transition: status=%s, expected %s", l.Status, StatusHypothesisFormed)
	}
	if prediction == "" {
		return errors.New("prediction is required")
	}
	l.Prediction = prediction
	l.Status = StatusPredictionMade
	l.UpdatedAt = time.Now().UTC()
	return nil
}

// SetExperimentObservation records experiment + observed result.
// Advances loop to Experiment phase.
func (l *Loop) SetExperimentObservation(experiment, observation string) error {
	if l.Status != StatusPredictionMade {
		return fmt.Errorf("invalid transition: status=%s, expected %s", l.Status, StatusPredictionMade)
	}
	if experiment == "" || observation == "" {
		return errors.New("experiment and observation are required")
	}
	l.Experiment = experiment
	l.Observation = observation
	l.Status = StatusExperimentRun
	l.UpdatedAt = time.Now().UTC()
	return nil
}

// Conclude advances loop to Conclusion phase + records final verdict.
// Conclusion is one of: confirmed | refuted | weak | unknown.
func (l *Loop) Conclude(verdict Conclusion) error {
	if l.Status != StatusExperimentRun {
		return fmt.Errorf("invalid transition: status=%s, expected %s", l.Status, StatusExperimentRun)
	}
	switch verdict {
	case ConclusionConfirmed, ConclusionRefuted, ConclusionWeak, ConclusionUnknown:
		// ok
	default:
		return fmt.Errorf("invalid conclusion: %s", verdict)
	}
	l.Conclusion = verdict
	l.Status = StatusConcluded
	l.UpdatedAt = time.Now().UTC()
	return nil
}

// AddCiteAnchor appends evidence to the loop. Acceptable in any status.
func (l *Loop) AddCiteAnchor(anchor string) {
	l.CiteAnchors = append(l.CiteAnchors, anchor)
	l.UpdatedAt = time.Now().UTC()
}

// IsComplete returns true when loop is in Concluded state.
func (l *Loop) IsComplete() bool {
	return l.Status == StatusConcluded
}

// Save persists the loop to canonical-storage.
func (l *Loop) Save(path string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}
	data, err := json.MarshalIndent(l, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal: %w", err)
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return fmt.Errorf("write tmp: %w", err)
	}
	return os.Rename(tmp, path)
}

// Load reads a hypothesis loop from disk.
func Load(path string) (*Loop, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read: %w", err)
	}
	var l Loop
	if err := json.Unmarshal(data, &l); err != nil {
		return nil, fmt.Errorf("unmarshal: %w", err)
	}
	return &l, nil
}

// CanonicalPath returns the canonical loop-storage path for a task + loop.
// Loops live alongside fault-tree under the task directory.
func CanonicalPath(homeDir, project, taskID, loopID string) string {
	return filepath.Join(homeDir, ".bot-hq", "projects", project, "tasks", taskID, "hypothesis-loops", loopID+".json")
}

// AssignDriver is the cl_strong_style_assign tool: auto-assigns a driver
// for a hypothesis-leaf such that driver != owner per R44 anti-cross.
//
// Inputs:
//   - hypothesisOwner: agent-id who proposed the hypothesis
//   - candidates: pool of available drivers (e.g. ["brian","rain"])
//
// Returns assigned driver-id or error if no eligible candidate exists.
// Strategy: pick first candidate where candidate != owner.
func AssignDriver(hypothesisOwner string, candidates []string) (string, error) {
	if hypothesisOwner == "" {
		return "", errors.New("hypothesis owner is required")
	}
	if len(candidates) == 0 {
		return "", errors.New("no candidate drivers")
	}
	for _, c := range candidates {
		if c != "" && c != hypothesisOwner {
			return c, nil
		}
	}
	return "", fmt.Errorf("R44 anti-cross: no candidate driver != owner %q in pool %v", hypothesisOwner, candidates)
}

// genID is a small UUID-style identifier helper. Imports kept minimal;
// uuid library used elsewhere in cl package.
func genID() string {
	// simple time-based ID; sufficient for per-task loop disambiguation
	return fmt.Sprintf("loop-%d", time.Now().UnixNano())
}
