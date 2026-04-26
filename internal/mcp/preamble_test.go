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

	mustNot := []string{"PUSH POLICY", "BLOCKED COMMANDS", "BRANCH NAMING"}
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
	if strings.Contains(got, "BLOCKED COMMANDS") {
		t.Error("empty CoderToolsBlocked should not include BLOCKED COMMANDS")
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
		"BLOCKED COMMANDS",
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
	// PUSH POLICY before FORCE-PUSH POLICY before BLOCKED COMMANDS before BRANCH NAMING.
	// Deterministic so coders see the same order across spawns.
	rules := &projects.Rules{
		BranchPattern:        ".*",
		PushRequiresApproval: true,
		ForcePushBlocked:     true,
		CoderToolsBlocked:    []string{"foo"},
	}
	got := buildCoderPreamble("ord", "", rules)

	pushIdx := strings.Index(got, "PUSH POLICY:")
	forceIdx := strings.Index(got, "FORCE-PUSH POLICY:")
	toolIdx := strings.Index(got, "BLOCKED COMMANDS:")
	branchIdx := strings.Index(got, "BRANCH NAMING:")

	if pushIdx < 0 || forceIdx < 0 || toolIdx < 0 || branchIdx < 0 {
		t.Fatal("all four sections should be present")
	}
	if !(pushIdx < forceIdx && forceIdx < toolIdx && toolIdx < branchIdx) {
		t.Errorf("policy order wrong: push=%d force=%d tool=%d branch=%d (want push<force<tool<branch)",
			pushIdx, forceIdx, toolIdx, branchIdx)
	}
}

// TestBuildCoderPreamble_ForcePushPolicyShown asserts the H-13 FORCE-PUSH POLICY
// section appears when rules.ForcePushBlocked is true. Pairs with
// protocol.H13ForcePushProtocol embedded in Brian's prompt — the coder side
// is the request shape; Brian side is the verification authority.
func TestBuildCoderPreamble_ForcePushPolicyShown(t *testing.T) {
	rules := &projects.Rules{ForcePushBlocked: true}
	got := buildCoderPreamble("fp1", "", rules)

	required := []string{
		"FORCE-PUSH POLICY:",
		"HARD-BLOCKED",
		"--force",
		"--force-with-lease",
		"request_force_push: <branch>@<sha>",
		"WAIT for brian",
		"Do NOT attempt to construct or guess the token",
	}
	for _, s := range required {
		if !strings.Contains(got, s) {
			t.Errorf("FORCE-PUSH POLICY section missing %q\nfull:\n%s", s, got)
		}
	}
}

// TestBuildCoderPreamble_ForcePushPolicyHidden asserts that lenient rules
// (ForcePushBlocked=false, e.g. bot-hq self-rules) suppress the FORCE-PUSH
// POLICY section. bot-hq force-pushes freely during phase rebases.
func TestBuildCoderPreamble_ForcePushPolicyHidden(t *testing.T) {
	rules := &projects.Rules{
		ProjectName:      "bot-hq",
		ForcePushBlocked: false,
	}
	got := buildCoderPreamble("fp2", "", rules)

	if strings.Contains(got, "FORCE-PUSH POLICY") {
		t.Error("ForcePushBlocked=false should NOT emit FORCE-PUSH POLICY (bot-hq lenient case)")
	}
}

// TestBuildCoderPreamble_WorktreeAndStrictRulesCombined locks the combined
// flow: a coder spawned in a worktree (bot-hq self-spawn) under strict
// rules (e.g. dispatched into a client project that happens to be cloned
// inside bot-hq's worktree dir) sees both the worktree note AND every
// applicable policy section. Per Rain msg 3294 obs #3.
func TestBuildCoderPreamble_WorktreeAndStrictRulesCombined(t *testing.T) {
	worktreeNote := `
NOTE: You are working in a git worktree at /tmp/wt-strict (branch: 346-test).
This is an isolated copy.
`
	rules := &projects.Rules{
		ProjectName:          "bcc-ad-manager",
		BranchPattern:        "^[0-9]+-[a-z0-9-]+$",
		PushRequiresApproval: true,
		ForcePushBlocked:     true,
		CoderToolsBlocked:    []string{"git push", "rm -rf"},
	}
	got := buildCoderPreamble("combined", worktreeNote, rules)

	required := []string{
		// Baseline always present
		"coder agent (ID: combined)",
		"Your task:",
		// Worktree note flowed through
		"git worktree at /tmp/wt-strict",
		"branch: 346-test",
		// All four conditional policy sections present
		"PUSH POLICY:",
		"FORCE-PUSH POLICY:",
		"BLOCKED COMMANDS:",
		"BRANCH NAMING:",
		// Strict rules content surfaced
		"  - git push",
		"  - rm -rf",
		"^[0-9]+-[a-z0-9-]+$",
	}
	for _, s := range required {
		if !strings.Contains(got, s) {
			t.Errorf("combined preamble missing %q\nfull:\n%s", s, got)
		}
	}
}
