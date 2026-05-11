package plan

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestNewSoloPlan_validInputs(t *testing.T) {
	p, err := NewSoloPlan("task-1", "brian", "Phase T-3 Plan", "Implement plan primitives")
	if err != nil {
		t.Fatalf("NewSoloPlan: %v", err)
	}
	if p.Mode != ModeSolo {
		t.Errorf("Mode = %q, want solo", p.Mode)
	}
	if p.Author != "brian" {
		t.Errorf("Author = %q", p.Author)
	}
}

func TestNewSoloPlan_validation(t *testing.T) {
	cases := []struct{ taskID, author, title string }{
		{"", "brian", "t"},
		{"t", "", "t"},
		{"t", "brian", ""},
	}
	for _, tc := range cases {
		if _, err := NewSoloPlan(tc.taskID, tc.author, tc.title, "s"); err == nil {
			t.Errorf("expected error for %+v", tc)
		}
	}
}

func TestAddStep_autoIncrementsIDs(t *testing.T) {
	p, _ := NewSoloPlan("task-1", "brian", "T", "S")
	s1 := p.AddStep("First", "do thing 1", nil)
	s2 := p.AddStep("Second", "do thing 2", []int{s1.ID})
	if s1.ID != 1 {
		t.Errorf("first step ID = %d, want 1", s1.ID)
	}
	if s2.ID != 2 {
		t.Errorf("second step ID = %d, want 2", s2.ID)
	}
	if len(p.Steps) != 2 {
		t.Errorf("steps = %d", len(p.Steps))
	}
}

func TestSaveLoad_roundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "plan.yaml")

	original, _ := NewSoloPlan("task-1", "brian", "T-3 plan", "Implement plan primitives")
	original.AddStep("Sketch primitives", "design Plan struct + step API", nil)
	original.AddCiteAnchor("internal/cl/ipav_runtime.go")

	if err := original.Save(path); err != nil {
		t.Fatalf("Save: %v", err)
	}
	loaded, err := Load(path)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if loaded.Title != "T-3 plan" {
		t.Errorf("title round-trip lost")
	}
	if len(loaded.Steps) != 1 {
		t.Errorf("steps round-trip lost")
	}
	if len(loaded.CiteAnchors) != 1 {
		t.Errorf("cite-anchors round-trip lost")
	}
}

func TestCanonicalPath_variants(t *testing.T) {
	cases := []struct {
		variant, want string
	}{
		{"", "/h/.bot-hq/projects/bot-hq/tasks/task-1/plan.yaml"},
		{"a", "/h/.bot-hq/projects/bot-hq/tasks/task-1/plan-a.yaml"},
		{"b", "/h/.bot-hq/projects/bot-hq/tasks/task-1/plan-b.yaml"},
	}
	for _, tc := range cases {
		got := CanonicalPath("/h", "bot-hq", "task-1", tc.variant)
		if got != tc.want {
			t.Errorf("variant %q: got %q, want %q", tc.variant, got, tc.want)
		}
	}
}

// ====== MergeBilateral tests ======

func TestMergeBilateral_alignmentClassification(t *testing.T) {
	a, _ := NewSoloPlan("t", "brian", "T", "S")
	a.AddStep("Step 1: setup", "do X", nil)

	b, _ := NewSoloPlan("t", "rain", "T", "S")
	b.AddStep("Step 1: setup", "do X", nil) // identical

	report, err := MergeBilateral(a, b)
	if err != nil {
		t.Fatalf("MergeBilateral: %v", err)
	}
	if report.GenuineForks != 0 {
		t.Errorf("GenuineForks = %d, want 0 (identical steps should align)", report.GenuineForks)
	}
	if len(report.Divergences) != 1 {
		t.Errorf("divergences = %d, want 1", len(report.Divergences))
	}
	if report.Divergences[0].Class != DivAlignment {
		t.Errorf("class = %q, want alignment", report.Divergences[0].Class)
	}
}

