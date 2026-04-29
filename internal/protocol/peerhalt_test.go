package protocol

import (
	"strings"
	"testing"
)

// TestPeerHaltPayloadRoundTrip locks Build → Parse symmetry.
func TestPeerHaltPayloadRoundTrip(t *testing.T) {
	want := PeerHaltPayload{
		TriggerClass:    TriggerClassSplitViolation,
		ObservedMsgID:   6358,
		RecoveryPointer: "~/.bot-hq/rain/discipline-anchors.md § class-split",
		Notes:           "fired gh pr create directly",
	}
	content, err := BuildPeerHaltContent(want)
	if err != nil {
		t.Fatalf("BuildPeerHaltContent: %v", err)
	}
	got, err := ParsePeerHaltContent(content)
	if err != nil {
		t.Fatalf("ParsePeerHaltContent: %v", err)
	}
	if got != want {
		t.Errorf("roundtrip mismatch:\n got=%+v\nwant=%+v", got, want)
	}
}

// TestPeerHaltPayloadOmitEmpty locks forward-compat: optional fields
// (ObservedMsgID=0, Notes="") are omitted from JSON output so future
// parsers don't break on absence + JSON output stays compact.
func TestPeerHaltPayloadOmitEmpty(t *testing.T) {
	minimal := PeerHaltPayload{
		TriggerClass:    TriggerAnchorChecksumMismatch,
		RecoveryPointer: "~/.bot-hq/brian/discipline-anchors.md",
	}
	content, err := BuildPeerHaltContent(minimal)
	if err != nil {
		t.Fatalf("BuildPeerHaltContent: %v", err)
	}
	if strings.Contains(content, `"observed_msg_id"`) {
		t.Errorf("observed_msg_id should be omitted when zero; got %s", content)
	}
	if strings.Contains(content, `"notes"`) {
		t.Errorf("notes should be omitted when empty; got %s", content)
	}
}

// TestPeerHaltPayloadParseMalformed locks defensive Parse: malformed
// JSON returns zero-value + non-nil error.
func TestPeerHaltPayloadParseMalformed(t *testing.T) {
	got, err := ParsePeerHaltContent("not valid json")
	if err == nil {
		t.Errorf("expected parse error on malformed JSON, got nil")
	}
	if got.TriggerClass != "" {
		t.Errorf("expected zero-value payload on parse error, got %+v", got)
	}
}

// TestPeerHaltAckPayloadRoundTrip locks ack roundtrip.
func TestPeerHaltAckPayloadRoundTrip(t *testing.T) {
	want := PeerHaltAckPayload{
		AnchorVerify:    AckAnchorMismatchRecovered,
		RecoverySummary: "re-read § class-split; resumed normal R16 flow",
		ReanchorMsgID:   6360,
	}
	content, err := BuildPeerHaltAckContent(want)
	if err != nil {
		t.Fatalf("BuildPeerHaltAckContent: %v", err)
	}
	got, err := ParsePeerHaltAckContent(content)
	if err != nil {
		t.Fatalf("ParsePeerHaltAckContent: %v", err)
	}
	if got != want {
		t.Errorf("ack roundtrip mismatch:\n got=%+v\nwant=%+v", got, want)
	}
}

// TestPeerHaltAckPayloadAllResultValues locks the 3 PeerHaltAckResult
// constants are usable + roundtrip cleanly.
func TestPeerHaltAckPayloadAllResultValues(t *testing.T) {
	results := []PeerHaltAckResult{
		AckAnchorMatch,
		AckAnchorMismatchRecovered,
		AckAnchorAbsent,
	}
	for _, r := range results {
		payload := PeerHaltAckPayload{AnchorVerify: r, RecoverySummary: "test"}
		content, err := BuildPeerHaltAckContent(payload)
		if err != nil {
			t.Errorf("BuildPeerHaltAckContent(%q): %v", r, err)
			continue
		}
		got, err := ParsePeerHaltAckContent(content)
		if err != nil {
			t.Errorf("ParsePeerHaltAckContent(%q): %v", r, err)
			continue
		}
		if got.AnchorVerify != r {
			t.Errorf("ack result roundtrip: got %q, want %q", got.AnchorVerify, r)
		}
	}
}

// TestPeerHaltAckPayloadOmitEmpty locks forward-compat for ack payload.
func TestPeerHaltAckPayloadOmitEmpty(t *testing.T) {
	minimal := PeerHaltAckPayload{
		AnchorVerify:    AckAnchorMatch,
		RecoverySummary: "no drift; verified clean",
	}
	content, err := BuildPeerHaltAckContent(minimal)
	if err != nil {
		t.Fatalf("BuildPeerHaltAckContent: %v", err)
	}
	if strings.Contains(content, `"reanchor_msg_id"`) {
		t.Errorf("reanchor_msg_id should be omitted when zero; got %s", content)
	}
}

// TestPeerHaltAllTriggerValues locks the 7 trigger constants are usable.
// Phase K K-17 enumeration covers today's bilateral-deviation taxonomy +
// 4 user-flagged lost-discipline classes.
func TestPeerHaltAllTriggerValues(t *testing.T) {
	triggers := []PeerHaltTrigger{
		TriggerClassSplitViolation,
		TriggerOutboundMissPattern,
		TriggerPerInstanceFireGreenflagSkip,
		TriggerR12BrainSecondSkip,
		TriggerAnchorChecksumMismatch,
		TriggerPMTreatedAsBroadcast,
		TriggerForcePushWithoutElevatedGate,
	}
	if len(triggers) != 7 {
		t.Errorf("expected 7 trigger constants; got %d", len(triggers))
	}
	for _, tr := range triggers {
		payload := PeerHaltPayload{
			TriggerClass:    tr,
			RecoveryPointer: "discipline-anchors.md",
		}
		content, err := BuildPeerHaltContent(payload)
		if err != nil {
			t.Errorf("BuildPeerHaltContent(%q): %v", tr, err)
		}
		got, err := ParsePeerHaltContent(content)
		if err != nil {
			t.Errorf("ParsePeerHaltContent(%q): %v", tr, err)
		}
		if got.TriggerClass != tr {
			t.Errorf("trigger roundtrip: got %q, want %q", got.TriggerClass, tr)
		}
	}
}
