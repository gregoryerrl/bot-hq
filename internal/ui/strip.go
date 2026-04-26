package ui

import (
	"fmt"
	"sort"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// stripAgentCap is the hard cap on agents shown in the hub-strip. Surplus
// agents collapse to a "+N" suffix. Spec §4 commit 4.
const stripAgentCap = 8

// agentTypeTier returns the display tier for an agent type. Lower = earlier
// in the strip ordering. Locked at three tiers per spec §4 commit 4.
//
// Robust to new agent types: unknown types fall to tier 4 (after coders).
func agentTypeTier(t protocol.AgentType) int {
	switch t {
	case protocol.AgentBrian, protocol.AgentQA, protocol.AgentVoice:
		return 1 // peer-coord agents — most relevant to first-order check
	case protocol.AgentDiscord, protocol.AgentGemma:
		return 2 // service agents
	case protocol.AgentCoder:
		return 3 // worker agents
	}
	return 4
}

// renderStrip renders the per-agent activity strip displayed above the
// HubTab input bar. Hides only ActivityOffline (explicitly disconnected /
// status=offline short-circuits to ActivityOffline at panestate.go:303-304).
// ActivityStale agents stay visible with the dim Stale dot so quiet-but-
// registered agents don't vanish during system-wide idle windows. Caps at
// stripAgentCap agents; surplus collapses to "+N".
//
// Returns an empty string when zero alive agents — caller should still
// reserve a separator line so the input bar position stays stable across
// strip-empty/non-empty transitions.
func renderStrip(snap []panestate.AgentSnapshot) string {
	alive := make([]panestate.AgentSnapshot, 0, len(snap))
	for _, s := range snap {
		if s.Activity != panestate.ActivityOffline {
			alive = append(alive, s)
		}
	}
	sort.SliceStable(alive, func(i, j int) bool {
		ti, tj := agentTypeTier(alive[i].Type), agentTypeTier(alive[j].Type)
		if ti != tj {
			return ti < tj
		}
		return alive[i].Name < alive[j].Name
	})

	visible := alive
	surplus := 0
	if len(alive) > stripAgentCap {
		visible = alive[:stripAgentCap]
		surplus = len(alive) - stripAgentCap
	}

	parts := make([]string, 0, len(visible)+1)
	for _, s := range visible {
		parts = append(parts, fmt.Sprintf("%s %s", activityDot(s.Activity), s.ID))
	}
	if surplus > 0 {
		parts = append(parts, fmt.Sprintf("+%d", surplus))
	}
	return strings.Join(parts, "  ")
}
