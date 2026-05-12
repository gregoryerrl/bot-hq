package cl

import (
	"errors"
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

func TestIPAVState_saveLoadRoundTrip(t *testing.T) {
	cl := newTestCL(t)

	original := mvt.NewTaskState("task-abc", "", mvt.DecisionHigh)
	original.PhaseArtifacts.InvestigationDoc = "/sim/inv.md"

	if err := cl.SaveIPAVState("bot-hq", original); err != nil {
		t.Fatalf("SaveIPAVState: %v", err)
	}

	loaded, err := cl.IPAVState("bot-hq", "task-abc")
	if err != nil {
		t.Fatalf("IPAVState load: %v", err)
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

func TestIPAVState_missing_returnsErrNotFound(t *testing.T) {
	cl := newTestCL(t)
	_, err := cl.IPAVState("bot-hq", "nonexistent-task")
	if !errors.Is(err, ErrNotFound) {
		t.Errorf("err = %v, want wrap of ErrNotFound", err)
	}
}

func TestIPAVState_emptyProjectOrTask_errors(t *testing.T) {
	cl := newTestCL(t)
	_, err := cl.IPAVState("", "task-1")
	if err == nil {
		t.Error("expected error for empty project")
	}
	_, err = cl.IPAVState("bot-hq", "")
	if err == nil {
		t.Error("expected error for empty taskID")
	}
}

func TestIPAVStatePath_canonical(t *testing.T) {
	cl := newTestCL(t)
	got := cl.IPAVStatePath("bot-hq", "task-xyz")
	want := filepath.Join(cl.Root(), "projects", "bot-hq", "tasks", "task-xyz", "ipav-state.yaml")
	if got != want {
		t.Errorf("IPAVStatePath = %q, want %q", got, want)
	}
}

func TestListIPAVStates_multipleTasks(t *testing.T) {
	cl := newTestCL(t)

	for _, id := range []string{"task-1", "task-2", "task-3"} {
		ts := mvt.NewTaskState(id, "", mvt.DecisionMedium)
		if err := cl.SaveIPAVState("bot-hq", ts); err != nil {
			t.Fatalf("SaveIPAVState %s: %v", id, err)
		}
	}

	states, err := cl.ListIPAVStates("bot-hq")
	if err != nil {
		t.Fatalf("ListIPAVStates: %v", err)
	}
	if len(states) != 3 {
		t.Errorf("count = %d, want 3", len(states))
	}
}

func TestListIPAVStates_emptyProjectReturnsNil(t *testing.T) {
	cl := newTestCL(t)
	states, err := cl.ListIPAVStates("nonexistent-project")
	if err != nil {
		t.Fatalf("ListIPAVStates: %v", err)
	}
	if len(states) != 0 {
		t.Errorf("count = %d, want 0", len(states))
	}
}

func TestListProjectsWithIPAVTasks(t *testing.T) {
	cl := newTestCL(t)

	// Add IPAV task to bot-hq + 988 projects
	for _, project := range []string{"bot-hq", "988"} {
		ts := mvt.NewTaskState("task-init", "", mvt.DecisionLow)
		if err := cl.SaveIPAVState(project, ts); err != nil {
			t.Fatalf("SaveIPAVState %s: %v", project, err)
		}
	}

	projects, err := cl.ListProjectsWithIPAVTasks()
	if err != nil {
		t.Fatalf("ListProjectsWithIPAVTasks: %v", err)
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
