package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// SettingsTab displays the current configuration (read-only).
type SettingsTab struct {
	config hub.Config
	width  int
	height int
}

// NewSettingsTab creates a new SettingsTab with the given config.
func NewSettingsTab(cfg hub.Config) SettingsTab {
	return SettingsTab{config: cfg}
}

// SetSize updates the tab's dimensions.
func (s *SettingsTab) SetSize(width, height int) {
	s.width = width
	s.height = height
}

// View renders the SettingsTab.
func (s SettingsTab) View() string {
	heading := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#FFFFFF"))
	label := lipgloss.NewStyle().Foreground(ColorStatus)
	value := lipgloss.NewStyle().Foreground(lipgloss.Color("#FFFFFF"))

	var b strings.Builder

	// HUB section
	b.WriteString(heading.Render("HUB"))
	b.WriteString("\n")
	b.WriteString(fmt.Sprintf("  %s  %s\n",
		label.Render(fmt.Sprintf("%-14s", "Database")),
		value.Render(s.config.Hub.DBPath),
	))
	b.WriteString(fmt.Sprintf("  %s  %s\n",
		label.Render(fmt.Sprintf("%-14s", "Live Port")),
		value.Render(fmt.Sprintf("%d", s.config.Hub.LivePort)),
	))

	b.WriteString("\n")

	// BOT-HQ LIVE section
	b.WriteString(heading.Render("BOT-HQ LIVE"))
	b.WriteString("\n")
	b.WriteString(fmt.Sprintf("  %s  %s\n",
		label.Render(fmt.Sprintf("%-14s", "Voice")),
		value.Render(s.config.Live.Voice),
	))
	b.WriteString(fmt.Sprintf("  %s  %s\n",
		label.Render(fmt.Sprintf("%-14s", "Gemini Key")),
		value.Render(maskSecret(s.config.Live.GeminiAPIKey)),
	))

	b.WriteString("\n")

	// DISCORD section
	b.WriteString(heading.Render("DISCORD"))
	b.WriteString("\n")
	b.WriteString(fmt.Sprintf("  %s  %s\n",
		label.Render(fmt.Sprintf("%-14s", "Token")),
		value.Render(maskSecret(s.config.Discord.Token)),
	))
	b.WriteString(fmt.Sprintf("  %s  %s\n",
		label.Render(fmt.Sprintf("%-14s", "Channel")),
		value.Render(s.config.Discord.ChannelID),
	))

	b.WriteString("\n")

	// BRAIN section
	b.WriteString(heading.Render("BRAIN"))
	b.WriteString("\n")
	b.WriteString(fmt.Sprintf("  %s  %s\n",
		label.Render(fmt.Sprintf("%-14s", "Auto-start")),
		value.Render(checkbox(s.config.Brain.AutoStart)),
	))

	return b.String()
}

// maskSecret replaces a secret string with dots, or shows "(not set)" if empty.
func maskSecret(s string) string {
	if s == "" {
		return "(not set)"
	}
	return strings.Repeat("●", 8)
}

// checkbox renders a boolean as a checkbox.
func checkbox(v bool) string {
	if v {
		return "[x]"
	}
	return "[ ]"
}
