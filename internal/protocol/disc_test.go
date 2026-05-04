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

// Phase L L-6 commit-1 ratchet: pin load-bearing recognition substrings
// of the PRE-PHASE-CLOSE-RETRO (R34) rule. These tokens are what the
// agent prompt uses to recognize phase-close consultation discipline +
// the AgentState-cite proof mechanism. Any wording change that drops
// one of these substrings breaks the rule's substance.
//
// Substring-lock anchor strategy (mirrors L-5 R33 pattern, per F6):
// anchor on rule-name + AgentState-cite-format identifier +
// graduation-criterion semantic anchor + baseline-vs-final semantic
// anchor + filesystem location + freshness-metric (F4-unified with
// R33). NOT on body example tokens. Locked anchors:
//   - Rule-name: "PRE-PHASE-CLOSE-RETRO (R34)"
//   - AgentState cite-format: "pre_phase_close_checklist_sha_seen"
//   - Graduation-criterion semantic: "graduate-or-deprecate"
//   - Baseline-comparison semantic: "baseline-vs-final"
//   - Filesystem location: "~/.bot-hq/gates/"
//   - Freshness-metric (F4 shared with R33): "5 self-agent messages"
//
// Origin: Phase L L-6 commit-1; codified to enforce phase-close
// consultation discipline at the per-phase boundary. Toolgate
// gate-CHECK deferred to Phase M (low-cadence event; PEER-CROSS-CHECK
// + prompt-rule sufficient for L-6).
//
// NB: anchor "5 self-agent messages" + "~/.bot-hq/gates/" are shared
// with R33 PhaseLv5GateProtocol substring-lock — substrings.Contains
// only checks presence not uniqueness, so the shared anchors are
// independently asserted in PhaseLv6PrePhaseCloseRetro (per Rain
// BRAIN-2nd NB1).
func TestPrePhaseCloseRetroSubstringLock(t *testing.T) {
	must := []string{
		"PRE-PHASE-CLOSE-RETRO (R34)",
		"pre_phase_close_checklist_sha_seen",
		"graduate-or-deprecate",
		"baseline-vs-final",
		"~/.bot-hq/gates/",
		"5 self-agent messages",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseLv6PrePhaseCloseRetro, lit) {
				t.Errorf("R34 PRE-PHASE-CLOSE-RETRO ratchet broken: missing literal %q in PhaseLv6PrePhaseCloseRetro", lit)
			}
		})
	}
}

// Phase L L-6 prompt-embed verification: PhaseLv6PrePhaseCloseRetro
// must be embedded in both rain.go + brian.go initialPrompt() so the
// new R34 rule is loaded at agent-spawn time. Mirrors L-5 wiring-lock
// pattern; catches the "const exists but isn't wired" class.
func TestPhaseLv6PrePhaseCloseRetroHeaderAnchor(t *testing.T) {
	// Verify const starts with R34 PRE-PHASE-CLOSE-RETRO header — slots
	// cleanly into agent prompt at the expected anchor position.
	if !strings.HasPrefix(PhaseLv6PrePhaseCloseRetro, "- PRE-PHASE-CLOSE-RETRO (R34):") {
		t.Errorf("rule must start with `- PRE-PHASE-CLOSE-RETRO (R34):` (prompt anchor); first 60 chars: %q", PhaseLv6PrePhaseCloseRetro[:60])
	}
}

