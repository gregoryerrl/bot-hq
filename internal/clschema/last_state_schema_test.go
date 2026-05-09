package clschema

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

// ====== ValidateLastStateJSON ======

func TestValidate_validInput(t *testing.T) {
	raw := []byte(`{
		"agent_id": "brian",
		"last_self_msg_id": 17266,
		"saved_at_utc": "2026-05-10T00:05:00Z",
		"phase": "Phase T REOPENED 2026-05-09",
		"active_workstream": "T-8 cluster firing"
	}`)
	if err := ValidateLastStateJSON(raw); err != nil {
		t.Errorf("expected valid input to pass, got: %v", err)
	}
}

func TestValidate_minimalRequiredOnlyValid(t *testing.T) {
	// Just the 4 required fields; optional active_workstream omitted.
	raw := []byte(`{
		"agent_id": "rain",
		"last_self_msg_id": 1,
		"saved_at_utc": "2026-05-10T00:00:00Z",
		"phase": "test"
	}`)
	if err := ValidateLastStateJSON(raw); err != nil {
		t.Errorf("minimal-required input should pass: %v", err)
	}
}

func TestValidate_emptyInputRejected(t *testing.T) {
	if err := ValidateLastStateJSON(nil); err == nil {
		t.Error("expected error for nil input")
	}
	if err := ValidateLastStateJSON([]byte{}); err == nil {
		t.Error("expected error for empty input")
	}
}

func TestValidate_invalidJSONRejected(t *testing.T) {
	err := ValidateLastStateJSON([]byte("{not valid json"))
	if err == nil {
		t.Fatal("expected error for malformed JSON")
	}
	if !errors.Is(err, ErrInvalidLastState) {
		t.Errorf("err = %v, want errors.Is ErrInvalidLastState", err)
	}
}

func TestValidate_missingRequiredFieldsRejected(t *testing.T) {
	cases := []struct {
		name string
		raw  string
	}{
		{"missing agent_id", `{"last_self_msg_id":1,"saved_at_utc":"x","phase":"x"}`},
		{"missing last_self_msg_id", `{"agent_id":"x","saved_at_utc":"x","phase":"x"}`},
		{"missing saved_at_utc", `{"agent_id":"x","last_self_msg_id":1,"phase":"x"}`},
		{"missing phase", `{"agent_id":"x","last_self_msg_id":1,"saved_at_utc":"x"}`},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			err := ValidateLastStateJSON([]byte(c.raw))
			if err == nil {
				t.Fatal("expected error for missing required field")
			}
			if !strings.Contains(err.Error(), "missing required field") {
				t.Errorf("err = %v, want containing 'missing required field'", err)
			}
		})
	}
}

func TestValidate_emptyRequiredStringFieldsRejected(t *testing.T) {
	raw := []byte(`{"agent_id":"","last_self_msg_id":1,"saved_at_utc":"x","phase":"x"}`)
	err := ValidateLastStateJSON(raw)
	if err == nil {
		t.Fatal("expected error for empty agent_id")
	}
	if !strings.Contains(err.Error(), "non-empty") {
		t.Errorf("err = %v, want containing 'non-empty'", err)
	}
}

func TestValidate_zeroOrNegativeMsgIDRejected(t *testing.T) {
	cases := []struct {
		name string
		raw  string
	}{
		{"zero msg-id", `{"agent_id":"x","last_self_msg_id":0,"saved_at_utc":"x","phase":"x"}`},
		{"negative msg-id", `{"agent_id":"x","last_self_msg_id":-1,"saved_at_utc":"x","phase":"x"}`},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			err := ValidateLastStateJSON([]byte(c.raw))
			if err == nil {
				t.Fatal("expected error for non-positive msg-id")
			}
			if !strings.Contains(err.Error(), "must be > 0") {
				t.Errorf("err = %v, want containing 'must be > 0'", err)
			}
		})
	}
}

func TestValidate_wrongTypeRejected(t *testing.T) {
	// last_self_msg_id as string instead of number
	raw := []byte(`{"agent_id":"x","last_self_msg_id":"not-a-number","saved_at_utc":"x","phase":"x"}`)
	err := ValidateLastStateJSON(raw)
	if err == nil {
		t.Fatal("expected type error")
	}
	if !strings.Contains(err.Error(), "must be a number") {
		t.Errorf("err = %v, want containing 'must be a number'", err)
	}
}

func TestValidate_unknownFieldsPermissive(t *testing.T) {
	// Forward-compat: unknown fields don't block validation.
	raw := []byte(`{
		"agent_id": "brian",
		"last_self_msg_id": 1,
		"saved_at_utc": "x",
		"phase": "x",
		"future_field_xyz": "value",
		"another_extension": {"nested": true}
	}`)
	if err := ValidateLastStateJSON(raw); err != nil {
		t.Errorf("unknown-fields should be permissive (forward-compat), got: %v", err)
	}
}

// ====== ParseLastState ======

func TestParseLastState_validInputReturnsStruct(t *testing.T) {
	raw := []byte(`{
		"agent_id": "brian",
		"last_self_msg_id": 17266,
		"saved_at_utc": "2026-05-10T00:05:00Z",
		"phase": "Phase T REOPENED",
		"active_workstream": "T-8 cluster"
	}`)
	ls, err := ParseLastState(raw)
	if err != nil {
		t.Fatalf("ParseLastState: %v", err)
	}
	if ls.AgentID != "brian" {
		t.Errorf("AgentID = %q", ls.AgentID)
	}
	if ls.LastSelfMsgID != 17266 {
		t.Errorf("LastSelfMsgID = %d", ls.LastSelfMsgID)
	}
	if ls.ActiveWorkstream != "T-8 cluster" {
		t.Errorf("ActiveWorkstream = %q", ls.ActiveWorkstream)
	}
}

func TestParseLastState_invalidPropagatesValidationError(t *testing.T) {
	_, err := ParseLastState([]byte(`{"agent_id":"x"}`))
	if err == nil {
		t.Fatal("expected validation error")
	}
	if !errors.Is(err, ErrInvalidLastState) {
		t.Errorf("err = %v, want errors.Is ErrInvalidLastState", err)
	}
}

func TestParseLastState_roundTripPreservesRequiredFields(t *testing.T) {
	original := &LastState{
		AgentID:       "brian",
		LastSelfMsgID: 17266,
		SavedAtUTC:    "2026-05-10T00:05:00Z",
		Phase:         "Phase T REOPENED",
	}
	raw, err := json.Marshal(original)
	if err != nil {
		t.Fatalf("Marshal: %v", err)
	}
	parsed, err := ParseLastState(raw)
	if err != nil {
		t.Fatalf("ParseLastState round-trip: %v", err)
	}
	if parsed.AgentID != original.AgentID || parsed.LastSelfMsgID != original.LastSelfMsgID {
		t.Errorf("round-trip diverged: original=%+v parsed=%+v", original, parsed)
	}
}

func TestParseLastState_omitemptyElidesZeroOptionalFields(t *testing.T) {
	ls := &LastState{
		AgentID:       "rain",
		LastSelfMsgID: 1,
		SavedAtUTC:    "x",
		Phase:         "x",
		// ActiveWorkstream zero-value — should be elided
	}
	raw, _ := json.Marshal(ls)
	if strings.Contains(string(raw), "active_workstream") {
		t.Errorf("zero-value optional field should be elided per omitempty: %s", raw)
	}
}
