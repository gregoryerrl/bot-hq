package cl

import (
	"errors"
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

func TestIPIVState_saveLoadRoundTrip(t *testing.T) {
	cl := newTestCL(t)

	original := mvt.NewTaskState("task-abc", mvt.DecisionHigh)
	original.PhaseArtifacts.InvestigationDoc = "/sim/inv.md"

	if err := cl.SaveIPIVState("bot-hq", original); err != nil {
		t.Fatalf("SaveIPIVState: %v", err)
	}

	loaded, err := cl.IPIVState("bot-hq", "task-abc")
	if err != nil {
		t.Fatalf("IPIVState load: %v", err)
	}

	if loaded.TaskID != "task-abc" {
		t.Errorf("TaskID round-trip: got %q, want task-abc", loaded.TaskID)
	}
	if loaded.DecisionClass != mvt.DecisionHigh {
		t.Errorf("DecisionClass round-trip: got %q", loaded.DecisionClass)
	}
	if loaded.PhaseArtifacts.InvestigationDoc != "/sim/inv.md" {
		t.Errorf("InvestigationDoc round-trip lost")
	}
}

func TestIPIVState_missing_returnsErrNotFound(t *testing.T) {
	cl := newTestCL(t)
	_, err := cl.IPIVState("bot-hq", "nonexistent-task")
	if !errors.Is(err, ErrNotFound) {
		t.Errorf("err = %v, want wrap of ErrNotFound", err)
	}
}

func TestIPIVState_emptyProjectOrTask_errors(t *testing.T) {
	cl := newTestCL(t)
	_, err := cl.IPIVState("", "task-1")
	if err == nil {
		t.Error("expected error for empty project")
	}
	_, err = cl.IPIVState("bot-hq", "")
	if err == nil {
		t.Error("expected error for empty taskID")
	}
}

func TestIPIVStatePath_canonical(t *testing.T) {
	cl := newTestCL(t)
	got := cl.IPIVStatePath("bot-hq", "task-xyz")
	want := filepath.Join(cl.Root(), "projects", "bot-hq", "tasks", "task-xyz", "ipiv-state.yaml")
	if got != want {
		t.Errorf("IPIVStatePath = %q, want %q", got, want)
	}
}

func TestListIPIVStates_multipleTasks(t *testing.T) {
	cl := newTestCL(t)

	for _, id := range []string{"task-1", "task-2", "task-3"} {
		ts := mvt.NewTaskState(id, mvt.DecisionMedium)
		if err := cl.SaveIPIVState("bot-hq", ts); err != nil {
			t.Fatalf("SaveIPIVState %s: %v", id, err)
		}
	}

	states, err := cl.ListIPIVStates("bot-hq")
	if err != nil {
		t.Fatalf("ListIPIVStates: %v", err)
	}
	if len(states) != 3 {
		t.Errorf("count = %d, want 3", len(states))
	}
}

func TestListIPIVStates_emptyProjectReturnsNil(t *testing.T) {
	cl := newTestCL(t)
	states, err := cl.ListIPIVStates("nonexistent-project")
	if err != nil {
		t.Fatalf("ListIPIVStates: %v", err)
	}
	if len(states) != 0 {
		t.Errorf("count = %d, want 0", len(states))
	}
}

func TestListProjectsWithIPIVTasks(t *testing.T) {
	cl := newTestCL(t)

	// Add IPIV task to bot-hq + 988 projects
	for _, project := range []string{"bot-hq", "988"} {
		ts := mvt.NewTaskState("task-init", mvt.DecisionLow)
		if err := cl.SaveIPIVState(project, ts); err != nil {
			t.Fatalf("SaveIPIVState %s: %v", project, err)
		}
	}

	projects, err := cl.ListProjectsWithIPIVTasks()
	if err != nil {
		t.Fatalf("ListProjectsWithIPIVTasks: %v", err)
	}

	seen := map[string]bool{}
	for _, p := range projects {
		seen[p] = true
	}
	if !seen["bot-hq"] {
		t.Error("bot-hq missing from project list")
	}
	if !seen["988"] {
		t.Error("988 missing from project list")
	}
}
