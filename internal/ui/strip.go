package ui

import (
	"fmt"
	"sort"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// stripAgentCap is the hard cap on agents shown in the hub-strip. Surplus
// agents collapse to a "+N" suffix. Spec §4 commit 4.
const stripAgentCap = 8

// usageSegmentMinPct is the threshold below which the right-aligned usage
// segment is omitted. ≥80% = squeeze worth surfacing; below that the segment
// would clutter the strip during normal operation. H-30.
const usageSegmentMinPct = 80

// usageSegmentMinSpacer is the minimum gap between the left dot row and the
// right usage segment. Below this we omit the segment rather than render
// agents touching the percent — narrow widths shouldn't squeeze the strip
// into illegibility.
const usageSegmentMinSpacer = 2

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
//
// width controls the right-aligned UsagePct segment (H-30). When width > 0
// and at least one visible agent has UsagePct ≥ usageSegmentMinPct, append a
// `<agent-id> NN%` segment color-tiered by severity at the right edge,
// padded by spaces. width ≤ 0 (legacy callsites / tests not exercising the
// segment) skips the right segment entirely — output is the bare dot row.
func renderStrip(snap []panestate.AgentSnapshot, width int) string {
	alive := make([]panestate.AgentSnapshot, 0, len(snap))
	for _, s := range snap {
		if s.Activity == panestate.ActivityOffline {
			continue
		}
		// Phase G v1 #20: stale-gen agents (registered pre-rebuild) are
		// omitted from the strip — they're definitely-stale and would
		// clutter the first-order check. Agents tab still shows them with
		// a (stale-gen) suffix for the user to prune manually.
		if s.StaleGen {
			continue
		}
		alive = append(alive, s)
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
	left := strings.Join(parts, "  ")

	right := renderUsageSegment(visible)
	if right == "" || width <= 0 {
		return left
	}
	spacer := width - lipgloss.Width(left) - lipgloss.Width(right)
	if spacer < usageSegmentMinSpacer {
		return left
	}
	return left + strings.Repeat(" ", spacer) + right
}

// renderUsageSegment returns the colored right-aligned segment when at least
// one visible agent has UsagePct ≥ usageSegmentMinPct. Empty string means
// "omit segment" — either no agent has known UsagePct or none crosses the
// threshold. Agents with UsagePct == -1 are excluded from the max calc per
// design (parse-unknown / no-pane should not surface as 0%).
func renderUsageSegment(visible []panestate.AgentSnapshot) string {
	maxPct := -1
	maxAgent := ""
	for _, s := range visible {
		if s.UsagePct < 0 {
			continue
		}
		if s.UsagePct > maxPct {
			maxPct = s.UsagePct
			maxAgent = s.ID
		}
	}
	if maxPct < usageSegmentMinPct {
		return ""
	}
	style := usageTierStyle(maxPct)
	return style.Render(fmt.Sprintf("%s %d%%", maxAgent, maxPct))
}

// usageTierStyle picks the color tier for the right-aligned usage segment.
// yellow ≥80, orange ≥90, red ≥95 per H-30 design. Reuses palette colors
// from styles.go for cross-surface consistency.
func usageTierStyle(pct int) lipgloss.Style {
	switch {
	case pct >= 95:
		return lipgloss.NewStyle().Foreground(ColorError).Bold(true)
	case pct >= 90:
		return lipgloss.NewStyle().Foreground(ColorBrian).Bold(true)
	default:
		return lipgloss.NewStyle().Foreground(ColorSession)
	}
}
