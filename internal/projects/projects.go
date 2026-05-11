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
//
// Schema-canonical-form (Phase N v3.x-2): nested categories under gates/branch/
// commit per `2026-05-07-phase-n-v3.x-1.5-agent-consumption-design-spike.md`
// §2.1. Identity scalars (project_name + remote_url) remain flat top-level.
// Back-compat: pre-migration YAMLs using flat keys (push_requires_approval
// etc.) still parse correctly via the custom UnmarshalYAML below — nested
// values win where both are present.
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
	Extensions                  ExtensionsBlock `yaml:"extensions,omitempty"`
}

// ExtensionsBlock declares per-project files/dirs beyond the canonical
// 9-subdir schema. Each class is a list of basenames relative to
// projects/<p>/. Filename convention: explicit-extension for files
// (e.g., "vision.md"), bare-name for dirs (e.g., "phase"). Absent
// extensions block = canonical-only with free-form discovery for any
// undeclared on-disk files.
//
// Project-private catch-all (5th class per Plan-B R.1 taxonomy) is
// implicit — undeclared on-disk files surface in nav under "More" without
// needing a field here.
type ExtensionsBlock struct {
	UniversalOptIn      []string `yaml:"universal_opt_in,omitempty"`
	ExternalDocsPointer []string `yaml:"external_docs_pointer,omitempty"`
	BrainDuoOperational []string `yaml:"brain_duo_operational,omitempty"`
	FoundationalAnchors []string `yaml:"foundational_anchors,omitempty"`
}

// AllNames returns every basename declared across all 4 classes. Used by
// the tree-walker to determine which non-canonical files/dirs have a
// declared class.
func (e ExtensionsBlock) AllNames() []string {
	out := make([]string, 0, len(e.UniversalOptIn)+len(e.ExternalDocsPointer)+len(e.BrainDuoOperational)+len(e.FoundationalAnchors))
	out = append(out, e.UniversalOptIn...)
	out = append(out, e.ExternalDocsPointer...)
	out = append(out, e.BrainDuoOperational...)
	out = append(out, e.FoundationalAnchors...)
	return out
}

// nestedGates / nestedBranch / nestedCommit are the canonical nested-form
// schema introduced in Phase N v3.x-2. Used by the dual-form unmarshaler
// to accept either flat or nested per-project YAMLs.
type nestedGates struct {
	Push *struct {
		RequiresApproval bool `yaml:"requiresApproval"`
	} `yaml:"push,omitempty"`
	ForcePush *struct {
		Blocked     bool   `yaml:"blocked"`
		TokenFormat string `yaml:"tokenFormat"`
	} `yaml:"forcePush,omitempty"`
	Coder *struct {
		ToolsBlocked       []string `yaml:"toolsBlocked"`
		PerActionApproval  []string `yaml:"perActionApproval"`
	} `yaml:"coder,omitempty"`
}

type nestedBranch struct {
	Pattern     string   `yaml:"pattern"`
	Examples    []string `yaml:"examples"`
	PatternHelp string   `yaml:"patternHelp"`
}

type nestedCommit struct {
	Style            string `yaml:"style"`
	RequireIssueLink bool   `yaml:"requireIssueLink"`
}

// rulesAux is the dual-form layout. Both flat fields (back-compat) and
// nested categories (canonical) are accepted; nested wins on conflict.
type rulesAux struct {
	// Identity scalars (always flat top-level).
	RemoteURL   string `yaml:"remote_url"`
	ProjectName string `yaml:"project_name"`

	// Nested canonical categories (Phase N v3.x-2 form).
	Gates  *nestedGates  `yaml:"gates,omitempty"`
	Branch *nestedBranch `yaml:"branch,omitempty"`
	Commit *nestedCommit `yaml:"commit,omitempty"`

	// Flat back-compat fields (legacy form).
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

	// Phase Z-CL-uniformity: extensions block (new in this IPAV).
	Extensions ExtensionsBlock `yaml:"extensions,omitempty"`
}

