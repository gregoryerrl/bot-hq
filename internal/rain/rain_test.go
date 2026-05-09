package rain

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func setupTestDB(t *testing.T) *hub.DB {
	t.Helper()
	path := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

// 1. TestNew_DefaultWorkDir — New(db, "") → workDir should be ~/Projects
func TestNew_DefaultWorkDir(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "")

	home, err := os.UserHomeDir()
	if err != nil {
		// If UserHomeDir fails, fallback is os.TempDir()
		home = os.TempDir()
	}
	expected := filepath.Join(home, "Projects")

	if r.workDir != expected {
		t.Errorf("expected workDir %q, got %q", expected, r.workDir)
	}
}

// 2. TestNew_CustomWorkDir — New(db, "/tmp/foo") → workDir = "/tmp/foo"
func TestNew_CustomWorkDir(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/foo")

	if r.workDir != "/tmp/foo" {
		t.Errorf("expected workDir %q, got %q", "/tmp/foo", r.workDir)
	}
}

// 3. TestNew_FieldsInitialized — stopCh not nil, running=false, lastMsgID=0
func TestNew_FieldsInitialized(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	if r.stopCh == nil {
		t.Error("expected stopCh to be non-nil")
	}
	if r.running {
		t.Error("expected running to be false")
	}
	if r.lastMsgID != 0 {
		t.Errorf("expected lastMsgID 0, got %d", r.lastMsgID)
	}
}

// 4. TestIsRunning_Default — false on fresh instance
func TestIsRunning_Default(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	if r.IsRunning() {
		t.Error("expected IsRunning() to return false on fresh instance")
	}
}

// 5. TestStop_NotRunning_NoOp — call Stop() on fresh instance, no panic
func TestStop_NotRunning_NoOp(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	// Should not panic
	r.Stop()

	if r.IsRunning() {
		t.Error("expected IsRunning() to return false after Stop() on fresh instance")
	}
}

// 6. TestFormatRainNudge_BasicFormat — compact [HUB:<sender>] tag, no IMPORTANT trailer.
func TestFormatRainNudge_BasicFormat(t *testing.T) {
	result := formatRainNudge(protocol.Message{FromAgent: "brian", Content: "Please review the code"}, "")

	if result != "[HUB:brian] Please review the code" {
		t.Errorf("expected compact tag, got %q", result)
	}
	if strings.Contains(result, "IMPORTANT") {
		t.Error("nudge should not contain the IMPORTANT trailer (moved to initial-prompt NUDGE contract)")
	}
}

// 7. TestFormatRainNudge_EmptyContent — handles empty content without dropping the tag.
func TestFormatRainNudge_EmptyContent(t *testing.T) {
	result := formatRainNudge(protocol.Message{FromAgent: "brian", Content: ""}, "")

	if !strings.HasPrefix(result, "[HUB:brian]") {
		t.Errorf("expected nudge to start with [HUB:brian], got %q", result)
	}
}

// 8. TestFormatRainNudge_SpecialChars — quotes, newlines, tabs survive compression.
func TestFormatRainNudge_SpecialChars(t *testing.T) {
	content := "He said \"hello\"\nand then\ttabs"
	result := formatRainNudge(protocol.Message{FromAgent: "user", Content: content}, "")

	if !strings.Contains(result, `"hello"`) {
		t.Errorf("expected nudge to preserve quotes, got %q", result)
	}
	if !strings.Contains(result, "\n") {
		t.Errorf("expected nudge to preserve newlines, got %q", result)
	}
	if !strings.Contains(result, "\t") {
		t.Errorf("expected nudge to preserve tabs, got %q", result)
	}
}

// Ratchet against regression: the OUTBOUND contract must survive any future
// prompt compression. Rain mirrors Brian's contract (see 2026-04-24 incident).
func TestInitialPromptContainsOutboundContract(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := "OUTBOUND: every reply is a hub_send tool call."
	if !strings.Contains(prompt, want) {
		t.Errorf("initial prompt must contain OUTBOUND contract substring %q", want)
	}
	if !strings.Contains(prompt, "you did not answer") {
		t.Error("initial prompt must keep the self-check clause ('you did not answer')")
	}
}

// Ratchet against regression: the prompt must embed the canonical
// DiscV2OutboundRule const verbatim. Mirror of brian_test.go's
// TestInitialPromptEmbedsDiscV2OutboundRule. The const itself is
// ratchet-tested in protocol/disc_test.go; this test locks the wiring
// on the rain end. Drift in either rain.go OR brian.go is now caught
// by the per-agent wiring tests.
func TestInitialPromptEmbedsDiscV2OutboundRule(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.DiscV2OutboundRule) {
		t.Errorf("initial prompt must embed protocol.DiscV2OutboundRule verbatim (bug #1 wiring lock)")
	}
}

