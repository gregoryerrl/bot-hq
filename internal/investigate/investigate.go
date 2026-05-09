// Package investigate provides the bilateral-Investigate orchestrator
// per phase-t.md v5 T-2.2 — wraps cl.IPIVRuntime + faulttree + hypothesis
// + R44 anti-cross-confirmation-bias into a high-level workflow:
//
//	OpenInvestigation → ProposeHypothesis (navigator) → AssignAntiCross
//	→ StartHypothesisLoop (driver) → AdvanceHypothesisLoop → IsConverged
//	→ Finalize (transition to Plan)
//
// Layering: investigate is a CONSUMER of cl/faulttree/hypothesis/mvt — no
// upward callbacks back into cl preserve one-way fan-out direction
// (Rain msg 17240 soft design-note). Filesystem persistence is delegated
// to caller-supplied TreeStore + LoopStore interfaces; orchestrator does
// not depend on filesystem layout directly.

package investigate

import (
	"errors"
	"fmt"

	"github.com/gregoryerrl/bot-hq/internal/cl"
	"github.com/gregoryerrl/bot-hq/internal/faulttree"
	"github.com/gregoryerrl/bot-hq/internal/hypothesis"
	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

// TreeStore persists fault-trees per task. Caller-supplied (filesystem
// adapter wraps faulttree.Save/Load + faulttree.CanonicalPath).
type TreeStore interface {
	Save(taskID string, tree *faulttree.Tree) error
	Load(taskID string) (*faulttree.Tree, error)
}

// LoopStore persists hypothesis loops per task. Caller-supplied
// (filesystem adapter wraps hypothesis.Save/Load + hypothesis.CanonicalPath).
type LoopStore interface {
	Save(taskID, loopID string, loop *hypothesis.Loop) error
	Load(taskID, loopID string) (*hypothesis.Loop, error)
	List(taskID string) ([]*hypothesis.Loop, error)
}

// Orchestrator wraps the bilateral-Investigate workflow.
type Orchestrator struct {
	rt        *cl.IPIVRuntime
	treeStore TreeStore
	loopStore LoopStore
}

// New constructs an Orchestrator scoped to the given IPIV runtime + storage.
func New(rt *cl.IPIVRuntime, treeStore TreeStore, loopStore LoopStore) (*Orchestrator, error) {
	if rt == nil {
		return nil, errors.New("IPIVRuntime is required")
	}
	if treeStore == nil || loopStore == nil {
		return nil, errors.New("treeStore and loopStore are required")
	}
	return &Orchestrator{rt: rt, treeStore: treeStore, loopStore: loopStore}, nil
}

// Investigation represents one bilateral-Investigate session. AgentA and
// AgentB are the two cross-model agents (e.g., brian-claude + rain-deepseek).
// Each agent acts as navigator (proposes hypotheses) for some nodes and
// driver (runs investigation) for nodes proposed by the other agent.
type Investigation struct {
	TaskID string
	AgentA string
	AgentB string
	ModelA string
	ModelB string

	rt        *cl.IPIVRuntime
	treeStore TreeStore
	loopStore LoopStore
}

// OpenInvestigation creates a new IPIV task at Investigate phase + initializes
// an empty fault-tree. agentA + agentB must differ (R44 anti-cross).
func (o *Orchestrator) OpenInvestigation(decisionClass mvt.DecisionClass, agentA, agentB, modelA, modelB string) (*Investigation, error) {
	if agentA == "" || agentB == "" {
		return nil, errors.New("agentA and agentB are required")
	}
	if agentA == agentB {
		return nil, errors.New("agentA and agentB must differ (R44 anti-cross)")
	}
	taskID, _, err := o.rt.OpenTask(decisionClass)
	if err != nil {
		return nil, fmt.Errorf("OpenTask: %w", err)
	}
	tree := faulttree.NewTree(taskID)
	if err := o.treeStore.Save(taskID, tree); err != nil {
		return nil, fmt.Errorf("save initial tree: %w", err)
	}
	return &Investigation{
		TaskID:    taskID,
		AgentA:    agentA,
		AgentB:    agentB,
		ModelA:    modelA,
		ModelB:    modelB,
		rt:        o.rt,
		treeStore: o.treeStore,
		loopStore: o.loopStore,
	}, nil
}

// ProposeHypothesis adds a new fault-tree node owned by the proposing
// agent. Per R44 anti-cross, the proposer cannot drive investigation.
func (i *Investigation) ProposeHypothesis(proposer, title, description, parentID string, nodeType faulttree.NodeType) (string, error) {
	if proposer != i.AgentA && proposer != i.AgentB {
		return "", fmt.Errorf("proposer %q not in this investigation (agentA=%s, agentB=%s)", proposer, i.AgentA, i.AgentB)
	}
	tree, err := i.treeStore.Load(i.TaskID)
	if err != nil {
		return "", fmt.Errorf("load tree: %w", err)
	}
	nodeID, err := tree.AddNode(&faulttree.Node{
		Type:        nodeType,
		Title:       title,
		Description: description,
		Owner:       proposer,
		ParentID:    parentID,
	})
	if err != nil {
		return "", fmt.Errorf("AddNode: %w", err)
	}
	if err := i.treeStore.Save(i.TaskID, tree); err != nil {
		return "", fmt.Errorf("save tree: %w", err)
	}
	return nodeID, nil
}

// AssignAntiCross auto-assigns the non-owner agent as investigator for
// the given node per R44 anti-cross-confirmation-bias.
func (i *Investigation) AssignAntiCross(nodeID string) (string, error) {
	tree, err := i.treeStore.Load(i.TaskID)
	if err != nil {
		return "", fmt.Errorf("load tree: %w", err)
	}
	node := tree.GetNode(nodeID)
	if node == nil {
		return "", fmt.Errorf("node %s not found", nodeID)
	}
	if node.Owner == "" {
		return "", fmt.Errorf("node %s has no owner; cannot anti-cross-assign", nodeID)
	}
	var investigator string
	switch node.Owner {
	case i.AgentA:
		investigator = i.AgentB
	case i.AgentB:
		investigator = i.AgentA
	default:
		return "", fmt.Errorf("node owner %q not in this investigation", node.Owner)
	}
	if err := tree.AssignInvestigator(nodeID, investigator); err != nil {
		return "", fmt.Errorf("AssignInvestigator: %w", err)
	}
	if err := i.treeStore.Save(i.TaskID, tree); err != nil {
		return "", fmt.Errorf("save tree: %w", err)
	}
	return investigator, nil
}

// StartHypothesisLoop initializes a Zeller loop on a leaf with an assigned
// investigator. Driver of the loop is the assigned investigator (R44).
func (i *Investigation) StartHypothesisLoop(nodeID, hypothesisStmt string) (*hypothesis.Loop, error) {
	tree, err := i.treeStore.Load(i.TaskID)
	if err != nil {
		return nil, fmt.Errorf("load tree: %w", err)
	}
	node := tree.GetNode(nodeID)
	if node == nil {
		return nil, fmt.Errorf("node %s not found", nodeID)
	}
	if node.Investigator == "" {
		return nil, fmt.Errorf("node %s has no investigator; call AssignAntiCross first", nodeID)
	}
	loop, err := hypothesis.NewLoop(i.TaskID, nodeID, node.Investigator, hypothesisStmt)
	if err != nil {
		return nil, fmt.Errorf("NewLoop: %w", err)
	}
	if err := i.loopStore.Save(i.TaskID, loop.ID, loop); err != nil {
		return nil, fmt.Errorf("save loop: %w", err)
	}
	return loop, nil
}

// AdvanceHypothesisLoop progresses an existing loop through one stage:
// Prediction (content=prediction), Experiment (content=experiment, observation=observation),
// or Concluded (verdict=conclusion). Final Concluded stage mirrors the
// verdict onto the fault-tree node status.
func (i *Investigation) AdvanceHypothesisLoop(loopID string, toStage hypothesis.LoopStatus, content, observation string, verdict hypothesis.Conclusion) (*hypothesis.Loop, error) {
	loop, err := i.loopStore.Load(i.TaskID, loopID)
	if err != nil {
		return nil, fmt.Errorf("load loop: %w", err)
	}
	switch toStage {
	case hypothesis.StatusPredictionMade:
		if err := loop.SetPrediction(content); err != nil {
			return nil, err
		}
	case hypothesis.StatusExperimentRun:
		if err := loop.SetExperimentObservation(content, observation); err != nil {
			return nil, err
		}
	case hypothesis.StatusConcluded:
		if err := loop.Conclude(verdict); err != nil {
			return nil, err
		}
		if err := i.mirrorConclusionToNode(loop.NodeID, verdict); err != nil {
			return nil, fmt.Errorf("mirror to node: %w", err)
		}
	default:
		return nil, fmt.Errorf("unsupported stage advance: %s", toStage)
	}
	if err := i.loopStore.Save(i.TaskID, loopID, loop); err != nil {
		return nil, fmt.Errorf("save loop: %w", err)
	}
	return loop, nil
}

func (i *Investigation) mirrorConclusionToNode(nodeID string, verdict hypothesis.Conclusion) error {
	var status faulttree.NodeStatus
	switch verdict {
	case hypothesis.ConclusionConfirmed:
		status = faulttree.StatusConfirmed
	case hypothesis.ConclusionRefuted:
		status = faulttree.StatusRefuted
	case hypothesis.ConclusionWeak:
		status = faulttree.StatusWeak
	default:
		return nil // Unknown — leave node status unchanged
	}
	tree, err := i.treeStore.Load(i.TaskID)
	if err != nil {
		return fmt.Errorf("load tree: %w", err)
	}
	if err := tree.SetStatus(nodeID, status); err != nil {
		return fmt.Errorf("SetStatus: %w", err)
	}
	return i.treeStore.Save(i.TaskID, tree)
}

// IsConverged returns true when the fault-tree has reached convergence
// (≥1 node + all leaves confirmed/refuted with ≥2 cite-anchors per
// faulttree.IsConvergence) AND every hypothesis loop has been concluded.
// Empty-tree returns false to reject vacuous-convergence.
func (i *Investigation) IsConverged() (bool, error) {
	tree, err := i.treeStore.Load(i.TaskID)
	if err != nil {
		return false, err
	}
	if len(tree.Nodes) == 0 {
		return false, nil // empty tree is not converged
	}
	if !tree.IsConvergence() {
		return false, nil
	}
	loops, err := i.loopStore.List(i.TaskID)
	if err != nil {
		return false, err
	}
	for _, l := range loops {
		if !l.IsComplete() {
			return false, nil
		}
	}
	return true, nil
}

// AddCiteAnchor appends an evidence reference to a fault-tree node.
// Required to clear faulttree.IsConvergence on confirmed nodes (≥2 anchors).
func (i *Investigation) AddCiteAnchor(nodeID, anchor string) error {
	tree, err := i.treeStore.Load(i.TaskID)
	if err != nil {
		return fmt.Errorf("load tree: %w", err)
	}
	if err := tree.AddCiteAnchor(nodeID, anchor); err != nil {
		return fmt.Errorf("AddCiteAnchor: %w", err)
	}
	return i.treeStore.Save(i.TaskID, tree)
}

// Finalize transitions the IPIV task from Investigate to Plan phase. Errors
// when investigation has not yet converged (use IsConverged() to check).
// Returns the post-transition TaskState.
func (i *Investigation) Finalize() (*mvt.TaskState, error) {
	converged, err := i.IsConverged()
	if err != nil {
		return nil, fmt.Errorf("IsConverged: %w", err)
	}
	if !converged {
		return nil, errors.New("investigation not yet converged; cannot finalize")
	}
	agentModels := map[string]string{
		i.AgentA: i.ModelA,
		i.AgentB: i.ModelB,
	}
	return i.rt.TransitionPhase(i.TaskID, mvt.StagePlan, agentModels)
}

// GetTree returns a snapshot of the current fault-tree.
func (i *Investigation) GetTree() (*faulttree.Tree, error) {
	return i.treeStore.Load(i.TaskID)
}

// GetLoops returns all hypothesis loops for the investigation.
func (i *Investigation) GetLoops() ([]*hypothesis.Loop, error) {
	return i.loopStore.List(i.TaskID)
}
