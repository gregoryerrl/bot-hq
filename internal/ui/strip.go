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
// slice 5 C1 (H-32) and reshaped to dual-window by the slice-5 hotfix.
// When width > 0 and hub.PlanUsagePct >= 0, append a `5h:NN% 7d:NN%`
// segment color-tiered by max severity at the right edge, padded by
// spaces. width <= 0 skips the right segment entirely — output is the
// bare dot row. Slice 5 dropped the per-pane context-% segment (now
// lives in agents_tab.go as a column); the right segment is
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

// renderPlanSegment renders the right-aligned plan-usage dual-window
// segment. Format: `5h:NN% 7d:NN%` where each NN is the per-window pct
// or `--` when the window is unobserved/missing. Color tier is driven by
// PlanUsagePct (max-of-all) so the user's worst-case window dominates
// urgency signaling. Slice-5 hotfix: previous single-max display
// concealed the non-binding window — dual-window surfaces both at once.
//
// Edge cases:
//   - PlanUsagePct == -1 + empty PlanWindow → return "" (fresh boot,
//     omit segment so the strip stays clean before the first poll).
//   - PlanUsagePct == -1 + non-empty PlanWindow → render single `--%`
//     (producer hit a transient error after first observation; preserves
//     "alive but failing" diagnostic surface from H-40).
//
// Color tier: green <70 / yellow 70-89 / red ≥90 against PlanUsagePct.
func renderPlanSegment(hub panestate.HubSnapshot) string {
	if hub.PlanUsagePct < 0 {
		if hub.PlanWindow == "" {
			return ""
		}
		style := lipgloss.NewStyle().Foreground(ColorStatus)
		return style.Render("--%")
	}
	style := planUsageTierStyle(hub.PlanUsagePct)
	return style.Render(fmt.Sprintf("5h:%s 7d:%s", planPctText(hub.FiveHourPct), planPctText(hub.SevenDayPct)))
}

// planPctText formats a per-window pct for the dual-window strip. -1
// renders as `--%` to signal "window absent from API response" without
// confusing the reader into thinking it's a real 0% reading.
func planPctText(pct int) string {
	if pct < 0 {
		return "--%"
	}
	return fmt.Sprintf("%d%%", pct)
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
