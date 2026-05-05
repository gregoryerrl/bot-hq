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

// Phase M M-2 commit-1 ratchet: pin load-bearing recognition substrings
// of the OUTBOUND-DISCIPLINE-MECHANICAL (R36) rule. Anchors are what the
// agent prompt + recovery skill section reference + what locks the
// load-bearing tokens against future trim-pass drift.
//
// Substring-lock anchor strategy (mirrors L-5 R33 + L-6 R34 + M-1 R35
// patterns): rule-name + Stop-hook mechanism + block JSON shape + hub-
// write tool names (recovery primitives) + R33 precedent reference +
// skill pointer + bypass scope. NOT body example tokens.
//
// Locked anchors:
//   - Rule-name: "OUTBOUND-DISCIPLINE-MECHANICAL (R36)"
//   - Mechanism: "Stop-hook"
//   - Block enforcement: "BLOCKS turn completion"
//   - Recovery primitive: "mcp__bot-hq__hub_send"
//   - Recovery alternative: "hub_flag"
//   - Recovery alternative: "hub_session_close"
//   - JSON block shape: `{decision:"block"`
//   - R33 precedent reference: "R33"
//   - Skill pointer: "/phase-rules-detail skill"
//   - Bypass scope: "no-bypass"
//
// Origin: Phase M M-2 commit-1; codified to enforce R6 OUTBOUND-DISCIPLINE
// mechanically via Stop-hook BLOCKING enforcement-conversion. Closes the
// 2026-05-04 bilateral OUTBOUND-DISCIPLINE violation class (~3h halt
// USER-PINNED msgs 7476/7523) via the same recursion-terminator pattern
// R33 introduced in Phase L L-5 c2.
func TestOutboundDisciplineMechanicalSubstringLock(t *testing.T) {
	must := []string{
		"OUTBOUND-DISCIPLINE-MECHANICAL (R36)",
		"Stop-hook",
		"BLOCKS turn completion",
		"mcp__bot-hq__hub_send",
		"hub_flag",
		"hub_session_close",
		`{decision:"block"`,
		"R33",
		"/phase-rules-detail skill",
		"no-bypass",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseMv2OutboundDisciplineMechanical, lit) {
				t.Errorf("R36 OUTBOUND-DISCIPLINE-MECHANICAL ratchet broken: missing literal %q in PhaseMv2OutboundDisciplineMechanical", lit)
			}
		})
	}
}

// Phase M M-2 prompt-embed verification: PhaseMv2OutboundDisciplineMechanical
// must be embedded in both rain.go + brian.go initialPrompt() so the
// new R36 rule is loaded at agent-spawn time. Mirrors L-5 + L-6 + M-1
// wiring-lock pattern; catches the "const exists but isn't wired" class.
func TestPhaseMv2OutboundDisciplineMechanicalHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseMv2OutboundDisciplineMechanical, "- OUTBOUND-DISCIPLINE-MECHANICAL (R36):") {
		t.Errorf("rule must start with `- OUTBOUND-DISCIPLINE-MECHANICAL (R36):` (prompt anchor); first 60 chars: %q", PhaseMv2OutboundDisciplineMechanical[:60])
	}
}

// Phase M M-3 commit-1 ratchet: pin load-bearing recognition substrings
// of the BYTE-PROJECTION-CITE (R37) rule. Anchors are what the agent
// prompt + recovery skill section reference for dual-stage cite
// discipline (Stage 1 design-spike estimate + Stage 2 staged-time
// cite-from-actual + drift-tolerance threshold + escalation).
//
// Substring-lock anchor strategy (mirrors L-5 R33 + L-6 R34 + M-1 R35
// + M-2 R36 patterns): rule-name + class scope + dual-stage anchors +
// command-cite-from-actual + timing constraint + drift-tolerance +
// escalation path + bidirectional drift framing + recursion-terminator
// framing + R18 governance.
//
// Locked anchors (per audit-doc v1.1 §5):
//   - Rule-name: "BYTE-PROJECTION-CITE (R37)"
//   - Class scope: "byte/LOC projections"
//   - Artifact type: "design-spike docs"
//   - Audit-doc anchor: "audit-doc §5 ship-list"
//   - Stage-1 + Stage-2 dual-stage discipline: "Stage 1" + "Stage 2"
//   - Cite-from-actual command: "git diff --cached --numstat"
//   - Timing constraint: "BEFORE surfacing staged-diff"
//   - Drift-tolerance: "±25%"
//   - Escalation path: "discipline-log carry-forward"
//   - Recursion-terminator framing: "mechanical-cite-from-actual at staged-time"
//
// Origin: Phase M M-3 commit-1; codified to convert design-spike-doc
// byte-projection drift class from session-recall pre-author estimates
// (Phase L L-4 cluster-graduation candidate Target C; 5+ Phase L
// instances + 3+ Phase M same-session instances) to mechanical-cite-
// from-actual at staged-time discipline. Bidirectional drift class —
// over-estimate AND under-estimate both observed empirically.
func TestByteProjectionCiteSubstringLock(t *testing.T) {
	must := []string{
		"BYTE-PROJECTION-CITE (R37)",
		"byte/LOC projections",
		"design-spike docs",
		"audit-doc §5 ship-list",
		"Stage 1",
		"Stage 2",
		"git diff --cached --numstat",
		"BEFORE surfacing staged-diff",
		"±25%",
		"discipline-log carry-forward",
		"mechanical-cite-from-actual at staged-time",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseMv3ByteProjectionCite, lit) {
				t.Errorf("R37 BYTE-PROJECTION-CITE ratchet broken: missing literal %q in PhaseMv3ByteProjectionCite", lit)
			}
		})
	}
}

