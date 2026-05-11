package daemoncron

// Simple-cluster surfaces — Phase S S-1a-5 (final S-1a sub-commit).
//
// Migrates 3 sparse-test-density emit surfaces from gemma to
// daemoncron, grouped per Rain msg 15799 plan ("4 sparse-test-
// density surfaces grouped reduces commit-overhead"):
//   1. context-cap-warning — per-agent UsagePct ≥95% halt-flag
//   2. delivery-gap — pending hub-msg queue past deliveryGapAge
//   3. egress-audit — pane-advanced-no-hub_send for N ticks
//
// 4th surface from Rain's seal — sentinel-queuefail — is ALREADY
// hub-side (internal/hub/hub.go:404 emits `[queue] Message N to A
// failed after K attempts`); gemma's sentinel.go is the detection-
// matching layer (NOT emit-source). No migration needed for this
// surface; cite-from-actual scope-correction noted in commit-body
// + S-close housekeeping.
//
// Phase-S-followup: gemma's emit-call-sites delegate unconditionally
// to the helpers below.

import (
	"fmt"
	"log"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// simpleClusterAgentID is the FromAgent for daemon-fired audit
	// events (context-cap CRITICAL, DELIVERY-GAP, EGRESS-GAP,
	// ROSTER-PRUNE). Z-8b: changed from "emma" to "system" per
	// hub-message-truth principle — these emits are threshold logic
	// only, no LLM invocation. Recipients still match on the content
	// prefix.
	simpleClusterAgentID = "system"

	// haltReasonPrefix mirrors gemma const value — locked literal
	// substring brian/rain STARTUP prompts match against ("agent
	// <id> at <N>%, halt"). Any reformat MUST stay consumer-
	// recognition-compatible.
	haltReasonPrefix = "agent %s at %d%%, halt + checkpoint via H-15 + idle for fresh session"
)

// BuildContextCapCriticalContent formats the per-agent context-cap
// halt-flag content. Recipient: user (MsgFlag class).
func BuildContextCapCriticalContent(agentID string, pct int) string {
	reason := fmt.Sprintf(haltReasonPrefix, agentID, pct)
	return fmt.Sprintf("[CRITICAL] %s", reason)
}

// BuildContextCapHaltReason formats the bare halt-reason string used
// in SetHaltActive call (gemma-side; daemoncron exposes the
// formatter for parity).
func BuildContextCapHaltReason(agentID string, pct int) string {
	return fmt.Sprintf(haltReasonPrefix, agentID, pct)
}

// BuildDeliveryGapContent formats the [DELIVERY-GAP] notice for a
// pending hub-msg queue row past deliveryGapAge.
func BuildDeliveryGapContent(messageID int64, targetAgent string, age time.Duration, queueID int64, attempts int) string {
	return fmt.Sprintf("[DELIVERY-GAP] msg %d to %s pending for %s (queue-id %d, %d attempts)", messageID, targetAgent, age, queueID, attempts)
}

// BuildEgressAuditContent formats the [EGRESS-GAP] notice for an
// agent whose pane advanced N ticks but emitted no hub_send.
func BuildEgressAuditContent(agentID string, gapTicks int, snippet string) string {
	return fmt.Sprintf("[EGRESS-GAP] agent %s pane advanced over %d ticks but no hub_send. Last line: %q", agentID, gapTicks, snippet)
}

// EmitContextCapCritical writes the [CRITICAL] context-cap halt-flag
// MsgFlag to user. Caller (gemma context_cap.go) is responsible for
// shouldFlag rate-cap + SetHaltActive side effects.
func EmitContextCapCritical(db dbInserter, now time.Time, agentID string, pct int) {
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: simpleClusterAgentID,
		ToAgent:   "user",
		Type:      protocol.MsgFlag,
		Content:   BuildContextCapCriticalContent(agentID, pct),
		Created:   now,
	}); err != nil {
		log.Printf("[daemoncron context-cap critical] flag insert failed for %s: %v", agentID, err)
	}
}

// EmitDeliveryGap writes the [DELIVERY-GAP] broadcast MsgUpdate.
// Caller (gemma delivery_audit.go) owns deliveryFlagTracker dedupe
// and pruning state.
func EmitDeliveryGap(db dbInserter, messageID int64, targetAgent string, age time.Duration, queueID int64, attempts int) {
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: simpleClusterAgentID,
		Type:      protocol.MsgUpdate,
		Content:   BuildDeliveryGapContent(messageID, targetAgent, age, queueID, attempts),
	}); err != nil {
		log.Printf("[daemoncron delivery-gap] insert failed for msg %d: %v", messageID, err)
	}
}

// EmitEgressAudit writes the [EGRESS-GAP] broadcast MsgUpdate.
// Caller (gemma egress_audit.go) owns egressFlagTracker dedupe and
// pane-baseline state.
func EmitEgressAudit(db dbInserter, agentID string, gapTicks int, snippet string) {
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: simpleClusterAgentID,
		Type:      protocol.MsgUpdate,
		Content:   BuildEgressAuditContent(agentID, gapTicks, snippet),
	}); err != nil {
		log.Printf("[daemoncron egress-audit] insert failed for %s: %v", agentID, err)
	}
}
