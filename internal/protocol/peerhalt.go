package protocol

import (
	"encoding/json"
	"fmt"
)

// PeerHaltTrigger enumerates the recognized drift classes that warrant
// emitting a MsgPeerHalt. New triggers added as future drift classes
// surface empirically; sticking to a closed enum prevents free-form
// trigger_class strings that obscure the discipline-rhythm.
//
// Phase K K-17 — covers today's bilateral-deviation taxonomy + the 4
// user-flagged lost-discipline classes (msgs 6381 / 6391).
type PeerHaltTrigger string

const (
	// TriggerClassSplitViolation: peer fired HANDS-class action while
	// EYES (rain firing gh pr create) or vice versa. K-16 gates pre-fire;
	// this trigger covers post-fire detection.
	TriggerClassSplitViolation PeerHaltTrigger = "class-split-violation"

	// TriggerOutboundMissPattern: peer's recent N msgs show pane-text-only
	// emission pattern (R20 OUTBOUND violations).
	TriggerOutboundMissPattern PeerHaltTrigger = "outbound-miss-pattern"

	// TriggerPerInstanceFireGreenflagSkip: peer fired execute-class action
	// (commit / push / merge / gh-pr-create / etc.) without explicit user
	// verbatim authorization; relied on broader prior auth instead.
	TriggerPerInstanceFireGreenflagSkip PeerHaltTrigger = "per-instance-fire-greenflag-skip"

	// TriggerR12BrainSecondSkip: peer fired commit without surfacing diff
	// for peer BRAIN-2nd review (R12 GATE-PROTOCOL violation).
	TriggerR12BrainSecondSkip PeerHaltTrigger = "r12-brain-2nd-skip"

	// TriggerAnchorChecksumMismatch: K-12 VerifyDisciplineAnchor returned
	// matches=false (auto-trigger via R16 bootstrap when stored SHA differs
	// from current).
	TriggerAnchorChecksumMismatch PeerHaltTrigger = "anchor-checksum-mismatch"

	// TriggerPMTreatedAsBroadcast: peer acted on PM-implied direction
	// without user broadcast authorization (PMs are peer-coord; only user
	// broadcast authorizes execute).
	TriggerPMTreatedAsBroadcast PeerHaltTrigger = "pm-treated-as-broadcast"

	// TriggerForcePushWithoutElevatedGate: peer fired force-push without
	// K-19 dual-greenflag (peer BRAIN-2nd + user explicit verbatim).
	TriggerForcePushWithoutElevatedGate PeerHaltTrigger = "force-push-without-elevated-gate"
)

// PeerHaltPayload is the structured JSON content of a MsgPeerHalt
// message. Caller MarshalJSON via BuildPeerHaltContent; recipient
// UnmarshalJSON via ParsePeerHaltContent. Forward-compat via omitempty
// on optional fields (future enrichment doesn't break old parsers).
//
// Phase K K-17.
type PeerHaltPayload struct {
	// TriggerClass identifies which drift class the sender observed.
	// One of the PeerHaltTrigger constants.
	TriggerClass PeerHaltTrigger `json:"trigger_class"`

	// ObservedMsgID points at the specific hub message that surfaced the
	// drift (e.g., the PM where the peer acted on PM-implied direction,
	// the commit message lacking peer-greenflag-msg-id reference, etc.).
	// 0 when no specific msg-id applies (e.g., anchor-checksum-mismatch
	// triggered by R16 bootstrap).
	ObservedMsgID int64 `json:"observed_msg_id,omitempty"`

	// RecoveryPointer is a file:section reference into discipline-anchors.md
	// (e.g., "~/.bot-hq/rain/discipline-anchors.md § class-split"). Gives
	// recipient exact re-read location during standdown.
	RecoveryPointer string `json:"recovery_pointer"`

	// Notes is free-form additional context the sender wants to attach
	// (e.g., "saw 3 OUTBOUND-MISS in last 10 msgs"). Optional.
	Notes string `json:"notes,omitempty"`
}

// PeerHaltAckResult enumerates the outcomes a MsgPeerHaltAck reports.
type PeerHaltAckResult string

const (
	// AckAnchorMatch: K-12 VerifyDisciplineAnchor returned matches=true
	// post-re-read; no mismatch detected.
	AckAnchorMatch PeerHaltAckResult = "match"

	// AckAnchorMismatchRecovered: K-12 returned matches=false on first
	// check; recipient re-read discipline-anchors.md + persisted current
	// SHA + resumed normal R16 flow.
	AckAnchorMismatchRecovered PeerHaltAckResult = "mismatch-recovered"

	// AckAnchorAbsent: discipline-anchors.md was missing at recipient's
	// per-agent dir. Recipient flagged the missing file (recovery surface
	// requires user / brian to write the file before next halt cycle can
	// verify).
	AckAnchorAbsent PeerHaltAckResult = "anchor-absent"
)

// PeerHaltAckPayload is the structured JSON content of a MsgPeerHaltAck
// message. Phase K K-17.
type PeerHaltAckPayload struct {
	// AnchorVerify reports the K-12 VerifyDisciplineAnchor outcome.
	AnchorVerify PeerHaltAckResult `json:"anchor_verify"`

	// RecoverySummary is brief free-form description of what was
	// re-anchored or fixed (e.g., "re-read § class-split; resumed").
	RecoverySummary string `json:"recovery_summary"`

	// ReanchorMsgID is the msg-id at which recipient resumed normal work.
	// Optional — supports traceability across the halt-recover cycle.
	ReanchorMsgID int64 `json:"reanchor_msg_id,omitempty"`
}

// BuildPeerHaltContent marshals a PeerHaltPayload to JSON for use as
// MsgPeerHalt.Content. Returns ("", err) on marshal error.
func BuildPeerHaltContent(payload PeerHaltPayload) (string, error) {
	data, err := json.Marshal(payload)
	if err != nil {
		return "", fmt.Errorf("marshal peer-halt payload: %w", err)
	}
	return string(data), nil
}

// ParsePeerHaltContent unmarshals a MsgPeerHalt.Content string into a
// PeerHaltPayload. Returns (zero-value, err) on parse error so callers
// can defensively fall through.
func ParsePeerHaltContent(content string) (PeerHaltPayload, error) {
	var p PeerHaltPayload
	if err := json.Unmarshal([]byte(content), &p); err != nil {
		return PeerHaltPayload{}, fmt.Errorf("parse peer-halt payload: %w", err)
	}
	return p, nil
}

// BuildPeerHaltAckContent marshals a PeerHaltAckPayload to JSON.
func BuildPeerHaltAckContent(payload PeerHaltAckPayload) (string, error) {
	data, err := json.Marshal(payload)
	if err != nil {
		return "", fmt.Errorf("marshal peer-halt-ack payload: %w", err)
	}
	return string(data), nil
}

// ParsePeerHaltAckContent unmarshals a MsgPeerHaltAck.Content string
// into a PeerHaltAckPayload.
func ParsePeerHaltAckContent(content string) (PeerHaltAckPayload, error) {
	var p PeerHaltAckPayload
	if err := json.Unmarshal([]byte(content), &p); err != nil {
		return PeerHaltAckPayload{}, fmt.Errorf("parse peer-halt-ack payload: %w", err)
	}
	return p, nil
}
