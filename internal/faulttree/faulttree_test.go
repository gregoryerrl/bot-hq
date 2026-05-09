package faulttree

import (
	"errors"
	"path/filepath"
	"strings"
	"testing"
)

func TestNewTree_initEmpty(t *testing.T) {
	tree := NewTree("task-1")
	if tree.TaskID != "task-1" {
		t.Errorf("TaskID = %q", tree.TaskID)
	}
	if len(tree.Nodes) != 0 {
		t.Errorf("expected 0 nodes")
	}
	if tree.RootID != "" {
		t.Errorf("RootID should be empty initially")
	}
}

func TestAddNode_rootAssignment(t *testing.T) {
	tree := NewTree("task-1")
	id, err := tree.AddNode(&Node{Type: NodeAction, Title: "Root hypothesis", Owner: "brian"})
	if err != nil {
		t.Fatalf("AddNode: %v", err)
	}
	if tree.RootID != id {
		t.Errorf("first added node should become root; got RootID=%q want %q", tree.RootID, id)
	}
}

func TestAddNode_validation(t *testing.T) {
	tree := NewTree("task-1")
	cases := []*Node{
		nil,
		{Type: "", Title: "x"},          // missing type
		{Type: NodeAction, Title: ""},   // missing title
		{Type: "invalid", Title: "x"},   // invalid type
	}
	for _, n := range cases {
		if _, err := tree.AddNode(n); err == nil {
			t.Errorf("expected error for %+v", n)
		}
	}
}

func TestAddNode_childOfParent(t *testing.T) {
	tree := NewTree("task-1")
	rootID, _ := tree.AddNode(&Node{Type: NodeAction, Title: "Root", Owner: "brian"})

	childID, err := tree.AddNode(&Node{Type: NodeCondition, Title: "Child", Owner: "brian", ParentID: rootID})
	if err != nil {
		t.Fatalf("AddNode child: %v", err)
	}

	root := tree.GetNode(rootID)
	if len(root.ChildrenIDs) != 1 || root.ChildrenIDs[0] != childID {
		t.Errorf("parent ChildrenIDs not updated; got %v", root.ChildrenIDs)
	}
}

func TestAddNode_unknownParent_errors(t *testing.T) {
	tree := NewTree("task-1")
	_, err := tree.AddNode(&Node{Type: NodeAction, Title: "x", Owner: "brian", ParentID: "no-such-id"})
	if err == nil {
		t.Error("expected error for unknown parent")
	}
}

func TestSetStatus(t *testing.T) {
	tree := NewTree("task-1")
	id, _ := tree.AddNode(&Node{Type: NodeAction, Title: "x", Owner: "brian"})

	if err := tree.SetStatus(id, StatusConfirmed); err != nil {
		t.Fatalf("SetStatus: %v", err)
	}
	if tree.GetNode(id).Status != StatusConfirmed {
		t.Error("status not updated")
	}
}

func TestSetStatus_unknownNode_returnsErrNodeNotFound(t *testing.T) {
	tree := NewTree("task-1")
	err := tree.SetStatus("nope", StatusConfirmed)
	if !errors.Is(err, ErrNodeNotFound) {
		t.Errorf("err = %v, want ErrNodeNotFound", err)
	}
}

func TestAssignInvestigator_blocksOwnerSelfAssignment(t *testing.T) {
	tree := NewTree("task-1")
	id, _ := tree.AddNode(&Node{Type: NodeAction, Title: "x", Owner: "brian"})

	err := tree.AssignInvestigator(id, "brian") // == owner
	if err == nil {
		t.Error("R44 anti-cross should reject investigator == owner")
	}
	if !strings.Contains(err.Error(), "anti-cross") {
		t.Errorf("error should mention anti-cross; got: %v", err)
	}
}

func TestAssignInvestigator_allowsPeer(t *testing.T) {
	tree := NewTree("task-1")
	id, _ := tree.AddNode(&Node{Type: NodeAction, Title: "x", Owner: "brian"})

	if err := tree.AssignInvestigator(id, "rain"); err != nil {
		t.Errorf("peer assignment should succeed: %v", err)
	}
	if tree.GetNode(id).Investigator != "rain" {
		t.Error("investigator not set")
	}
}

func TestAddCiteAnchor_appends(t *testing.T) {
	tree := NewTree("task-1")
	id, _ := tree.AddNode(&Node{Type: NodeAction, Title: "x", Owner: "brian"})

	if err := tree.AddCiteAnchor(id, "msg 17094"); err != nil {
		t.Fatalf("AddCiteAnchor: %v", err)
	}
	if err := tree.AddCiteAnchor(id, "msg 17128"); err != nil {
		t.Fatalf("AddCiteAnchor 2: %v", err)
	}
	n := tree.GetNode(id)
	if len(n.CiteAnchors) != 2 {
		t.Errorf("anchors = %d, want 2", len(n.CiteAnchors))
	}
}