// Phase L L-3b commit-1 ratchet: pin all R1-R23 rule-name substrings
// + the skill-pointer header in PhaseIv1ProtocolHardening. After the
// L-3b commit-1 trim (~1,180-1,650 bytes removed via 14 rule-specific
// edits per L-3a v1.1 audit-doc §5.1), the recognition layer for each
// rule must remain inline + the skill-pointer header anchoring
// agents to /phase-rules-detail must survive any future trim-pass.
//
// Substring-lock anchor strategy (per L-3a v1.1 §5.3 + Brian Q4
// disposition with 24th-anchor enhancement): assert all 23 rule-name
// recognition substrings + the "PHASE-I PROTOCOL HARDENING" skill-pointer
// header. Substring.Contains is presence-not-uniqueness (mirrors L-5/L-6
// pattern); failure-localization via t.Run sub-tests.
//
// The 24th anchor ("PHASE-I PROTOCOL HARDENING") is load-bearing as the
// skill-pointer to ~/.claude/skills/phase-rules-detail/SKILL.md — without
// it, agents lose the explicit cue that prose-detail lives in the skill
// + the relocation pattern itself decays. Per Rain L-3a v1.1 BRAIN-2nd-PASS
// strong-ack on 24th-anchor.
//
// Origin: Phase L L-3b commit-1; codified post-prompt-shrink to ratchet
// recognition-layer survival across future trims. Catches accidental
// rule-deletion-during-trim class at compile-test time.
func TestPhaseIv1RuleNamesPresent(t *testing.T) {
	must := []string{
		"PHASE-I PROTOCOL HARDENING",
		"HANDSHAKE-TERMINATOR",
		"CROSS-TIMING-DEDUP",
		"QUOTE-TRIM",
		"SNAP-GATING",
		"BRAIN-CYCLE-RESPONSE-SHAPE",
		"TOOL-RESULT-DISCIPLINE",
		"SUBAGENT-DISPATCH",
		"COMPACT-COMMIT-FORMAT",
		"AUDIENCE-CLASS-DISCRIMINATOR",
		"SCOPE-LOCK-BEFORE-IMPL",
		"HALT-DISCIPLINE",
		"GATE-PROTOCOL",
		"SCOPE-VERIFY-PRE-DRAFT",
		"HALT-95%-SNAP",
		"AGENT-AUTHORITY-MATRIX",
		"CROSS-RESTART-RESUME-OPERATIONAL",
		"SOURCE-OF-TRUTH-HIERARCHY",
		"CITE-ANCHOR-REQUIRED",
		"CYCLE-CLOSE-USER-BLOCKING",
		"BOOTSTRAP-ON-CONVERSATION-RESUME",
		"PRE-COMPACT-SNAP",
		"HEARTBEAT-LEDGER",
		"MSG-TYPE-TAXONOMY",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseIv1ProtocolHardening, lit) {
				t.Errorf("PhaseIv1 rule-name ratchet broken: missing substring %q in PhaseIv1ProtocolHardening (24-anchor lock; survival across trims)", lit)
			}
		})
	}
}

// Phase M M-1 commit-1 ratchet: pin load-bearing recognition substrings
// of the PRE-FLIGHT-HOOK-CHECK (R35) rule. Anchors are what the agent
// prompt uses to recognize preflight discipline + invocation primitive
// + remediation pointers + caller-invariant ordering.
//
// Substring-lock anchor strategy (mirrors L-5 R33 + L-6 R34 patterns):
// anchor on rule-name + invocation-primitive + env-var name + hook
// target + filesystem location + AFTER-register ordering invariant +
// brian-carve-out condition + Finding-3 remediation invariant + skill
// pointer. NOT on body example tokens or runtime CLI output strings.
//
// Locked anchors:
//   - Rule-name: "PRE-FLIGHT-HOOK-CHECK (R35)"
//   - Invocation primitive: "bot-hq preflight-check"
//   - Bootstrap point: "BOOTSTRAP-ON-CONVERSATION-RESUME"
//   - Env-var name: "BOT_HQ_AGENT_ID"
//   - Hook target: "PreToolUse-Bash"
//   - Substring expectation: "tool-permission-hook"
//   - Caller invariant: "AFTER hub_register"
//   - Brian carve-out: "Rain unreachable >60s"
//   - Finding-3 invariant: "session-restart"
//   - Skill pointer: "/phase-rules-detail skill"
//
// Origin: Phase M M-1 commit-1; codified to enforce preflight self-check
// discipline at first scope-affecting turn-start. Closes Phase L
// Finding-1 (installer-not-run) + Finding-3 (settings.json hot-reload-
// unsupported) recurrence class.
func TestPreFlightHookCheckSubstringLock(t *testing.T) {
	must := []string{
		"PRE-FLIGHT-HOOK-CHECK (R35)",
		"bot-hq preflight-check",
		"BOOTSTRAP-ON-CONVERSATION-RESUME",
		"BOT_HQ_AGENT_ID",
		"PreToolUse-Bash",
		"tool-permission-hook",
		"AFTER hub_register",
		"Rain unreachable >60s",
		"session-restart",
		"/phase-rules-detail skill",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseMv1PreflightHookCheck, lit) {
				t.Errorf("R35 PRE-FLIGHT-HOOK-CHECK ratchet broken: missing literal %q in PhaseMv1PreflightHookCheck", lit)
			}
		})
	}
}

// Phase M M-1 prompt-embed verification: PhaseMv1PreflightHookCheck
// must be embedded in both rain.go + brian.go initialPrompt() so the
// new R35 rule is loaded at agent-spawn time. Mirrors L-5 + L-6 wiring-
// lock pattern; catches the "const exists but isn't wired" class.
func TestPhaseMv1PreflightHookCheckHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseMv1PreflightHookCheck, "- PRE-FLIGHT-HOOK-CHECK (R35):") {
		t.Errorf("rule must start with `- PRE-FLIGHT-HOOK-CHECK (R35):` (prompt anchor); first 60 chars: %q", PhaseMv1PreflightHookCheck[:60])
	}
}
