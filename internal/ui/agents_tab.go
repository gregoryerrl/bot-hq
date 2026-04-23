package ui

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// stripANSI removes ANSI escape sequences from a string to prevent terminal injection.
var ansiRegex = regexp.MustCompile(`\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?(\x07|\x1b\\)|\x1b[^[\]()]`)

func stripANSI(s string) string {
	return ansiRegex.ReplaceAllString(s, "")
}

// AgentsUpdated is a Bubbletea message sent when the agent list changes.
type AgentsUpdated struct {
	Agents []protocol.Agent
}

// AgentsTab displays a list of agents with status indicators.
type AgentsTab struct {
	agents []protocol.Agent
	width  int
	height int
}

// NewAgentsTab creates a new AgentsTab.
func NewAgentsTab() AgentsTab {
	return AgentsTab{}
}

// SetSize updates the tab's dimensions.
func (a *AgentsTab) SetSize(width, height int) {
	a.width = width
	a.height = height
}

// Update handles messages for the AgentsTab.
func (a AgentsTab) Update(msg tea.Msg) (AgentsTab, tea.Cmd) {
	switch msg := msg.(type) {
	case AgentsUpdated:
		a.agents = msg.Agents
	}
	return a, nil
}

// View renders the AgentsTab.
func (a AgentsTab) View() string {
	if len(a.agents) == 0 {
		return lipgloss.NewStyle().
			Foreground(ColorStatus).
			Render("No agents registered yet.")
	}

	// Find max name length for padding (use sanitized names)
	maxName := 0
	for _, ag := range a.agents {
		name := stripANSI(ag.Name)
		if len(name) > maxName {
			maxName = len(name)
		}
	}
	if maxName < 8 {
		maxName = 8
	}

	var lines []string
	onlineCount := 0

	for _, ag := range a.agents {
		// Status dot
		var dot string
		switch ag.Status {
		case protocol.StatusOnline, protocol.StatusWorking:
			dot = StatusOnline.String()
			onlineCount++
		default:
			dot = StatusOffline.String()
		}

		// Status text with color
		var statusStyle lipgloss.Style
		switch ag.Status {
		case protocol.StatusOnline, protocol.StatusWorking:
			statusStyle = lipgloss.NewStyle().Foreground(ColorSystem)
		case protocol.StatusIdle:
			statusStyle = lipgloss.NewStyle().Foreground(ColorStatus)
		default:
			statusStyle = lipgloss.NewStyle().Foreground(ColorStatus)
		}

		safeName := stripANSI(ag.Name)
		name := lipgloss.NewStyle().Foreground(agentColor(ag.ID)).Render(
			fmt.Sprintf("%-*s", maxName, safeName),
		)
		status := statusStyle.Render(fmt.Sprintf("%-10s", ag.Status))
		safeProject := stripANSI(ag.Project)
		project := lipgloss.NewStyle().Foreground(ColorSession).Render(
			fmt.Sprintf("%-18s", safeProject),
		)
		elapsed := formatElapsed(ag.LastSeen)
		timeStr := lipgloss.NewStyle().Foreground(ColorStatus).Render(elapsed)

		// Extract tmux target from agent meta
		tmuxStr := ""
		if ag.Meta != "" {
			var meta struct {
				TmuxTarget string `json:"tmux_target"`
			}
			if json.Unmarshal([]byte(ag.Meta), &meta) == nil && meta.TmuxTarget != "" {
				tmuxStr = lipgloss.NewStyle().Foreground(ColorStatus).Render(
					fmt.Sprintf("  tmux:%s", meta.TmuxTarget),
				)
			}
		}

		lines = append(lines, fmt.Sprintf("%s %s  %s  %s  %s%s", dot, name, status, project, timeStr, tmuxStr))
	}

	offlineCount := len(a.agents) - onlineCount
	summary := lipgloss.NewStyle().Foreground(ColorStatus).Render(
		fmt.Sprintf("\n[%d online, %d offline]", onlineCount, offlineCount),
	)
	lines = append(lines, summary)

	return strings.Join(lines, "\n")
}

// formatElapsed returns a human-readable elapsed time string.
func formatElapsed(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	d := time.Since(t)
	switch {
	case d < time.Minute:
		return "now"
	case d < time.Hour:
		return fmt.Sprintf("%dm ago", int(d.Minutes()))
	case d < 24*time.Hour:
		return fmt.Sprintf("%dh ago", int(d.Hours()))
	default:
		return fmt.Sprintf("%dd ago", int(d.Hours()/24))
	}
}
