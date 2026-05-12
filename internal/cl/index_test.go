package cl

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func setupIndexFixture(t *testing.T) (string, *CL) {
	t.Helper()
	root := t.TempDir()
	mustMkdir := func(p string) {
		if err := os.MkdirAll(p, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	mustWrite := func(p, content string) {
		mustMkdir(filepath.Dir(p))
		if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	mustMkdir(filepath.Join(root, "projects"))
	mustWrite(filepath.Join(root, "projects", "myproj.yaml"), `project_name: myproj
library:
  external_docs_root: ~/Projects/myproj/docs/
`)
	projDir := filepath.Join(root, "projects", "myproj")
	for _, sub := range indexSubdirs {
		mustMkdir(filepath.Join(projDir, sub))
	}
	mustWrite(filepath.Join(projDir, "README.md"), "# myproj\n")
	mustWrite(filepath.Join(projDir, "plans", "2026-05-01-foo.md"), "plan content\n")
	mustWrite(filepath.Join(projDir, "plans", "2026-05-02-bar.md"), "another plan\n")
	mustWrite(filepath.Join(projDir, "decisions", "README.md"), "# decisions/\n\nADRs.\n")

	c, err := NewCL(root)
	if err != nil {
		t.Fatal(err)
	}
	return root, c
}

func TestIndexProject_writesFrontmatterAndBody(t *testing.T) {
	_, c := setupIndexFixture(t)
	rendered, changed, err := c.IndexProject("myproj")
	if err != nil {
		t.Fatal(err)
	}
	if !changed {
		t.Errorf("first index call should report changed=true")
	}
	if !strings.HasPrefix(rendered, "---\n") {
		t.Errorf("missing frontmatter delimiter")
	}
	if !strings.Contains(rendered, "project: myproj") {
		t.Errorf("frontmatter missing project field")
	}
	if !strings.Contains(rendered, "schema_version: 1") {
		t.Errorf("frontmatter missing schema_version")
	}
	if !strings.Contains(rendered, "# myproj — library index") {
		t.Errorf("missing body header")
	}
	if !strings.Contains(rendered, "## Summary") {
		t.Errorf("missing summary section")
	}
	if !strings.Contains(rendered, "2026-05-02-bar.md") {
		t.Errorf("plans entries missing")
	}
}

// session-lifecycle-cleanup: every project's INDEX.md must point the
// BRAIN-duo at projects/<p>.yaml as the canonical rules file with an
// enumerated set of load-bearing fields. The duo's spawn-time bootstrap
// includes INDEX.md content, so this is what makes "session open → read
// CL → CL points at rules → read rules" work without explicit prompt
// injection.
func TestIndexProject_PointsAtProjectRulesFile(t *testing.T) {
	_, c := setupIndexFixture(t)
	rendered, _, err := c.IndexProject("myproj")
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"## Project rules",
		"projects/myproj.yaml",
		"Read this BEFORE any HANDS-class action",
		"branch.{pattern, examples, patternHelp}",
		"gates.push.requiresApproval",
		"Rain BRAIN-2nd alone is sufficient",
		"per-instance user verbatim still required",
		"gates.forcePush.{blocked, tokenFormat}",
		"gates.coder.{toolsBlocked, perActionApproval}",
		"commit.{style, requireIssueLink}",
		"project_feedback.*",
	} {
		if !strings.Contains(rendered, want) {
			t.Errorf("INDEX.md missing required literal %q (the agent flow needs this to discover rules)", want)
		}
	}
}

func TestIndexProject_idempotent(t *testing.T) {
	_, c := setupIndexFixture(t)
	if _, _, err := c.IndexProject("myproj"); err != nil {
		t.Fatal(err)
	}
	// Pause briefly so the timestamp advances; result should still be no-op.
	time.Sleep(10 * time.Millisecond)
	_, changed, err := c.IndexProject("myproj")
	if err != nil {
		t.Fatal(err)
	}
	if changed {
		t.Errorf("second call against unchanged disk should report changed=false")
	}
}

func TestIndexProject_externalDocsCrossLink(t *testing.T) {
	_, c := setupIndexFixture(t)
	rendered, _, err := c.IndexProject("myproj")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(rendered, "## Related external docs") {
		t.Errorf("missing external-docs section")
	}
	if !strings.Contains(rendered, "~/Projects/myproj/docs/") {
		t.Errorf("missing external-docs path")
	}
}

func TestIndexProject_subdirReadmeSkipped(t *testing.T) {
	_, c := setupIndexFixture(t)
	rendered, _, err := c.IndexProject("myproj")
	if err != nil {
		t.Fatal(err)
	}
	// decisions/ has only README.md → should report 0 in summary, not 1.
	// The summary table shows "decisions" with count 0.
	for _, line := range strings.Split(rendered, "\n") {
		if strings.Contains(line, "| decisions |") {
			if !strings.Contains(line, "| 0 |") {
				t.Errorf("decisions count should be 0 (README excluded), got line: %q", line)
			}
		}
	}
}

func TestIndexProject_missingProjectErrors(t *testing.T) {
	root := t.TempDir()
	c, err := NewCL(root)
	if err != nil {
		t.Fatal(err)
	}
	_, _, err = c.IndexProject("nonexistent")
	if err == nil {
		t.Errorf("expected error for missing project")
	}
}

func TestListProjects_filtersToYAMLBacked(t *testing.T) {
	root, c := setupIndexFixture(t)
	// Add a stray subdir without a matching .yaml — should be filtered.
	if err := os.MkdirAll(filepath.Join(root, "projects", "stray"), 0o755); err != nil {
		t.Fatal(err)
	}
	projects, err := c.ListProjects()
	if err != nil {
		t.Fatal(err)
	}
	if len(projects) != 1 || projects[0] != "myproj" {
		t.Errorf("ListProjects = %v; want [myproj]", projects)
	}
}

func TestIndexAll_writesTopLevelIndex(t *testing.T) {
	root, c := setupIndexFixture(t)
	_, topChanged, err := c.IndexAll()
	if err != nil {
		t.Fatal(err)
	}
	if !topChanged {
		t.Errorf("first IndexAll should report topChanged=true")
	}
	topPath := filepath.Join(root, "INDEX.md")
	data, err := os.ReadFile(topPath)
	if err != nil {
		t.Fatalf("top-level INDEX.md not written: %v", err)
	}
	got := string(data)
	if !strings.Contains(got, "Bot-HQ — cross-project IPAV index") {
		t.Errorf("top-level INDEX missing header")
	}
	if !strings.Contains(got, "[myproj](projects/myproj/INDEX.md)") {
		t.Errorf("top-level INDEX missing project link")
	}
}

func TestIndexProject_ipavTasks(t *testing.T) {
	root, c := setupIndexFixture(t)
	taskDir := filepath.Join(root, "projects", "myproj", "tasks", "task-abc")
	if err := os.MkdirAll(taskDir, 0o755); err != nil {
		t.Fatal(err)
	}
	stateYAML := `task_id: task-abc
current_phase: investigate
decision_class: medium
opened_at: 2026-05-09T10:00:00Z
`
	if err := os.WriteFile(filepath.Join(taskDir, "ipav-state.yaml"), []byte(stateYAML), 0o644); err != nil {
		t.Fatal(err)
	}
	rendered, _, err := c.IndexProject("myproj")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(rendered, "task-abc") {
		t.Errorf("active IPAV task not surfaced: %s", rendered)
	}
	if !strings.Contains(rendered, "investigate") {
		t.Errorf("phase not surfaced")
	}
}

func TestBytesEqualIgnoringFrontmatterTimestamp(t *testing.T) {
	a := []byte("---\ngenerated_at: 2026-01-01T00:00:00Z\nproject: x\n---\nbody\n")
	b := []byte("---\ngenerated_at: 2026-12-31T23:59:59Z\nproject: x\n---\nbody\n")
	if !bytesEqualIgnoringFrontmatterTimestamp(a, b) {
		t.Errorf("should treat as equal modulo timestamp")
	}
	c := []byte("---\ngenerated_at: 2026-01-01T00:00:00Z\nproject: y\n---\nbody\n")
	if bytesEqualIgnoringFrontmatterTimestamp(a, c) {
		t.Errorf("should treat as different when project differs")
	}
}

func TestFormatSize(t *testing.T) {
	cases := []struct {
		in   int64
		want string
	}{
		{0, "0"},
		{500, "500"},
		{1024, "1.0K"},
		{1500, "1.5K"},
		{1024 * 1024, "1.0M"},
		{int64(1024 * 1024 * 1024), "1.0G"},
	}
	for _, tc := range cases {
		if got := formatSize(tc.in); got != tc.want {
			t.Errorf("formatSize(%d) = %q; want %q", tc.in, got, tc.want)
		}
	}
}
