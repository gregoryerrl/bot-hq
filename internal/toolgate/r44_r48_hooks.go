// Package toolgate — r44_r48_hooks.go: R44/R45/R46/R47/R48 mechanical-
// enforcement hooks per phase-t.md v5 T-2.5.
//
// Each hook implements one R-rule's mechanical-enforcement layer.
// All follow the same template established by R49 (r49.go) + R50
// (outboundhook/baredot.go): inspect tool input, decide block-vs-allow,
// emit diagnostic to stderr.
//
// Hooks:
//   - R44: anti-cross PreToolUse hook (bilateral-Investigate strong-style
//     anti-confirmation-bias enforcement; navigator who proposes hypothesis
//     H cannot be driver investigating H)
//   - R45: mode-tag PreToolUse hook (mode-conflation detection; agent
//     mode-state matches tool-permission profile)
//   - R46: tool-mediated-convergence audit (Implement-class operations
//     run mechanical-discipline-tool checks; warn if check skipped)
//   - R47: decision-class-tagging hook (high-stakes operations require
//     explicit decision-class tag; bilateral routing per R47 revised)
//   - R48: cognitive-diversity at user-gates (greenflag-gate operations
//     are user-only; agent-attempted greenflag blocked)
//
// All hooks accept a HookInput-like generic context + return Verdict
// (Block + Reason) to allow the caller to integrate with PreToolUse
// hook chain or in-process discipline-machinery.

package toolgate

import (
	"fmt"
	"os"
	"strings"
)

// HookVerdict is the generic result of an R-rule mechanical-enforcement
// check. ShouldBlock=true halts the tool call; Reason is the human-
// readable diagnostic surfaced to stderr.
type HookVerdict struct {
	ShouldBlock bool
	Reason      string
	Rule        string // "R44" | "R45" | etc.
}

// R44AntiCrossCheck implements R44 BILATERAL-INVESTIGATION-DISCIPLINE
// strong-style anti-confirmation-bias enforcement. When bilateral-fires
// per R44 expanded, the agent who proposes hypothesis H (navigator) MUST
// NOT be the driver investigating H.
//
// Inputs:
//   - agentID: invoking agent ("brian" | "rain")
//   - hypothesisOwner: agent who proposed the active hypothesis
//   - operation: "investigate" | "navigate" | other
//
// Returns Block when agentID == hypothesisOwner AND operation == "investigate".
func R44AntiCrossCheck(agentID, hypothesisOwner, operation string) HookVerdict {
	if operation != "investigate" {
		return HookVerdict{Rule: "R44"}
	}
	if agentID == "" || hypothesisOwner == "" {
		return HookVerdict{Rule: "R44"}
	}
	if agentID == hypothesisOwner {
		return HookVerdict{
			Rule:        "R44",
			ShouldBlock: true,
			Reason: fmt.Sprintf("R44 BILATERAL-INVESTIGATION strong-style anti-confirmation-bias VIOLATION: agent %q is hypothesis-owner; cannot drive investigation of own hypothesis. Reassign to peer or escalate per cl_strong_style_assign.", agentID),
		}
	}
	return HookVerdict{Rule: "R44"}
}

// R45ModeTagCheck implements R45 ROLE-MODE-SWITCH-DISCIPLINE mode-tag
// enforcement. Tool calls fired during a mode must match the tool-
// permission profile for that mode. Mode is read from env-var
// BOT_HQ_AGENT_MODE (set at mode-transition by IPAV state machine).
//
// Restricted modes (subset; full enforcement is T-2 expansion):
//   - "implement-verify" (Rain): must NOT use Edit/Write tools (Verify is
//     read-only adversarial review; modifications are Brian-HANDS only)
//   - "plan-verify" (Rain): same restriction
func R45ModeTagCheck(toolName string) HookVerdict {
	mode := os.Getenv("BOT_HQ_AGENT_MODE")
	if mode == "" {
		return HookVerdict{Rule: "R45"}
	}
	verifyModes := map[string]bool{
		"implement-verify": true,
		"plan-verify":      true,
	}
	if !verifyModes[mode] {
		return HookVerdict{Rule: "R45"}
	}
	writeTools := map[string]bool{
		"Edit":         true,
		"Write":        true,
		"NotebookEdit": true,
	}
	if writeTools[toolName] {
		return HookVerdict{
			Rule:        "R45",
			ShouldBlock: true,
			Reason: fmt.Sprintf("R45 ROLE-MODE-SWITCH-DISCIPLINE VIOLATION: tool %q rejected in mode %q (Verify-class is read-only adversarial review per phase-t.md v5 R45 EXTENDED). Recovery: switch to implement-mode OR delegate write to Brian-HANDS.", toolName, mode),
		}
	}
	return HookVerdict{Rule: "R45"}
}

