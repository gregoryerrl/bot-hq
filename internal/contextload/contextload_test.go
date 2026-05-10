package contextload

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// makeTestCL builds a temp canonical-store with the layered yamls + an
// optional project library README. Returns the canonRoot path.
func makeTestCL(t *testing.T) string {
	t.Helper()
	root := t.TempDir()

	if err := os.MkdirAll(filepath.Join(root, "rules"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(root, "projects", "demo"), 0o755); err != nil {
		t.Fatal(err)
	}

	// general.yaml — universal trio defaults
	generalYAML := `tone:
  reply: "compact + cite-anchored"
greenlight:
  push: "user-gated explicit per branch"
workflow_discipline:
  no_time_pressure:
    rule: "drop ETA framing"
    cite: "feedback_no_time_pressure.md"
`
	if err := os.WriteFile(filepath.Join(root, "rules", "general.yaml"), []byte(generalYAML), 0o644); err != nil {
		t.Fatal(err)
	}

	// projects/demo.yaml — project-specific rules
	demoYAML := `project_name: "demo"
remote_url: "https://example.invalid/demo"
gates:
  push:
    requiresApproval: false
project_feedback:
  demo_specific_rule:
    rule: "always include foo in bar"
    cite: "feedback_demo.md"
`
	if err := os.WriteFile(filepath.Join(root, "projects", "demo.yaml"), []byte(demoYAML), 0o644); err != nil {
		t.Fatal(err)
	}

	// projects/demo/README.md — library overview
	readme := "# demo project\n\nDemo project for contextload testing.\n"
	if err := os.WriteFile(filepath.Join(root, "projects", "demo", "README.md"), []byte(readme), 0o644); err != nil {
		t.Fatal(err)
	}

	return root
}

func TestLoad_MergesGeneralAndProject(t *testing.T) {
	root := makeTestCL(t)

	ctx, err := Load(root, "demo")
	if err != nil {
		t.Fatalf("Load: %v", err)
	}

	if ctx.Project != "demo" {
		t.Errorf("Project = %q, want demo", ctx.Project)
	}

	// general.yaml's tone block must be present (Layer 1 / general)
	if _, ok := ctx.Rules["tone"]; !ok {
		t.Errorf("merged rules missing 'tone' from general.yaml")
	}

	// projects/demo.yaml's project_feedback must be present (Layer 2 / project)
	pf, ok := ctx.Rules["project_feedback"].(map[string]any)
	if !ok {
		t.Fatalf("project_feedback missing or wrong type: %T", ctx.Rules["project_feedback"])
	}
	if _, ok := pf["demo_specific_rule"]; !ok {
		t.Errorf("project_feedback.demo_specific_rule missing")
	}
}

func TestLoad_MissingProject(t *testing.T) {
	root := makeTestCL(t)

	// project key with no yaml/library — should still succeed (treats
	// missing layers as empty), returning just the general.yaml content.
	ctx, err := Load(root, "nonexistent")
	if err != nil {
		t.Fatalf("Load missing project should not error: %v", err)
	}
	if _, ok := ctx.Rules["tone"]; !ok {
		t.Errorf("expected general.yaml content even when project absent")
	}
	if ctx.Overview != "" {
		t.Errorf("expected empty Overview for missing library, got %q", ctx.Overview)
	}
}

func TestLoad_RequiresProjectKey(t *testing.T) {
	root := makeTestCL(t)
	_, err := Load(root, "")
	if err == nil {
		t.Error("expected error for empty project key")
	}
}

func TestMarkdown_HasExpectedSections(t *testing.T) {
	root := makeTestCL(t)
	ctx, err := Load(root, "demo")
	if err != nil {
		t.Fatal(err)
	}

	md := ctx.Markdown()

	wantSubs := []string{
		"# Project context: demo",
		"## Resolved rules (general → project)",
		"## Library overview",
		"# demo project", // README content
		"## Sources",
		"general.yaml",
		"demo.yaml",
		"README.md",
	}
	for _, sub := range wantSubs {
		if !strings.Contains(md, sub) {
			t.Errorf("Markdown missing %q\nFull output:\n%s", sub, md)
		}
	}
}

func TestMarkdown_StableOrdering(t *testing.T) {
	root := makeTestCL(t)
	ctx, err := Load(root, "demo")
	if err != nil {
		t.Fatal(err)
	}

	// Two consecutive renders should produce identical output (modulo
	// the LoadedAt timestamp) so the agent's prompt cache stays warm.
	md1 := ctx.Markdown()
	md2 := ctx.Markdown()
	if md1 != md2 {
		t.Error("Markdown output not deterministic across renders")
	}
}