// TestInitialPromptEmbedsPhaseIv1ProtocolHardening — rain-side mirror of
// the brian_test.go variant. Wiring lock for Phase I rules in Rain prompt.
func TestInitialPromptEmbedsPhaseIv1ProtocolHardening(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseIv1ProtocolHardening) {
		t.Errorf("initial prompt must embed protocol.PhaseIv1ProtocolHardening verbatim (Phase I wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseLv1RulebookHardening — rain-side mirror of
// the brian_test.go variant. Wiring lock for Phase L R31/R32 rules in
// Rain prompt. Catches the "const exists but isn't wired" class
// observed for K-Tier-1 R24-R30 (defined in disc.go but not embedded
// in initialPrompt()); surfaced in Phase L L-2 rule-locus-inventory
// exercise.
func TestRainPromptEmbedsPhaseLv1RulebookHardening(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseLv1RulebookHardening) {
		t.Errorf("initial prompt must embed protocol.PhaseLv1RulebookHardening verbatim (Phase L wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseLv5GateProtocol — rain-side wiring lock
// for Phase L L-5 commit-1 R33 PRE-EXECUTE-GATE-FILE-READ rule. Mirrors
// brian-side embed test. Same const-exists-but-not-wired class
// prevention.
func TestRainPromptEmbedsPhaseLv5GateProtocol(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseLv5GateProtocol) {
		t.Errorf("initial prompt must embed protocol.PhaseLv5GateProtocol verbatim (Phase L L-5 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseLv6PrePhaseCloseRetro — rain-side wiring lock
// for Phase L L-6 commit-1 R34 PRE-PHASE-CLOSE-RETRO rule. Mirrors
// brian-side embed test. Same const-exists-but-not-wired class
// prevention.
func TestRainPromptEmbedsPhaseLv6PrePhaseCloseRetro(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseLv6PrePhaseCloseRetro) {
		t.Errorf("initial prompt must embed protocol.PhaseLv6PrePhaseCloseRetro verbatim (Phase L L-6 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseMv1PreflightHookCheck — rain-side wiring lock
// for Phase M M-1 commit-1 R35 PRE-FLIGHT-HOOK-CHECK rule. Mirrors
// brian-side embed test. Same const-exists-but-not-wired class
// prevention.
func TestRainPromptEmbedsPhaseMv1PreflightHookCheck(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseMv1PreflightHookCheck) {
		t.Errorf("initial prompt must embed protocol.PhaseMv1PreflightHookCheck verbatim (Phase M M-1 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseMv2OutboundDisciplineMechanical — rain-side
// wiring lock for Phase M M-2 commit-1 R36 OUTBOUND-DISCIPLINE-MECHANICAL
// rule. Mirrors brian-side embed test. Same const-exists-but-not-wired
// class prevention.
func TestRainPromptEmbedsPhaseMv2OutboundDisciplineMechanical(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseMv2OutboundDisciplineMechanical) {
		t.Errorf("initial prompt must embed protocol.PhaseMv2OutboundDisciplineMechanical verbatim (Phase M M-2 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseMv3ByteProjectionCite — rain-side wiring lock
// for Phase M M-3 commit-1 R37 BYTE-PROJECTION-CITE rule. Mirrors
// brian-side embed test. Same const-exists-but-not-wired class
// prevention.
func TestRainPromptEmbedsPhaseMv3ByteProjectionCite(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseMv3ByteProjectionCite) {
		t.Errorf("initial prompt must embed protocol.PhaseMv3ByteProjectionCite verbatim (Phase M M-3 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseNv1LogTheFailingSide — rain-side wiring lock
// for Phase N N-5 commit-1 R38 LOG-THE-FAILING-SIDE rule. Mirrors
// brian-side embed test. Same const-exists-but-not-wired class
// prevention.
func TestRainPromptEmbedsPhaseNv1LogTheFailingSide(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseNv1LogTheFailingSide) {
		t.Errorf("initial prompt must embed protocol.PhaseNv1LogTheFailingSide verbatim (Phase N N-5 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseNv2OverClaimDiscipline — rain-side wiring lock
// for Phase N N-4 commit-2 R31 sub-clause OVER-CLAIM-DISCIPLINE.
// Mirrors brian-side embed test.
func TestRainPromptEmbedsPhaseNv2OverClaimDiscipline(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseNv2OverClaimDiscipline) {
		t.Errorf("initial prompt must embed protocol.PhaseNv2OverClaimDiscipline verbatim (Phase N N-4 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseNv3HandshakeAckBlindSpot — rain-side wiring
// lock for Phase N v2 N-T2-bundle commit-1 R36 sub-clause HANDSHAKE-ACK-
// BLIND-SPOT. Mirrors brian-side embed test.
func TestRainPromptEmbedsPhaseNv3HandshakeAckBlindSpot(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseNv3HandshakeAckBlindSpot) {
		t.Errorf("initial prompt must embed protocol.PhaseNv3HandshakeAckBlindSpot verbatim (Phase N v2 N-T2-bundle wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseNv4FilesystemSignalCite — rain-side wiring
// lock for Phase N v2 N-T2-bundle commit-1 R31 sub-clause FILESYSTEM-
// SIGNAL-CITE. Mirrors brian-side embed test.
func TestRainPromptEmbedsPhaseNv4FilesystemSignalCite(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseNv4FilesystemSignalCite) {
		t.Errorf("initial prompt must embed protocol.PhaseNv4FilesystemSignalCite verbatim (Phase N v2 N-T2-bundle wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseNv5TestIsolation — rain-side wiring lock for
// Phase N v2 #2 commit R39 TEST-ISOLATION. Mirrors brian-side embed
// test.
func TestRainPromptEmbedsPhaseNv5TestIsolation(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseNv5TestIsolation) {
		t.Errorf("initial prompt must embed protocol.PhaseNv5TestIsolation verbatim (Phase N v2 #2 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseNv6VoiceMirrorDiscipline — rain-side wiring
// lock for Phase N v2 #3 commit R40 VOICE-MIRROR-DISCIPLINE. Mirrors
// brian-side embed test.
func TestRainPromptEmbedsPhaseNv6VoiceMirrorDiscipline(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseNv6VoiceMirrorDiscipline) {
		t.Errorf("initial prompt must embed protocol.PhaseNv6VoiceMirrorDiscipline verbatim (Phase N v2 #3 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseRv1ContextLibraryTerminology — rain-side
// wiring lock for Phase R R3-b CL terminology. Mirrors brian-side.
func TestRainPromptEmbedsPhaseRv1ContextLibraryTerminology(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseRv1ContextLibraryTerminology) {
		t.Errorf("initial prompt must embed protocol.PhaseRv1ContextLibraryTerminology verbatim (Phase R R3-b wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseRv2BrainCycleHardening — rain-side wiring
// lock for Phase R R1 BRAIN-cycle hardening. Mirrors brian-side.
func TestRainPromptEmbedsPhaseRv2BrainCycleHardening(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseRv2BrainCycleHardening) {
		t.Errorf("initial prompt must embed protocol.PhaseRv2BrainCycleHardening verbatim (Phase R R1 wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseRv3AutoBoundaryDiscipline — rain-side wiring
// lock for Phase R R5 (d-1) auto-boundary-discipline. Mirrors brian-side.
func TestRainPromptEmbedsPhaseRv3AutoBoundaryDiscipline(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseRv3AutoBoundaryDiscipline) {
		t.Errorf("initial prompt must embed protocol.PhaseRv3AutoBoundaryDiscipline verbatim (Phase R R5 (d-1) wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseRv4EstimateShapeDisclosure — rain-side wiring
// lock for Phase-R-followup (c) R37 sub-clause. Mirrors brian-side.
func TestRainPromptEmbedsPhaseRv4EstimateShapeDisclosure(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseRv4EstimateShapeDisclosure) {
		t.Errorf("initial prompt must embed protocol.PhaseRv4EstimateShapeDisclosure verbatim (Phase-R-followup (c) wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseRv5MechanicalCiteFromHubRead — rain-side wiring
// lock for Phase-R-followup (d) R31 sub-clause. Mirrors brian-side.
func TestRainPromptEmbedsPhaseRv5MechanicalCiteFromHubRead(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseRv5MechanicalCiteFromHubRead) {
		t.Errorf("initial prompt must embed protocol.PhaseRv5MechanicalCiteFromHubRead verbatim (Phase-R-followup (d) wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseSv1AudienceClassLoadBearing — rain-side
// wiring lock for Phase S S-4-followup R6 hardening const. Mirrors
// brian-side.
func TestRainPromptEmbedsPhaseSv1AudienceClassLoadBearing(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseSv1AudienceClassLoadBearing) {
		t.Errorf("initial prompt must embed protocol.PhaseSv1AudienceClassLoadBearing verbatim (Phase S S-4-followup wiring lock)")
	}
}

// TestRainPromptEmbedsPhaseSv2IgnoreNoiseDiscipline verifies the
// Phase-S-followup-1 F1-7 §117 ignore-noise discipline const is
// embedded verbatim in rain's initial prompt.
func TestRainPromptEmbedsPhaseSv2IgnoreNoiseDiscipline(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.PhaseSv2IgnoreNoiseDiscipline) {
		t.Errorf("initial prompt must embed protocol.PhaseSv2IgnoreNoiseDiscipline verbatim (Phase-S-followup-1 F1-7 wiring lock)")
	}
}

// TestFormatRainNudgeWithSessionPrefix verifies Phase R R5 (d-1)
// `[SESSION:<8>] ` pane-header prepend behavior on rain side.
func TestFormatRainNudgeWithSessionPrefix(t *testing.T) {
	msg := protocol.Message{FromAgent: "brian", Content: "concur"}
	withPrefix := formatRainNudge(msg, "[SESSION:abcd1234] ")
	if !strings.HasPrefix(withPrefix, "[SESSION:abcd1234] ") {
		t.Errorf("expected SESSION prefix, got %q", withPrefix)
	}
	if !strings.Contains(withPrefix, "[HUB:brian] concur") {
		t.Errorf("expected base nudge tag preserved, got %q", withPrefix)
	}
	withoutPrefix := formatRainNudge(msg, "")
	if strings.Contains(withoutPrefix, "[SESSION:") {
		t.Errorf("empty prefix should not produce SESSION tag, got %q", withoutPrefix)
	}
}

// TestRainActiveSessionPrefix_NoActiveSessions verifies zero-open → empty
// prefix per Refine-A.
func TestRainActiveSessionPrefix_NoActiveSessions(t *testing.T) {
	db := setupTestDB(t)
	r := &Rain{db: db}
	if got := r.activeSessionPrefix(); got != "" {
		t.Errorf("expected empty prefix when no active sessions, got %q", got)
	}
}

// TestRainActiveSessionPrefix_WithActiveSession verifies first-row 8-char
// prefix selection on rain side.
func TestRainActiveSessionPrefix_WithActiveSession(t *testing.T) {
	db := setupTestDB(t)
	if err := db.CreateSession(protocol.Session{
		ID: "abcdef12-3456-7890-abcd-ef1234567890", Mode: protocol.SessionMode("implement"),
		Purpose: "test", Status: protocol.SessionActive,
	}); err != nil {
		t.Fatalf("CreateSession: %v", err)
	}
	r := &Rain{db: db}
	got := r.activeSessionPrefix()
	want := "[SESSION:abcdef12] "
	if got != want {
		t.Errorf("expected prefix %q, got %q", want, got)
	}
}

// TestRainPromptEmbedsIdSessionsSkillPointer — rain-side wiring lock
// for Phase N v2 #7 /id-sessions skill-pointer (per Rain msg 8146
// PASS-1 push-back). Without active-prompt-cite, agents have skill-
// on-disk + auto-discovery surface but no runtime-active rule-text
// awareness for session-event handling.
func TestRainPromptEmbedsIdSessionsSkillPointer(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.IdSessionsSkillPointer) {
		t.Errorf("initial prompt must embed protocol.IdSessionsSkillPointer verbatim (Phase N v2 #7 wiring lock)")
	}
}

// TestRainPromptEmbedsDiscV2RoleAndPolicyShared — rain-side wiring lock
// for Phase M M-4 commit-1 DiscV2RoleAndPolicyShared const (9 shared
// bullets + header). Per audit-doc v1.1 §3.5 (b) per-agent-split: shared
// const embeds in rain's prompt alongside RainAddendum. Same const-
// exists-but-not-wired class prevention.
func TestRainPromptEmbedsDiscV2RoleAndPolicyShared(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.DiscV2RoleAndPolicyShared) {
		t.Errorf("initial prompt must embed protocol.DiscV2RoleAndPolicyShared verbatim (Phase M M-4 wiring lock)")
	}
}

// TestRainPromptEmbedsDiscV2RoleAndPolicyRainAddendum — rain-side wiring
// lock for Phase M M-4 commit-1 DiscV2RoleAndPolicyRainAddendum const
// (rain-specific TRUST bullet). Per audit-doc v1.1 §3.5 (b) per-agent-
// split: rain agent prompt embeds Shared + RainAddendum (not BrianAddendum).
func TestRainPromptEmbedsDiscV2RoleAndPolicyRainAddendum(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	if !strings.Contains(prompt, protocol.DiscV2RoleAndPolicyRainAddendum) {
		t.Errorf("initial prompt must embed protocol.DiscV2RoleAndPolicyRainAddendum verbatim (Phase M M-4 wiring lock)")
	}
	// Negative-lock: rain MUST NOT embed BrianAddendum (TRUST-brian + SNAP)
	if strings.Contains(prompt, protocol.DiscV2RoleAndPolicyBrianAddendum) {
		t.Errorf("rain initial prompt MUST NOT embed protocol.DiscV2RoleAndPolicyBrianAddendum (per-agent-split discipline; brian-specific TRUST + SNAP belongs to brian only)")
	}
}

// Ratchet against the cliff-hang failure mode observed at msg 2086-2092
// on 2026-04-25: scope changes within an ongoing decision require a
// fresh flag, not silent continuation. The old "1 concern = 1 flag"
// wording let us read scope-morphs as "still on the same flag" and
// hold quietly while the user watched a silent hub. DISC v2.1 reframes
// from per-concern accounting to per-state — every pending-on-user
// state gets a flag once entering it, including refinements that
// materially alter the pending shape. Rain mirrors Brian's ratchet.
func TestInitialPromptContainsDISCv21FlagRule(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	// Phase H slice 2 H-2: FLAG governance shifted from symmetric to
	// asymmetric (Rain owns elevation). Mirrors Brian's ratchet.
	want := []string{
		"Rain owns elevation",
		// Phase S S-4 rewrite: "Brian PMs Rain" → "Brian uses @rain mention"
		"Brian uses @rain mention on flag-worthy events",
		"scope changes mid-decision",
		"cliff-hang",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain DISC v2.2 FLAG literal %q", w)
		}
	}
}

// TestRainPromptContainsHalterPusher locks the Phase H slice 2 H-1
// halter/pusher ratchet into Rain's initial prompt. Mirrors Brian's
// ratchet — same literals, same load-bearing role.
func TestRainPromptContainsHalterPusher(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := []string{
		"HALTER/PUSHER",
		"Rain halts, Brian pushes through",
		"BRAIN-cycle exempt",
		"Mutual-halt deadlock impossible by construction",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-1 halter/pusher literal %q", w)
		}
	}
}

// TestRainPromptContainsHaltAllWork locks the H-31 halt-all-work convention
// into Rain's initial prompt. Mirrors Brian's ratchet so both agents
// recognize Emma's context-cap FLAG identically. Slice 4 C7 M1 fold per
// Rain msg 3820: rephrased from regex-anchor notation to "contains
// substring" framing.
//
// Phase I W2 hotfix Fix-3 (msg 4926/4929 user (D)+SNAP-gate): halt
// protocol shifted from "kill-and-rebuild-fresh" to "idle-in-pane and
// watch for RESUME". Old assertions about fresh-context session removed;
// new assertions covering the idle-in-pane + RESUME-FROM-HALT protocol.
func TestRainPromptContainsHaltAllWork(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := []string{
		"HALT-ALL-WORK (H-31, H-33)",
		`"agent <id> at <N>%, halt"`,
		`"plan usage at <N>%, halt"`,
		"Match by substring meaning across BOTH triggers (agent context-cap OR plan-usage), not regex anchors",
		"Both fire HALT-ALL-WORK",
		"hub_session_close",
		"idle in pane",
		"do NOT close the claude session",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain HALT-ALL-WORK literal %q", w)
		}
	}
}

// TestRainPromptContainsResumeFromHalt locks the Phase I W2 hotfix Fix-3
// RESUME-FROM-HALT rule into Rain's initial prompt. Mirrors Brian's
// ratchet so both agents recognize Emma's resume-substring identically
// and apply the same SNAP-gate discipline.
//
// User msg 4929 SNAP-gate refinement: 0% resume flag must check for
// last-session SNAP. If SNAP exists, R16 bootstrap. If no SNAP, idle —
// do NOT auto-engage on empty state.
func TestRainPromptContainsResumeFromHalt(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := []string{
		"RESUME-FROM-HALT",
		`"plan usage reset"`,
		"last_session_snap",
		"if SNAP exists",
		"R16 CROSS-RESTART-RESUME-OPERATIONAL",
		"if no SNAP exists, remain idle",
		"do NOT auto-engage on empty state",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain RESUME-FROM-HALT literal %q", w)
		}
	}
}

// TestRainPromptContainsCarveOutEnumeration locks the Phase H slice 2 H-2
// self-flag carve-out enumeration into Rain's initial prompt. Mirrors
// Brian's ratchet so both agents share the same canonical carve-out list.
func TestRainPromptContainsCarveOutEnumeration(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := []string{
		"push-failure",
		"repo-corruption",
		"auth-failure",
		"hub-disconnect",
		"git-state-unexpected-on-write-path",
		"Rain unreachable >60s",
		"[self-flag-carve-out: <reason>]",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-2 self-flag carve-out literal %q", w)
		}
	}
}

// TestRainPromptHasNoPollingRule locks Phase H slice 2 H-18 — Rain's
// initial prompt must NOT instruct hub_read polling. Hub-push delivers
// messages automatically; polling was dead weight per session-3 evidence.
// Symmetric with Brian's prompt (brian.go:237 already correct).
func TestRainPromptHasNoPollingRule(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	// The OLD polling rule must be GONE. Match the specific retired phrasing
	// (with cadence) — not the bare token "poll hub_read", since the H-18
	// replacement guidance itself contains "do NOT poll hub_read" as the
	// intentional negative instruction.
	for _, gone := range []string{
		"poll hub_read (no agent filter) every 5-10s",
		"poll hub_read every",
	} {
		if strings.Contains(prompt, gone) {
			t.Errorf("initial prompt must NOT contain H-18 retired polling literal %q", gone)
		}
	}
	// Replacement guidance must be present.
	want := []string{
		"Messages arrive automatically",
		"do NOT poll hub_read",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-18 replacement literal %q", w)
		}
	}
}

// TestRainPromptContainsAsymmetricPivot locks the H-2 consistency fold:
// PIVOT scenario routes through Rain's hub_flag elevation, not Brian's
// self-flag. Mirrors Brian's ratchet — identical canonical text per
// single-source-of-truth pattern established for HALTER/PUSHER + FLAG.
func TestRainPromptContainsAsymmetricPivot(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := []string{
		"PIVOT: user w/o executor",
		// Phase S S-4 rewrite: "Brian PMs Rain" → "Brian uses @rain mention"
		"Brian uses @rain mention",
		"Rain holds 60s",
		"elevates via hub_flag",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-2 asymmetric PIVOT literal %q", w)
		}
	}
	// And the OLD symmetric framing must be GONE.
	for _, gone := range []string{"Brian flags first; step in if no ack"} {
		if strings.Contains(prompt, gone) {
			t.Errorf("initial prompt must NOT contain old symmetric PIVOT literal %q", gone)
		}
	}
}

// Ratchet against regression: DISC v2 role split (HANDS/EYES/BRAIN) + OUTPUT
// class rules must survive future prompt compression. Rain mirrors Brian's
// ratchet — same literals, same diagnostic load (see 2026-04-24 discussion).
func TestInitialPromptContainsDISCv2(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := []string{
		"HANDS (brian):",
		"EYES (rain):",
		"BRAIN (both):",
		"Neither rubber-stamps; silence = implicit approval.",
		"Class-split suspended.",
		"Cannot expand Emma's allowlist",
		"EYES is read-only",
		"Rain cannot edit code",
		"OUTPUT: user replies split by class",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain DISC v2 literal %q", w)
		}
	}
}

// 8b. TestFormatRainNudge_FlagVariant — MsgFlag elevates to [HUB:FLAG]
// per Phase R R2 authorless display-strip (sender hidden at render).
func TestFormatRainNudge_FlagVariant(t *testing.T) {
	result := formatRainNudge(protocol.Message{FromAgent: "brian", Type: protocol.MsgFlag, Content: "scope disagreement"}, "")

	if result != "[HUB:FLAG] scope disagreement" {
		t.Errorf("expected FLAG-prefixed tag (Phase R R2 authorless), got %q", result)
	}
}

// Phase-S-followup-2 F2-4: directed-to-other-agent collapses to [HUB:<sender>]
// (was [HUB-OBS:<from>→<to>] pre-purge). DB ToAgent preserved for forensics.
func TestFormatRainNudge_ObserveVariant(t *testing.T) {
	result := formatRainNudge(protocol.Message{FromAgent: "brian", ToAgent: "discord", Content: "posting update"}, "")

	if result != "[HUB:brian] posting update" {
		t.Errorf("expected post-purge [HUB:<sender>] for cross-traffic, got %q", result)
	}
}

// Ratchet against regression: nudge tags must distinguish directed (PM) from
// broadcast (HUB) routing so Rain can tell at a glance whether she's the sole
// recipient or one of many. Mirror of brian_test.go TestFormatNudgePMAndHubVariants.
func TestFormatRainNudgePMAndHubVariants(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want string
	}{
		// Phase-S-followup-2 F2-4: [PM:*] + [HUB-OBS:*] purged. All
		// messages collapse to [HUB:*] regardless of ToAgent value.
		{"directed from brian (was PM)", protocol.Message{FromAgent: "brian", ToAgent: "rain", Type: protocol.MsgResponse, Content: "private"}, "[HUB:brian] private"},
		{"directed from user (was PM)", protocol.Message{FromAgent: "user", ToAgent: "rain", Type: protocol.MsgCommand, Content: "do x"}, "[HUB:user] do x"},
		{"directed from discord (was PM)", protocol.Message{FromAgent: "discord", ToAgent: "rain", Type: protocol.MsgResponse, Content: "hi"}, "[HUB:discord] hi"},
		{"directed from coder (was PM)", protocol.Message{FromAgent: "7a776ee2", ToAgent: "rain", Type: protocol.MsgResult, Content: "done"}, "[HUB:7a776ee2] done"},
		{"directed FLAG (was PM:FLAG)", protocol.Message{FromAgent: "brian", ToAgent: "rain", Type: protocol.MsgFlag, Content: "stop"}, "[HUB:FLAG] stop"},
		{"HUB broadcast from brian", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgResponse, Content: "broad"}, "[HUB:brian] broad"},
		{"HUB broadcast from user", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgCommand, Content: "all"}, "[HUB:user] all"},
		{"HUB FLAG broadcast", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgFlag, Content: "bug"}, "[HUB:FLAG] bug"},
		{"cross-traffic (was HUB-OBS)", protocol.Message{FromAgent: "brian", ToAgent: "user", Type: protocol.MsgResponse, Content: "reply"}, "[HUB:brian] reply"},
		{"cross-traffic to discord (was HUB-OBS)", protocol.Message{FromAgent: "brian", ToAgent: "discord", Type: protocol.MsgResponse, Content: "post"}, "[HUB:brian] post"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := formatRainNudge(tc.msg, ""); got != tc.want {
				t.Errorf("formatRainNudge = %q, want %q", got, tc.want)
			}
		})
	}
}

// Phase-S-followup-2 F2-4: only the [HUB:<sender>] + [HUB:FLAG:<sender>]
// tags are runtime-rendered now. [PM:*] + [HUB-OBS:*] purged. Initial
// prompt must document the surviving HUB tag set.
func TestInitialPromptDocumentsPMTag(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	for _, literal := range []string{"[HUB:<sender>]", "[HUB:FLAG:<sender>]"} {
		if !strings.Contains(prompt, literal) {
			t.Errorf("initial prompt must document tag %q", literal)
		}
	}
}

// Ratchet against regression: Rain must see peer replies to user/discord in
// real time. The bug was two-layered: SQL filter at db.go:354 excluded
// cross-traffic from Rain's query, AND rain.go pollLoop used ReadMessages("rain")
// which activated that SQL filter. Fix moves filter authority fully into
// shouldForwardToRain() by calling ReadMessages("") — this test locks the
// Go-layer decisions. See 2026-04-24 incident.
func TestShouldForwardToRain_PeerToUserVisibility(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want bool
	}{
		{"brian to user forwards", protocol.Message{FromAgent: "brian", ToAgent: "user", Type: protocol.MsgResponse, Content: "x"}, true},
		{"brian to discord forwards", protocol.Message{FromAgent: "brian", ToAgent: "discord", Type: protocol.MsgResponse, Content: "x"}, true},
		{"brian broadcast response forwards (peer visibility)", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgResponse, Content: "x"}, true},
		{"brian broadcast update forwards (Phase I I-7 fix — was dropped)", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgUpdate, Content: "scope-dump x"}, true},
		{"brian broadcast handshake forwards (any Type from Brian)", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgHandshake, Content: "."}, true},
		{"user to brian forwards (visible coordination)", protocol.Message{FromAgent: "user", ToAgent: "brian", Type: protocol.MsgCommand, Content: "x"}, true},
		{"directed to rain forwards", protocol.Message{FromAgent: "brian", ToAgent: "rain", Type: protocol.MsgResponse, Content: "x"}, true},
		{"user broadcast forwards", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgCommand, Content: "x"}, true},
		{"coder result forwards (QA coverage)", protocol.Message{FromAgent: "7a776ee2", ToAgent: "brian", Type: protocol.MsgResult, Content: "x"}, true},
		{"flag forwards", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgFlag, Content: "x"}, true},
		{"hub_flag mention forwards", protocol.Message{FromAgent: "emma", ToAgent: "", Type: protocol.MsgUpdate, Content: "calling hub_flag"}, true},
		{"coder broadcast response skips (no flood)", protocol.Message{FromAgent: "7a776ee2", ToAgent: "", Type: protocol.MsgResponse, Content: "ack"}, false},
		{"coder to coder skips", protocol.Message{FromAgent: "6058b444", ToAgent: "b4e5593f", Type: protocol.MsgUpdate, Content: "x"}, false},
		{"emma to brian update skips", protocol.Message{FromAgent: "emma", ToAgent: "brian", Type: protocol.MsgUpdate, Content: "x"}, false},
		{"own message skipped", protocol.Message{FromAgent: "rain", ToAgent: "user", Type: protocol.MsgResponse, Content: "x"}, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := shouldForwardToRain(tc.msg); got != tc.want {
				t.Errorf("shouldForwardToRain(%+v) = %v, want %v", tc.msg, got, tc.want)
			}
		})
	}
}

// Ratchet against the SQL-layer half of the bug: Rain's poll MUST use
// ReadMessages("", ...) not ReadMessages("rain", ...). The agentID-scoped
// SQL query filters cross-traffic before Go ever sees it, silently
// invalidating shouldForwardToRain()'s user/discord escape clauses.
// This test asserts the source contains the unscoped call.
func TestProcessNewMessagesUsesUnscoppedRead(t *testing.T) {
	data, err := os.ReadFile("rain.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	want := `r.db.ReadMessages("", r.lastMsgID, 50)`
	if !strings.Contains(src, want) {
		t.Errorf("rain.go must call %s — scoped ReadMessages(agentID, ...) reintroduces the SQL-layer blindspot", want)
	}
	unwanted := `r.db.ReadMessages(agentID, r.lastMsgID`
	if strings.Contains(src, unwanted) {
		t.Errorf("rain.go must not call %s — that reintroduces the bug", unwanted)
	}
}

// 9. TestWriteMCPConfig_JSONStructure — create Rain with t.TempDir() workDir,
// call writeMCPConfig(), read and parse the JSON file, verify structure.
func TestWriteMCPConfig_JSONStructure(t *testing.T) {
	db := setupTestDB(t)
	tmpDir := t.TempDir()
	r := New(db, tmpDir)

	if err := r.writeMCPConfig(); err != nil {
		t.Fatal(err)
	}

	configPath := filepath.Join(tmpDir, ".bot-hq-rain-mcp.json")
	data, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatalf("failed to read config file: %v", err)
	}

	var config map[string]any
	if err := json.Unmarshal(data, &config); err != nil {
		t.Fatalf("failed to parse JSON: %v", err)
	}

	mcpServers, ok := config["mcpServers"].(map[string]any)
	if !ok {
		t.Fatal("expected mcpServers key in config")
	}

	botHQ, ok := mcpServers["bot-hq"].(map[string]any)
	if !ok {
		t.Fatal("expected bot-hq key in mcpServers")
	}

	if _, ok := botHQ["command"]; !ok {
		t.Error("expected command field in bot-hq config")
	}

	args, ok := botHQ["args"]
	if !ok {
		t.Fatal("expected args field in bot-hq config")
	}

	argsList, ok := args.([]any)
	if !ok {
		t.Fatalf("expected args to be an array, got %T", args)
	}

	if len(argsList) != 1 || argsList[0] != "mcp" {
		t.Errorf("expected args=[\"mcp\"], got %v", argsList)
	}

	// Verify file permissions
	info, err := os.Stat(configPath)
	if err != nil {
		t.Fatal(err)
	}
	perm := info.Mode().Perm()
	if perm != 0600 {
		t.Errorf("expected file permission 0600, got %04o", perm)
	}
}

// 10. TestProcessNewMessages_SkipsSelf — insert a message from "rain" to "rain",
// call processNewMessages, verify no SendCommand attempt.
func TestProcessNewMessages_SkipsSelf(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, t.TempDir())

	// Register rain agent so messages can be addressed to it
	db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
	})

	// Insert a message from rain to rain
	_, err := db.InsertMessage(protocol.Message{
		FromAgent: "rain",
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   "self-message that should be skipped",
		Created:   time.Now(),
	})
	if err != nil {
		t.Fatal(err)
	}

	// processNewMessages should skip messages from self.
	// Since Rain is not running (no tmux), SendCommand would return an error.
	// If the skip logic works, SendCommand is never called and no error occurs.
	// We verify by checking that lastMsgID advances (message was seen) but
	// no panic/error from trying to send to a non-existent tmux session.
	r.processNewMessages()

	if r.lastMsgID == 0 {
		t.Error("expected lastMsgID to advance after processing messages")
	}
}

// TestStartInitDoesNotPreSeedLastMsgID is a source ratchet locking the C2
// deletion: rain.go must NOT pre-seed lastMsgID to the highest existing ID at
// init. The pre-fix init block called GetRecentMessages(1) and assigned
// msgs[0].ID to r.lastMsgID, which silently skipped any pre-restart backlog.
// First poll-tick now relies on ReadMessages tail semantics (sinceID=0 →
// latest N) to replay recent context.
func TestStartInitDoesNotPreSeedLastMsgID(t *testing.T) {
	data, err := os.ReadFile("rain.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	for _, banned := range []string{
		"r.db.GetRecentMessages(1)",
		"r.lastMsgID = msgs[0].ID",
	} {
		if strings.Contains(src, banned) {
			t.Errorf("rain.go must not contain %q — reintroduces the pre-restart backlog skip bug", banned)
		}
	}
}

// TestProcessNewMessagesNoSpuriousReplay locks polling stability: a second
// processNewMessages call after the watermark has advanced returns nothing.
// This is the C1+C2 interaction at the floor — verifies sinceID=lastID
// returns empty (Rain's third C1 test assertion expressed at the call site).
func TestProcessNewMessagesNoSpuriousReplay(t *testing.T) {
	db := setupTestDB(t)

	for i := 0; i < 30; i++ {
		if _, err := db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgCommand,
			Content:   "msg",
		}); err != nil {
			t.Fatal(err)
		}
	}

	r := New(db, t.TempDir())
	r.processNewMessages()
	advanced := r.lastMsgID
	if advanced == 0 {
		t.Fatal("first poll: expected lastMsgID to advance, got 0")
	}

	r.processNewMessages()
	if r.lastMsgID != advanced {
		t.Errorf("second poll: lastMsgID = %d, want %d (no spurious replay)", r.lastMsgID, advanced)
	}
}

// Regression-lock for the autostart env-var injection. The Stop hook in
// internal/outboundhook/hook.go:88 reads BOT_HQ_AGENT_ID to attribute
// OUTBOUND-MISS sentinel events to a specific agent. Without the -e flag,
// hooks fire anonymously. See msg 4197/4205 for the failure-mode framing.
func TestNewSessionArgsInjectsAgentIDEnvFlag(t *testing.T) {
	r := &Rain{tmuxSession: "test-session", workDir: "/tmp"}
	args := r.newSessionArgs()

	want := "BOT_HQ_AGENT_ID=" + agentID
	found := false
	for i := 0; i < len(args)-1; i++ {
		if args[i] == "-e" && args[i+1] == want {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("newSessionArgs missing `-e %s` env-injection flag pair; got %v", want, args)
	}

	if !strings.Contains(strings.Join(args, " "), "test-session") {
		t.Errorf("session name not in args: %v", args)
	}
}

// TestNewSessionArgsInjectsModelConfigEnvVarsForDeepSeek locks Phase T T-1.4
// (R51 + R52): when rain's agent_model_config row points at a non-Claude
// provider (e.g. DeepSeek-V4-Pro via Anthropic-compatible endpoint), the
// tmux new-session args MUST include the env-var swap (ANTHROPIC_BASE_URL +
// ANTHROPIC_AUTH_TOKEN + ANTHROPIC_MODEL). Validates env-var injection at
// agent-spawn-time per phase-t.md v5.
func TestNewSessionArgsInjectsModelConfigEnvVarsForDeepSeek(t *testing.T) {
	t.Setenv("DEEPSEEK_API_KEY", "sk-test-rain-deepseek")

	dir := t.TempDir()
	db, err := hub.OpenDB(filepath.Join(dir, "hub.db"))
	if err != nil {
		t.Fatalf("OpenDB: %v", err)
	}
	defer db.Close()
	// migrate auto-seeds rain with DeepSeek default config

	r := &Rain{db: db, tmuxSession: "test-session-deepseek", workDir: "/tmp"}
	args := r.newSessionArgs()

	joined := strings.Join(args, " ")
	wantSubstrings := []string{
		"ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic",
		"ANTHROPIC_AUTH_TOKEN=sk-test-rain-deepseek",
		"ANTHROPIC_MODEL=deepseek-v4-pro",
	}
	for _, want := range wantSubstrings {
		if !strings.Contains(joined, want) {
			t.Errorf("expected %q in tmux args; got: %v", want, args)
		}
	}
}

// TestNewSessionArgsClaudeOAuthPathInjectsNothing locks the inverse: when
// rain's config is the Claude OAuth path (e.g. fallback-to-Claude per R51
// fallback-config), the env-var swap is NOT injected (subprocess inherits
// CLAUDE_CODE_OAUTH_TOKEN from env directly).
func TestNewSessionArgsClaudeOAuthPathInjectsNothing(t *testing.T) {
	dir := t.TempDir()
	db, err := hub.OpenDB(filepath.Join(dir, "hub.db"))
	if err != nil {
		t.Fatalf("OpenDB: %v", err)
	}
	defer db.Close()

	// Override rain's config to Claude OAuth path (simulating fallback)
	override := &hub.AgentModelConfig{
		AgentID:       "rain",
		Provider:      "anthropic",
		ModelName:     "claude-default",
		BaseURL:       "",
		AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
		Enabled:       true,
		Notes:         "Claude OAuth fallback for test",
	}
	if err := db.SetAgentModelConfig(override); err != nil {
		t.Fatalf("Set: %v", err)
	}

	r := &Rain{db: db, tmuxSession: "test-session-claude", workDir: "/tmp"}
	args := r.newSessionArgs()

	joined := strings.Join(args, " ")
	if strings.Contains(joined, "ANTHROPIC_BASE_URL") || strings.Contains(joined, "ANTHROPIC_AUTH_TOKEN") || strings.Contains(joined, "ANTHROPIC_MODEL") {
		t.Errorf("Claude OAuth path should NOT inject ANTHROPIC_* env-vars; got: %v", args)
	}
}

// TestSendCommandRequiresSinkInitialized locks the Phase I W2 Layer-2 (c)
// safety branch: if Rain is marked running but the sink field is nil
// (defensive — should never happen if Start completed), SendCommand
// returns a "sink not initialized" error instead of panicking with a nil
// dereference inside Sink.Deliver.
func TestSendCommandRequiresSinkInitialized(t *testing.T) {
	r := &Rain{running: true} // sink intentionally nil
	err := r.SendCommand("test")
	if err == nil {
		t.Fatal("expected error when sink not initialized, got nil")
	}
	if !strings.Contains(err.Error(), "sink not initialized") {
		t.Errorf("expected 'sink not initialized' error, got: %v", err)
	}
}

// TestSendCommandRoutesThroughSink is a source ratchet locking the
// Phase I W2 Layer-2 (c) refactor: rain.go SendCommand must NOT contain
// the prior naked tmux send-keys + sleep + Enter pattern. All pane
// delivery routes through tmuxsink.Sink so isReady-check + retry-queue
// semantics apply uniformly with hub.dispatchToTmux.
func TestSendCommandRoutesThroughSink(t *testing.T) {
	data, err := os.ReadFile("rain.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	for _, banned := range []string{
		`exec.Command("tmux", "send-keys", "-t", session, "-l", text)`,
		`exec.Command("tmux", "send-keys", "-t", session, "Enter")`,
	} {
		if strings.Contains(src, banned) {
			t.Errorf("rain.go SendCommand must not contain %q — bypasses tmuxsink isReady+retry", banned)
		}
	}
	for _, want := range []string{
		"sink.Deliver",
		"tmuxsink.New",
		"hub.NewTmuxSinkStore",
	} {
		if !strings.Contains(src, want) {
			t.Errorf("rain.go must contain %q — Phase I W2 Layer-2 (c) sink wiring", want)
		}
	}
}

// TestFormatRainNudge_PhaseR_R2_AuthorlessHR — Phase R R2 authorless [HR]
// display-strip mirror on rain side.
func TestFormatRainNudge_PhaseR_R2_AuthorlessHR(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want string
	}{
		{"HR broadcast", protocol.Message{FromAgent: "rain", ToAgent: "", Content: "[HR] final draft"}, "[HUB] [HR] final draft"},
		// Phase-S-followup-2 F2-4: directed-class collapses to broadcast-class render.
		{"HR directed to rain (was [PM])", protocol.Message{FromAgent: "user", ToAgent: "rain", Content: "[HR] direct"}, "[HUB] [HR] direct"},
		{"non-HR broadcast unchanged", protocol.Message{FromAgent: "brian", ToAgent: "", Content: "regular"}, "[HUB:brian] regular"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := formatRainNudge(tc.msg, ""); got != tc.want {
				t.Errorf("formatRainNudge = %q, want %q", got, tc.want)
			}
		})
	}
}
