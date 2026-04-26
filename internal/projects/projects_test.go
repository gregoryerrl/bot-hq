package projects

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestDeriveProjectName(t *testing.T) {
	cases := []struct {
		in, want string
	}{
		{"git@github.com:gregoryerrl/bcc-ad-manager.git", "bcc-ad-manager"},
		{"git@github.com:gregoryerrl/bcc-ad-manager", "bcc-ad-manager"},
		{"https://github.com/gregoryerrl/bcc-ad-manager.git", "bcc-ad-manager"},
		{"https://github.com/gregoryerrl/bcc-ad-manager", "bcc-ad-manager"},
		{"https://github.com/gregoryerrl/bcc-ad-manager/", "bcc-ad-manager"},
		{"git@github.com:org/foo_bar.git", "foo_bar"},
		{"https://gitlab.com/group/sub/project.git", "project"},
		{"", ""},
		{"not-a-url", ""}, // no `/` or `:` separator → cannot derive, return empty
	}
	for _, c := range cases {
		t.Run(c.in, func(t *testing.T) {
			got := DeriveProjectName(c.in)
			if got != c.want {
				t.Errorf("DeriveProjectName(%q) = %q, want %q", c.in, got, c.want)
			}
		})
	}
}

// initGitRepo creates a tmp git repo with a synthetic origin URL. Returns
// the repo path. Caller is responsible for tmp cleanup via t.TempDir().
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

// TestCanonicalizeRemoteURL_FalseNegative locks Phase H slice 2 H-29 — the
// canonicalizer MUST equate variant forms of the same repo. SSH and HTTPS
// clones, .git suffix presence, trailing slash, ssh:// URL form, http vs
// https, and shell-escape quoting all canonicalize to the same key.
func TestCanonicalizeRemoteURL_FalseNegative(t *testing.T) {
	pairs := [][2]string{
		{"git@github.com:org/foo.git", "https://github.com/org/foo.git"},
		{"git@github.com:org/foo", "https://github.com/org/foo"},
		{"git@github.com:org/foo.git", "https://github.com/org/foo"},
		{"https://github.com/org/foo.git", "https://github.com/org/foo"},
		{"ssh://git@github.com/org/foo.git", "https://github.com/org/foo"},
		{"git@gitlab.com:group/sub/proj.git", "https://gitlab.com/group/sub/proj"},
		{"https://github.com/org/foo/", "https://github.com/org/foo"},
		{"'git@github.com:org/foo.git'", "git@github.com:org/foo.git"},
		{"  https://github.com/org/foo.git  ", "https://github.com/org/foo"},
		{"http://github.com/org/foo", "https://github.com/org/foo"}, // deliberate http<->https equivalence
	}
	for _, p := range pairs {
		a, b := canonicalizeRemoteURL(p[0]), canonicalizeRemoteURL(p[1])
		if a != b {
			t.Errorf("canonicalizeRemoteURL(%q) = %q, canonicalizeRemoteURL(%q) = %q — should be equal", p[0], a, p[1], b)
		}
	}
}

// TestCanonicalizeRemoteURL_FalsePositive locks H-29 — the canonicalizer
// MUST keep distinct repos distinct. Different host, org, repo, gist host,
// GitHub Enterprise host, and case-difference all stay non-equal.
func TestCanonicalizeRemoteURL_FalsePositive(t *testing.T) {
	pairs := [][2]string{
		{"git@github.com:org/foo", "git@gitlab.com:org/foo"},        // different host
		{"git@github.com:upstream/foo", "git@github.com:fork/foo"},  // different org
		{"git@github.com:org/foo", "git@github.com:org/bar"},        // different repo
		{"https://gist.github.com/abc", "https://github.com/org/abc"}, // gist vs repo (different host)
		{"git@github.acme.corp:foo/bar", "git@github.com:foo/bar"},  // GitHub Enterprise vs github.com
		{"https://github.com/Org/foo", "https://github.com/org/foo"}, // case sensitivity
	}
	for _, p := range pairs {
		a, b := canonicalizeRemoteURL(p[0]), canonicalizeRemoteURL(p[1])
		if a == b {
			t.Errorf("canonicalizeRemoteURL(%q) = canonicalizeRemoteURL(%q) = %q — should be DISTINCT", p[0], p[1], a)
		}
	}
}

