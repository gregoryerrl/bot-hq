package discord

import (
	"strings"
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
	_, err := NewBot("", "channel-id", "", "", nil)
	if err == nil {
		t.Error("expected error for empty token")
	}
}

func TestNewBotRequiresChannel(t *testing.T) {
	// Phase R R4: legacy channelID OR new hubChannelID must be set
	_, err := NewBot("token", "", "", "", nil)
	if err == nil {
		t.Error("expected error when both channel_id and hub_channel_id empty")
	}
}

func TestNewBotRequiresHub(t *testing.T) {
	_, err := NewBot("token", "channel-id", "", "", nil)
	if err == nil {
		t.Error("expected error for nil hub")
	}
}

// Phase R R4 — 5-state multi-channel routing matrix per Rain msg 15538
// Refine-2/3 BRAIN-2nd. Covers (i) legacy single-channel / (ii) partial-
// migration hub-only / (iii) partial-migration hub+flags / (iv) partial-
// migration hub+flags / (iv) fully-migrated R4 + Z-7. Z-7 removed
// the SessionsChannelID + [SESSION: content-prefix branch — session
// routing is now thread-based via msg.SessionID lookup (covered in
// threads_registry_test.go).
func TestChannelForMessage_RoutingMatrix(t *testing.T) {
	cases := []struct {
		name           string
		channelID      string
		hubChannelID   string
		flagsChannelID string
		msg            protocol.Message
		want           string
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
		// (ii) partial-migration hub-only
		{
			name:         "hub-only: all classes route to hubChannelID",
			hubChannelID: "hub",
			msg:          protocol.Message{Type: protocol.MsgFlag, Content: "alert"},
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
		// (iv) fully-migrated R4 + Z-7 (sessions are threads, tested
		// separately in threads_registry_test.go)
		{
			name:           "fully-migrated: hub-class → hub",
			hubChannelID:   "hub",
			flagsChannelID: "flags",
			msg:            protocol.Message{Type: protocol.MsgUpdate, Content: "regular update"},
			want:           "hub",
		},
		{
			name:           "fully-migrated: flag-class → flags",
			hubChannelID:   "hub",
			flagsChannelID: "flags",
			msg:            protocol.Message{Type: protocol.MsgFlag, Content: "alert"},
			want:           "flags",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			b := &Bot{
				channelID:      tc.channelID,
				hubChannelID:   tc.hubChannelID,
				flagsChannelID: tc.flagsChannelID,
			}
			if got := b.channelForMessage(tc.msg); got != tc.want {
				t.Errorf("channelForMessage(%q, type=%q) = %q, want %q", tc.msg.Content, tc.msg.Type, got, tc.want)
			}
		})
	}
}

