package contextload

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
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

// makeBootstrapCL extends makeTestCL with the durable-substrate files
// LoadBootstrap surfaces: phase/<latest>.md, ratchets/active.md, and
// per-agent <agent>/last_state.json + discipline-anchors.md.
func makeBootstrapCL(t *testing.T, agent string) string {
	t.Helper()
	root := makeTestCL(t)

	if err := os.MkdirAll(filepath.Join(root, "phase"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(root, "ratchets"), 0o755); err != nil {
		t.Fatal(err)
	}
	if agent != "" {
		if err := os.MkdirAll(filepath.Join(root, agent), 0o755); err != nil {
			t.Fatal(err)
		}
	}

	if err := os.WriteFile(filepath.Join(root, "phase", "phase-a.md"),
		[]byte("# phase a (older)\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// Older phase doc, then sleep then write the newer to guarantee
	// distinct mtimes on coarse-resolution filesystems.
	time.Sleep(10 * time.Millisecond)
	if err := os.WriteFile(filepath.Join(root, "phase", "phase-b.md"),
		[]byte("# phase b (active)\n\nActive scope-lock content.\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := os.WriteFile(filepath.Join(root, "ratchets", "active.md"),
		[]byte("# active ratchets\n\n- R1 closed\n- R2 open\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	if agent != "" {
		if err := os.WriteFile(filepath.Join(root, agent, "last_state.json"),
			[]byte(`{"phase":"phase-b","watermark":17730,"task":"investigating"}`), 0o644); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(root, agent, "discipline-anchors.md"),
			[]byte("# anchors\n\n- never push without greenlight\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	return root
}

func TestLoadBootstrap_AllSourcesPresent(t *testing.T) {
	root := makeBootstrapCL(t, "brian")
	bc, err := LoadBootstrap(root, "demo", "brian")
	if err != nil {
		t.Fatal(err)
	}
	if bc.Agent != "brian" {
		t.Errorf("Agent = %q, want brian", bc.Agent)
	}
	if !strings.Contains(bc.PhaseDoc, "phase b (active)") {
		t.Errorf("PhaseDoc should contain newest phase content; got %q", bc.PhaseDoc)
	}
	if !strings.HasSuffix(bc.PhaseDocPath, "phase-b.md") {
		t.Errorf("PhaseDocPath = %q, want suffix phase-b.md", bc.PhaseDocPath)
	}
	if !strings.Contains(bc.Ratchets, "R2 open") {
		t.Errorf("Ratchets should contain ratchet content; got %q", bc.Ratchets)
	}
	if !strings.Contains(bc.LastState, "watermark") {
		t.Errorf("LastState should contain JSON; got %q", bc.LastState)
	}
	if !strings.Contains(bc.DisciplineAnchors, "greenlight") {
		t.Errorf("DisciplineAnchors should contain anchor content; got %q", bc.DisciplineAnchors)
	}
}

func TestLoadBootstrap_PicksNewestPhaseDocByMtime(t *testing.T) {
	root := makeBootstrapCL(t, "brian")
	// phase-b.md was written second so it should win over phase-a.md.
	bc, err := LoadBootstrap(root, "demo", "brian")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasSuffix(bc.PhaseDocPath, "phase-b.md") {
		t.Errorf("expected newest phase doc phase-b.md, got %q", bc.PhaseDocPath)
	}
}

func TestLoadBootstrap_MissingPerAgentFilesAreEmpty(t *testing.T) {
	root := makeBootstrapCL(t, "")
	// Ask for agent=rain which has no files in this fixture.
	bc, err := LoadBootstrap(root, "demo", "rain")
	if err != nil {
		t.Fatal(err)
	}
	if bc.LastState != "" {
		t.Errorf("LastState should be empty for absent file; got %q", bc.LastState)
	}
	if bc.DisciplineAnchors != "" {
		t.Errorf("DisciplineAnchors should be empty for absent file; got %q", bc.DisciplineAnchors)
	}
}

func TestLoadBootstrap_MissingPhaseAndRatchetsAreEmpty(t *testing.T) {
	root := makeTestCL(t) // base CL without phase/ or ratchets/
	bc, err := LoadBootstrap(root, "demo", "brian")
	if err != nil {
		t.Fatal(err)
	}
	if bc.PhaseDoc != "" || bc.PhaseDocPath != "" {
		t.Errorf("PhaseDoc/Path should be empty when phase/ absent")
	}
	if bc.Ratchets != "" {
		t.Errorf("Ratchets should be empty when ratchets/active.md absent")
	}
}

func TestLoadBootstrap_MarkdownIncludesAllSections(t *testing.T) {
	root := makeBootstrapCL(t, "brian")
	bc, err := LoadBootstrap(root, "demo", "brian")
	if err != nil {
		t.Fatal(err)
	}
	md := bc.Markdown()

	wantStrings := []string{
		"# Project context: demo",  // base context
		"## Bootstrap context (agent=brian)",
		"### Active phase doc",
		"phase-b.md",
		"phase b (active)",
		"### Active ratchets",
		"R2 open",
		"### Last state",
		"last_state.json",
		"watermark",
		"### Discipline anchors",
		"greenlight",
	}
	for _, s := range wantStrings {
		if !strings.Contains(md, s) {
			t.Errorf("Markdown missing %q", s)
		}
	}
}

func TestLoadBootstrap_NoAgentSkipsAgentSections(t *testing.T) {
	root := makeBootstrapCL(t, "brian")
	bc, err := LoadBootstrap(root, "demo", "")
	if err != nil {
		t.Fatal(err)
	}
	if bc.LastState != "" || bc.DisciplineAnchors != "" {
		t.Errorf("agent='' should yield empty per-agent sections")
	}
	md := bc.Markdown()
	if strings.Contains(md, "Bootstrap context (agent=") {
		t.Errorf("agent='' Markdown should not render agent header; got: %q", md[:200])
	}
}
