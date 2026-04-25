package ui

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
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
	pane   *panestate.Manager // Activity source for Status column (Phase E commit 4)
}

// NewAgentsTab creates a new AgentsTab.
func NewAgentsTab() AgentsTab {
	return AgentsTab{}
}

// SetPane wires a panestate.Manager so the Status column reads activity
// recency rather than the raw protocol.AgentStatus field. App calls this
// after construction.
func (a *AgentsTab) SetPane(p *panestate.Manager) {
	a.pane = p
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

	// Build an activity lookup from the panestate snapshot so the Status column
	// reflects derived activity (working/online/stale/offline) rather than the
	// raw protocol.AgentStatus field. Phase E commit 4: panestate is the source
	// of truth for what the user sees.
	activityByID := map[string]panestate.AgentActivity{}
	if a.pane != nil {
		for _, s := range a.pane.Snapshot() {
			activityByID[s.ID] = s.Activity
		}
	}

	var lines []string
	aliveCount, staleCount, offlineCount := 0, 0, 0

	for _, ag := range a.agents {
		// Resolve activity. Fall back to status-based mapping when pane is
		// absent (e.g. headless tests pre-SetPane).
		activity, ok := activityByID[ag.ID]
		if !ok {
			if ag.Status == protocol.StatusOffline {
				activity = panestate.ActivityOffline
			} else {
				activity = panestate.ActivityStale
			}
		}

		dot := activityDot(activity)
		switch activity {
		case panestate.ActivityWorking, panestate.ActivityOnline:
			aliveCount++
		case panestate.ActivityStale:
			staleCount++
		case panestate.ActivityOffline:
			offlineCount++
		default:
			offlineCount++ // unknown activity buckets to offline; F-core may add cases
		}

		statusStyle := lipgloss.NewStyle().Foreground(ColorStatus)
		switch activity {
		case panestate.ActivityWorking, panestate.ActivityOnline:
			statusStyle = lipgloss.NewStyle().Foreground(ColorSystem)
		}

		safeName := stripANSI(ag.Name)
		name := lipgloss.NewStyle().Foreground(agentColor(ag.ID)).Render(
			fmt.Sprintf("%-*s", maxName, safeName),
		)
		status := statusStyle.Render(fmt.Sprintf("%-10s", activity))
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

	summary := lipgloss.NewStyle().Foreground(ColorStatus).Render(
		fmt.Sprintf("\n[%d alive, %d stale, %d offline]", aliveCount, staleCount, offlineCount),
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
