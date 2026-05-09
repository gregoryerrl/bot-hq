package investigate

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/cl"
	"github.com/gregoryerrl/bot-hq/internal/faulttree"
	"github.com/gregoryerrl/bot-hq/internal/hypothesis"
	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

// mustMakeCL constructs a CL handle on a fresh temp dir + mkdir-if-needed
// for tests. Per R39 TEST-ISOLATION: each test gets its own temp tree.
func mustMakeCL(t *testing.T) *cl.CL {
	t.Helper()
	dir := t.TempDir()
	root := filepath.Join(dir, ".bot-hq")
	if err := os.MkdirAll(root, 0o755); err != nil {
		t.Fatalf("mkdir cl root: %v", err)
	}
	c, err := cl.NewCL(root)
	if err != nil {
		t.Fatalf("cl.NewCL: %v", err)
	}
	return c
}

// memTreeStore is an in-memory TreeStore for tests.
type memTreeStore struct {
	trees map[string]*faulttree.Tree
}

func newMemTreeStore() *memTreeStore { return &memTreeStore{trees: map[string]*faulttree.Tree{}} }

func (m *memTreeStore) Save(taskID string, tree *faulttree.Tree) error {
	m.trees[taskID] = tree
	return nil
}

func (m *memTreeStore) Load(taskID string) (*faulttree.Tree, error) {
	if t, ok := m.trees[taskID]; ok {
		return t, nil
	}
	return faulttree.NewTree(taskID), nil
}

// memLoopStore is an in-memory LoopStore for tests.
type memLoopStore struct {
	loops map[string]map[string]*hypothesis.Loop // taskID → loopID → loop
}

func newMemLoopStore() *memLoopStore {
	return &memLoopStore{loops: map[string]map[string]*hypothesis.Loop{}}
}

func (m *memLoopStore) Save(taskID, loopID string, loop *hypothesis.Loop) error {
	if m.loops[taskID] == nil {
		m.loops[taskID] = map[string]*hypothesis.Loop{}
	}
	m.loops[taskID][loopID] = loop
	return nil
}

func (m *memLoopStore) Load(taskID, loopID string) (*hypothesis.Loop, error) {
	if tloops, ok := m.loops[taskID]; ok {
		if l, ok := tloops[loopID]; ok {
			return l, nil
		}
	}
	return nil, &loopNotFoundErr{taskID: taskID, loopID: loopID}
}

func (m *memLoopStore) List(taskID string) ([]*hypothesis.Loop, error) {
	out := []*hypothesis.Loop{}
	for _, l := range m.loops[taskID] {
		out = append(out, l)
	}
	return out, nil
}

type loopNotFoundErr struct{ taskID, loopID string }

func (e *loopNotFoundErr) Error() string {
	return "loop " + e.loopID + " not found in task " + e.taskID
}

// newTestOrchestrator constructs an Orchestrator wired to in-memory CL +
// stores for test isolation per R39 TEST-ISOLATION.
func newTestOrchestrator(t *testing.T) *Orchestrator {
	t.Helper()
	c := mustMakeCL(t)
	rt, err := cl.NewIPIVRuntime(c, "test-project")
	if err != nil {
		t.Fatalf("NewIPIVRuntime: %v", err)
	}
	o, err := New(rt, newMemTreeStore(), newMemLoopStore())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	return o
}

// ====== New / OpenInvestigation ======

func TestNew_validation(t *testing.T) {
	if _, err := New(nil, newMemTreeStore(), newMemLoopStore()); err == nil {
		t.Error("expected error for nil rt")
	}

	c := mustMakeCL(t)
	rt, _ := cl.NewIPIVRuntime(c, "test")

	if _, err := New(rt, nil, newMemLoopStore()); err == nil {
		t.Error("expected error for nil treeStore")
	}
	if _, err := New(rt, newMemTreeStore(), nil); err == nil {
		t.Error("expected error for nil loopStore")
	}
}

func TestOpenInvestigation_assignsBothAgents(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, err := o.OpenInvestigation(mvt.DecisionMedium, "brian", "rain", "claude-default", "deepseek-v4-pro")
	if err != nil {
		t.Fatalf("OpenInvestigation: %v", err)
	}
	if inv.AgentA != "brian" || inv.AgentB != "rain" {
		t.Errorf("agents = (%s, %s), want (brian, rain)", inv.AgentA, inv.AgentB)
	}
	if inv.ModelA != "claude-default" || inv.ModelB != "deepseek-v4-pro" {
		t.Errorf("models = (%s, %s)", inv.ModelA, inv.ModelB)
	}
	if inv.TaskID == "" {
		t.Error("TaskID empty")
	}
}

