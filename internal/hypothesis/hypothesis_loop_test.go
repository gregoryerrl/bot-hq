package hypothesis

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestNewLoop_validInputs(t *testing.T) {
	l, err := NewLoop("task-1", "node-1", "rain", "Hypothesis: cite-drift caused by manual-discipline.")
	if err != nil {
		t.Fatalf("NewLoop: %v", err)
	}
	if l.Status != StatusHypothesisFormed {
		t.Errorf("status = %q, want %q", l.Status, StatusHypothesisFormed)
	}
	if l.Driver != "rain" {
		t.Errorf("driver = %q", l.Driver)
	}
}

func TestNewLoop_validation(t *testing.T) {
	if _, err := NewLoop("t", "n", "", "h"); err == nil {
		t.Error("expected error for empty driver")
	}
	if _, err := NewLoop("t", "n", "rain", ""); err == nil {
		t.Error("expected error for empty hypothesis")
	}
}

func TestZellerLoop_endToEnd(t *testing.T) {
	l, err := NewLoop("task-1", "node-1", "rain", "Cite-drift recurs without mechanical-validation.")
	if err != nil {
		t.Fatalf("NewLoop: %v", err)
	}

	if err := l.SetPrediction("If we deploy R49 mechanical-audit, drift rate drops by ≥50%."); err != nil {
		t.Fatalf("SetPrediction: %v", err)
	}
	if l.Status != StatusPredictionMade {
		t.Errorf("status after prediction = %q", l.Status)
	}

	if err := l.SetExperimentObservation(
		"Run cite-validate against phase-t.md v5 with hub.DB.MessageExists wired.",
		"166 anchors / 126 valid / 40 invalid (file-paths) / 113 msg-id checks now mechanically validated.",
	); err != nil {
		t.Fatalf("SetExperimentObservation: %v", err)
	}
	if l.Status != StatusExperimentRun {
		t.Errorf("status after experiment = %q", l.Status)
	}

	if err := l.Conclude(ConclusionConfirmed); err != nil {
		t.Fatalf("Conclude: %v", err)
	}
	if !l.IsComplete() {
		t.Errorf("loop should be complete")
	}
	if l.Conclusion != ConclusionConfirmed {
		t.Errorf("conclusion = %q", l.Conclusion)
	}
}

func TestSetPrediction_invalidTransition(t *testing.T) {
	l, _ := NewLoop("t", "n", "rain", "h")
	l.Status = StatusExperimentRun
	if err := l.SetPrediction("p"); err == nil {
		t.Error("should error on invalid status transition")
	}
}

func TestSetExperimentObservation_validation(t *testing.T) {
	l, _ := NewLoop("t", "n", "rain", "h")
	l.SetPrediction("p")

	if err := l.SetExperimentObservation("", "obs"); err == nil {
		t.Error("expected error for empty experiment")
	}
	if err := l.SetExperimentObservation("exp", ""); err == nil {
		t.Error("expected error for empty observation")
	}
}

func TestConclude_invalidVerdict(t *testing.T) {
	l, _ := NewLoop("t", "n", "rain", "h")
	l.SetPrediction("p")
	l.SetExperimentObservation("e", "o")

	if err := l.Conclude("garbage"); err == nil {
		t.Error("expected error for invalid verdict")
	}
}

func TestAddCiteAnchor(t *testing.T) {
	l, _ := NewLoop("t", "n", "rain", "h")
	l.AddCiteAnchor("msg 17094")
	l.AddCiteAnchor("docs/research/perf-baseline-pre-phase-t.md")
	if len(l.CiteAnchors) != 2 {
		t.Errorf("anchors = %d, want 2", len(l.CiteAnchors))
	}
}

func TestSaveLoad_roundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "loop.json")

	original, _ := NewLoop("task-1", "node-1", "rain", "h")
	original.AddCiteAnchor("msg 17094")
	if err := original.Save(path); err != nil {
		t.Fatalf("Save: %v", err)
	}

	loaded, err := Load(path)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if loaded.Hypothesis != "h" {
		t.Errorf("hypothesis round-trip lost")
	}
	if len(loaded.CiteAnchors) != 1 {
		t.Errorf("anchors round-trip count = %d", len(loaded.CiteAnchors))
	}
}

func TestCanonicalPath(t *testing.T) {
	got := CanonicalPath("/h", "bot-hq", "task-abc", "loop-xyz")
	want := "/h/.bot-hq/projects/bot-hq/tasks/task-abc/hypothesis-loops/loop-xyz.json"
	if got != want {
		t.Errorf("CanonicalPath = %q, want %q", got, want)
	}
}

func TestAssignDriver_picksPeerNotOwner(t *testing.T) {
	driver, err := AssignDriver("brian", []string{"brian", "rain"})
	if err != nil {
		t.Fatalf("AssignDriver: %v", err)
	}
	if driver != "rain" {
		t.Errorf("driver = %q, want rain (peer of brian)", driver)
	}
}

func TestAssignDriver_emptyOwner_errors(t *testing.T) {
	_, err := AssignDriver("", []string{"brian", "rain"})
	if err == nil {
		t.Error("expected error for empty owner")
	}
}

func TestAssignDriver_emptyCandidates_errors(t *testing.T) {
	_, err := AssignDriver("brian", nil)
	if err == nil {
		t.Error("expected error for empty candidates")
	}
}

func TestAssignDriver_noEligibleCandidate(t *testing.T) {
	_, err := AssignDriver("brian", []string{"brian"})
	if err == nil {
		t.Error("R44 anti-cross should reject when only owner is in pool")
	}
	if !strings.Contains(err.Error(), "anti-cross") {
		t.Errorf("error should mention anti-cross; got: %v", err)
	}
}

func TestAssignDriver_skipsEmptyCandidate(t *testing.T) {
	driver, err := AssignDriver("brian", []string{"", "rain"})
	if err != nil {
		t.Fatalf("AssignDriver: %v", err)
	}
	if driver != "rain" {
		t.Errorf("expected rain, got %q", driver)
	}
}