func TestMergeBilateral_convergenceNeededClassification(t *testing.T) {
	a, _ := NewSoloPlan("t", "brian", "T", "S")
	a.AddStep("Step 1: setup", "do X with detail Y and Z", nil)

	b, _ := NewSoloPlan("t", "rain", "T", "S")
	b.AddStep("Step 1: setup", "do X with detail Y", nil) // substring of A

	report, err := MergeBilateral(a, b)
	if err != nil {
		t.Fatalf("MergeBilateral: %v", err)
	}
	if report.GenuineForks != 0 {
		t.Errorf("substring overlap should not be genuine fork; got %d", report.GenuineForks)
	}
	if report.Divergences[0].Class != DivConvergenceNeeded {
		t.Errorf("class = %q, want convergence-needed", report.Divergences[0].Class)
	}
}

func TestMergeBilateral_genuineForkClassification(t *testing.T) {
	a, _ := NewSoloPlan("t", "brian", "T", "S")
	a.AddStep("Step 1: deploy strategy", "use blue/green deployment", nil)

	b, _ := NewSoloPlan("t", "rain", "T", "S")
	b.AddStep("Step 1: deploy strategy", "use canary deployment", nil) // material difference

	report, err := MergeBilateral(a, b)
	if err != nil {
		t.Fatalf("MergeBilateral: %v", err)
	}
	if report.GenuineForks < 1 {
		t.Errorf("material differences should produce genuine forks; got %d", report.GenuineForks)
	}
	if report.Divergences[0].Class != DivGenuineFork {
		t.Errorf("class = %q, want genuine-fork", report.Divergences[0].Class)
	}
}

func TestMergeBilateral_aOnlyAndBOnly(t *testing.T) {
	a, _ := NewSoloPlan("t", "brian", "T", "S")
	a.AddStep("Step A-only", "exists in A", nil)

	b, _ := NewSoloPlan("t", "rain", "T", "S")
	b.AddStep("Step B-only", "exists in B", nil)

	report, err := MergeBilateral(a, b)
	if err != nil {
		t.Fatalf("MergeBilateral: %v", err)
	}
	if report.GenuineForks != 2 {
		t.Errorf("A-only + B-only should produce 2 genuine forks; got %d", report.GenuineForks)
	}
	// Merged should preserve both as A-only / B-only
	titles := []string{}
	for _, s := range report.Merged.Steps {
		titles = append(titles, s.Title)
	}
	joined := strings.Join(titles, "|")
	if !strings.Contains(joined, "[A-only]") {
		t.Errorf("merged should preserve A-only marker; got: %v", titles)
	}
	if !strings.Contains(joined, "[B-only]") {
		t.Errorf("merged should preserve B-only marker; got: %v", titles)
	}
}

func TestMergeBilateral_taskIDMismatch_errors(t *testing.T) {
	a, _ := NewSoloPlan("task-A", "brian", "T", "S")
	b, _ := NewSoloPlan("task-B", "rain", "T", "S")
	if _, err := MergeBilateral(a, b); err == nil {
		t.Error("expected error for TaskID mismatch")
	}
}

func TestMergeBilateral_mergedHasBilateralModeAndAuthor(t *testing.T) {
	a, _ := NewSoloPlan("t", "brian", "T", "S")
	a.AddStep("S", "d", nil)
	b, _ := NewSoloPlan("t", "rain", "T", "S")
	b.AddStep("S", "d", nil)

	report, _ := MergeBilateral(a, b)
	if report.Merged.Mode != ModeBilateral {
		t.Errorf("merged mode = %q, want bilateral", report.Merged.Mode)
	}
	if report.Merged.Author != "bilateral-merged" {
		t.Errorf("merged author = %q, want bilateral-merged", report.Merged.Author)
	}
}

func TestMergeBilateral_dedupesCiteAnchors(t *testing.T) {
	a, _ := NewSoloPlan("t", "brian", "T", "S")
	a.AddCiteAnchor("anchor-1")
	a.AddCiteAnchor("anchor-2")
	a.AddStep("S", "d", nil)

	b, _ := NewSoloPlan("t", "rain", "T", "S")
	b.AddCiteAnchor("anchor-2") // dup
	b.AddCiteAnchor("anchor-3")
	b.AddStep("S", "d", nil)

	report, _ := MergeBilateral(a, b)
	if len(report.Merged.CiteAnchors) != 3 {
		t.Errorf("merged anchors = %d, want 3 (dedup)", len(report.Merged.CiteAnchors))
	}
}