func TestOpenInvestigation_sameAgentsRejected(t *testing.T) {
	o := newTestOrchestrator(t)
	if _, err := o.OpenInvestigation(mvt.DecisionMedium, "brian", "brian", "x", "y"); err == nil {
		t.Error("expected error for agentA==agentB (R44 anti-cross)")
	}
}

func TestOpenInvestigation_emptyAgentsRejected(t *testing.T) {
	o := newTestOrchestrator(t)
	if _, err := o.OpenInvestigation(mvt.DecisionMedium, "", "rain", "x", "y"); err == nil {
		t.Error("expected error for empty agentA")
	}
	if _, err := o.OpenInvestigation(mvt.DecisionMedium, "brian", "", "x", "y"); err == nil {
		t.Error("expected error for empty agentB")
	}
}

// ====== ProposeHypothesis + AssignAntiCross ======

func TestProposeHypothesis_addsNode(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")

	nodeID, err := inv.ProposeHypothesis("brian", "DB index missing", "perf-cliff at 50K rows", "", faulttree.NodeAction)
	if err != nil {
		t.Fatalf("ProposeHypothesis: %v", err)
	}
	if nodeID == "" {
		t.Error("nodeID empty")
	}
	tree, _ := inv.GetTree()
	if len(tree.Nodes) != 1 {
		t.Errorf("nodes = %d, want 1", len(tree.Nodes))
	}
	if tree.Nodes[0].Owner != "brian" {
		t.Errorf("owner = %q, want brian", tree.Nodes[0].Owner)
	}
}

func TestProposeHypothesis_outsideAgentsRejected(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")

	if _, err := inv.ProposeHypothesis("emma", "x", "x", "", faulttree.NodeAction); err == nil {
		t.Error("expected error for non-investigation agent")
	}
}

func TestAssignAntiCross_assignsOpposite(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")

	nodeID, _ := inv.ProposeHypothesis("brian", "X", "x", "", faulttree.NodeAction)
	investigator, err := inv.AssignAntiCross(nodeID)
	if err != nil {
		t.Fatalf("AssignAntiCross: %v", err)
	}
	if investigator != "rain" {
		t.Errorf("investigator = %q, want rain (opposite of brian-owned)", investigator)
	}

	// Rain-owned node → brian assigned
	rainNode, _ := inv.ProposeHypothesis("rain", "Y", "y", "", faulttree.NodeAction)
	investigator2, _ := inv.AssignAntiCross(rainNode)
	if investigator2 != "brian" {
		t.Errorf("investigator on rain-owned = %q, want brian", investigator2)
	}
}

func TestAssignAntiCross_unknownNodeError(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")

	if _, err := inv.AssignAntiCross("nonexistent-node-id"); err == nil {
		t.Error("expected error for unknown node")
	}
}

// ====== Hypothesis loop ======

func TestStartHypothesisLoop_requiresInvestigator(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")
	nodeID, _ := inv.ProposeHypothesis("brian", "X", "x", "", faulttree.NodeAction)

	// Without AssignAntiCross, loop start should fail
	if _, err := inv.StartHypothesisLoop(nodeID, "DB index hypothesis"); err == nil {
		t.Error("expected error: investigator not yet assigned")
	}

	// After assignment, loop should succeed
	_, _ = inv.AssignAntiCross(nodeID)
	loop, err := inv.StartHypothesisLoop(nodeID, "DB index hypothesis")
	if err != nil {
		t.Fatalf("StartHypothesisLoop: %v", err)
	}
	if loop.Driver != "rain" {
		t.Errorf("loop driver = %q, want rain (anti-cross of brian-owner)", loop.Driver)
	}
	if loop.Status != hypothesis.StatusHypothesisFormed {
		t.Errorf("status = %q, want hypothesis-formed", loop.Status)
	}
}

func TestAdvanceHypothesisLoop_progressesStages(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")
	nodeID, _ := inv.ProposeHypothesis("brian", "X", "x", "", faulttree.NodeAction)
	_, _ = inv.AssignAntiCross(nodeID)
	loop, _ := inv.StartHypothesisLoop(nodeID, "H1")

	// Prediction
	loop, err := inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusPredictionMade, "P1", "", "")
	if err != nil {
		t.Fatalf("advance to prediction: %v", err)
	}
	if loop.Prediction != "P1" || loop.Status != hypothesis.StatusPredictionMade {
		t.Errorf("loop after prediction = %+v", loop)
	}

	// Experiment + observation
	loop, err = inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusExperimentRun, "E1", "O1", "")
	if err != nil {
		t.Fatalf("advance to experiment: %v", err)
	}
	if loop.Experiment != "E1" || loop.Observation != "O1" {
		t.Errorf("loop after experiment = %+v", loop)
	}

	// Concluded → mirrors to fault-tree
	loop, err = inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusConcluded, "", "", hypothesis.ConclusionConfirmed)
	if err != nil {
		t.Fatalf("advance to concluded: %v", err)
	}
	if loop.Conclusion != hypothesis.ConclusionConfirmed {
		t.Errorf("conclusion = %q", loop.Conclusion)
	}

	// Verify mirroring: fault-tree node now has StatusConfirmed
	tree, _ := inv.GetTree()
	node := tree.GetNode(nodeID)
	if node.Status != faulttree.StatusConfirmed {
		t.Errorf("fault-tree node status = %q, want confirmed (mirrored from loop)", node.Status)
	}
}

