package mcp

import (
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/projects"
)

func TestBuildCoderPreamble_BaselineAlwaysIncluded(t *testing.T) {
	got := buildCoderPreamble("abc123", "", nil)

	must := []string{
		"coder agent (ID: abc123)",
		"bot-hq MCP tools available",
		`hub_send(from="abc123", to="brian"`,
		"Your task:",
	}
	for _, s := range must {
		if !strings.Contains(got, s) {
			t.Errorf("preamble missing baseline fragment %q\nfull:\n%s", s, got)
		}
	}
}

func TestBuildCoderPreamble_NilRulesEmitsNoPolicy(t *testing.T) {
	got := buildCoderPreamble("xyz", "", nil)

	mustNot := []string{"PUSH POLICY", "TOOL ALLOWLIST", "BRANCH NAMING"}
	for _, s := range mustNot {
		if strings.Contains(got, s) {
			t.Errorf("nil rules should emit no policy section, but %q present\nfull:\n%s", s, got)
		}
	}
}

func TestBuildCoderPreamble_LenientBotHqRules(t *testing.T) {
	rules := &projects.Rules{
		ProjectName:          "bot-hq",
		BranchPattern:        "^(brian|rain|coder-[a-f0-9]+)/[a-z0-9.-]+$",
		PushRequiresApproval: false,
		ForcePushBlocked:     false,
		CoderToolsBlocked:    nil,
	}
	got := buildCoderPreamble("s1", "", rules)

	if strings.Contains(got, "PUSH POLICY") {
		t.Error("lenient rules (no push approval) should not include PUSH POLICY")
	}
	if strings.Contains(got, "TOOL ALLOWLIST") {
		t.Error("empty CoderToolsBlocked should not include TOOL ALLOWLIST")
	}
	// Branch pattern is set, so naming guidance should appear.
	if !strings.Contains(got, "BRANCH NAMING") {
		t.Error("non-empty BranchPattern should produce BRANCH NAMING section")
	}
	if !strings.Contains(got, "brian|rain|coder-") {
		t.Error("BRANCH NAMING section should include the pattern")
	}
}

func TestBuildCoderPreamble_StrictRules(t *testing.T) {
	rules := &projects.Rules{
		ProjectName:          "bcc-ad-manager",
		BranchPattern:        "^[0-9]+-[a-z0-9-]+$",
		BranchPatternHelp:    "Use [issueNo]-[title-with-dashes]; lowercase only",
		BranchExamples:       []string{"346-streamline-onboarding", "355-duplicate-fix"},
		PushRequiresApproval: true,
		CoderToolsBlocked:    []string{"git push", "gh pr create", "rm -rf"},
	}
	got := buildCoderPreamble("strict1", "", rules)

	required := []string{
		"PUSH POLICY",
		"explicit user approval before any git push",
		"awaiting approval",
		"TOOL ALLOWLIST",
		"  - git push",
		"  - gh pr create",
		"  - rm -rf",
		"BRANCH NAMING",
		"^[0-9]+-[a-z0-9-]+$",
		"Hint: Use [issueNo]-[title-with-dashes]",
		"Examples: 346-streamline-onboarding, 355-duplicate-fix",
	}
	for _, s := range required {
		if !strings.Contains(got, s) {
			t.Errorf("strict rules preamble missing %q\nfull:\n%s", s, got)
		}
	}
}

func TestBuildCoderPreamble_WorktreeNoteFlowsThrough(t *testing.T) {
	worktreeNote := `
NOTE: You are working in a git worktree at /tmp/wt (branch: coder-abc).
This is an isolated copy.
`
	got := buildCoderPreamble("wt1", worktreeNote, nil)

	if !strings.Contains(got, "git worktree at /tmp/wt") {
		t.Error("worktree note should flow into preamble")
	}
	if !strings.Contains(got, "branch: coder-abc") {
		t.Error("worktree branch name should flow into preamble")
	}
}

func TestBuildCoderPreamble_PolicyOrderingDeterministic(t *testing.T) {
	// PUSH POLICY before TOOL ALLOWLIST before BRANCH NAMING — deterministic
	// so coders see the same order across spawns.
	rules := &projects.Rules{
		BranchPattern:        ".*",
		PushRequiresApproval: true,
		CoderToolsBlocked:    []string{"foo"},
	}
	got := buildCoderPreamble("ord", "", rules)

	pushIdx := strings.Index(got, "PUSH POLICY")
	toolIdx := strings.Index(got, "TOOL ALLOWLIST")
	branchIdx := strings.Index(got, "BRANCH NAMING")

	if pushIdx < 0 || toolIdx < 0 || branchIdx < 0 {
		t.Fatal("all three sections should be present")
	}
	if !(pushIdx < toolIdx && toolIdx < branchIdx) {
		t.Errorf("policy order wrong: push=%d tool=%d branch=%d (want push<tool<branch)", pushIdx, toolIdx, branchIdx)
	}
}
