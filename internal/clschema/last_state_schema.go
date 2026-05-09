// Package clschema implements schema-enforcement for Context Library
// (CL) artifacts per phase-t.md v5 T-8.9a.
//
// Initial scope: last_state.json validator. Per-agent R20 AgentState
// anchors are critical for cross-restart resume operational discipline
// (R16 + R20); a typed-validator catches malformed writes early instead
// of failing silently at next-resume bootstrap.
//
// Future T-8.9a-followup: schema-enforcement for markdown artifacts
// (phase-doc / ratchet-ledger / discipline-log); deferred since markdown
// classes need a different schema-class (frontmatter + structured-row
// validators rather than JSON-shape).
//
// Layering: clschema imports only stdlib (encoding/json + errors + fmt).
// NO hub or agentconfig dep — leaf package, no circular-import risk.

package clschema

import (
	"encoding/json"
	"errors"
	"fmt"
)

// LastState mirrors the brian/rain/<agent>/last_state.json document
// shape. Required fields enforced via ValidateLastStateJSON; optional
// fields use omitempty for marshaling.
type LastState struct {
	AgentID         string `json:"agent_id"`
	LastSelfMsgID   int64  `json:"last_self_msg_id"`
	SavedAtUTC      string `json:"saved_at_utc"`
	Phase           string `json:"phase"`
	ActiveWorkstream string `json:"active_workstream,omitempty"`
}

// requiredLastStateFields enumerates fields that MUST be present + non-
// empty for a last_state.json document to validate. Permissive on
// unknown fields (forward-compat per Rain msg 17277 design-flag).
var requiredLastStateFields = []string{
	"agent_id",
	"last_self_msg_id",
	"saved_at_utc",
	"phase",
}

// ErrInvalidLastState wraps validation failures.
var ErrInvalidLastState = errors.New("invalid last_state.json")

// ValidateLastStateJSON parses raw JSON + checks required-field presence
// + non-empty-value for string fields. Returns descriptive error
// (wrapping ErrInvalidLastState) on validation failure; nil on pass.
//
// Permissive on unknown fields: forward-compat as last_state.json shape
// grows without breaking older validators.
func ValidateLastStateJSON(raw []byte) error {
	if len(raw) == 0 {
		return fmt.Errorf("%w: empty input", ErrInvalidLastState)
	}
	var doc map[string]interface{}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return fmt.Errorf("%w: parse JSON: %v", ErrInvalidLastState, err)
	}
	for _, field := range requiredLastStateFields {
		v, ok := doc[field]
		if !ok {
			return fmt.Errorf("%w: missing required field %q", ErrInvalidLastState, field)
		}
		if err := validateRequiredFieldType(field, v); err != nil {
			return err
		}
	}
	return nil
}

// validateRequiredFieldType checks that required-field values are
// non-empty + correctly typed.
func validateRequiredFieldType(field string, v interface{}) error {
	switch field {
	case "last_self_msg_id":
		// JSON numbers parse as float64; reject if not numeric or zero.
		f, ok := v.(float64)
		if !ok {
			return fmt.Errorf("%w: %q must be a number, got %T", ErrInvalidLastState, field, v)
		}
		if f <= 0 {
			return fmt.Errorf("%w: %q must be > 0 (got %v)", ErrInvalidLastState, field, f)
		}
	default:
		// All other required fields are strings + non-empty.
		s, ok := v.(string)
		if !ok {
			return fmt.Errorf("%w: %q must be a string, got %T", ErrInvalidLastState, field, v)
		}
		if s == "" {
			return fmt.Errorf("%w: %q must be non-empty", ErrInvalidLastState, field)
		}
	}
	return nil
}

// ParseLastState validates raw JSON + unmarshals into a typed LastState
// struct. Returns descriptive error on validation failure (caller can
// errors.Is(err, ErrInvalidLastState) for the validation class).
//
// Round-trip-safe: ParseLastState→json.Marshal preserves required fields;
// optional fields omitempty-elided when zero-value.
func ParseLastState(raw []byte) (*LastState, error) {
	if err := ValidateLastStateJSON(raw); err != nil {
		return nil, err
	}
	var ls LastState
	if err := json.Unmarshal(raw, &ls); err != nil {
		return nil, fmt.Errorf("%w: unmarshal LastState: %v", ErrInvalidLastState, err)
	}
	return &ls, nil
}