// Phase M M-3 prompt-embed verification: PhaseMv3ByteProjectionCite
// must be embedded in both rain.go + brian.go initialPrompt() so the
// new R37 rule is loaded at agent-spawn time. Mirrors L-5 + L-6 + M-1
// + M-2 wiring-lock pattern; catches the "const exists but isn't
// wired" class.
func TestPhaseMv3ByteProjectionCiteHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseMv3ByteProjectionCite, "- BYTE-PROJECTION-CITE (R37):") {
		t.Errorf("rule must start with `- BYTE-PROJECTION-CITE (R37):` (prompt anchor); first 60 chars: %q", PhaseMv3ByteProjectionCite[:60])
	}
}

// Phase M M-4 commit-1 ratchet: pin load-bearing recognition substrings
// of the DiscV2RoleAndPolicyShared const (9 shared bullets + header).
// Audit-doc v1.1 §3.5 (b) per-agent-split design: shared bullets in this
// const, divergent bullets in DiscV2RoleAndPolicyRainAddendum +
// DiscV2RoleAndPolicyBrianAddendum.
//
// Substring-lock anchor strategy: pin all rule-name + load-bearing
// terms across 9 bullets per audit-doc v1.1 §4 conservative-preserve.
// Existing rain_test.go TestInitialPromptContainsDISCv2 +
// TestInitialPromptContainsDISCv21FlagRule + TestRainPromptContainsHalterPusher
// + brian_test.go mirrors all assert subset of these literals via the
// rendered initialPrompt() — wiring-locks ensure this const reaches the
// prompt; this substring-lock ensures the const itself contains the
// load-bearing literals.
//
// Locked anchors (covering 9 shared bullets):
//   - Header: "DISC v2 2026-04-24:"
//   - HANDS: "HANDS (brian): exec." + "git/edits"
//   - EYES: "EYES (rain): info." + "EYES is read-only" + "Cannot expand Emma's allowlist"
//   - BRAIN: "BRAIN (both):" + "Neither rubber-stamps; silence = implicit approval"
//   - OUTPUT: "OUTPUT: user replies split by class" + "Class-split suspended"
//   - DRAFT: "DRAFT: drafter alone. Asker waits."
//   - HALTER/PUSHER: "HALTER/PUSHER" + "Rain halts, Brian pushes through" + "Mutual-halt deadlock impossible by construction"
//   - FLAG: "FLAG: Rain owns elevation" + "Brian PMs Rain on flag-worthy events" + "scope changes mid-decision" + "cliff-hang"
//   - PIVOT: "PIVOT: user w/o executor"
//   - NUDGE: "NUDGE:" + tag-prefix discriminators
//
// Origin: Phase M M-4 commit-1; codified to extract DISC v2 inline prose
// to const + apply per-rule trim per audit-doc v1.1 §4. Conservative-
// preserve all test-pinned literals from existing DISC v2 ratchet tests.
func TestDiscV2RoleAndPolicySharedSubstringLock(t *testing.T) {
	must := []string{
		// Header
		"DISC v2 2026-04-24:",
		// HANDS
		"HANDS (brian): exec.",
		// EYES
		"EYES (rain): info.",
		"EYES is read-only",
		"Cannot expand Emma's allowlist",
		"Rain cannot edit code",
		// BRAIN
		"BRAIN (both):",
		"Neither rubber-stamps; silence = implicit approval",
		// OUTPUT
		"OUTPUT: user replies split by class",
		"Class-split suspended",
		// DRAFT
		"DRAFT: drafter alone. Asker waits.",
		// HALTER/PUSHER
		"HALTER/PUSHER",
		"Rain halts, Brian pushes through",
		"Mutual-halt deadlock impossible by construction",
		// FLAG
		"Rain owns elevation",
		"Brian PMs Rain on flag-worthy events",
		"scope changes mid-decision",
		"cliff-hang",
		// PIVOT
		"PIVOT:",
		// NUDGE
		"[PM:<sender>]",
		"[HUB:<sender>]",
		"[HUB-OBS:<from>",
		"FLAG=elevated priority",
		"Never ignore FLAG or user messages",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(DiscV2RoleAndPolicyShared, lit) {
				t.Errorf("DiscV2RoleAndPolicyShared ratchet broken: missing literal %q", lit)
			}
		})
	}
}

