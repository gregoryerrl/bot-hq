package verify

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestNewReport_validInputs(t *testing.T) {
	r, err := NewReport("task-1", "rain", ModePlanVerify, "deepseek-v4-pro")
	if err != nil {
		t.Fatalf("NewReport: %v", err)
	}
	if r.Mode != ModePlanVerify {
		t.Errorf("Mode = %q", r.Mode)
	}
	if r.Verifier != "rain" {
		t.Errorf("Verifier = %q", r.Verifier)
	}
	if r.Result != ResultPending {
		t.Errorf("initial Result = %q, want pending", r.Result)
	}
}

func TestNewReport_validation(t *testing.T) {
	cases := []struct {
		taskID, verifier string
		mode             Mode
	}{
		{"", "rain", ModePlanVerify},
		{"t", "", ModePlanVerify},
		{"t", "rain", "garbage"},
	}
	for _, tc := range cases {
		if _, err := NewReport(tc.taskID, tc.verifier, tc.mode, ""); err == nil {
			t.Errorf("expected error for %+v", tc)
		}
	}
}

func TestAddFinding_appends(t *testing.T) {
	r, _ := NewReport("t", "rain", ModePlanVerify, "")
	r.AddFinding("warn", "test-failure", "FailingCase", "details", "msg 17094")
	r.AddFinding("block", "security", "SecHole", "details", "")
	if len(r.Findings) != 2 {
		t.Errorf("findings = %d, want 2", len(r.Findings))
	}
}

func TestConclude_validResults(t *testing.T) {
	for _, res := range []Result{ResultPass, ResultFail, ResultEscalate} {
		r, _ := NewReport("t", "rain", ModePlanVerify, "")
		if err := r.Conclude(res, "reason"); err != nil {
			t.Errorf("Conclude %s: %v", res, err)
		}
		if r.Result != res {
			t.Errorf("result = %q, want %q", r.Result, res)
		}
	}
}

func TestConclude_invalidResult(t *testing.T) {
	r, _ := NewReport("t", "rain", ModePlanVerify, "")
	if err := r.Conclude("garbage", "x"); err == nil {
		t.Error("expected error for invalid result")
	}
}

func TestHasBlockingFindings(t *testing.T) {
	r, _ := NewReport("t", "rain", ModePlanVerify, "")
	if r.HasBlockingFindings() {
		t.Error("empty report should not have blocking findings")
	}
	r.AddFinding("warn", "x", "x", "x", "")
	if r.HasBlockingFindings() {
		t.Error("warn-only finding should not block")
	}
	r.AddFinding("block", "security", "x", "x", "")
	if !r.HasBlockingFindings() {
		t.Error("block-class finding should report true")
	}
}

func TestSaveLoad_roundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "report.yaml")

	original, _ := NewReport("task-1", "rain", ModeImplementVerify, "deepseek-v4-pro")
	original.AddFinding("block", "regression", "test fail", "x.go test failed", "msg 17162")
	original.Conclude(ResultFail, "regression")

	if err := original.Save(path); err != nil {
		t.Fatalf("Save: %v", err)
	}
	loaded, err := Load(path)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if loaded.Result != ResultFail {
		t.Errorf("result round-trip lost")
	}
	if len(loaded.Findings) != 1 {
		t.Errorf("findings round-trip lost")
	}
}

func TestCanonicalPath(t *testing.T) {
	got := CanonicalPath("/h", "bot-hq", "task-1", ModePlanVerify, 1)
	want := "/h/.bot-hq/projects/bot-hq/tasks/task-1/verify-reports/plan-verify-1.yaml"
	if got != want {
		t.Errorf("CanonicalPath = %q, want %q", got, want)
	}
}

// ====== Prompt-template registry tests ======

func TestLookupPromptTemplate_planVerifyDefault(t *testing.T) {
	tmpl, err := LookupPromptTemplate(ModePlanVerify, "")
	if err != nil {
		t.Fatalf("LookupPromptTemplate: %v", err)
	}
	if !strings.Contains(tmpl, "Plan-Verify-mode") {
		t.Errorf("template should mention Plan-Verify-mode")
	}
	if !strings.Contains(tmpl, "BLOCK-AUTHORITY") {
		t.Errorf("template should mention BLOCK-AUTHORITY")
	}
}

func TestLookupPromptTemplate_implementVerifyDefault(t *testing.T) {
	tmpl, err := LookupPromptTemplate(ModeImplementVerify, "")
	if err != nil {
		t.Fatalf("LookupPromptTemplate: %v", err)
	}
	if !strings.Contains(tmpl, "Implement-Verify-mode") {
		t.Errorf("template should mention Implement-Verify-mode")
	}
}

func TestLookupPromptTemplate_modelFallthroughToDefault(t *testing.T) {
	// Model-specific template doesn't exist; should fall through to default.
	tmpl, err := LookupPromptTemplate(ModePlanVerify, "deepseek-v4-pro")
	if err != nil {
		t.Fatalf("LookupPromptTemplate: %v", err)
	}
	if tmpl == "" {
		t.Error("fall-through to default failed")
	}
}

func TestAllTemplates_returnsRegistry(t *testing.T) {
	all := AllTemplates()
	if len(all) < 2 {
		t.Errorf("expected >=2 templates; got %d", len(all))
	}
}

// ====== VerifyResultCache tests ======

func TestVerifyResultCache_putGet(t *testing.T) {
	c := NewVerifyResultCache()
	r, _ := NewReport("t", "rain", ModePlanVerify, "")
	c.Put("key-1", r)

	got, ok := c.Get("key-1")
	if !ok {
		t.Error("expected hit")
	}
	if got != r {
		t.Error("retrieved report differs")
	}
}

func TestVerifyResultCache_miss(t *testing.T) {
	c := NewVerifyResultCache()
	_, ok := c.Get("nonexistent")
	if ok {
		t.Error("expected miss for empty cache")
	}
}

func TestVerifyResultCache_size(t *testing.T) {
	c := NewVerifyResultCache()
	if c.Size() != 0 {
		t.Errorf("initial size = %d, want 0", c.Size())
	}
	r, _ := NewReport("t", "rain", ModePlanVerify, "")
	c.Put("a", r)
	c.Put("b", r)
	if c.Size() != 2 {
		t.Errorf("size after 2 puts = %d, want 2", c.Size())
	}
}
