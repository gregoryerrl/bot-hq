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
	_, err := NewBot("", "channel-id", nil)
	if err == nil {
		t.Error("expected error for empty token")
	}
}

func TestNewBotRequiresChannel(t *testing.T) {
	_, err := NewBot("token", "", nil)
	if err == nil {
		t.Error("expected error for empty channel")
	}
}

func TestNewBotRequiresHub(t *testing.T) {
	_, err := NewBot("token", "channel-id", nil)
	if err == nil {
		t.Error("expected error for nil hub")
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