func TestLeafNodes_returnsTerminals(t *testing.T) {
	tree := NewTree("task-1")
	rootID, _ := tree.AddNode(&Node{Type: NodeAction, Title: "Root", Owner: "brian"})
	leaf1, _ := tree.AddNode(&Node{Type: NodeCondition, Title: "L1", Owner: "brian", ParentID: rootID})
	leaf2, _ := tree.AddNode(&Node{Type: NodeCondition, Title: "L2", Owner: "brian", ParentID: rootID})

	leaves := tree.LeafNodes()
	if len(leaves) != 2 {
		t.Errorf("leaves = %d, want 2", len(leaves))
	}
	leafIDs := map[string]bool{leaves[0].ID: true, leaves[1].ID: true}
	if !leafIDs[leaf1] || !leafIDs[leaf2] {
		t.Error("leaf set incomplete")
	}
}

func TestOpenLeafNodes_filtersClosed(t *testing.T) {
	tree := NewTree("task-1")
	rootID, _ := tree.AddNode(&Node{Type: NodeAction, Title: "Root", Owner: "brian"})
	leaf1, _ := tree.AddNode(&Node{Type: NodeCondition, Title: "L1", Owner: "brian", ParentID: rootID})
	leaf2, _ := tree.AddNode(&Node{Type: NodeCondition, Title: "L2", Owner: "brian", ParentID: rootID})

	tree.SetStatus(leaf1, StatusConfirmed)
	tree.AddCiteAnchor(leaf1, "msg 1")
	tree.AddCiteAnchor(leaf1, "msg 2")
	// leaf2 stays open

	open := tree.OpenLeafNodes()
	if len(open) != 1 || open[0].ID != leaf2 {
		t.Errorf("expected only leaf2 open; got %v", open)
	}
}

func TestIsConvergence_falseWhenLeavesOpen(t *testing.T) {
	tree := NewTree("task-1")
	rootID, _ := tree.AddNode(&Node{Type: NodeAction, Title: "Root", Owner: "brian"})
	tree.AddNode(&Node{Type: NodeCondition, Title: "L", Owner: "brian", ParentID: rootID})
	if tree.IsConvergence() {
		t.Error("convergence should be false when leaves still open")
	}
}

func TestIsConvergence_falseWhenConfirmedHasInsufficientCites(t *testing.T) {
	tree := NewTree("task-1")
	id, _ := tree.AddNode(&Node{Type: NodeAction, Title: "x", Owner: "brian"})
	tree.SetStatus(id, StatusConfirmed)
	tree.AddCiteAnchor(id, "msg 1") // only 1 cite

	if tree.IsConvergence() {
		t.Error("convergence requires confirmed nodes have ≥2 cite-anchors")
	}
}

func TestIsConvergence_trueWhenAllResolvedAndCited(t *testing.T) {
	tree := NewTree("task-1")
	id, _ := tree.AddNode(&Node{Type: NodeAction, Title: "x", Owner: "brian"})
	tree.SetStatus(id, StatusConfirmed)
	tree.AddCiteAnchor(id, "msg 1")
	tree.AddCiteAnchor(id, "msg 2")

	if !tree.IsConvergence() {
		t.Error("expected convergence")
	}
}

func TestSaveLoad_roundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "fault-tree.json")

	original := NewTree("task-1")
	id, _ := original.AddNode(&Node{Type: NodeAction, Title: "Root", Owner: "brian"})
	original.AddCiteAnchor(id, "msg 17094")

	if err := original.Save(path); err != nil {
		t.Fatalf("Save: %v", err)
	}

	loaded, err := Load(path)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if loaded.TaskID != "task-1" {
		t.Errorf("TaskID round-trip lost")
	}
	if len(loaded.Nodes) != 1 {
		t.Errorf("Nodes round-trip lost; count=%d", len(loaded.Nodes))
	}
	if loaded.GetNode(id).CiteAnchors[0] != "msg 17094" {
		t.Errorf("cite-anchor round-trip lost")
	}
}

func TestLoad_missing_returnsErrTreeNotFound(t *testing.T) {
	_, err := Load("/nonexistent/fault-tree.json")
	if !errors.Is(err, ErrTreeNotFound) {
		t.Errorf("err = %v, want ErrTreeNotFound", err)
	}
}

func TestCanonicalPath(t *testing.T) {
	got := CanonicalPath("/home/x", "bot-hq", "task-abc")
	want := "/home/x/.bot-hq/projects/bot-hq/tasks/task-abc/fault-tree.json"
	if got != want {
		t.Errorf("CanonicalPath = %q, want %q", got, want)
	}
}