// TestCanonicalizeRemoteURL_Idempotent locks H-29 — applying the
// canonicalizer twice produces the same result as applying it once.
// Idempotency guards against drift if the canonical form is itself
// canonicalized (e.g., during stable-equality caching).
func TestCanonicalizeRemoteURL_Idempotent(t *testing.T) {
	inputs := []string{
		"git@github.com:org/foo.git",
		"https://github.com/org/foo.git",
		"https://github.com/org/foo",
		"ssh://git@github.com/org/foo.git",
		"http://github.com/org/foo",
		"'git@github.com:org/foo.git'",
		"  https://github.com/org/foo/  ",
		"",
		"github.com/org/foo", // already canonical
	}
	for _, in := range inputs {
		once := canonicalizeRemoteURL(in)
		twice := canonicalizeRemoteURL(once)
		if once != twice {
			t.Errorf("canonicalizeRemoteURL not idempotent for %q: once=%q, twice=%q", in, once, twice)
		}
	}
}

// TestLoadForProjectAcceptsCanonicallyEqualForms is the H-29 integration
// test — when the rules file specifies SSH form but `git remote get-url
// origin` returns HTTPS form (or vice versa), LoadForProject must accept
// the rules instead of erroring with ErrRemoteMismatch. Closes the slice
// 1 runtime-test bug class (msg 3327) at the gate level.
func TestLoadForProjectAcceptsCanonicallyEqualForms(t *testing.T) {
	// Repo's actual origin returns HTTPS form.
	repo := initGitRepo(t, "https://github.com/gregoryerrl/canon-test.git")
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)
	if err := os.MkdirAll(filepath.Join(home, "projects"), 0o755); err != nil {
		t.Fatal(err)
	}
	// Rules file specifies SSH form. Pre-H-29, this would have raised
	// ErrRemoteMismatch (verbatim string compare); post-H-29, canonical
	// equality accepts it.
	yaml := `remote_url: "git@github.com:gregoryerrl/canon-test.git"
project_name: "canon-test"
branch_pattern: ".*"
`
	if err := os.WriteFile(filepath.Join(home, "projects", "canon-test.yaml"), []byte(yaml), 0o600); err != nil {
		t.Fatal(err)
	}
	rules, err := LoadForProject(repo)
	if err != nil {
		t.Fatalf("expected canonical-equality acceptance, got error: %v", err)
	}
	if rules == nil {
		t.Fatal("expected non-nil rules")
	}
	if rules.ProjectName != "canon-test" {
		t.Errorf("expected project_name 'canon-test', got %q", rules.ProjectName)
	}
}

func TestLoadForProjectMissingFile(t *testing.T) {
	repo := initGitRepo(t, "git@github.com:gregoryerrl/test-missing.git")
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	rules, err := LoadForProject(repo)
	if rules != nil {
		t.Fatalf("expected nil rules, got %+v", rules)
	}
	if !errors.Is(err, ErrNoRulesFound) {
		t.Fatalf("expected ErrNoRulesFound, got %v", err)
	}
	if !strings.Contains(err.Error(), "test-missing") {
		t.Errorf("error should name the project; got %v", err)
	}
}

