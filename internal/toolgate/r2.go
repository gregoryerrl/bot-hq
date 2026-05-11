// Package toolgate — Phase R R2 hub_broadcast Rain-gate hook.
//
// Per Phase R R2 cluster + Rain msg 15561 BRAIN-2nd Refine-5: literal-
// string match on `mcp__bot-hq__hub_broadcast` tool name; Rain-only
// invocation (BOT_HQ_AGENT_ID=rain). Brian invocation hard-blocks at
// PreToolUse hook layer with reason cite + recovery hint.
//
// Mirrors R33 PRE-EXECUTE-GATE-FILE-READ + R36 OUTBOUND-DISCIPLINE-
// MECHANICAL precedents (literal tool-name match in hook + agent-id
// env-var check).

package toolgate

const (
	// HubBroadcastToolName is the literal MCP tool name protected by
	// the Rain-only gate per Phase R R2 Refine-5 exact-match discipline.
	HubBroadcastToolName = "mcp__bot-hq__hub_broadcast"

	// HubBroadcastRainGateBlockMsg is the stderr block message emitted
	// when a non-Rain agent attempts hub_broadcast invocation.
	HubBroadcastRainGateBlockMsg = "Phase R R2 hub_broadcast Rain-only gate: tool blocked.\n" +
		"Reason: hub_broadcast is reserved for Rain (BOT_HQ_AGENT_ID=rain) per Phase R R1 [HR]-only-Rain authority.\n" +
		"Re-anchor: ~/.bot-hq/projects/bot-hq/phase/phase-r.md R2 cluster + R1 BRAIN-cycle hardening.\n" +
		"Recovery: use hub_send for non-[HR] broadcasts; only Rain emits [HR]-tagged duo-consensus content.\n"
)

// VerifyHubBroadcastRainGate returns ExitBlock if a non-Rain agent
// attempts hub_broadcast invocation; ExitAllow otherwise. agentID is
// read from BOT_HQ_AGENT_ID env-var by the caller (RunHook).
//
// Empty agentID → ExitAllow (defensive: don't block non-duo Claude
// Code instances; mirrors K-16 + R33 defensive default).
func VerifyHubBroadcastRainGate(agentID string) (allow bool, reason string) {
	if agentID == "" {
		return true, ""
	}
	if agentID == "rain" {
		return true, ""
	}
	return false, HubBroadcastRainGateBlockMsg
}