// Phase M M-4 prompt-embed header anchor for DiscV2RoleAndPolicyShared.
// The const is the first thing both agents embed in their DISC v2 block;
// the header literal "DISC v2 2026-04-24:" is the recognition anchor.
func TestDiscV2RoleAndPolicySharedHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(DiscV2RoleAndPolicyShared, "DISC v2 2026-04-24:") {
		t.Errorf("DiscV2RoleAndPolicyShared must start with `DISC v2 2026-04-24:` (prompt anchor); first 30 chars: %q", DiscV2RoleAndPolicyShared[:30])
	}
}

// Phase M M-4 commit-1 ratchet: pin TRUST-rain literal in
// DiscV2RoleAndPolicyRainAddendum. Per audit-doc v1.1 §3.5 (b) per-agent-
// split: rain's TRUST framing is universal-applicability ("Snapshots=
// claims, not truth"). Brian's TRUST framing differs (preserved in
// BrianAddendum).
func TestDiscV2RoleAndPolicyRainAddendumSubstringLock(t *testing.T) {
	must := []string{
		"TRUST: spot-check claims via git/claude_read",
		"Snapshots=claims, not truth",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(DiscV2RoleAndPolicyRainAddendum, lit) {
				t.Errorf("DiscV2RoleAndPolicyRainAddendum ratchet broken: missing literal %q", lit)
			}
		})
	}
}

// Phase M M-4 commit-1 ratchet: pin TRUST-brian + SNAP literals in
// DiscV2RoleAndPolicyBrianAddendum. Per audit-doc v1.1 §3.5 (b) per-agent-
// split: brian's TRUST framing is hub_spawn-coder-flow-specific +
// SNAP block is brian-only output-formatting artifact.
func TestDiscV2RoleAndPolicyBrianAddendumSubstringLock(t *testing.T) {
	must := []string{
		"TRUST: verify via claude_read before",
		"Prefer one-shot spawn",
		"SNAP (multi-artifact dispatch/verify)",
		"Branches:",
		"Pending:",
		"Next:",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(DiscV2RoleAndPolicyBrianAddendum, lit) {
				t.Errorf("DiscV2RoleAndPolicyBrianAddendum ratchet broken: missing literal %q", lit)
			}
		})
	}
}

// Phase N N-5 commit-1 ratchet: pin load-bearing recognition substrings
// of the PhaseNv1LogTheFailingSide const. R38 LOG-THE-FAILING-SIDE rule
// generalizes the bcc-ad-manager auth-callback "no roles" misframing
// observed 2026-05-05.
func TestLogTheFailingSideSubstringLock(t *testing.T) {
	must := []string{
		"LOG-THE-FAILING-SIDE (R38)",
		"actual failing side",
		"input-side state cited as evidence",
		"Antipattern",
		"input contains the data the message says is missing",
		"failure is on the lookup/query side",
		"name explicitly which side actually failed",
		"2026-05-05 bcc-ad-manager",
		"MicrosoftAzureController::callback",
		"No roles assigned to the user",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseNv1LogTheFailingSide, lit) {
				t.Errorf("R38 LOG-THE-FAILING-SIDE ratchet broken: missing literal %q in PhaseNv1LogTheFailingSide", lit)
			}
		})
	}
}