func TestLoadForProjectRemoteMismatch(t *testing.T) {
	repo := initGitRepo(t, "git@github.com:gregoryerrl/test-mismatch.git")
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	if err := os.MkdirAll(filepath.Join(home, "projects"), 0o755); err != nil {
		t.Fatal(err)
	}
	yaml := `remote_url: "git@github.com:someone-else/different-project.git"
project_name: "test-mismatch"
branch_pattern: ".*"
`
	if err := os.WriteFile(filepath.Join(home, "projects", "test-mismatch.yaml"), []byte(yaml), 0o600); err != nil {
		t.Fatal(err)
	}

	rules, err := LoadForProject(repo)
	if rules != nil {
		t.Fatalf("expected nil rules, got %+v", rules)
	}
	if !errors.Is(err, ErrRemoteMismatch) {
		t.Fatalf("expected ErrRemoteMismatch, got %v", err)
	}
}

func TestLoadForProjectSuccess(t *testing.T) {
	repo := initGitRepo(t, "git@github.com:gregoryerrl/test-ok.git")
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	if err := os.MkdirAll(filepath.Join(home, "projects"), 0o755); err != nil {
		t.Fatal(err)
	}
	yaml := `remote_url: "git@github.com:gregoryerrl/test-ok.git"
project_name: "test-ok"
branch_pattern: "^[0-9]+-[a-z0-9-]+$"
push_requires_approval: true
force_push_blocked: true
coder_tools_blocked:
  - "git push"
  - "gh pr create"
`
	if err := os.WriteFile(filepath.Join(home, "projects", "test-ok.yaml"), []byte(yaml), 0o600); err != nil {
		t.Fatal(err)
	}

	rules, err := LoadForProject(repo)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if rules.ProjectName != "test-ok" {
		t.Errorf("ProjectName = %q, want test-ok", rules.ProjectName)
	}
	if !rules.PushRequiresApproval {
		t.Error("PushRequiresApproval should be true")
	}
	if !rules.ForcePushBlocked {
		t.Error("ForcePushBlocked should be true")
	}
	if len(rules.CoderToolsBlocked) != 2 {
		t.Errorf("CoderToolsBlocked len = %d, want 2", len(rules.CoderToolsBlocked))
	}
}

// TestLoadForProjectFullSchemaRoundTrip locks parsing of every Rules field —
// catches a future schema field-rename or yaml-tag drift that the partial
// assertions in TestLoadForProjectSuccess would miss. Per Rain msg 3273
// obs #3 (C1 fold).
func TestLoadForProjectFullSchemaRoundTrip(t *testing.T) {
	repo := initGitRepo(t, "git@github.com:gregoryerrl/full-schema.git")
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	if err := os.MkdirAll(filepath.Join(home, "projects"), 0o755); err != nil {
		t.Fatal(err)
	}
	yaml := `remote_url: "git@github.com:gregoryerrl/full-schema.git"
project_name: "full-schema"
branch_pattern: "^[0-9]+-[a-z0-9-]+$"
branch_examples:
  - "346-test-one"
  - "355-test-two"
branch_pattern_help: "Use [issueNo]-[title-with-dashes]; lowercase only"
push_requires_approval: true
force_push_blocked: true
force_push_token_format: "force-push-greenlight: {branch}@{sha}"
coder_tools_blocked:
  - "git push"
  - "gh pr create"
  - "rm -rf"
coder_tools_per_action_approval:
  - "git commit"
commit_style: "imperative-mood"
require_issue_link: true
`
	if err := os.WriteFile(filepath.Join(home, "projects", "full-schema.yaml"), []byte(yaml), 0o600); err != nil {
		t.Fatal(err)
	}

	rules, err := LoadForProject(repo)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Assert every Rules field surfaces correctly post-parse.
	if rules.RemoteURL != "git@github.com:gregoryerrl/full-schema.git" {
		t.Errorf("RemoteURL = %q", rules.RemoteURL)
	}
	if rules.ProjectName != "full-schema" {
		t.Errorf("ProjectName = %q", rules.ProjectName)
	}
	if rules.BranchPattern != `^[0-9]+-[a-z0-9-]+$` {
		t.Errorf("BranchPattern = %q", rules.BranchPattern)
	}
	if len(rules.BranchExamples) != 2 || rules.BranchExamples[0] != "346-test-one" || rules.BranchExamples[1] != "355-test-two" {
		t.Errorf("BranchExamples = %v", rules.BranchExamples)
	}
	if rules.BranchPatternHelp != "Use [issueNo]-[title-with-dashes]; lowercase only" {
		t.Errorf("BranchPatternHelp = %q", rules.BranchPatternHelp)
	}
	if !rules.PushRequiresApproval {
		t.Error("PushRequiresApproval should be true")
	}
	if !rules.ForcePushBlocked {
		t.Error("ForcePushBlocked should be true")
	}
	if rules.ForcePushTokenFormat != "force-push-greenlight: {branch}@{sha}" {
		t.Errorf("ForcePushTokenFormat = %q", rules.ForcePushTokenFormat)
	}
	if len(rules.CoderToolsBlocked) != 3 {
		t.Errorf("CoderToolsBlocked len = %d, want 3", len(rules.CoderToolsBlocked))
	}
	if len(rules.CoderToolsPerActionApproval) != 1 || rules.CoderToolsPerActionApproval[0] != "git commit" {
		t.Errorf("CoderToolsPerActionApproval = %v", rules.CoderToolsPerActionApproval)
	}
	if rules.CommitStyle != "imperative-mood" {
		t.Errorf("CommitStyle = %q", rules.CommitStyle)
	}
	if !rules.RequireIssueLink {
		t.Error("RequireIssueLink should be true")
	}
}

