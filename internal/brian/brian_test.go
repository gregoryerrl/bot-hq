package brian

import (
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestNudgeContainsMessageContent(t *testing.T) {
	content := "fix the login bug"
	nudge := formatNudge(protocol.Message{FromAgent: "user", Content: content})

	if !strings.Contains(nudge, content) {
		t.Errorf("nudge should contain message content %q, got: %s", content, nudge)
	}
	if !strings.Contains(nudge, "user") {
		t.Errorf("nudge should contain sender 'user', got: %s", nudge)
	}
}

func TestFormatNudgeIsNotEmpty(t *testing.T) {
	nudge := formatNudge(protocol.Message{FromAgent: "user", Content: "hello"})
	if nudge == "" {
		t.Error("formatNudge should return non-empty string")
	}
	if !strings.Contains(nudge, "hello") {
		t.Error("formatNudge should include content")
	}
	if !strings.Contains(nudge, "[HUB:user]") {
		t.Error("formatNudge should include sender in [HUB:<sender>] tag")
	}
}

func TestInitialPromptMentionsHandshake(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	if !strings.Contains(prompt, "handshake") {
		t.Error("initial prompt should mention handshake protocol")
	}
	if !strings.Contains(prompt, "hub_session_create") {
		t.Error("initial prompt should mention hub_session_create")
	}
}

// Ratchet against regression: the OUTBOUND contract must survive any future
// prompt compression. If this line goes missing, replies regress to tmux-only
// and the user sees silence (see 2026-04-24 incident).
func TestInitialPromptContainsOutboundContract(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := "OUTBOUND: every reply is a hub_send tool call."
	if !strings.Contains(prompt, want) {
		t.Errorf("initial prompt must contain OUTBOUND contract substring %q", want)
	}
	if !strings.Contains(prompt, "you did not answer") {
		t.Error("initial prompt must keep the self-check clause ('you did not answer')")
	}
}

// Ratchet against regression: DISC v2 role split (HANDS/EYES/BRAIN) + OUTPUT
// class rules must survive future prompt compression. Each literal is
// load-bearing — missing any of these silently re-opens a drift mode we
// already diagnosed (2026-04-24). Failure is mechanical: restore the literal.
func TestInitialPromptContainsDISCv2(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"HANDS (brian):",
		"EYES (rain):",
		"BRAIN (both):",
		"Neither rubber-stamps; silence = implicit approval.",
		"Class-split suspended.",
		"Cannot expand Emma's allowlist",
		"OUTPUT: user replies split by class",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain DISC v2 literal %q", w)
		}
	}
}

func TestFormatNudgeCompactTagAndNoTrailer(t *testing.T) {
	nudge := formatNudge(protocol.Message{FromAgent: "user", Content: "hello"})
	if nudge != "[HUB:user] hello" {
		t.Errorf("expected compact tag, got %q", nudge)
	}
	// The IMPORTANT trailer has been moved to the initial-prompt DISCIPLINE/NUDGE
	// contract. It must not appear per-message any more — that's the compression win.
	if strings.Contains(nudge, "IMPORTANT") {
		t.Error("formatNudge should not contain the IMPORTANT trailer (moved to initial prompt)")
	}
	if strings.Contains(nudge, "hub_send") {
		t.Error("formatNudge should not contain routing instructions (hub_send)")
	}
}

func TestFormatNudgeFlagVariant(t *testing.T) {
	nudge := formatNudge(protocol.Message{FromAgent: "rain", Type: protocol.MsgFlag, Content: "disagree on scope"})
	if nudge != "[HUB:FLAG:rain] disagree on scope" {
		t.Errorf("expected FLAG-prefixed tag, got %q", nudge)
	}
}
