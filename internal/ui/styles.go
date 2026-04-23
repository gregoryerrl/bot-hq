package ui

import "github.com/charmbracelet/lipgloss"

var (
	// Tab bar styles
	ActiveTabStyle   = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#FFFFFF")).Background(lipgloss.Color("#7D56F4"))
	InactiveTabStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#888888"))
	TabBarStyle      = lipgloss.NewStyle().BorderBottom(true).BorderStyle(lipgloss.NormalBorder())

	// Message feed colors
	ColorSystem  = lipgloss.Color("#22C55E") // green — system events
	ColorLive    = lipgloss.Color("#3B82F6") // blue — Bot-HQ Live
	ColorCoder   = lipgloss.Color("#A855F7") // purple — Claude Code sessions
	ColorDiscord = lipgloss.Color("#F97316") // orange — Discord / Brain
	ColorError   = lipgloss.Color("#EF4444") // red — errors
	ColorStatus  = lipgloss.Color("#6B7280") // gray — status updates
	ColorSession = lipgloss.Color("#EAB308") // yellow — handshakes, sessions

	// Status indicator styles
	StatusOnline  = lipgloss.NewStyle().Foreground(lipgloss.Color("#22C55E")).SetString("●")
	StatusOffline = lipgloss.NewStyle().Foreground(lipgloss.Color("#6B7280")).SetString("○")

	// General styles
	TitleStyle    = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#FFFFFF"))
	SubtitleStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#888888"))
	BorderStyle   = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(lipgloss.Color("#555555"))
)
