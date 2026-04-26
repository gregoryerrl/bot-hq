// Package projects loads per-project rules from ~/.bot-hq/projects/<name>.yaml.
//
// Rules govern bot-hq's interaction with each managed project (branch naming,
// push approval, force-push gates, coder tool allowlist). Rules live in the
// user's home directory — never inside client repos — to keep client repos
// pristine of bot-hq AI infrastructure.
//
// Bootstrap: missing rules file = ErrNoRulesFound. Caller (e.g. hub_spawn)
// must surface a friendly-fail message guiding user through bootstrap. The
// loader never auto-applies a default fallback; rules must be explicit.
package projects

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"

	"gopkg.in/yaml.v3"
)

// Rules describes how bot-hq must behave inside a given project.
type Rules struct {
	RemoteURL                   string   `yaml:"remote_url"`
	ProjectName                 string   `yaml:"project_name"`
	BranchPattern               string   `yaml:"branch_pattern"`
	BranchExamples              []string `yaml:"branch_examples"`
	BranchPatternHelp           string   `yaml:"branch_pattern_help"`
	PushRequiresApproval        bool     `yaml:"push_requires_approval"`
	ForcePushBlocked            bool     `yaml:"force_push_blocked"`
	ForcePushTokenFormat        string   `yaml:"force_push_token_format"`
	CoderToolsBlocked           []string `yaml:"coder_tools_blocked"`
	CoderToolsPerActionApproval []string `yaml:"coder_tools_per_action_approval"`
	CommitStyle                 string   `yaml:"commit_style"`
	RequireIssueLink            bool     `yaml:"require_issue_link"`
}

// Errors surfaced by LoadForProject.
var (
	ErrNoRulesFound   = errors.New("no project rules file found")
	ErrRemoteMismatch = errors.New("project rules file remote_url does not match actual remote")
)

// projectsDir returns ~/.bot-hq/projects, honoring BOT_HQ_HOME for tests.
func projectsDir() (string, error) {
	if h := os.Getenv("BOT_HQ_HOME"); h != "" {
		return filepath.Join(h, "projects"), nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("locate home dir: %w", err)
	}
	return filepath.Join(home, ".bot-hq", "projects"), nil
}

// remoteNameRE extracts the bare project name from a git remote URL.
//   git@github.com:org/foo.git    -> foo
//   https://github.com/org/foo    -> foo
//   https://github.com/org/foo.git -> foo
var remoteNameRE = regexp.MustCompile(`[/:]([\w.-]+?)(?:\.git)?/?$`)

// DeriveProjectName extracts the canonical project name from a git remote URL.
// Returns "" if the URL doesn't match an expected shape.
func DeriveProjectName(remoteURL string) string {
	remoteURL = strings.TrimSpace(remoteURL)
	if remoteURL == "" {
		return ""
	}
	m := remoteNameRE.FindStringSubmatch(remoteURL)
	if len(m) < 2 {
		return ""
	}
	return m[1]
}

// LoadForProject reads ~/.bot-hq/projects/<derived_name>.yaml for the project
// at projectDir. Returns ErrNoRulesFound when the file is absent (bootstrap
// case) and ErrRemoteMismatch when the file's remote_url disagrees with the
// project's actual origin (likely wrong file, refuse to use it).
func LoadForProject(projectDir string) (*Rules, error) {
	out, err := exec.Command("git", "-C", projectDir, "remote", "get-url", "origin").Output()
	if err != nil {
		return nil, fmt.Errorf("read origin remote for %s: %w", projectDir, err)
	}
	remoteURL := strings.TrimSpace(string(out))
	name := DeriveProjectName(remoteURL)
	if name == "" {
		return nil, fmt.Errorf("could not derive project name from remote: %q", remoteURL)
	}

	dir, err := projectsDir()
	if err != nil {
		return nil, err
	}
	rulesPath := filepath.Join(dir, name+".yaml")
	data, err := os.ReadFile(rulesPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, fmt.Errorf("%w: expected at %s for project %q", ErrNoRulesFound, rulesPath, name)
		}
		return nil, fmt.Errorf("read %s: %w", rulesPath, err)
	}

	var rules Rules
	if err := yaml.Unmarshal(data, &rules); err != nil {
		return nil, fmt.Errorf("parse %s: %w", rulesPath, err)
	}
	if rules.RemoteURL != "" && rules.RemoteURL != remoteURL {
		return nil, fmt.Errorf("%w: file says %q, actual is %q", ErrRemoteMismatch, rules.RemoteURL, remoteURL)
	}
	return &rules, nil
}

// ValidationError carries the offending name + the rule's help text so the
// caller can surface an actionable error to the coder/user.
type ValidationError struct {
	Name    string
	Pattern string
	Help    string
}

func (e *ValidationError) Error() string {
	if e.Help != "" {
		return fmt.Sprintf("branch name %q does not match pattern %q (%s)", e.Name, e.Pattern, e.Help)
	}
	return fmt.Sprintf("branch name %q does not match pattern %q", e.Name, e.Pattern)
}

// ValidateBranchName checks name against r.BranchPattern. An empty pattern
// disables validation and always returns nil.
func (r *Rules) ValidateBranchName(name string) error {
	if r.BranchPattern == "" {
		return nil
	}
	re, err := regexp.Compile(r.BranchPattern)
	if err != nil {
		return fmt.Errorf("invalid branch_pattern %q: %w", r.BranchPattern, err)
	}
	if !re.MatchString(name) {
		return &ValidationError{Name: name, Pattern: r.BranchPattern, Help: r.BranchPatternHelp}
	}
	return nil
}

// IsCoderToolBlocked reports whether the given command-line is blocked under
// r.CoderToolsBlocked. Match is prefix-based (the blocked entry must appear
// as a leading token sequence in cmdline).
func (r *Rules) IsCoderToolBlocked(cmdline string) bool {
	cmdline = strings.TrimSpace(cmdline)
	if cmdline == "" {
		return false
	}
	for _, blocked := range r.CoderToolsBlocked {
		blocked = strings.TrimSpace(blocked)
		if blocked == "" {
			continue
		}
		if cmdline == blocked || strings.HasPrefix(cmdline, blocked+" ") {
			return true
		}
	}
	return false
}
