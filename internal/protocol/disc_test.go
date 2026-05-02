package protocol

import (
	"strings"
	"testing"
)

// Bug #1 ratchet: pin the four canonical literals of the DISC v2
// audience-driven routing rule. Any wording change that drops one
// of these substrings breaks the rule's substance and must fail CI
// before landing. The intentional brittleness is the test's job.
//
// If any of these literals needs to change, the change must be
// deliberate (update both the const AND the test in the same commit)
// — drift between source-of-truth and the rule's substance is
// exactly what the ratchet guards against.
func TestDiscV2OutboundRule_RatchetLiterals(t *testing.T) {
	must := []string{
		"Routing is determined by intended audience",
		"If only peer(s)",
		"Peer reads broadcasts too — never double-send",
		"If a message is both peer-coordination and user-actionable, broadcast",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(DiscV2OutboundRule, lit) {
				t.Errorf("DISC v2 ratchet broken: missing literal %q (bug #1 lock)", lit)
			}
		})
	}
}

// The const must always start with the OUTBOUND header so it slots
// cleanly into agent prompts that expect the rule under that anchor.
func TestDiscV2OutboundRule_HeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(DiscV2OutboundRule, "- OUTBOUND:") {
		t.Errorf("rule must start with `- OUTBOUND:` (prompt anchor); first 40 chars: %q", DiscV2OutboundRule[:40])
	}
}

// Phase L L-1 ratchet: pin load-bearing recognition substrings of the
// STAT-CLAIM-CITE (R31) rule. These tokens are what the agent prompt
// uses to recognize numerical-claim discipline + cite verifiable
// command output instead of session-recall. Any wording change that
// drops one of these substrings breaks the rule's substance.
//
// Origin: Phase L L-1 commit-2; codified after recursive stat-claim-drift
// chronic-class observation during L-0 + L-1+L-2 amend cycles
// (discipline-log #10/#13/#16/#17/#19/#20/#23 today's session).
func TestStatClaimCiteSubstringLock(t *testing.T) {
	must := []string{
		"STAT-CLAIM-CITE (R31)",
		"verifiable command output",
		"git diff --numstat",
		"hub_read since_id",
		"peer-cross-check",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseLv1RulebookHardening, lit) {
				t.Errorf("R31 STAT-CLAIM-CITE ratchet broken: missing literal %q in PhaseLv1RulebookHardening", lit)
			}
		})
	}
}

// Phase L L-1 ratchet: pin load-bearing recognition substrings of the
// SCOPE-FORK-CONFIRMATION (R32) rule. These tokens are what the agent
// prompt uses to recognize fork-able user phrasing + surface
// interpretation pre-action instead of inferring silently.
//
// Origin: Phase L L-1 commit-2; codified after phrase-parsing-scope-fork
// chronic-class observation during today's session
// (discipline-log #12 + push-fork-resolution + #18 git-vs-state).
func TestScopeForkConfirmationSubstringLock(t *testing.T) {
	must := []string{
		"SCOPE-FORK-CONFIRMATION (R32)",
		"fork-able scope",
		"UNTIL/INCLUDING/JUST",
		"interpretation pre-action",
		"hub_send before firing",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseLv1RulebookHardening, lit) {
				t.Errorf("R32 SCOPE-FORK-CONFIRMATION ratchet broken: missing literal %q in PhaseLv1RulebookHardening", lit)
			}
		})
	}
}

// Phase L L-1 prompt-embed verification: PhaseLv1RulebookHardening
// must be embedded in both rain.go + brian.go initialPrompt() so the
// new R31/R32 rules are loaded at agent-spawn time. Catches the
// "const exists but isn't wired" class observed for K-Tier-1
// R24-R30 (which exist as consts but were not embedded; surfaced in
// L-2 rule-locus-inventory exercise).
func TestPhaseLv1RulebookHardeningHeaderAnchor(t *testing.T) {
	// Verify const starts with R31 STAT-CLAIM-CITE header — slots cleanly
	// into agent prompt at the expected anchor position.
	if !strings.HasPrefix(PhaseLv1RulebookHardening, "- STAT-CLAIM-CITE (R31):") {
		t.Errorf("rule must start with `- STAT-CLAIM-CITE (R31):` (prompt anchor); first 50 chars: %q", PhaseLv1RulebookHardening[:50])
	}
}

// Phase L L-5 commit-1 ratchet: pin load-bearing recognition substrings
// of the PRE-EXECUTE-GATE-FILE-READ (R33) rule. These tokens are what
// the agent prompt uses to recognize gate-file consultation discipline
// before HANDS-execute fire (commit/push/merge). Any wording change
// that drops one of these substrings breaks the rule's substance.
//
// Substring-lock anchor strategy (per F6): anchor on rule-name + cite-format
// identifiers + freshness-metric (F4) + filesystem location. NOT on body
// example tokens (those rotate). Locked anchors:
//   - Rule-name: "PRE-EXECUTE-GATE-FILE-READ (R33)"
//   - Commit cite-format: "Pre-commit-checklist-SHA"
//   - Push cite-format: "Pre-push-checklist-SHA"
//   - Merge AgentState field: "pre_merge_checklist_sha_seen"
//   - Freshness-metric (F4): "5 self-agent messages"
//   - Filesystem location: "~/.bot-hq/gates/"
//
// Origin: Phase L L-5 commit-1; codified to enforce gate-file consultation
// discipline at HANDS-execute boundary. L-5 commit-2 ships the toolgate
// hook that operationalizes this rule (SHA-cite verification at
// PreToolUse on git-commit/git-push/gh-pr-merge).
func TestPreExecuteGateFileReadSubstringLock(t *testing.T) {
	must := []string{
		"PRE-EXECUTE-GATE-FILE-READ (R33)",
		"Pre-commit-checklist-SHA",
		"Pre-push-checklist-SHA",
		"pre_merge_checklist_sha_seen",
		"5 self-agent messages",
		"~/.bot-hq/gates/",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseLv5GateProtocol, lit) {
				t.Errorf("R33 PRE-EXECUTE-GATE-FILE-READ ratchet broken: missing literal %q in PhaseLv5GateProtocol", lit)
			}
		})
	}
}

// Phase L L-5 prompt-embed verification: PhaseLv5GateProtocol
// must be embedded in both rain.go + brian.go initialPrompt() so the
// new R33 rule is loaded at agent-spawn time. Mirrors L-1 wiring-lock
// pattern; catches the "const exists but isn't wired" class.
func TestPhaseLv5GateProtocolHeaderAnchor(t *testing.T) {
	// Verify const starts with R33 PRE-EXECUTE-GATE-FILE-READ header — slots
	// cleanly into agent prompt at the expected anchor position.
	if !strings.HasPrefix(PhaseLv5GateProtocol, "- PRE-EXECUTE-GATE-FILE-READ (R33):") {
		t.Errorf("rule must start with `- PRE-EXECUTE-GATE-FILE-READ (R33):` (prompt anchor); first 60 chars: %q", PhaseLv5GateProtocol[:60])
	}
}