// ====== RunPlanVerify tests ======

func TestRunPlanVerify_approveValidPlan(t *testing.T) {
	p, _ := NewSoloPlan("t", "brian", "Title", "Summary")
	p.AddStep("S", "d", nil)
	p.AddCiteAnchor("anchor-1")

	verdict, _, _ := RunPlanVerify(p, nil)
	if verdict != VerifyApproved {
		t.Errorf("verdict = %q, want approved", verdict)
	}
	if p.VerifyVerdict != VerifyApproved {
		t.Error("plan VerifyVerdict not set")
	}
}

func TestRunPlanVerify_rejectsEmptyTitle(t *testing.T) {
	p := &Plan{TaskID: "t", Author: "brian", Mode: ModeSolo, Title: "", Summary: "s"}
	p.AddStep("S", "d", nil)
	p.AddCiteAnchor("a")
	verdict, _, _ := RunPlanVerify(p, nil)
	if verdict != VerifyRejected {
		t.Errorf("expected REJECTED for empty title; got %q", verdict)
	}
}

func TestRunPlanVerify_rejectsZeroSteps(t *testing.T) {
	p, _ := NewSoloPlan("t", "brian", "T", "S")
	p.AddCiteAnchor("a")
	verdict, _, _ := RunPlanVerify(p, nil)
	if verdict != VerifyRejected {
		t.Errorf("expected REJECTED for zero steps; got %q", verdict)
	}
}

func TestRunPlanVerify_rejectsMissingCiteAnchors(t *testing.T) {
	p, _ := NewSoloPlan("t", "brian", "T", "S")
	p.AddStep("S", "d", nil)
	// no cite-anchors
	verdict, _, _ := RunPlanVerify(p, nil)
	if verdict != VerifyRejected {
		t.Errorf("expected REJECTED for missing cite-anchors; got %q", verdict)
	}
}

func TestRunPlanVerify_rejectsInvalidDependsOn(t *testing.T) {
	p, _ := NewSoloPlan("t", "brian", "T", "S")
	p.AddStep("S1", "d", nil)
	p.AddStep("S2", "d", []int{99}) // invalid dep-id
	p.AddCiteAnchor("a")
	verdict, _, _ := RunPlanVerify(p, nil)
	if verdict != VerifyRejected {
		t.Errorf("expected REJECTED for invalid dependsOn; got %q", verdict)
	}
}

func TestRunPlanVerify_rejectsBilateralWithGenuineForks(t *testing.T) {
	a, _ := NewSoloPlan("t", "brian", "T", "S")
	a.AddStep("Different A", "d", nil)
	a.AddCiteAnchor("a")
	b, _ := NewSoloPlan("t", "rain", "T", "S")
	b.AddStep("Different B", "d", nil)
	b.AddCiteAnchor("a")

	report, _ := MergeBilateral(a, b)
	if report.GenuineForks == 0 {
		t.Fatal("expected genuine forks for diff steps")
	}

	verdict, _, reason := RunPlanVerify(report.Merged, report)
	if verdict != VerifyRejected {
		t.Errorf("bilateral with genuine-forks should REJECT; got %q (reason: %s)", verdict, reason)
	}
}

func TestRunPlanVerify_acceptsBilateralWithoutForks(t *testing.T) {
	a, _ := NewSoloPlan("t", "brian", "T", "S")
	a.AddStep("Same step", "same", nil)
	a.AddCiteAnchor("anc")

	b, _ := NewSoloPlan("t", "rain", "T", "S")
	b.AddStep("Same step", "same", nil)
	b.AddCiteAnchor("anc")

	report, _ := MergeBilateral(a, b)
	verdict, _, _ := RunPlanVerify(report.Merged, report)
	if verdict != VerifyApproved {
		t.Errorf("bilateral aligned plan should APPROVE; got %q", verdict)
	}
}

func TestRunPlanVerify_nilPlan(t *testing.T) {
	verdict, _, _ := RunPlanVerify(nil, nil)
	if verdict != VerifyRejected {
		t.Errorf("nil plan should REJECT; got %q", verdict)
	}
}
