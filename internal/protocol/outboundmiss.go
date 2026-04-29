package protocol

import "strings"

// outboundMissPrefix is the canonical prefix the OUTBOUND-MISS Stop hook
// emits to flag pane-text-only emissions (per outboundhook package
// hook.go format).
const outboundMissPrefix = "[OUTBOUND-MISS]"

// IsOutboundMissNotification reports whether a hub Message is an
// OUTBOUND-MISS system flag against the given self agent ID. Returns
// true when:
//  1. msg.FromAgent == selfAgentID — the flag is "from" the agent
//     being flagged because the OUTBOUND-MISS hook attributes the
//     notification to the agent whose pane-text-only emission was
//     detected. This makes the system-flag LOOK like the agent's own
//     legitimate output in the hub feed.
//  2. msg.Content starts with the canonical [OUTBOUND-MISS] prefix
//     (deterministic; resistant to false-positives from conversational
//     text that quotes the prefix mid-content).
//
// Both conditions required: from-self alone could be the agent's own
// legit output; prefix alone could be a flag against another agent
// (peer's OUTBOUND-MISS) — the observer ignores those (per
// MsgClassObservation per K-18; not actionable for non-target).
//
// Phase K K-14. Closes the recognition-gap surfaced bcc-ad-manager
// session 2026-04-29 (Rain misread msg 6071 OUTBOUND-MISS notification
// as her own legit reply per msg 6076 "already-answered-user-via-msg-
// 6071" claim — Rain pattern-matched on from_agent=rain without
// inspecting content prefix). Pairs with K-18 MsgClassSystemFlag (the
// notification's class) and R27 discipline rule (recovery flow).
func IsOutboundMissNotification(msg Message, selfAgentID string) bool {
	if msg.FromAgent != selfAgentID {
		return false
	}
	return strings.HasPrefix(strings.TrimSpace(msg.Content), outboundMissPrefix)
}
