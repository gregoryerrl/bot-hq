package projects

import (
	"strings"
	"testing"

	"gopkg.in/yaml.v3"
)

// TestNormalize_FlatToNested: legacy flat-form input → canonical nested
// output. Covers the load-bearing migration case.
func TestNormalize_FlatToNested(t *testing.T) {
	flatIn := []byte(`
project_name: bot-hq
remote_url: git@github.com:gregoryerrl/bot-hq.git
branch_pattern: feat/*
branch_examples:
  - feat/foo
  - feat/bar
push_requires_approval: true
force_push_blocked: true
force_push_token_format: "force-push <branch>"
coder_tools_blocked:
  - rm
  - dd
commit_style: imperative-mood
require_issue_link: true
`)

	out, err := Normalize(flatIn)
	if err != nil {
		t.Fatalf("Normalize: %v", err)
	}
	s := string(out)

	for _, want := range []string{
		"project_name: bot-hq",
		"remote_url: git@github.com:gregoryerrl/bot-hq.git",
		"branch:",
		"pattern: feat/*",
		"commit:",
		"style: imperative-mood",
		"requireIssueLink: true",
		"gates:",
		"push:",
		"requiresApproval: true",
		"forcePush:",
		"blocked: true",
		"tokenFormat: force-push <branch>",
		"coder:",
		"toolsBlocked:",
		"- rm",
	} {
		if !strings.Contains(s, want) {
			t.Errorf("normalized output missing %q.\nGot:\n%s", want, s)
		}
	}

	// Negative: flat keys MUST NOT appear in canonical output.
	for _, banned := range []string{
		"branch_pattern:",
		"push_requires_approval:",
		"force_push_blocked:",
		"coder_tools_blocked:",
		"commit_style:",
		"require_issue_link:",
	} {
		if strings.Contains(s, banned) {
			t.Errorf("normalized output still contains flat key %q.\nGot:\n%s", banned, s)
		}
	}
}

// TestNormalize_NestedPassesThrough: already-canonical input →
// equivalent canonical output. Round-trip stability.
func TestNormalize_NestedPassesThrough(t *testing.T) {
	nestedIn := []byte(`
project_name: example
remote_url: git@github.com:foo/bar.git
branch:
  pattern: main
commit:
  style: imperative-mood
gates:
  push:
    requiresApproval: true
`)

	out, err := Normalize(nestedIn)
	if err != nil {
		t.Fatalf("Normalize: %v", err)
	}

	// Decode both, compare semantic equivalence via Rules.
	var inRules, outRules Rules
	if err := yaml.Unmarshal(nestedIn, &inRules); err != nil {
		t.Fatalf("decode in: %v", err)
	}
	if err := yaml.Unmarshal(out, &outRules); err != nil {
		t.Fatalf("decode out: %v", err)
	}
	if inRules.ProjectName != outRules.ProjectName ||
		inRules.RemoteURL != outRules.RemoteURL ||
		inRules.BranchPattern != outRules.BranchPattern ||
		inRules.CommitStyle != outRules.CommitStyle ||
		inRules.PushRequiresApproval != outRules.PushRequiresApproval {
		t.Errorf("round-trip lost data:\nin = %+v\nout = %+v", inRules, outRules)
	}
}

// TestNormalize_Idempotent: Normalize(Normalize(x)) == Normalize(x).
// Critical property — webui write-handler may invoke normalize on every
// save, and repeated applications must converge.
func TestNormalize_Idempotent(t *testing.T) {
	inputs := []string{
		"project_name: a\npush_requires_approval: true\n",
		"project_name: b\nbranch:\n  pattern: main\n",
		"project_name: c\ngates:\n  push:\n    requiresApproval: false\nbranch_pattern: trunk\n", // mixed dual-form
		"",
	}
	for _, in := range inputs {
		first, err := Normalize([]byte(in))
		if err != nil {
			t.Fatalf("first normalize of %q: %v", in, err)
		}
		second, err := Normalize(first)
		if err != nil {
			t.Fatalf("second normalize of %q: %v", in, err)
		}
		if string(first) != string(second) {
			t.Errorf("not idempotent for input %q:\nfirst:\n%s\nsecond:\n%s", in, string(first), string(second))
		}
	}
}

// TestNormalize_DualFormNestedWins: when both flat + nested set, the
// nested value wins (matches Rules.UnmarshalYAML precedence). Verifies
// Normalize honors the same precedence so users hand-editing flat form
// alongside an existing nested don't silently lose their nested value
// on next save.
func TestNormalize_DualFormNestedWins(t *testing.T) {
	dualIn := []byte(`
project_name: dual
push_requires_approval: false
gates:
  push:
    requiresApproval: true
`)

	out, err := Normalize(dualIn)
	if err != nil {
		t.Fatalf("Normalize: %v", err)
	}

	var r Rules
	if err := yaml.Unmarshal(out, &r); err != nil {
		t.Fatalf("decode out: %v", err)
	}
	if !r.PushRequiresApproval {
		t.Errorf("nested-wins violated: PushRequiresApproval = false, want true.\nOut:\n%s", string(out))
	}
}

// TestNormalize_EmptyInput: empty/whitespace input → returned as-is.
// Avoids producing artificial "{}\n"-style stubs.
func TestNormalize_EmptyInput(t *testing.T) {
	for _, in := range []string{"", "   ", "\n\n"} {
		out, err := Normalize([]byte(in))
		if err != nil {
			t.Errorf("empty input %q errored: %v", in, err)
		}
		if string(out) != in {
			t.Errorf("empty input %q changed: got %q", in, string(out))
		}
	}
}

// TestNormalize_BadYAML: malformed YAML → error, not silent normalize.
// Guards write-handler against persisting garbage.
func TestNormalize_BadYAML(t *testing.T) {
	badIn := []byte("project_name: [unclosed-bracket\n")
	if _, err := Normalize(badIn); err == nil {
		t.Errorf("expected error on malformed YAML; got nil")
	}
}

// TestNormalize_OmitsEmptyCategories: input with no gates/branch/commit
// values → output has no empty `gates: {}` / `branch: {}` / `commit:
// {}` stub keys.
func TestNormalize_OmitsEmptyCategories(t *testing.T) {
	in := []byte("project_name: minimal\n")
	out, err := Normalize(in)
	if err != nil {
		t.Fatalf("Normalize: %v", err)
	}
	s := string(out)
	for _, banned := range []string{"gates:", "branch:", "commit:"} {
		if strings.Contains(s, banned) {
			t.Errorf("minimal input emitted empty category %q.\nGot:\n%s", banned, s)
		}
	}
	if !strings.Contains(s, "project_name: minimal") {
		t.Errorf("project_name lost.\nGot:\n%s", s)
	}
}