// TestListensOnChannel — incoming-message filter for {channelID,
// hubChannelID, flagsChannelID}. Session-thread acceptance is tested
// separately in threads_registry_test.go.
func TestListensOnChannel(t *testing.T) {
	b := &Bot{
		channelID:      "legacy",
		hubChannelID:   "hub",
		flagsChannelID: "flags",
	}
	cases := []struct {
		id   string
		want bool
	}{
		{"legacy", true},
		{"hub", true},
		{"flags", true},
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

// Phase-R-followup embed-mode test matrix per Rain msg 15589 Refine-5.
// 13-sub-case sweep covering 7-class precedence + 3-direction context +
// Author=nil enforcement on Flag/HR + footer msg-ID cite.
func TestBuildEmbed_PrecedenceMatrix(t *testing.T) {
	cases := []struct {
		name        string
		msg         protocol.Message
		wantColor   int
		wantTitle   string
		wantAuthor  string // empty = expect nil author per Refine-3
		wantFooter  string
	}{
		// 7-class baseline
		{"Flag broadcast", protocol.Message{ID: 1, FromAgent: "rain", ToAgent: "", Type: protocol.MsgFlag, Content: "alert"},
			colorFlagRed, "🚨 ATTENTION NEEDED", "", "msg 1"},
		{"Error", protocol.Message{ID: 2, FromAgent: "brian", Type: protocol.MsgError, Content: "boom"},
			colorErrorRed, "Error", "brian", "msg 2"},
		{"HR broadcast", protocol.Message{ID: 3, FromAgent: "rain", ToAgent: "", Type: protocol.MsgResponse, Content: "[HR] final"},
			colorHRPink, "[HR]", "", "msg 3"},
		{"Result", protocol.Message{ID: 4, FromAgent: "brian", Type: protocol.MsgResult, Content: "done"},
			colorResultGreen, "Result", "brian", "msg 4"},
		{"Command", protocol.Message{ID: 5, FromAgent: "user", Type: protocol.MsgCommand, Content: "do x"},
			colorCommandYellow, "Command", "user", "msg 5"},
		{"Response", protocol.Message{ID: 6, FromAgent: "brian", Type: protocol.MsgResponse, Content: "concur"},
			colorResponseBlurple, "", "brian", "msg 6"},
		{"Update default", protocol.Message{ID: 7, FromAgent: "brian", Type: protocol.MsgUpdate, Content: "fyi"},
			colorUpdateGray, "", "brian", "msg 7"},
		// Precedence edge-cases per Refine-5
		{"Flag PM still Flag-shape (no PM-prefix per R2)", protocol.Message{ID: 8, FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgFlag, Content: "stop"},
			colorFlagRed, "🚨 ATTENTION NEEDED", "", "msg 8"},
		{"Flag with HR prefix still Flag-shape (Flag wins precedence)", protocol.Message{ID: 9, FromAgent: "rain", Type: protocol.MsgFlag, Content: "[HR] alert"},
			colorFlagRed, "🚨 ATTENTION NEEDED", "", "msg 9"},
		{"Error with HR prefix still Error-shape (Error wins precedence)", protocol.Message{ID: 10, FromAgent: "brian", Type: protocol.MsgError, Content: "[HR] boom"},
			colorErrorRed, "Error", "brian", "msg 10"},
		// Author-format Refine-2 unified
		{"Update PM has from→to author", protocol.Message{ID: 11, FromAgent: "brian", ToAgent: "rain", Type: protocol.MsgUpdate, Content: "ping"},
			colorUpdateGray, "", "brian → rain", "msg 11"},
		{"Update broadcast has from-only author", protocol.Message{ID: 12, FromAgent: "brian", ToAgent: "", Type: protocol.MsgUpdate, Content: "global"},
			colorUpdateGray, "", "brian", "msg 12"},
		{"HR PM still HR-shape Author-nil per R2", protocol.Message{ID: 13, FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgResponse, Content: "[HR] direct"},
			colorHRPink, "[HR]", "", "msg 13"},
	}

	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			e := buildEmbed(c.msg)
			if e.Color != c.wantColor {
				t.Errorf("Color = 0x%06x, want 0x%06x", e.Color, c.wantColor)
			}
			if e.Title != c.wantTitle {
				t.Errorf("Title = %q, want %q", e.Title, c.wantTitle)
			}
			if c.wantAuthor == "" {
				if e.Author != nil {
					t.Errorf("Author should be nil per R2 authorless, got %+v", e.Author)
				}
			} else {
				if e.Author == nil || e.Author.Name != c.wantAuthor {
					t.Errorf("Author.Name = %v, want %q", e.Author, c.wantAuthor)
				}
			}
			if e.Footer == nil || e.Footer.Text != c.wantFooter {
				t.Errorf("Footer.Text = %v, want %q", e.Footer, c.wantFooter)
			}
			if e.Description != c.msg.Content {
				t.Errorf("Description = %q, want %q", e.Description, c.msg.Content)
			}
		})
	}
}

func TestAuthorFor_PMVsBroadcast(t *testing.T) {
	bm := protocol.Message{FromAgent: "brian", ToAgent: ""}
	pm := protocol.Message{FromAgent: "brian", ToAgent: "rain"}

	if got := authorFor(bm); got.Name != "brian" {
		t.Errorf("broadcast author = %q, want %q", got.Name, "brian")
	}
	if got := authorFor(pm); got.Name != "brian → rain" {
		t.Errorf("PM author = %q, want %q", got.Name, "brian → rain")
	}
}

func TestBuildEmbedChunks_SingleForShortContent(t *testing.T) {
	msg := protocol.Message{ID: 100, FromAgent: "brian", Type: protocol.MsgUpdate, Content: "short"}
	embeds := buildEmbedChunks(msg)
	if len(embeds) != 1 {
		t.Errorf("short content should produce 1 embed, got %d", len(embeds))
	}
	if embeds[0].Description != "short" {
		t.Errorf("first embed description = %q, want %q", embeds[0].Description, "short")
	}
}

func TestBuildEmbedChunks_MultipleForLongContent(t *testing.T) {
	// Build content longer than embedDescriptionLimit (4096)
	long := strings.Repeat("a", embedDescriptionLimit+500)
	msg := protocol.Message{ID: 101, FromAgent: "brian", Type: protocol.MsgUpdate, Content: long}
	embeds := buildEmbedChunks(msg)
	if len(embeds) < 2 {
		t.Errorf("long content should produce 2+ embeds, got %d", len(embeds))
	}
	// First embed has metadata; continuation embeds have only description + color
	if embeds[0].Footer == nil || embeds[0].Footer.Text != "msg 101" {
		t.Errorf("first embed should have footer, got %v", embeds[0].Footer)
	}
	if embeds[1].Footer != nil {
		t.Errorf("continuation embed should not have footer, got %v", embeds[1].Footer)
	}
	if embeds[1].Color != embeds[0].Color {
		t.Errorf("continuation embed color = 0x%06x, want %0x06x (same as first)", embeds[1].Color, embeds[0].Color)
	}
	if embeds[1].Title != "" {
		t.Errorf("continuation embed should not have title, got %q", embeds[1].Title)
	}
}