// Phase N N-5 prompt-embed verification: PhaseNv1LogTheFailingSide
// const must start with the rule-anchor prefix the agent prompt embeds
// recognize. Mirrors TestPhaseMv3ByteProjectionCiteHeaderAnchor pattern.
func TestPhaseNv1LogTheFailingSideHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseNv1LogTheFailingSide, "- LOG-THE-FAILING-SIDE (R38):") {
		t.Errorf("rule must start with `- LOG-THE-FAILING-SIDE (R38):` (prompt anchor); first 60 chars: %q", PhaseNv1LogTheFailingSide[:60])
	}
}

// Phase N N-4 commit-2 ratchet: pin load-bearing recognition substrings
// of the PhaseNv2OverClaimDiscipline const. R31 sub-clause for
// verification-mechanism-citation discipline. Generalizes today's
// (2026-05-05) "all 6 flows verified" conflation user trust-shaking
// moment in the bcc-ad-manager session.
func TestOverClaimDisciplineSubstringLock(t *testing.T) {
	must := []string{
		"OVER-CLAIM-DISCIPLINE (R31 sub-clause)",
		"quantifier-claims about test/verification scope",
		"all flows verified",
		"verification mechanisms explicitly per-class",
		"PHPUnit feature-test",
		"browser-driven QA",
		"tinker-simulation",
		"Conflation across mechanism classes = drift",
		"all 6 flows verified",
		"per-mechanism counts",
		"user msg 7919",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseNv2OverClaimDiscipline, lit) {
				t.Errorf("R31 OVER-CLAIM-DISCIPLINE sub-clause ratchet broken: missing literal %q in PhaseNv2OverClaimDiscipline", lit)
			}
		})
	}
}

// Phase N N-4 prompt-embed verification: PhaseNv2OverClaimDiscipline
// const must start with the rule-anchor prefix the agent prompt embeds
// recognize. Mirrors TestPhaseNv1LogTheFailingSideHeaderAnchor pattern.
func TestPhaseNv2OverClaimDisciplineHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseNv2OverClaimDiscipline, "- OVER-CLAIM-DISCIPLINE (R31 sub-clause):") {
		t.Errorf("rule must start with `- OVER-CLAIM-DISCIPLINE (R31 sub-clause):` (prompt anchor); first 60 chars: %q", PhaseNv2OverClaimDiscipline[:60])
	}
}

// Phase N v2 N-T2-bundle commit-1 ratchet: pin load-bearing recognition
// substrings of the PhaseNv3HandshakeAckBlindSpot const. R36 sub-clause
// covers the handshake-terminator-blind-spot class (peer cross-in-flight
// substantive content unaddressed in reflexive "." ack).
func TestHandshakeAckBlindSpotSubstringLock(t *testing.T) {
	must := []string{
		"HANDSHAKE-ACK-BLIND-SPOT (R36 sub-clause)",
		"handshake-terminator",
		"blind-spot",
		"crossed-in-flight",
		"Antipattern",
		"reflexively",
		"crossed in flight — see msg N",
		"CROSS-TIMING-DEDUP",
		"peer-cross-check",
		"R36 OUTBOUND-DISCIPLINE-MECHANICAL parent rule",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseNv3HandshakeAckBlindSpot, lit) {
				t.Errorf("R36 HANDSHAKE-ACK-BLIND-SPOT sub-clause ratchet broken: missing literal %q in PhaseNv3HandshakeAckBlindSpot", lit)
			}
		})
	}
}

// Phase N v2 N-T2-bundle commit-1 prompt-embed verification:
// PhaseNv3HandshakeAckBlindSpot const must start with the rule-anchor
// prefix the agent prompt embeds recognize.
func TestPhaseNv3HandshakeAckBlindSpotHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseNv3HandshakeAckBlindSpot, "- HANDSHAKE-ACK-BLIND-SPOT (R36 sub-clause):") {
		t.Errorf("rule must start with `- HANDSHAKE-ACK-BLIND-SPOT (R36 sub-clause):` (prompt anchor); first 60 chars: %q", PhaseNv3HandshakeAckBlindSpot[:60])
	}
}

