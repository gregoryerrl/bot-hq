package projects

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// writeFile is a t.TempDir-friendly write helper.
func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

// TestAuditCanonical_AllCanonical: directory with only nested-form
// YAMLs reports all CANONICAL, zero drift.
func TestAuditCanonical_AllCanonical(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "a.yaml"), `
project_name: a
remote_url: git@x:y.git
branch:
  pattern: main
gates:
  push:
    requiresApproval: true
`)
	writeFile(t, filepath.Join(dir, "b.yaml"), `project_name: b
commit:
  style: imperative-mood
`)

	results, err := AuditCanonical(dir)
	if err != nil {
		t.Fatalf("AuditCanonical: %v", err)
	}
	if len(results) != 2 {
		t.Fatalf("got %d results, want 2", len(results))
	}
	for _, r := range results {
		if r.Status != StatusCanonical {
			t.Errorf("%s: status = %s, want CANONICAL (flat=%v)", r.Path, r.Status, r.FlatKeys)
		}
	}
}

// TestAuditCanonical_DriftDetection: file with flat-form keys produces
// StatusDrift, listing exactly which flat keys triggered.
func TestAuditCanonical_DriftDetection(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "legacy.yaml"), `
project_name: legacy
push_requires_approval: true
branch_pattern: feat/*
commit_style: imperative-mood
`)

	results, err := AuditCanonical(dir)
	if err != nil {
		t.Fatalf("AuditCanonical: %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("got %d results, want 1", len(results))
	}
	r := results[0]
	if r.Status != StatusDrift {
		t.Errorf("status = %s, want DRIFT", r.Status)
	}
	wantKeys := map[string]bool{
		"push_requires_approval": true,
		"branch_pattern":         true,
		"commit_style":           true,
	}
	if len(r.FlatKeys) != len(wantKeys) {
		t.Errorf("FlatKeys = %v, want exactly %d keys", r.FlatKeys, len(wantKeys))
	}
	for _, k := range r.FlatKeys {
		if !wantKeys[k] {
			t.Errorf("unexpected flat key in result: %q", k)
		}
	}
}

// TestAuditCanonical_MixedDualForm: file with BOTH flat and nested forms
// reports DRIFT (flat keys present is the discriminator). Caller sees
// migration is incomplete even if some nested form has been added.
func TestAuditCanonical_MixedDualForm(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "mixed.yaml"), `
project_name: mixed
push_requires_approval: false
gates:
  push:
    requiresApproval: true
`)

	results, _ := AuditCanonical(dir)
	if len(results) != 1 || results[0].Status != StatusDrift {
		t.Errorf("mixed dual-form must be DRIFT; got %+v", results)
	}
}

// TestAuditCanonical_BadYAML: malformed file → StatusError with parse-
// error populated. Other files in same dir continue to audit normally.
func TestAuditCanonical_BadYAML(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "broken.yaml"), "project_name: [unclosed\n")
	writeFile(t, filepath.Join(dir, "ok.yaml"), "project_name: ok\n")

	results, err := AuditCanonical(dir)
	if err != nil {
		t.Fatalf("AuditCanonical: %v", err)
	}
	if len(results) != 2 {
		t.Fatalf("got %d results, want 2", len(results))
	}
	var broken, ok *AuditResult
	for i := range results {
		if strings.HasSuffix(results[i].Path, "broken.yaml") {
			broken = &results[i]
		} else {
			ok = &results[i]
		}
	}
	if broken == nil || broken.Status != StatusError {
		t.Errorf("broken.yaml status = %v, want ERROR", broken)
	}
	if broken.ParseError == "" {
		t.Errorf("broken.yaml ParseError empty")
	}
	if ok == nil || ok.Status != StatusCanonical {
		t.Errorf("ok.yaml status = %v, want CANONICAL", ok)
	}
}

// TestAuditCanonical_SkipsNonYAML: non-yaml files (.md, .json, dirs) are
// skipped; only *.yaml files audited.
func TestAuditCanonical_SkipsNonYAML(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "real.yaml"), "project_name: real\n")
	writeFile(t, filepath.Join(dir, "notes.md"), "# notes\n")
	writeFile(t, filepath.Join(dir, "data.json"), `{"x":1}`)
	if err := os.Mkdir(filepath.Join(dir, "subdir"), 0o755); err != nil {
		t.Fatal(err)
	}

	results, _ := AuditCanonical(dir)
	if len(results) != 1 {
		t.Errorf("audited %d files; want 1 (.yaml only). Results: %+v", len(results), results)
	}
}

// TestAuditCanonical_MissingDir: nonexistent projects dir → error
// surfaced (caller decides how to render).
func TestAuditCanonical_MissingDir(t *testing.T) {
	results, err := AuditCanonical(filepath.Join(t.TempDir(), "nonexistent"))
	if err == nil {
		t.Errorf("expected error for missing dir; got nil with results=%v", results)
	}
}

// TestAuditCanonical_SortedByPath: results returned in sorted-by-path
// order so CLI output is stable across runs.
func TestAuditCanonical_SortedByPath(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "z-last.yaml"), "project_name: z\n")
	writeFile(t, filepath.Join(dir, "a-first.yaml"), "project_name: a\n")
	writeFile(t, filepath.Join(dir, "m-mid.yaml"), "project_name: m\n")

	results, _ := AuditCanonical(dir)
	if len(results) != 3 {
		t.Fatalf("got %d, want 3", len(results))
	}
	if !strings.HasSuffix(results[0].Path, "a-first.yaml") ||
		!strings.HasSuffix(results[1].Path, "m-mid.yaml") ||
		!strings.HasSuffix(results[2].Path, "z-last.yaml") {
		t.Errorf("results not sorted: %v", []string{results[0].Path, results[1].Path, results[2].Path})
	}
}

// TestFormatAuditResults: rendering produces stable human-readable
// output covering all status classes + summary.
func TestFormatAuditResults(t *testing.T) {
	rs := []AuditResult{
		{Path: "/a.yaml", Status: StatusCanonical},
		{Path: "/b.yaml", Status: StatusDrift, FlatKeys: []string{"push_requires_approval", "branch_pattern"}},
		{Path: "/c.yaml", Status: StatusError, ParseError: "yaml parse: bad"},
	}
	out := FormatAuditResults(rs)
	for _, want := range []string{
		"/a.yaml: CANONICAL",
		"/b.yaml: DRIFT (flat keys: push_requires_approval, branch_pattern)",
		"/c.yaml: ERROR (yaml parse: bad)",
		"3 files audited: 1 canonical / 1 drift / 1 error",
		"DRIFT files have legacy flat-form keys",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q in output:\n%s", want, out)
		}
	}
}
