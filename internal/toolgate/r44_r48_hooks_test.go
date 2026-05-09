package toolgate

import (
	"strings"
	"testing"
)

func TestR44AntiCrossCheck_blocksHypothesisOwnerInvestigating(t *testing.T) {
	v := R44AntiCrossCheck("brian", "brian", "investigate")
	if !v.ShouldBlock {
		t.Error("expected block when agent == hypothesis-owner")
	}
	if v.Rule != "R44" {
		t.Errorf("rule = %q, want R44", v.Rule)
	}
	if !strings.Contains(v.Reason, "anti-confirmation-bias") {
		t.Errorf("reason missing 'anti-confirmation-bias': %s", v.Reason)
	}
}

func TestR44AntiCrossCheck_allowsPeerInvestigating(t *testing.T) {
	v := R44AntiCrossCheck("rain", "brian", "investigate")
	if v.ShouldBlock {
		t.Error("peer investigating should be allowed")
	}
}

func TestR44AntiCrossCheck_allowsOwnerOnNonInvestigateOps(t *testing.T) {
	v := R44AntiCrossCheck("brian", "brian", "navigate")
	if v.ShouldBlock {
		t.Error("non-investigate operation should not block (owner can navigate own hypothesis)")
	}
}

func TestR44AntiCrossCheck_emptyContextNoBlock(t *testing.T) {
	v := R44AntiCrossCheck("", "brian", "investigate")
	if v.ShouldBlock {
		t.Error("empty agent should not block")
	}
}

func TestR45ModeTagCheck_blocksWriteInVerifyMode(t *testing.T) {
	t.Setenv("BOT_HQ_AGENT_MODE", "implement-verify")
	for _, tool := range []string{"Edit", "Write", "NotebookEdit"} {
		v := R45ModeTagCheck(tool)
		if !v.ShouldBlock {
			t.Errorf("tool %s in verify mode should block", tool)
		}
	}
}

func TestR45ModeTagCheck_allowsReadInVerifyMode(t *testing.T) {
	t.Setenv("BOT_HQ_AGENT_MODE", "implement-verify")
	for _, tool := range []string{"Read", "Bash", "Grep"} {
		v := R45ModeTagCheck(tool)
		if v.ShouldBlock {
			t.Errorf("tool %s should be allowed in verify mode (read-only)", tool)
		}
	}
}

func TestR45ModeTagCheck_allowsWriteInImplementMode(t *testing.T) {
	t.Setenv("BOT_HQ_AGENT_MODE", "implement")
	v := R45ModeTagCheck("Write")
	if v.ShouldBlock {
		t.Error("Write should be allowed in implement mode")
	}
}

func TestR45ModeTagCheck_unsetModeNoBlock(t *testing.T) {
	t.Setenv("BOT_HQ_AGENT_MODE", "")
	v := R45ModeTagCheck("Write")
	if v.ShouldBlock {
		t.Error("unset mode should not block (defensive)")
	}
}

func TestR46ToolMediatedConvergenceCheck_warnsOnSkip(t *testing.T) {
	v := R46ToolMediatedConvergenceCheck("implement", true)
	if v.ShouldBlock {
		t.Error("R46 should warn-only, not block")
	}
	if v.Reason == "" {
		t.Error("R46 should produce warning reason when checks skipped")
	}
}

func TestR46ToolMediatedConvergenceCheck_silentOnNonImplement(t *testing.T) {
	v := R46ToolMediatedConvergenceCheck("investigate", true)
	if v.Reason != "" {
		t.Errorf("R46 should be silent on non-implement; got reason: %s", v.Reason)
	}
}

func TestR46ToolMediatedConvergenceCheck_silentOnChecksRan(t *testing.T) {
	v := R46ToolMediatedConvergenceCheck("implement", false)
	if v.Reason != "" {
		t.Errorf("R46 should be silent when checks ran; got reason: %s", v.Reason)
	}
}

func TestR47DecisionClassTagCheck_blocksUntaggedHighStakes(t *testing.T) {
	for _, op := range []string{"scope-lock-write", "push", "amend", "force-push", "merge"} {
		v := R47DecisionClassTagCheck(op, "")
		if !v.ShouldBlock {
			t.Errorf("untagged %s should block", op)
		}
	}
}

func TestR47DecisionClassTagCheck_allowsTaggedHighStakes(t *testing.T) {
	for _, class := range []string{"medium", "high"} {
		v := R47DecisionClassTagCheck("push", class)
		if v.ShouldBlock {
			t.Errorf("push tagged %s should be allowed", class)
		}
	}
}

func TestR47DecisionClassTagCheck_blocksLowTaggedHighStakes(t *testing.T) {
	v := R47DecisionClassTagCheck("push", "low")
	if !v.ShouldBlock {
		t.Error("push tagged 'low' should block (high-stakes ops require medium|high)")
	}
}

func TestR47DecisionClassTagCheck_allowsNonHighStakes(t *testing.T) {
	v := R47DecisionClassTagCheck("read-doc", "")
	if v.ShouldBlock {
		t.Error("non-high-stakes op should not block regardless of tag")
	}
}

func TestR48UserGreenflagGateCheck_blocksAgentGreenflag(t *testing.T) {
	t.Setenv("BOT_HQ_GREENFLAG_PRE_DELEGATED", "")
	v := R48UserGreenflagGateCheck("brian", "greenflag-fire")
	if !v.ShouldBlock {
		t.Error("agent-initiated greenflag without pre-delegation should block")
	}
}

func TestR48UserGreenflagGateCheck_allowsPreDelegatedGreenflag(t *testing.T) {
	t.Setenv("BOT_HQ_GREENFLAG_PRE_DELEGATED", "msg-17146")
	v := R48UserGreenflagGateCheck("brian", "greenflag-fire")
	if v.ShouldBlock {
		t.Error("agent-initiated greenflag WITH pre-delegation env-var should be allowed")
	}
}

func TestR48UserGreenflagGateCheck_silentOnNonGreenflag(t *testing.T) {
	v := R48UserGreenflagGateCheck("brian", "investigate")
	if v.ShouldBlock {
		t.Error("non-greenflag op should not block")
	}
}

func TestFormatHookVerdict_passWarnBlock(t *testing.T) {
	cases := []struct {
		v    HookVerdict
		want string
	}{
		{HookVerdict{Rule: "R44"}, "[R44] PASS: ok"},
		{HookVerdict{Rule: "R45", Reason: "warning text"}, "[R45] WARN: warning text"},
		{HookVerdict{Rule: "R46", ShouldBlock: true, Reason: "block text"}, "[R46] BLOCK: block text"},
	}
	for _, tc := range cases {
		got := FormatHookVerdict(tc.v)
		if got != tc.want {
			t.Errorf("FormatHookVerdict(%+v) = %q, want %q", tc.v, got, tc.want)
		}
	}
}

func TestAllHooksSummary_combinesVerdicts(t *testing.T) {
	out := AllHooksSummary(
		HookVerdict{Rule: "R44"},
		HookVerdict{Rule: "R45", ShouldBlock: true, Reason: "block"},
		HookVerdict{Rule: "R46", Reason: "warn"},
	)
	if !strings.Contains(out, "[R44] PASS") {
		t.Errorf("missing R44 pass; got: %s", out)
	}
	if !strings.Contains(out, "[R45] BLOCK") {
		t.Errorf("missing R45 block; got: %s", out)
	}
	if !strings.Contains(out, "[R46] WARN") {
		t.Errorf("missing R46 warn; got: %s", out)
	}
}