func TestValidateBranchName(t *testing.T) {
	r := &Rules{
		BranchPattern:     `^[0-9]+-[a-z0-9-]+$`,
		BranchPatternHelp: "Use [issueNo]-[title-with-dashes]; lowercase only",
	}
	cases := []struct {
		name    string
		wantErr bool
	}{
		{"346-streamline-onboarding-process", false},
		{"355-duplicateadjob-fails-for-lead-generation", false},
		{"feature/something", true},
		{"brian/foo", true},
		{"346-Streamline", true},   // uppercase rejected
		{"streamline-only", true},  // missing leading digits
		{"", true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			err := r.ValidateBranchName(c.name)
			if c.wantErr && err == nil {
				t.Errorf("expected error for %q", c.name)
			}
			if !c.wantErr && err != nil {
				t.Errorf("unexpected error for %q: %v", c.name, err)
			}
			if c.wantErr && err != nil {
				var ve *ValidationError
				if !errors.As(err, &ve) {
					t.Errorf("expected *ValidationError for %q, got %T", c.name, err)
				}
			}
		})
	}

	t.Run("empty pattern disables", func(t *testing.T) {
		empty := &Rules{}
		if err := empty.ValidateBranchName("anything-goes"); err != nil {
			t.Errorf("empty pattern should accept any name, got %v", err)
		}
	})
}

func TestIsCoderToolBlocked(t *testing.T) {
	r := &Rules{CoderToolsBlocked: []string{"git push", "gh pr create", "rm -rf"}}
	cases := []struct {
		cmd  string
		want bool
	}{
		{"git push", true},
		{"git push origin main", true},
		{"git push --force", true},
		{"gh pr create --title foo", true},
		{"rm -rf /tmp/scratch", true},
		{"git status", false},
		{"git pushover", false},     // prefix-must-be-token-bounded
		{"echo git push", false},    // not at start
		{"gh issue list", false},
		{"", false},
	}
	for _, c := range cases {
		t.Run(c.cmd, func(t *testing.T) {
			got := r.IsCoderToolBlocked(c.cmd)
			if got != c.want {
				t.Errorf("IsCoderToolBlocked(%q) = %v, want %v", c.cmd, got, c.want)
			}
		})
	}

	t.Run("empty blocklist allows all", func(t *testing.T) {
		empty := &Rules{}
		if empty.IsCoderToolBlocked("git push --force") {
			t.Error("empty blocklist should allow any command")
		}
	})
}
