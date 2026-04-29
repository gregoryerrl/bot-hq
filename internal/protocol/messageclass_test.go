package protocol

import "testing"

// TestMessageClassAuthorizationEligibility_Exhaustive locks the
// IsAuthorizationEligible mapping for all 6 classes. Catches accidental
// drift in the auth-eligible set (e.g., MsgClassPeerPM accidentally
// flipped to true would let peer PMs authorize execute actions —
// regression on the user-msg-6391 lost-discipline class).
//
// Phase K K-18.
func TestMessageClassAuthorizationEligibility_Exhaustive(t *testing.T) {
	cases := []struct {
		class      MessageClass
		wantEligible bool
	}{
		{MsgClassUserBroadcast, true},
		{MsgClassUserPM, true},
		{MsgClassPeerPM, false},
		{MsgClassPeerBroadcast, false},
		{MsgClassObservation, false},
		{MsgClassSystemFlag, false},
	}
	for _, tc := range cases {
		got := tc.class.IsAuthorizationEligible()
		if got != tc.wantEligible {
			t.Errorf("MessageClass(%q).IsAuthorizationEligible() = %v, want %v",
				tc.class, got, tc.wantEligible)
		}
	}
}

// TestMessageClassValid_GarbageReturnsFalse locks defensive behavior:
// unknown / typo'd class strings return false (never accidentally
// authorize). Caller can't sneak a custom class through.
func TestMessageClassValid_GarbageReturnsFalse(t *testing.T) {
	cases := []MessageClass{
		"",
		"garbage",
		"USER-BROADCAST", // case-sensitive
		"user_broadcast", // wrong separator
		"hub-user",
	}
	for _, c := range cases {
		if c.IsAuthorizationEligible() {
			t.Errorf("MessageClass(%q).IsAuthorizationEligible() = true, want false (unknown class)", c)
		}
	}
}

// TestMessageClassUserBroadcast_SemanticAnchor locks the load-bearing
// invariant: [HUB:user] broadcast traffic IS auth-eligible. This is
// THE class that authorizes execute actions when user message-class
// matches.
func TestMessageClassUserBroadcast_SemanticAnchor(t *testing.T) {
	if !MsgClassUserBroadcast.IsAuthorizationEligible() {
		t.Errorf("MsgClassUserBroadcast.IsAuthorizationEligible() = false; expected true (user broadcasts authorize execute actions)")
	}
}

// TestMessageClassObservation_SemanticAnchor locks the inverse
// load-bearing invariant: observation traffic is NEVER auth-eligible.
// Cross-traffic the observer sees but isn't a direct recipient of
// must never authorize execute actions, regardless of from-direction
// (user→peer OR peer→peer).
func TestMessageClassObservation_SemanticAnchor(t *testing.T) {
	if MsgClassObservation.IsAuthorizationEligible() {
		t.Errorf("MsgClassObservation.IsAuthorizationEligible() = true; expected false (observation traffic never authorizes)")
	}
}

// TestAllMessageClasses_Closure locks the schema-lock invariant: the
// AllMessageClasses slice must contain exactly 6 entries (one per
// declared MsgClass* constant). Mirrors the ActiveMessageTypes pattern
// from R21 Phase J T1.7.
//
// Catches: accidental class removal without slice update (slice would
// shrink); accidental class addition without slice update (slice
// would have stale entries vs constants); accidental rename
// (slice has dropped + new entries don't match constants).
func TestAllMessageClasses_Closure(t *testing.T) {
	if len(AllMessageClasses) != 6 {
		t.Errorf("AllMessageClasses count = %d, want 6", len(AllMessageClasses))
	}

	wantSet := map[MessageClass]bool{
		MsgClassUserBroadcast: true,
		MsgClassUserPM:        true,
		MsgClassPeerPM:        true,
		MsgClassPeerBroadcast: true,
		MsgClassObservation:   true,
		MsgClassSystemFlag:    true,
	}
	for _, c := range AllMessageClasses {
		if !wantSet[c] {
			t.Errorf("AllMessageClasses contains unexpected class %q", c)
		}
	}
	if len(wantSet) != len(AllMessageClasses) {
		t.Errorf("wantSet size %d != AllMessageClasses size %d (drift in test or constant set)",
			len(wantSet), len(AllMessageClasses))
	}
}
