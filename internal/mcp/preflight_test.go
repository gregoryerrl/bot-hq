package mcp

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/projects"
)

// initGitRepo creates a tmp git repo with a synthetic origin URL.
func initGitRepo(t *testing.T, originURL string) string {
	t.Helper()
	dir := t.TempDir()
	for _, args := range [][]string{
		{"init", "-q"},
		{"remote", "add", "origin", originURL},
	} {
		full := append([]string{"-C", dir}, args...)
		if err := exec.Command("git", full...).Run(); err != nil {
			t.Fatalf("git %v: %v", args, err)
		}
	}
	return dir
}

func TestPreflightProjectRules_BlocksOnMissingRules(t *testing.T) {
	repo := initGitRepo(t, "git@github.com:gregoryerrl/missing-rules.git")
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	rules, err := preflightProjectRules(repo)
	if rules != nil {
		t.Fatalf("expected nil rules on miss, got %+v", rules)
	}
	if err == nil {
		t.Fatal("expected error on missing rules")
	}

	msg := err.Error()
	requiredFragments := []string{
		"hub_spawn blocked",
		"missing-rules",                  // derived project name surfaced
		"Bootstrap required",             // bootstrap framing present
		"docs/examples/projects/_default.yaml", // template path
		"git -C",                         // inspect command guidance
		"docs/arcs/phase-h.md",           // doctrine pointer
	}
	for _, frag := range requiredFragments {
		if !strings.Contains(msg, frag) {
			t.Errorf("error message missing fragment %q\nfull message:\n%s", frag, msg)
		}
	}
}

func TestPreflightProjectRules_BlocksOnRemoteMismatch(t *testing.T) {
	repo := initGitRepo(t, "git@github.com:gregoryerrl/mismatch-real.git")
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	if err := os.MkdirAll(filepath.Join(home, "projects"), 0o755); err != nil {
		t.Fatal(err)
	}
	yaml := `remote_url: "git@github.com:someone/different.git"
project_name: "mismatch-real"
branch_pattern: ".*"
`
	if err := os.WriteFile(filepath.Join(home, "projects", "mismatch-real.yaml"), []byte(yaml), 0o600); err != nil {
		t.Fatal(err)
	}

	rules, err := preflightProjectRules(repo)
	if rules != nil {
		t.Fatalf("expected nil rules on mismatch, got %+v", rules)
	}
	if err == nil {
		t.Fatal("expected error on remote mismatch")
	}
	if !strings.Contains(err.Error(), "remote_url does not match") {
		t.Errorf("error should describe remote mismatch; got: %v", err)
	}
	// Ensure the underlying sentinel is wrapped (callers may want errors.Is).
	// preflightProjectRules wraps with %v not %w on this branch, so we just
	// check the text rather than asserting errors.Is. If a future refactor
	// switches to %w, both checks pass.
	if errors.Is(err, projects.ErrRemoteMismatch) {
		t.Log("(also wraps ErrRemoteMismatch; OK)")
	}
}

func TestPreflightProjectRules_AllowsWithMatchingRules(t *testing.T) {
	repo := initGitRepo(t, "git@github.com:gregoryerrl/allowed-project.git")
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	if err := os.MkdirAll(filepath.Join(home, "projects"), 0o755); err != nil {
		t.Fatal(err)
	}
	yaml := `remote_url: "git@github.com:gregoryerrl/allowed-project.git"
project_name: "allowed-project"
branch_pattern: "^[0-9]+-[a-z0-9-]+$"
push_requires_approval: true
force_push_blocked: true
coder_tools_blocked:
  - "git push"
`
	if err := os.WriteFile(filepath.Join(home, "projects", "allowed-project.yaml"), []byte(yaml), 0o600); err != nil {
		t.Fatal(err)
	}

	rules, err := preflightProjectRules(repo)
	if err != nil {
		t.Fatalf("expected gate to pass, got error: %v", err)
	}
	if rules == nil {
		t.Fatal("expected non-nil rules on success")
	}
	if rules.ProjectName != "allowed-project" {
		t.Errorf("ProjectName = %q, want allowed-project", rules.ProjectName)
	}
	if !rules.PushRequiresApproval {
		t.Error("PushRequiresApproval should be true (round-trip)")
	}
}

func TestPreflightProjectRules_NoGitRemote(t *testing.T) {
	// A directory without git remote should fail at LoadForProject's git command.
	dir := t.TempDir()
	// Initialize as git repo but DON'T add remote.
	if err := exec.Command("git", "-C", dir, "init", "-q").Run(); err != nil {
		t.Fatal(err)
	}
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	rules, err := preflightProjectRules(dir)
	if rules != nil {
		t.Fatalf("expected nil rules on no-remote, got %+v", rules)
	}
	if err == nil {
		t.Fatal("expected error on no remote")
	}
	// Should pass through as a generic load error.
	if !strings.Contains(err.Error(), "hub_spawn blocked") {
		t.Errorf("error should be framed as hub_spawn blocked; got: %v", err)
	}
}
