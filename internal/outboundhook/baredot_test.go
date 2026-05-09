package outboundhook

import "testing"

func TestIsBareDotPattern_positives(t *testing.T) {
	cases := []string{
		".",
		"..",
		"...",
		"Standing.",
		"standing",
		"Idle.",
		"IDLE",
		"Acknowledged.",
		"ack",
		"OK.",
		"done.",
		"Continuing.",
		"Loop fully closed.",
		"Loop closed.",
		"Holding.",
		"  Standing.  ", // whitespace tolerated
		"WAITING",
	}
	for _, c := range cases {
		if !isBareDotPattern(c) {
			t.Errorf("isBareDotPattern(%q) = false, want true", c)
		}
	}
}

func TestIsBareDotPattern_negatives(t *testing.T) {
	cases := []string{
		"",
		"This is a substantive message about something.",
		"Standing for Rain BRAIN-2nd verify on T-1.",
		"Continuing T-1 implementation per phase-t.md v5 sub-task ordering.", // longer than 50 chars
		"Idle. Working on something else.",                                    // multi-word with content
		"Standing\nfor user direction",                                        // multi-line
		"emma|status|active:5",                                                // hub_send-style emit
		"Brian-1st-pass on msg 17146",
		"R36 OUTBOUND-DISCIPLINE-MECHANICAL",
	}
	for _, c := range cases {
		if isBareDotPattern(c) {
			t.Errorf("isBareDotPattern(%q) = true, want false", c)
		}
	}
}

// TestShouldFlag_bareDotTriggersFlag locks the integration: a turnSummary
// with bare-dot text + no hub_send should trigger the flag (R50 mechanical-
// block), independent of length-threshold or planning-keyword presence.
func TestShouldFlag_bareDotTriggersFlag(t *testing.T) {
	s := turnSummary{
		TextLen:  9,           // below minTextLenForFlag
		TextSnip: "Standing.", // bare-dot pattern
		HubSent:  false,
	}
	if !shouldFlag(s) {
		t.Error("expected bare-dot pattern to trigger shouldFlag (R50)")
	}
}

// TestShouldFlag_bareDotWithHubSentDoesNotTrigger ensures R50 respects
// the hub_send escape-clause: bare-dot pattern with accompanying hub_send
// is OK (agent properly closed the loop via hub).
func TestShouldFlag_bareDotWithHubSentDoesNotTrigger(t *testing.T) {
	s := turnSummary{
		TextLen:  9,
		TextSnip: "Standing.",
		HubSent:  true,
	}
	if shouldFlag(s) {
		t.Error("hub_send escape-clause violated; should not flag when HubSent=true")
	}
}

// TestShouldFlag_substantiveTextDoesNotTriggerBareDot confirms substantive
// text takes the existing R36 length-threshold path, not the R50 bare-dot
// path. (Both classes flag, but via different clauses.)
func TestShouldFlag_substantiveTextNonBareDot(t *testing.T) {
	s := turnSummary{
		TextLen:  300, // > minTextLenForFlag
		TextSnip: "This is a long substantive turn but no hub_send.",
		HubSent:  false,
	}
	if !shouldFlag(s) {
		t.Error("substantive text + no hub_send should still flag (R36)")
	}
}