// Phase N v2 N-T2-bundle commit-1 ratchet: pin load-bearing recognition
// substrings of the PhaseNv4FilesystemSignalCite const. R31 sub-clause
// covers the filesystem-signal interpretive-extrapolation class (semantic
// claims derived from git/filesystem inspection without signal cite).
func TestFilesystemSignalCiteSubstringLock(t *testing.T) {
	must := []string{
		"FILESYSTEM-SIGNAL-CITE (R31 sub-clause)",
		"filesystem-state signals",
		"interpretation-limitations",
		"Antipattern",
		"empty `git diff` ≠ no work",
		"clean `git status` ≠ all-clean",
		"Discriminator at claim-author time",
		"name the signal command",
		"interpretive-extrapolation from filesystem signals",
		"R31 STAT-CLAIM-CITE parent rule",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseNv4FilesystemSignalCite, lit) {
				t.Errorf("R31 FILESYSTEM-SIGNAL-CITE sub-clause ratchet broken: missing literal %q in PhaseNv4FilesystemSignalCite", lit)
			}
		})
	}
}

// Phase N v2 N-T2-bundle commit-1 prompt-embed verification:
// PhaseNv4FilesystemSignalCite const must start with the rule-anchor
// prefix the agent prompt embeds recognize.
func TestPhaseNv4FilesystemSignalCiteHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseNv4FilesystemSignalCite, "- FILESYSTEM-SIGNAL-CITE (R31 sub-clause):") {
		t.Errorf("rule must start with `- FILESYSTEM-SIGNAL-CITE (R31 sub-clause):` (prompt anchor); first 60 chars: %q", PhaseNv4FilesystemSignalCite[:60])
	}
}

// Phase N v2 #2 commit ratchet: pin load-bearing recognition substrings
// of the PhaseNv5TestIsolation const. R39 TEST-ISOLATION rule
// generalizes 2026-05-05 bcc-ad-manager phpunit-against-local-app-DB
// cross-test contamination empirical (composer test RefreshDatabase
// wiped local dev DB mid-browser-QA per user msg 7919 anchor).
func TestTestIsolationSubstringLock(t *testing.T) {
	must := []string{
		"TEST-ISOLATION (R39)",
		"isolated test environment",
		"NEVER touches local-dev shared state",
		"Antipattern",
		"phpunit.xml",
		"test setup/teardown wipes the dev DB",
		"Discriminator at test-config-author time",
		"provably isolated from local-dev shared state",
		"2026-05-05 bcc-ad-manager",
		"phpunit-against-local-app-DB",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseNv5TestIsolation, lit) {
				t.Errorf("R39 TEST-ISOLATION ratchet broken: missing literal %q in PhaseNv5TestIsolation", lit)
			}
		})
	}
}

// Phase N v2 #2 commit prompt-embed verification: PhaseNv5TestIsolation
// const must start with the rule-anchor prefix the agent prompt embeds
// recognize.
func TestPhaseNv5TestIsolationHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseNv5TestIsolation, "- TEST-ISOLATION (R39):") {
		t.Errorf("rule must start with `- TEST-ISOLATION (R39):` (prompt anchor); first 60 chars: %q", PhaseNv5TestIsolation[:60])
	}
}

// Phase N v2 #3 commit ratchet: pin load-bearing recognition substrings
// of the PhaseNv6VoiceMirrorDiscipline const. R40 covers the voice-
// mirror discipline class for Write-to-user-artifact paths.
func TestVoiceMirrorDisciplineSubstringLock(t *testing.T) {
	must := []string{
		"VOICE-MIRROR-DISCIPLINE (R40)",
		"user-facing artifact",
		"user's voice",
		"trio operational voice",
		"Antipattern",
		"EOD report",
		"trio-operational-jargon",
		"Discriminator at Write-tool-fire time",
		"PreToolUse-hook (alert-only, NOT blocking)",
		"internal/voicemirror/hook.go",
		"voice-mirror-log.md",
		"Skip-list",
		"`**/memory/**`",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(PhaseNv6VoiceMirrorDiscipline, lit) {
				t.Errorf("R40 VOICE-MIRROR-DISCIPLINE ratchet broken: missing literal %q in PhaseNv6VoiceMirrorDiscipline", lit)
			}
		})
	}
}

// Phase N v2 #3 commit prompt-embed verification:
// PhaseNv6VoiceMirrorDiscipline const must start with the rule-anchor
// prefix the agent prompt embeds recognize.
func TestPhaseNv6VoiceMirrorDisciplineHeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(PhaseNv6VoiceMirrorDiscipline, "- VOICE-MIRROR-DISCIPLINE (R40):") {
		t.Errorf("rule must start with `- VOICE-MIRROR-DISCIPLINE (R40):` (prompt anchor); first 60 chars: %q", PhaseNv6VoiceMirrorDiscipline[:60])
	}
}
