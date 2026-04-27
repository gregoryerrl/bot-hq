package ui

import (
	"fmt"
	"sort"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/anthropic"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// stripAgentCap is the hard cap on agents shown in the hub-strip. Surplus
// agents collapse to a "+N" suffix. Spec §4 commit 4.
const stripAgentCap = 8

// planSegmentMinSpacer is the minimum gap between the left dot row and
// the right plan-usage segment. Below this we omit the segment rather
// than render agents touching the percent — narrow widths shouldn't
// squeeze the strip into illegibility.
const planSegmentMinSpacer = 2

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
// HubTab input bar. Hides only ActivityOffline; ActivityStale agents stay
// visible. Caps at stripAgentCap agents; surplus collapses to "+N".
//
// width controls the right-aligned plan-usage segment introduced by
// slice 5 C1 (H-32). When width > 0 and hub.PlanUsagePct >= 0, append a
// `${pct}%${tag}` segment color-tiered by severity at the right edge,
// padded by spaces. width <= 0 skips the right segment entirely — output
// is the bare dot row. Slice 5 dropped the per-pane context-% segment
// (now lives in agents_tab.go as a column); the right segment is
// account-scoped plan utilization.
func renderStrip(snap []panestate.AgentSnapshot, hub panestate.HubSnapshot, width int) string {
	alive := make([]panestate.AgentSnapshot, 0, len(snap))
	for _, s := range snap {
		if s.Activity == panestate.ActivityOffline {
			continue
		}
		// Phase G v1 #20: stale-gen agents (registered pre-rebuild) are
		// omitted from the strip — they'd clutter the first-order check.
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

	right := renderPlanSegment(hub)
	if right == "" || width <= 0 {
		return left
	}
	spacer := width - lipgloss.Width(left) - lipgloss.Width(right)
	if spacer < planSegmentMinSpacer {
		return left
	}
	return left + strings.Repeat(" ", spacer) + right
}

// renderPlanSegment renders the right-aligned plan-usage segment. Empty
// string means "omit segment" — only when hub.PlanUsagePct == -1 AND no
// window has been observed yet (fresh-boot strip stays clean). Format:
//
//   - five_hour or unknown-window: `${pct}%`
//   - non-five_hour:               `${pct}%${tag}` with tag in
//     { " weekly", " opus", " extra" } per planWindowTag.
//   - PlanUsagePct = -1 with a known window: `--%` (signals the producer
//     hit a transient error, prior known state lost).
//
// Color tier: green <70 / yellow 70-89 / red ≥90.
func renderPlanSegment(hub panestate.HubSnapshot) string {
	if hub.PlanUsagePct < 0 {
		if hub.PlanWindow == "" {
			return ""
		}
		style := lipgloss.NewStyle().Foreground(ColorStatus)
		return style.Render("--%")
	}
	tag := planWindowTag(hub.PlanWindow)
	style := planUsageTierStyle(hub.PlanUsagePct)
	return style.Render(fmt.Sprintf("%d%%%s", hub.PlanUsagePct, tag))
}

// planWindowTag maps an oauth_usage window name to the strip suffix tag.
// five_hour (the default, most-frequently-binding window) renders bare so
// the segment stays compact; other windows surface as " weekly" /
// " opus" / " extra" so the binding limit is obvious.
func planWindowTag(window string) string {
	switch window {
	case anthropic.WindowFiveHour, "":
		return ""
	case anthropic.WindowSevenDay:
		return " weekly"
	case anthropic.WindowSevenDayOpus:
		return " opus"
	case anthropic.WindowSevenDaySonnet:
		return " extra"
	}
	return " " + window
}

// planUsageTierStyle picks the color tier for the right-aligned plan-
// usage segment. green <70 / yellow 70-89 / red ≥90.
func planUsageTierStyle(pct int) lipgloss.Style {
	switch {
	case pct >= 90:
		return lipgloss.NewStyle().Foreground(ColorError).Bold(true)
	case pct >= 70:
		return lipgloss.NewStyle().Foreground(ColorBrian).Bold(true)
	default:
		return lipgloss.NewStyle().Foreground(ColorSession)
	}
}