// R46ToolMediatedConvergenceCheck implements R46 SINGLE-AUTHOR-WITH-
// TOOL-MEDIATED-CONVERGENCE audit. Implement-class operations should
// run mechanical-discipline-tool checks (cite-anchor validation +
// scope-lock validation + R-rule compliance hooks). This hook warns
// when an Implement-class operation skips the mechanical-tool check.
//
// Inputs:
//   - operation: "implement" | other
//   - mechanicalChecksSkipped: true if pre-flight discipline-tool checks were not run
//
// Returns warn-only HookVerdict (informational; no block) per R46
// graduate-on-mechanical-tooling-catches-90% empirical-criteria.
func R46ToolMediatedConvergenceCheck(operation string, mechanicalChecksSkipped bool) HookVerdict {
	if operation != "implement" || !mechanicalChecksSkipped {
		return HookVerdict{Rule: "R46"}
	}
	return HookVerdict{
		Rule:   "R46",
		Reason: "R46 SINGLE-AUTHOR-WITH-TOOL-MEDIATED-CONVERGENCE audit: Implement-class operation skipped mechanical-discipline-tool checks. Recommendation: run cite-anchor validation + scope-lock check before completion (informational; not blocking).",
	}
}

// R47DecisionClassTagCheck implements R47 BILATERAL-ON-DEMAND-DISCIPLINE
// decision-class-tag enforcement. High-stakes operations require explicit
// decision-class tag (low/medium/high) per R47 revised. Operations classed
// as scope-lock-doc-write OR push-class without tag are flagged for
// bilateral routing.
//
// Inputs:
//   - operation: e.g. "scope-lock-write" | "push" | "amend" | other
//   - decisionClass: "low" | "medium" | "high" | "" (untagged)
//
// Returns Block when operation is high-stakes-class AND decisionClass is empty.
func R47DecisionClassTagCheck(operation, decisionClass string) HookVerdict {
	highStakesOps := map[string]bool{
		"scope-lock-write": true,
		"push":             true,
		"amend":            true,
		"force-push":       true,
		"merge":            true,
	}
	if !highStakesOps[operation] {
		return HookVerdict{Rule: "R47"}
	}
	if decisionClass == "high" || decisionClass == "medium" {
		return HookVerdict{Rule: "R47"}
	}
	return HookVerdict{
		Rule:        "R47",
		ShouldBlock: true,
		Reason: fmt.Sprintf("R47 BILATERAL-ON-DEMAND-DISCIPLINE VIOLATION: operation %q is high-stakes-class but decision-class is %q (require medium|high). Tag via cl_decision_class_tag + route to bilateral per R47 revised.", operation, decisionClass),
	}
}

// R48UserGreenflagGateCheck implements R48 GENUINE-COGNITIVE-DIVERSITY-
// AT-USER-GATES enforcement. Greenflag-gate operations (e.g. GG-N
// transitions, sub-phase-fire) are user-only per R48; agent-initiated
// greenflag is blocked.
//
// Inputs:
//   - agentID: agent attempting the greenflag-fire
//   - operation: "greenflag-fire" | "GG-N-fire" | other
//
// Returns Block when an agent attempts a greenflag-fire (user is the only
// authorized greenflag-issuer per R48 partial-supersede).
func R48UserGreenflagGateCheck(agentID, operation string) HookVerdict {
	greenflagOps := map[string]bool{
		"greenflag-fire": true,
		"GG-N-fire":      true,
		"sub-phase-fire": true,
	}
	if !greenflagOps[operation] {
		return HookVerdict{Rule: "R48"}
	}
	// User pre-delegation per phase-t.md v5: agent-initiated allowed when
	// pre-delegation env-var is set (e.g. msg 17146 directive).
	if os.Getenv("BOT_HQ_GREENFLAG_PRE_DELEGATED") != "" {
		return HookVerdict{
			Rule:   "R48",
			Reason: fmt.Sprintf("R48 user-greenflag bypass via pre-delegation env-var (set by user-directive class). Agent %q permitted to fire %q.", agentID, operation),
		}
	}
	return HookVerdict{
		Rule:        "R48",
		ShouldBlock: true,
		Reason: fmt.Sprintf("R48 GENUINE-COGNITIVE-DIVERSITY-AT-USER-GATES VIOLATION: agent %q attempted %q without user-pre-delegation. Greenflag-gate operations are user-only per R48 partial-supersede.", agentID, operation),
	}
}

// FormatHookVerdict produces a structured stderr-friendly diagnostic
// string for any HookVerdict. Used by CLI integration + test diagnostic.
func FormatHookVerdict(v HookVerdict) string {
	prefix := "PASS"
	if v.ShouldBlock {
		prefix = "BLOCK"
	} else if v.Reason != "" {
		prefix = "WARN"
	}
	if v.Reason == "" {
		return fmt.Sprintf("[%s] %s: ok", v.Rule, prefix)
	}
	return fmt.Sprintf("[%s] %s: %s", v.Rule, prefix, v.Reason)
}

// AllHooksSummary applies multiple checks to the same operation context
// and returns a combined-summary string. Useful for diagnostic dashboards
// + audit-trail logging.
func AllHooksSummary(verdicts ...HookVerdict) string {
	var sb strings.Builder
	for i, v := range verdicts {
		if i > 0 {
			sb.WriteString("\n")
		}
		sb.WriteString(FormatHookVerdict(v))
	}
	return sb.String()
}
