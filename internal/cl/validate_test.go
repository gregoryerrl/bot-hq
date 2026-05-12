package cl

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Phase Z S2 ValidateProject tests — R39 TEST-ISOLATION via t.TempDir().

func TestValidateProject_MissingYaml_ErrNotFound(t *testing.T) {
	c := newCLFixture(t)
	_, err := c.ValidateProject("ghost-project")
	if !errors.Is(err, ErrNotFound) {
		t.Fatalf("want ErrNotFound, got %v", err)
	}
}

func TestValidateProject_MalformedYaml_SyntheticIssue(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "broken", "project_name: \"broken\"\nextensions:\n  : not-a-key\n")
	issues, err := c.ValidateProject("broken")
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	if len(issues) != 1 || issues[0].Severity != "error" {
		t.Fatalf("want one error issue, got %+v", issues)
	}
	if !strings.Contains(issues[0].Rule, "yaml parse failed") {
		t.Errorf("rule should reference yaml parse failure; got %q", issues[0].Rule)
	}
}

func TestValidateProject_BrainDuoOperational_NonBotHQ_Error(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "client-x", `project_name: "client-x"
extensions:
  brain_duo_operational:
    - phase
`)
	issues, err := c.ValidateProject("client-x")
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	if got := countSeverity(issues, "error"); got == 0 {
		t.Fatalf("expected error issue for brain_duo_operational on non-bot-hq; got %+v", issues)
	}
}

func TestValidateProject_BrainDuoOperational_BotHQ_OK(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "bot-hq", `project_name: "bot-hq"
extensions:
  brain_duo_operational:
    - phase
    - ratchets
`)
	if err := os.MkdirAll(filepath.Join(c.root, "projects", "bot-hq", "phase"), 0o755); err != nil {
		t.Fatalf("mkdir phase: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(c.root, "projects", "bot-hq", "ratchets"), 0o755); err != nil {
		t.Fatalf("mkdir ratchets: %v", err)
	}
	issues, err := c.ValidateProject("bot-hq")
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	if got := countSeverity(issues, "error"); got != 0 {
		t.Fatalf("expected no errors for bot-hq brain_duo_operational; got %+v", issues)
	}
}

func TestValidateProject_DuplicateBasename_Error(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "client-x", `project_name: "client-x"
extensions:
  universal_opt_in:
    - shared
  external_docs_pointer:
    - shared
`)
	issues, err := c.ValidateProject("client-x")
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	found := false
	for _, i := range issues {
		if i.Severity == "error" && strings.Contains(i.Rule, "duplicate basename") {
			found = true
		}
	}
	if !found {
		t.Fatalf("expected duplicate-basename error; got %+v", issues)
	}
}

func TestValidateProject_BadBasename_Error(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "client-x", `project_name: "client-x"
extensions:
  universal_opt_in:
    - ".dotfile"
    - "has/slash"
`)
	issues, err := c.ValidateProject("client-x")
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	if got := countSeverity(issues, "error"); got < 2 {
		t.Fatalf("expected ≥2 basename errors; got %+v", issues)
	}
}

func TestValidateProject_DeclaredNotOnDisk_Warning(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "client-x", `project_name: "client-x"
extensions:
  foundational_anchors:
    - missing-anchor.md
`)
	if err := os.MkdirAll(filepath.Join(c.root, "projects", "client-x"), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	issues, err := c.ValidateProject("client-x")
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	if got := countSeverity(issues, "warning"); got == 0 {
		t.Fatalf("expected on-disk-missing warning; got %+v", issues)
	}
	if got := countSeverity(issues, "error"); got != 0 {
		t.Fatalf("did not expect errors for on-disk-missing; got %+v", issues)
	}
}

func TestValidateProject_EmptyName_Rejected(t *testing.T) {
	c := newCLFixture(t)
	if _, err := c.ValidateProject(""); err == nil {
		t.Fatalf("expected error for empty project")
	}
}

func TestValidateAll_AggregatesAcrossProjects(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "good", `project_name: "good"
`)
	writeYaml(t, c, "client-x", `project_name: "client-x"
extensions:
  universal_opt_in:
    - ".dotfile"
`)
	issues, err := c.ValidateAll()
	if err != nil {
		t.Fatalf("validate-all: %v", err)
	}
	seenProjects := map[string]bool{}
	for _, i := range issues {
		seenProjects[i.Project] = true
	}
	if !seenProjects["client-x"] {
		t.Errorf("expected issue for client-x in validate-all output; got %+v", issues)
	}
}

func TestValidationIssue_StringFormats(t *testing.T) {
	cases := []struct {
		name string
		i    ValidationIssue
		want string
	}{
		{
			name: "with path",
			i:    ValidationIssue{Project: "p", Class: "c", Name: "n", Severity: "warning", Rule: "r", Path: "/tmp/p/n"},
			want: "[warning] p/c/\"n\" at /tmp/p/n: r",
		},
		{
			name: "without path",
			i:    ValidationIssue{Project: "p", Class: "c", Name: "n", Severity: "error", Rule: "r"},
			want: "[error] p/c/\"n\": r",
		},
		{
			name: "name-only blank",
			i:    ValidationIssue{Project: "p", Severity: "error", Rule: "r"},
			want: "[error] p: r",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := tc.i.String(); got != tc.want {
				t.Errorf("got %q, want %q", got, tc.want)
			}
		})
	}
}

// newCLFixture creates a t.TempDir() CL root with projects/ subdir +
// returns a CL pointing at it.
func newCLFixture(t *testing.T) *CL {
	t.Helper()
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "projects"), 0o755); err != nil {
		t.Fatalf("mkdir projects: %v", err)
	}
	c, err := NewCL(root)
	if err != nil {
		t.Fatalf("NewCL: %v", err)
	}
	return c
}

func writeYaml(t *testing.T, c *CL, project, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(c.root, "projects", project), 0o755); err != nil {
		t.Fatalf("mkdir project dir: %v", err)
	}
	path := filepath.Join(c.root, "projects", project+".yaml")
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatalf("write yaml %s: %v", path, err)
	}
}

func countSeverity(issues []ValidationIssue, sev string) int {
	n := 0
	for _, i := range issues {
		if i.Severity == sev {
			n++
		}
	}
	return n
}
