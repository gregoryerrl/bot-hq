package ui

import (
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
)

var (
	// Tab bar styles
	ActiveTabStyle   = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#FFFFFF")).Background(lipgloss.Color("#7D56F4"))
	InactiveTabStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#888888"))
	TabBarStyle      = lipgloss.NewStyle().BorderBottom(true).BorderStyle(lipgloss.NormalBorder())

	// Message feed colors
	ColorSystem  = lipgloss.Color("#22C55E") // green — system events
	ColorClive   = lipgloss.Color("#3B82F6") // blue — Clive (voice)
	ColorBrian   = lipgloss.Color("#F97316") // orange — Brian (orchestrator)
	ColorRain    = lipgloss.Color("#EF4444") // red — Rain (adversarial QA)
	ColorCoder   = lipgloss.Color("#A855F7") // purple — Claude Code sessions
	ColorDiscord = lipgloss.Color("#22D3EE") // cyan — Discord
	ColorError   = lipgloss.Color("#EF4444") // red — errors
	ColorStatus  = lipgloss.Color("#6B7280") // gray — status updates
	ColorSession = lipgloss.Color("#EAB308") // yellow — handshakes, sessions

	// Activity dot styles. Supersedes Phase D's StatusOnline / StatusOffline pair —
	// the activity-derived model (panestate) is the single source of dot truth.
	ActivityWorkingStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#22C55E")).SetString("●") // green: actively executing
	ActivityOnlineStyle  = lipgloss.NewStyle().Foreground(lipgloss.Color("#06B6D4")).SetString("◐") // cyan: present, not active
	ActivityStaleStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("#6B7280")).SetString("○") // gray: quiet, escalate
	ActivityOfflineStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#404040")).SetString("·") // dim: disconnected

	// General styles
	TitleStyle    = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#FFFFFF"))
	SubtitleStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#888888"))
	BorderStyle   = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(lipgloss.Color("#555555"))
)

// activityDot returns the rendered glyph (with color) for the given activity.
// Used by both the hub-strip render and the agents-tab Status column so the
// two surfaces stay visually consistent.
func activityDot(a panestate.AgentActivity) string {
	switch a {
	case panestate.ActivityWorking:
		return ActivityWorkingStyle.String()
	case panestate.ActivityOnline:
		return ActivityOnlineStyle.String()
	case panestate.ActivityStale:
		return ActivityStaleStyle.String()
	case panestate.ActivityOffline:
		return ActivityOfflineStyle.String()
	}
	return "?"
}
