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

// Ratchet against regression: OUTBOUND rule must default user-facing replies
// to broadcast, not private to:"user". The private-default was the convention
// half of the 2026-04-24 peer-visibility incident — even with the filter fix,
// defaulting private still hides intent from the peer. Both clauses must
// survive prompt compression.
func TestInitialPromptContainsBroadcastDefault(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	for _, literal := range []string{
		"Default broadcast for user-facing replies",
		`Private to:"user" only when`,
	} {
		if !strings.Contains(prompt, literal) {
			t.Errorf("initial prompt must contain OUTBOUND broadcast-default literal %q", literal)
		}
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
		t.Errorf("expected broadcast FLAG tag, got %q", nudge)
	}
}

// Ratchet against regression: nudge tags must distinguish directed (PM) from
// broadcast (HUB) routing so Brian can tell at a glance whether he's the sole
// recipient or one of many. Missing PM variant silently reverts to [HUB:X]
// for direct sends — the exact confusion surfaced by the 2026-04-24 incident.
func TestFormatNudgePMAndHubVariants(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want string
	}{
		{"PM from rain", protocol.Message{FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgResponse, Content: "private"}, "[PM:rain] private"},
		{"PM from user", protocol.Message{FromAgent: "user", ToAgent: "brian", Type: protocol.MsgCommand, Content: "do x"}, "[PM:user] do x"},
		{"PM from discord", protocol.Message{FromAgent: "discord", ToAgent: "brian", Type: protocol.MsgResponse, Content: "hi"}, "[PM:discord] hi"},
		{"PM from coder", protocol.Message{FromAgent: "7a776ee2", ToAgent: "brian", Type: protocol.MsgResult, Content: "done"}, "[PM:7a776ee2] done"},
		{"PM FLAG from rain", protocol.Message{FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgFlag, Content: "stop"}, "[PM:FLAG:rain] stop"},
		{"HUB broadcast from rain", protocol.Message{FromAgent: "rain", ToAgent: "", Type: protocol.MsgResponse, Content: "broad"}, "[HUB:rain] broad"},
		{"HUB broadcast from user", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgCommand, Content: "all"}, "[HUB:user] all"},
		{"HUB FLAG broadcast", protocol.Message{FromAgent: "rain", ToAgent: "", Type: protocol.MsgFlag, Content: "bug"}, "[HUB:FLAG:rain] bug"},
		{"HUB-OBS cross-traffic", protocol.Message{FromAgent: "rain", ToAgent: "user", Type: protocol.MsgResponse, Content: "reply"}, "[HUB-OBS:rain→user] reply"},
		{"HUB-OBS to discord", protocol.Message{FromAgent: "rain", ToAgent: "discord", Type: protocol.MsgResponse, Content: "post"}, "[HUB-OBS:rain→discord] post"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := formatNudge(tc.msg); got != tc.want {
				t.Errorf("formatNudge = %q, want %q", got, tc.want)
			}
		})
	}
}

// Ratchet against regression: initial prompt must document the PM/HUB/HUB-OBS
// tag split so the agent knows which tag means which routing.
func TestInitialPromptDocumentsPMTag(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	for _, literal := range []string{"[PM:<sender>]", "[HUB:<sender>]", "[HUB-OBS:<from>→<to>]"} {
		if !strings.Contains(prompt, literal) {
			t.Errorf("initial prompt must document tag %q", literal)
		}
	}
}

// Ratchet against regression: Brian must see peer replies to user/discord in
// real time. Without this escape Brian is blind to Rain's to="user" replies
// (2026-04-24 incident: parallel drafts, neither agent saw the other's send).
// Mirror of rain.go:319-325 escape.
func TestShouldForwardToBrian_PeerToUserVisibility(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want bool
	}{
		{"rain to user forwards", protocol.Message{FromAgent: "rain", ToAgent: "user", Type: protocol.MsgResponse, Content: "x"}, true},
		{"rain to discord forwards", protocol.Message{FromAgent: "rain", ToAgent: "discord", Type: protocol.MsgResponse, Content: "x"}, true},
		{"user to rain forwards (visible coordination)", protocol.Message{FromAgent: "user", ToAgent: "rain", Type: protocol.MsgCommand, Content: "x"}, true},
		{"discord to rain forwards", protocol.Message{FromAgent: "discord", ToAgent: "rain", Type: protocol.MsgResponse, Content: "x"}, true},
		{"user broadcast forwards", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgCommand, Content: "x"}, true},
		{"rain to brian forwards", protocol.Message{FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgResponse, Content: "x"}, true},
		{"broadcast forwards", protocol.Message{FromAgent: "rain", ToAgent: "", Type: protocol.MsgResponse, Content: "x"}, true},
		{"rain to emma skips (peer-to-coder chatter)", protocol.Message{FromAgent: "rain", ToAgent: "emma", Type: protocol.MsgResponse, Content: "x"}, false},
		{"coder to coder skips", protocol.Message{FromAgent: "6058b444", ToAgent: "b4e5593f", Type: protocol.MsgUpdate, Content: "x"}, false},
		{"own message skipped", protocol.Message{FromAgent: "brian", ToAgent: "user", Type: protocol.MsgResponse, Content: "x"}, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := shouldForwardToBrian(tc.msg); got != tc.want {
				t.Errorf("shouldForwardToBrian(%+v) = %v, want %v", tc.msg, got, tc.want)
			}
		})
	}
}
