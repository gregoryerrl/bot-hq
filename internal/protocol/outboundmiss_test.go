package protocol

import "testing"

// TestIsOutboundMissNotification_Positive locks the load-bearing case:
// from_agent=self + content starts with [OUTBOUND-MISS] → true.
// Phase K K-14.
func TestIsOutboundMissNotification_Positive(t *testing.T) {
	msg := Message{
		FromAgent: "rain",
		Content:   "[OUTBOUND-MISS] agent rain emitted pane text at 2026-04-29T...",
	}
	if !IsOutboundMissNotification(msg, "rain") {
		t.Errorf("expected true (self + prefix); got false")
	}
}

// TestIsOutboundMissNotification_NotFromSelf locks the inverse: when
// from_agent != self, the message is not against this agent regardless
// of content. Table-driven across 3 sender classes (peer brian /
// emma system / unknown agent) to assert from-class doesn't matter —
// only from-self vs not-from-self.
//
// Rain msg 6450 refinement: table-driven covers both "peer" (msg 6450
// item 2 NotFromSelf) and "system-agent" (optional 6th test). Phase K K-14.
func TestIsOutboundMissNotification_NotFromSelf(t *testing.T) {
	cases := []struct {
		name    string
		msg     Message
		selfID  string
	}{
		{
			name:   "from-peer-brian",
			msg:    Message{FromAgent: "brian", Content: "[OUTBOUND-MISS] agent brian emitted pane text..."},
			selfID: "rain",
		},
		{
			name:   "from-system-emma",
			msg:    Message{FromAgent: "emma", Content: "[OUTBOUND-MISS] agent emma emitted pane text..."},
			selfID: "rain",
		},
		{
			name:   "from-unknown-agent",
			msg:    Message{FromAgent: "unknown-agent", Content: "[OUTBOUND-MISS] some content"},
			selfID: "rain",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if IsOutboundMissNotification(tc.msg, tc.selfID) {
				t.Errorf("expected false (not from self); got true")
			}
		})
	}
}

// TestIsOutboundMissNotification_NoPrefix locks: from_agent=self but
// content doesn't start with [OUTBOUND-MISS] → false (this is the
// agent's own legit output, not a system flag).
// Phase K K-14.
func TestIsOutboundMissNotification_NoPrefix(t *testing.T) {
	cases := []string{
		"rain|peer-coord-ack|some content",
		"some legit reply",
		"",
		"agent rain emitted pane text", // similar prose but no prefix
	}
	for _, content := range cases {
		msg := Message{FromAgent: "rain", Content: content}
		if IsOutboundMissNotification(msg, "rain") {
			t.Errorf("expected false (self without prefix; content=%q); got true", content)
		}
	}
}

// TestIsOutboundMissNotification_PrefixWithLeadingWhitespace locks the
// TrimSpace handling — leading whitespace before the canonical prefix
// shouldn't defeat detection (defensive against accidental whitespace
// in OUTBOUND-MISS hook output).
// Phase K K-14.
func TestIsOutboundMissNotification_PrefixWithLeadingWhitespace(t *testing.T) {
	cases := []string{
		"  [OUTBOUND-MISS] agent rain emitted pane text",
		"\t[OUTBOUND-MISS] agent rain emitted pane text",
		"\n[OUTBOUND-MISS] agent rain emitted pane text",
	}
	for _, content := range cases {
		msg := Message{FromAgent: "rain", Content: content}
		if !IsOutboundMissNotification(msg, "rain") {
			t.Errorf("expected true (TrimSpace should handle leading whitespace; content=%q); got false", content)
		}
	}
}

// TestIsOutboundMissNotification_PrefixSubstringMidContent locks the
// false-positive resistance: content containing [OUTBOUND-MISS] mid-text
// (e.g., conversational quoting of the prefix) but NOT starting with it
// → false. Without this, "we discussed [OUTBOUND-MISS] earlier" would
// false-trigger.
// Phase K K-14.
func TestIsOutboundMissNotification_PrefixSubstringMidContent(t *testing.T) {
	cases := []string{
		"we discussed [OUTBOUND-MISS] earlier",
		"see also [OUTBOUND-MISS] notification at msg 6068",
		"contains [OUTBOUND-MISS] but starts with prose",
	}
	for _, content := range cases {
		msg := Message{FromAgent: "rain", Content: content}
		if IsOutboundMissNotification(msg, "rain") {
			t.Errorf("expected false (prefix mid-content not at start; content=%q); got true", content)
		}
	}
}