// UnmarshalYAML accepts both the canonical nested form (gates/branch/commit
// keys) and the legacy flat form (push_requires_approval etc.). Where a
// field is set in BOTH forms, nested wins (canonical-form is authoritative
// going forward).
func (r *Rules) UnmarshalYAML(value *yaml.Node) error {
	var a rulesAux
	if err := value.Decode(&a); err != nil {
		return err
	}
	// Identity
	r.RemoteURL = a.RemoteURL
	r.ProjectName = a.ProjectName

	// Branch — nested wins, flat is fallback.
	if a.Branch != nil {
		r.BranchPattern = a.Branch.Pattern
		r.BranchExamples = a.Branch.Examples
		r.BranchPatternHelp = a.Branch.PatternHelp
	} else {
		r.BranchPattern = a.BranchPattern
		r.BranchExamples = a.BranchExamples
		r.BranchPatternHelp = a.BranchPatternHelp
	}

	// Commit — nested wins, flat is fallback.
	if a.Commit != nil {
		r.CommitStyle = a.Commit.Style
		r.RequireIssueLink = a.Commit.RequireIssueLink
	} else {
		r.CommitStyle = a.CommitStyle
		r.RequireIssueLink = a.RequireIssueLink
	}

	// Gates — sub-categories merged independently so a nested gates: { push: ... }
	// without forcePush still falls back to flat force_push_blocked. Each leaf
	// is "nested if present else flat".
	if a.Gates != nil && a.Gates.Push != nil {
		r.PushRequiresApproval = a.Gates.Push.RequiresApproval
	} else {
		r.PushRequiresApproval = a.PushRequiresApproval
	}
	if a.Gates != nil && a.Gates.ForcePush != nil {
		r.ForcePushBlocked = a.Gates.ForcePush.Blocked
		r.ForcePushTokenFormat = a.Gates.ForcePush.TokenFormat
	} else {
		r.ForcePushBlocked = a.ForcePushBlocked
		r.ForcePushTokenFormat = a.ForcePushTokenFormat
	}
	if a.Gates != nil && a.Gates.Coder != nil {
		r.CoderToolsBlocked = a.Gates.Coder.ToolsBlocked
		r.CoderToolsPerActionApproval = a.Gates.Coder.PerActionApproval
	} else {
		r.CoderToolsBlocked = a.CoderToolsBlocked
		r.CoderToolsPerActionApproval = a.CoderToolsPerActionApproval
	}

	// Extensions: no flat/nested duality — single form only.
	r.Extensions = a.Extensions
	return nil
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
//
// Character class extends [\w-] (per slice 1 design schema) to [\w.-] to
// support project names like "988.utah.gov" or "foo.bar". Dots are common
// enough in real project names that excluding them caused derivation
// failure on legitimate remotes; safe extension since we still reject any
// URL without a leading `/` or `:` (returns "" on bare tokens).
var remoteNameRE = regexp.MustCompile(`[/:]([\w.-]+?)(?:\.git)?/?$`)

// canonicalizeRemoteURL normalizes a git remote URL to a scheme-agnostic,
// suffix-agnostic form for equality comparison. Used by LoadForProject's
// remote_url mismatch check so SSH and HTTPS clones of the same repo are
// treated as equal — the gate concerns project identity, not transport.
//
// Recognized transformations:
//
//	git@github.com:org/foo.git       -> github.com/org/foo
//	git@github.com:org/foo           -> github.com/org/foo
//	https://github.com/org/foo.git   -> github.com/org/foo
//	https://github.com/org/foo       -> github.com/org/foo
//	http://github.com/org/foo        -> github.com/org/foo  (deliberate http<->https equivalence)
//	ssh://git@github.com/org/foo.git -> github.com/org/foo
//	'git@github.com:org/foo.git'     -> github.com/org/foo  (shell-escape stripped)
//	  https://github.com/org/foo/    -> github.com/org/foo  (whitespace + trailing-slash trimmed)
//
// Org/owner segments are PRESERVED — `upstream/foo` and `fork/foo` stay
// distinct. Host segments are PRESERVED — `github.com` and `gitlab.com`
// stay distinct, as do GitHub Enterprise hosts (`github.acme.corp`) and
// gist hosts (`gist.github.com`). Case is preserved (Git is case-sensitive
// on most hosts).
//
// http vs https equivalence is deliberate: the gate concerns project
// identity, not transport security. User handling of transport security
// is out-of-band.
//
// SSH custom-port handling: scp-form URLs `user@host:path` cannot specify
// a port (per scp syntax). For URL-form SSH `ssh://user@host:port/path`,
// the user@ triggers scp-form normalization, which absorbs the port digits
// into the path key (`host/port/path`). Different ports still canonicalize
// distinctly (port digits make path-keys differ), but the mechanism is
// absorption, not preservation. Custom SSH ports are rare in practice.
//
// The function never mutates user-visible URLs — it only produces a
// comparison key. Error messages still surface the original verbatim
// strings so users see what mismatched.
func canonicalizeRemoteURL(u string) string {
	u = strings.TrimSpace(u)
	u = strings.Trim(u, `'"`)
	if u == "" {
		return ""
	}
	for _, scheme := range []string{"https://", "http://", "git://", "ssh://"} {
		u = strings.TrimPrefix(u, scheme)
	}
	// scp-form SSH: `[user@]host:path`. Convert the host:path colon to a
	// slash so the result aligns with HTTPS's `host/path` shape. We only
	// transform when there is a `user@` prefix to avoid touching URLs that
	// already used `:` as a port separator after their scheme was stripped
	// (e.g., `https://host:8443/path` left as `host:8443/path`).
	if i := strings.Index(u, "@"); i >= 0 {
		// Drop the `user@` prefix entirely — user component (typically `git`)
		// is irrelevant to project identity.
		rest := u[i+1:]
		if j := strings.Index(rest, ":"); j >= 0 {
			rest = rest[:j] + "/" + rest[j+1:]
		}
		u = rest
	}
	// Order matters: trim trailing slashes BEFORE `.git` suffix so the
	// `.git/` combo canonicalizes the same as `.git` alone.
	u = strings.TrimRight(u, "/")
	u = strings.TrimSuffix(u, ".git")
	return u
}

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
	// Equality is canonical — SSH and HTTPS clones of the same repo are
	// treated as equal per H-29. The error message surfaces the verbatim
	// strings so users see exactly what mismatched, not the canonical form.
	if rules.RemoteURL != "" && canonicalizeRemoteURL(rules.RemoteURL) != canonicalizeRemoteURL(remoteURL) {
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

// ExtensionsValidationError carries one finding from ValidateExtensions.
// Severity is "error" (blocks cl-index --validate exit-zero) or "warning"
// (non-blocking; surfaced for review). Class names the offending taxonomy
// bucket; Name names the offending basename; Rule is the human-readable
// explanation.
type ExtensionsValidationError struct {
	Project  string
	Class    string
	Name     string
	Severity string
	Rule     string
}

func (e *ExtensionsValidationError) Error() string {
	return fmt.Sprintf("extensions[%s].%s/%q [%s]: %s", e.Project, e.Class, e.Name, e.Severity, e.Rule)
}

// extensionsBasenameRE constrains extension declarations to safe
// path-segment basenames. No `/` (path traversal), no leading `.`
// (dotfiles outside HIDE list), no consecutive dots or trailing dot
// (per filesystem-portability + readability).
var extensionsBasenameRE = regexp.MustCompile(`^[a-z][a-z0-9-]*(\.[a-z0-9]+)*$`)

// ValidateExtensions checks the Rules.Extensions block for semantic
// validity per Plan-doc §1.1 rules:
//
//  1. brain_duo_operational non-empty when ProjectName != "bot-hq" → error
//  2. each basename matches extensionsBasenameRE → error if not
//  3. duplicate basename across any 2 classes → error
//  4. declared basename not present on disk (under canonRoot/projects/<p>/) → warning
//
// canonRoot is the CL root (e.g., ~/.bot-hq). Empty canonRoot disables
// rule #4 (on-disk check skipped — useful for parse-only tests).
//
// Returns a flat slice of findings. Caller filters by Severity.
func (r *Rules) ValidateExtensions(canonRoot string) []ExtensionsValidationError {
	var out []ExtensionsValidationError
	proj := r.ProjectName

	// Rule 1: brain_duo_operational is bot-hq-only.
	if len(r.Extensions.BrainDuoOperational) > 0 && proj != "bot-hq" {
		for _, name := range r.Extensions.BrainDuoOperational {
			out = append(out, ExtensionsValidationError{
				Project: proj, Class: "brain_duo_operational", Name: name,
				Severity: "error",
				Rule:     "brain_duo_operational class is bot-hq meta-project privilege; other projects cannot declare it",
			})
		}
	}

	// Rule 2: filename convention per class.
	classes := map[string][]string{
		"universal_opt_in":      r.Extensions.UniversalOptIn,
		"external_docs_pointer": r.Extensions.ExternalDocsPointer,
		"brain_duo_operational": r.Extensions.BrainDuoOperational,
		"foundational_anchors":  r.Extensions.FoundationalAnchors,
	}
	for class, names := range classes {
		for _, name := range names {
			if !extensionsBasenameRE.MatchString(name) {
				out = append(out, ExtensionsValidationError{
					Project: proj, Class: class, Name: name,
					Severity: "error",
					Rule:     "basename must match ^[a-z][a-z0-9-]*(\\.[a-z0-9]+)*$ (no `/`, no `..`, no leading `.`)",
				})
			}
		}
	}

	// Rule 3: duplicate basenames across classes.
	seen := make(map[string]string) // basename -> first class
	for class, names := range classes {
		for _, name := range names {
			if prev, ok := seen[name]; ok && prev != class {
				out = append(out, ExtensionsValidationError{
					Project: proj, Class: class, Name: name,
					Severity: "error",
					Rule:     fmt.Sprintf("duplicate basename %q already declared in class %q", name, prev),
				})
			} else if !ok {
				seen[name] = class
			}
		}
	}

	// Rule 4: declared-not-on-disk (warning). Requires canonRoot.
	if canonRoot != "" && proj != "" {
		projDir := filepath.Join(canonRoot, "projects", proj)
		for class, names := range classes {
			for _, name := range names {
				path := filepath.Join(projDir, name)
				if _, err := os.Stat(path); err != nil {
					if os.IsNotExist(err) {
						out = append(out, ExtensionsValidationError{
							Project: proj, Class: class, Name: name,
							Severity: "warning",
							Rule:     fmt.Sprintf("declared but not present on disk at %s", path),
						})
					}
				}
			}
		}
	}

	return out
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
