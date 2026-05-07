package discord

import (
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// Bug #1 fix: the discord bridge filter must forward broadcasts (ToAgent=="")
// in addition to discord-routed and flag messages. Pre-fix, the filter
// silently dropped broadcasts → ~half of agent activity invisible to Discord.
//
// Cases lock the audience-driven routing rule per DISC v2 (msg 2147 lock):
//   - explicit "to": discord    → forward (legacy behavior, must keep)
//   - broadcast (to: "")        → forward (NEW — bug #1 fix)
//   - flag, any audience        → forward (attention notifications bypass routing)
//   - peer-routed (to: "brian") → skip   (peer coordination, not user-facing)
func TestShouldForwardToDiscord_AudienceCases(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want bool
	}{
		{
			name: "explicit to=discord forwards",
			msg:  protocol.Message{ToAgent: "discord", Type: protocol.MsgResponse, FromAgent: "brian"},
			want: true,
		},
		{
			name: "broadcast (empty to) forwards (bug-#1 fix)",
			msg:  protocol.Message{ToAgent: "", Type: protocol.MsgResponse, FromAgent: "brian"},
			want: true,
		},
		{
			name: "flag with empty to forwards",
			msg:  protocol.Message{ToAgent: "", Type: protocol.MsgFlag, FromAgent: "rain"},
			want: true,
		},
		{
			name: "peer-routed PM (to=brian) skips",
			msg:  protocol.Message{ToAgent: "brian", Type: protocol.MsgResponse, FromAgent: "rain"},
			want: false,
		},
		{
			name: "flag explicitly addressed to peer still forwards (attention bypasses routing)",
			msg:  protocol.Message{ToAgent: "brian", Type: protocol.MsgFlag, FromAgent: "rain"},
			want: true,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := shouldForwardToDiscord(tc.msg); got != tc.want {
				t.Errorf("shouldForwardToDiscord(to=%q,type=%q) = %v, want %v", tc.msg.ToAgent, tc.msg.Type, got, tc.want)
			}
		})
	}
}

func TestNewBotRequiresToken(t *testing.T) {
	_, err := NewBot("", "channel-id", "", "", "", nil)
	if err == nil {
		t.Error("expected error for empty token")
	}
}

func TestNewBotRequiresChannel(t *testing.T) {
	// Phase R R4: legacy channelID OR new hubChannelID must be set
	_, err := NewBot("token", "", "", "", "", nil)
	if err == nil {
		t.Error("expected error when both channel_id and hub_channel_id empty")
	}
}

func TestNewBotRequiresHub(t *testing.T) {
	_, err := NewBot("token", "channel-id", "", "", "", nil)
	if err == nil {
		t.Error("expected error for nil hub")
	}
}

// Phase R R4 — 5-state multi-channel routing matrix per Rain msg 15538
// Refine-2/3 BRAIN-2nd. Covers (i) legacy single-channel / (ii) partial-
// migration hub-only / (iii) partial-migration hub+flags / (iv) partial-
// migration hub+sessions / (v) fully-migrated.
func TestChannelForMessage_RoutingMatrix(t *testing.T) {
	cases := []struct {
		name              string
		channelID         string
		hubChannelID      string
		flagsChannelID    string
		sessionsChannelID string
		msg               protocol.Message
		want              string
	}{
		// (i) legacy single-channel: pre-R4 deployments
		{
			name:      "legacy: hub-class routes to channelID",
			channelID: "legacy",
			msg:       protocol.Message{Type: protocol.MsgUpdate, Content: "hello"},
			want:      "legacy",
		},
		{
			name:      "legacy: flag-class routes to channelID (no flags-channel)",
			channelID: "legacy",
			msg:       protocol.Message{Type: protocol.MsgFlag, Content: "alert"},
			want:      "legacy",
		},
		{
			name:      "legacy: session-event routes to channelID (no sessions-channel)",
			channelID: "legacy",
			msg:       protocol.Message{Type: protocol.MsgUpdate, Content: "[SESSION:abc12345] opened"},
			want:      "legacy",
		},
		// (ii) partial-migration hub-only
		{
			name:         "hub-only: all classes route to hubChannelID",
			hubChannelID: "hub",
			msg:          protocol.Message{Type: protocol.MsgFlag, Content: "alert"},
			want:         "hub",
		},
		{
			name:         "hub-only: session-event also routes to hubChannelID",
			hubChannelID: "hub",
			msg:          protocol.Message{Type: protocol.MsgUpdate, Content: "[SESSION:abc12345]"},
			want:         "hub",
		},
		// (iii) partial-migration hub+flags
		{
			name:           "hub+flags: flag routes to flagsChannelID",
			hubChannelID:   "hub",
			flagsChannelID: "flags",
			msg:            protocol.Message{Type: protocol.MsgFlag, Content: "alert"},
			want:           "flags",
		},
		{
			name:           "hub+flags: session-event falls back to hub (no sessions-channel)",
			hubChannelID:   "hub",
			flagsChannelID: "flags",
			msg:            protocol.Message{Type: protocol.MsgUpdate, Content: "[SESSION:abc12345]"},
			want:           "hub",
		},
		// (iv) partial-migration hub+sessions
		{
			name:              "hub+sessions: session-event routes to sessionsChannelID",
			hubChannelID:      "hub",
			sessionsChannelID: "sessions",
			msg:               protocol.Message{Type: protocol.MsgUpdate, Content: "[SESSION:abc12345]"},
			want:              "sessions",
		},
		{
			name:              "hub+sessions: flag falls back to hub (no flags-channel)",
			hubChannelID:      "hub",
			sessionsChannelID: "sessions",
			msg:               protocol.Message{Type: protocol.MsgFlag, Content: "alert"},
			want:               "hub",
		},
		// (v) fully-migrated R4
		{
			name:              "fully-migrated: hub-class → hub",
			hubChannelID:      "hub",
			flagsChannelID:    "flags",
			sessionsChannelID: "sessions",
			msg:               protocol.Message{Type: protocol.MsgUpdate, Content: "regular update"},
			want:              "hub",
		},
		{
			name:              "fully-migrated: flag-class → flags",
			hubChannelID:      "hub",
			flagsChannelID:    "flags",
			sessionsChannelID: "sessions",
			msg:               protocol.Message{Type: protocol.MsgFlag, Content: "alert"},
			want:              "flags",
		},
		{
			name:              "fully-migrated: session-event → sessions",
			hubChannelID:      "hub",
			flagsChannelID:    "flags",
			sessionsChannelID: "sessions",
			msg:               protocol.Message{Type: protocol.MsgUpdate, Content: "[SESSION:abc12345] opened"},
			want:              "sessions",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			b := &Bot{
				channelID:         tc.channelID,
				hubChannelID:      tc.hubChannelID,
				flagsChannelID:    tc.flagsChannelID,
				sessionsChannelID: tc.sessionsChannelID,
			}
			if got := b.channelForMessage(tc.msg); got != tc.want {
				t.Errorf("channelForMessage(%q, type=%q) = %q, want %q", tc.msg.Content, tc.msg.Type, got, tc.want)
			}
		})
	}
}

