package protocol

import "testing"

// TestMessageTypeTaxonomy locks the Phase J T1.7 (B7 codification) sets:
// 6 active types + 2 deprecated types. Catches accidental drift in either
// direction (active type removed without DEPRECATED marker; deprecated
// type re-promoted without removing from DeprecatedMessageTypes).
func TestMessageTypeTaxonomy(t *testing.T) {
	wantActive := map[MessageType]bool{
		MsgResponse: true,
		MsgCommand:  true,
		MsgUpdate:   true,
		MsgResult:   true,
		MsgError:    true,
		MsgFlag:     true,
	}
	wantDeprecated := map[MessageType]bool{
		MsgHandshake: true,
		MsgQuestion:  true,
	}

	if len(ActiveMessageTypes) != 6 {
		t.Errorf("ActiveMessageTypes count = %d, want 6", len(ActiveMessageTypes))
	}
	for _, m := range ActiveMessageTypes {
		if !wantActive[m] {
			t.Errorf("ActiveMessageTypes contains unexpected type %q", m)
		}
		if !m.IsActive() {
			t.Errorf("%q.IsActive() = false; expected true", m)
		}
		if m.IsDeprecated() {
			t.Errorf("%q.IsDeprecated() = true; expected false (active type)", m)
		}
	}

	if len(DeprecatedMessageTypes) != 2 {
		t.Errorf("DeprecatedMessageTypes count = %d, want 2", len(DeprecatedMessageTypes))
	}
	for _, m := range DeprecatedMessageTypes {
		if !wantDeprecated[m] {
			t.Errorf("DeprecatedMessageTypes contains unexpected type %q", m)
		}
		if m.IsActive() {
			t.Errorf("%q.IsActive() = true; expected false (deprecated type)", m)
		}
		if !m.IsDeprecated() {
			t.Errorf("%q.IsDeprecated() = false; expected true", m)
		}
	}

	// Cross-check: active ∪ deprecated covers all Valid() types.
	all := []MessageType{MsgHandshake, MsgQuestion, MsgResponse, MsgCommand, MsgUpdate, MsgResult, MsgError, MsgFlag}
	for _, m := range all {
		if !m.Valid() {
			t.Errorf("%q.Valid() = false; expected all enumerated types valid", m)
		}
		if !m.IsActive() && !m.IsDeprecated() {
			t.Errorf("%q is neither active nor deprecated; taxonomy gap", m)
		}
		if m.IsActive() && m.IsDeprecated() {
			t.Errorf("%q is BOTH active and deprecated; taxonomy contradiction", m)
		}
	}
}