func TestAdvanceHypothesisLoop_unsupportedStage(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")
	nodeID, _ := inv.ProposeHypothesis("brian", "X", "x", "", faulttree.NodeAction)
	_, _ = inv.AssignAntiCross(nodeID)
	loop, _ := inv.StartHypothesisLoop(nodeID, "H1")

	if _, err := inv.AdvanceHypothesisLoop(loop.ID, "bogus-stage", "", "", ""); err == nil {
		t.Error("expected error for unsupported stage")
	}
}

// ====== IsConverged + Finalize ======

func TestIsConverged_falseUntilLoopsClosed(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")
	nodeID, _ := inv.ProposeHypothesis("brian", "X", "x", "", faulttree.NodeAction)
	_, _ = inv.AssignAntiCross(nodeID)
	loop, _ := inv.StartHypothesisLoop(nodeID, "H1")

	// Open leaf + open loop → not converged
	got, err := inv.IsConverged()
	if err != nil {
		t.Fatalf("IsConverged: %v", err)
	}
	if got {
		t.Error("expected not-converged with open loop")
	}

	// Drive loop to conclusion → mirrors to fault-tree
	_, _ = inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusPredictionMade, "P", "", "")
	_, _ = inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusExperimentRun, "E", "O", "")
	_, _ = inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusConcluded, "", "", hypothesis.ConclusionConfirmed)

	// Confirmed nodes require ≥2 cite-anchors per faulttree.IsConvergence
	_ = inv.AddCiteAnchor(nodeID, "msg-12345")
	_ = inv.AddCiteAnchor(nodeID, "file:internal/foo/bar.go:42")

	got, err = inv.IsConverged()
	if err != nil {
		t.Fatalf("IsConverged after concluded: %v", err)
	}
	if !got {
		t.Error("expected converged after all loops concluded + tree reaches convergence")
	}
}

func TestFinalize_transitionsToPlan(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "claude-default", "deepseek-v4-pro")
	nodeID, _ := inv.ProposeHypothesis("brian", "X", "x", "", faulttree.NodeAction)
	_, _ = inv.AssignAntiCross(nodeID)
	loop, _ := inv.StartHypothesisLoop(nodeID, "H1")
	_, _ = inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusPredictionMade, "P", "", "")
	_, _ = inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusExperimentRun, "E", "O", "")
	_, _ = inv.AdvanceHypothesisLoop(loop.ID, hypothesis.StatusConcluded, "", "", hypothesis.ConclusionConfirmed)
	_ = inv.AddCiteAnchor(nodeID, "msg-12345")
	_ = inv.AddCiteAnchor(nodeID, "file:internal/foo/bar.go:42")

	ts, err := inv.Finalize()
	if err != nil {
		t.Fatalf("Finalize: %v", err)
	}
	if ts.CurrentPhase != mvt.StagePlan {
		t.Errorf("current phase = %q, want StagePlan", ts.CurrentPhase)
	}
}

func TestFinalize_notConvergedRejected(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionHigh, "brian", "rain", "x", "y")
	// No nodes proposed; tree has no leaves; faulttree.IsConvergence returns
	// false on empty tree (no leaves to confirm). Finalize should reject.
	if _, err := inv.Finalize(); err == nil {
		t.Error("expected error: not-converged on empty tree")
	}
}

// ====== Helpers exposed via getters ======

func TestGetTree_returnsCurrentSnapshot(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionMedium, "brian", "rain", "x", "y")
	tree, err := inv.GetTree()
	if err != nil {
		t.Fatalf("GetTree: %v", err)
	}
	if tree.TaskID != inv.TaskID {
		t.Errorf("tree TaskID = %q, want %q", tree.TaskID, inv.TaskID)
	}
}

func TestGetLoops_emptyInitially(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.OpenInvestigation(mvt.DecisionMedium, "brian", "rain", "x", "y")
	loops, err := inv.GetLoops()
	if err != nil {
		t.Fatalf("GetLoops: %v", err)
	}
	if len(loops) != 0 {
		t.Errorf("loops = %d, want 0 initially", len(loops))
	}
}