// TestListensOnChannel — Phase R R4 incoming-message filter expanded to
// {channelID, hubChannelID, flagsChannelID, sessionsChannelID}.
func TestListensOnChannel(t *testing.T) {
	b := &Bot{
		channelID:         "legacy",
		hubChannelID:      "hub",
		flagsChannelID:    "flags",
		sessionsChannelID: "sessions",
	}
	cases := []struct {
		id   string
		want bool
	}{
		{"legacy", true},
		{"hub", true},
		{"flags", true},
		{"sessions", true},
		{"unknown", false},
		{"", false},
	}
	for _, c := range cases {
		t.Run(c.id, func(t *testing.T) {
			if got := b.listensOnChannel(c.id); got != c.want {
				t.Errorf("listensOnChannel(%q) = %v, want %v", c.id, got, c.want)
			}
		})
	}
}

// TestListensOnChannel_LegacyOnly — pre-R4 single-channel deployments
// only listen on channelID.
func TestListensOnChannel_LegacyOnly(t *testing.T) {
	b := &Bot{channelID: "legacy"}
	if !b.listensOnChannel("legacy") {
		t.Error("legacy single-channel: should listen on channelID")
	}
	if b.listensOnChannel("other") {
		t.Error("legacy single-channel: should NOT listen on other channels")
	}
}

func TestSplitMessageShort(t *testing.T) {
	chunks := splitMessage("hello world", 2000)
	if len(chunks) != 1 {
		t.Errorf("expected 1 chunk, got %d", len(chunks))
	}
	if chunks[0] != "hello world" {
		t.Errorf("expected 'hello world', got %q", chunks[0])
	}
}

func TestSplitMessageLong(t *testing.T) {
	// Create a message longer than 100 chars and split at limit=100
	msg := ""
	for i := 0; i < 250; i++ {
		msg += "a"
	}
	chunks := splitMessage(msg, 100)
	if len(chunks) < 2 {
		t.Errorf("expected at least 2 chunks, got %d", len(chunks))
	}
	for _, c := range chunks {
		if len(c) > 100 {
			t.Errorf("chunk exceeds limit: len=%d", len(c))
		}
	}
	// Verify all content is preserved
	total := 0
	for _, c := range chunks {
		total += len(c)
	}
	if total != 250 {
		t.Errorf("expected total length 250, got %d", total)
	}
}

func TestSplitMessageNewlineBreak(t *testing.T) {
	// Build a message with a newline in the right spot for splitting
	msg := ""
	for i := 0; i < 70; i++ {
		msg += "a"
	}
	msg += "\n"
	for i := 0; i < 50; i++ {
		msg += "b"
	}
	// Total = 121 chars, limit 100 — should split at the newline (pos 70)
	chunks := splitMessage(msg, 100)
	if len(chunks) != 2 {
		t.Errorf("expected 2 chunks, got %d", len(chunks))
	}
}
