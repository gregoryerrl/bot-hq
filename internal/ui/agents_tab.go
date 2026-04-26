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
	agents    []protocol.Agent
	width     int
	height    int
	pane      *panestate.Manager // Activity source for Status column (Phase E commit 4)
	cursor    int                // selected row index for Enter→pane-modal
	paneModal *PaneModal         // active modal overlay; nil when closed
	capture   PaneCaptureFunc    // tmux capture fn injected at construction
}

// NewAgentsTab creates a new AgentsTab. capture is the tmux pane-capture
// function; production passes tmux.CapturePane, tests pass a stub.
func NewAgentsTab(capture PaneCaptureFunc) AgentsTab {
	return AgentsTab{capture: capture}
}

// SetPane wires a panestate.Manager so the Status column reads activity
// recency rather than the raw protocol.AgentStatus field. App calls this
// after construction.
func (a *AgentsTab) SetPane(p *panestate.Manager) {
	a.pane = p
}

// SetSize updates the tab's dimensions and propagates to the active modal
// (if any) so it resizes alongside the surrounding TUI.
func (a *AgentsTab) SetSize(width, height int) {
	a.width = width
	a.height = height
	if a.paneModal != nil {
		a.paneModal.SetSize(width, height)
	}
}

// Update handles messages for the AgentsTab. When a modal overlay is
// active it owns input; only PaneModalClosed unblocks the underlying tab.
func (a AgentsTab) Update(msg tea.Msg) (AgentsTab, tea.Cmd) {
	if a.paneModal != nil {
		// Forward everything to the modal until it signals close.
		newModal, cmd := a.paneModal.Update(msg)
		a.paneModal = newModal
		// Watch for close signal carried in cmd output.
		if cmd != nil {
			out := cmd()
			if _, ok := out.(PaneModalClosed); ok {
				a.paneModal = nil
				return a, nil
			}
			// Re-wrap remaining tea.Msg into a Cmd so caller still gets it.
			return a, func() tea.Msg { return out }
		}
		return a, nil
	}

	switch msg := msg.(type) {
	case AgentsUpdated:
		a.agents = msg.Agents
		// Clamp cursor when list shrinks.
		if a.cursor >= len(a.agents) {
			a.cursor = len(a.agents) - 1
		}
		if a.cursor < 0 {
			a.cursor = 0
		}
	case tea.KeyMsg:
		switch msg.String() {
		case "j", "down":
			if a.cursor < len(a.agents)-1 {
				a.cursor++
			}
		case "k", "up":
			if a.cursor > 0 {
				a.cursor--
			}
		case "enter":
			if len(a.agents) == 0 || a.cursor >= len(a.agents) || a.capture == nil {
				return a, nil
			}
			target := agentTmuxTarget(a.agents[a.cursor])
			if target == "" {
				return a, nil // agent has no tmux pane to view
			}
			modal := NewPaneModal(target, a.capture)
			modal.SetSize(a.width, a.height)
			_ = modal.Refresh()
			a.paneModal = modal
		}
	}
	return a, nil
}

// agentTmuxTarget extracts the tmux_target field from an agent's Meta
// JSON, mirroring the parse done elsewhere. Returns empty string when
// absent or unparseable.
func agentTmuxTarget(ag protocol.Agent) string {
	if ag.Meta == "" {
		return ""
	}
	var meta struct {
		TmuxTarget string `json:"tmux_target"`
	}
	if err := json.Unmarshal([]byte(ag.Meta), &meta); err != nil {
		return ""
	}
	return meta.TmuxTarget
}

// View renders the AgentsTab. When a modal is active, render it on top of
// the agents list (lipgloss.Place centers it within the tab area).
func (a AgentsTab) View() string {
	if a.paneModal != nil {
		modal := a.paneModal.View()
		if a.width <= 0 || a.height <= 0 {
			return modal
		}
		return lipgloss.Place(a.width, a.height, lipgloss.Center, lipgloss.Center, modal)
	}
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
	staleGenByID := map[string]bool{}
	if a.pane != nil {
		for _, s := range a.pane.Snapshot() {
			activityByID[s.ID] = s.Activity
			staleGenByID[s.ID] = s.StaleGen
		}
	}

	var lines []string
	aliveCount, staleCount, offlineCount := 0, 0, 0

	for i, ag := range a.agents {
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
		// Phase G v1 #20: append (stale-gen) suffix to agents from a prior
		// rebuild generation. Visible-but-flagged in agents tab (so user
		// can prune via hub_unregister); strip omits them entirely.
		if staleGenByID[ag.ID] {
			safeName = safeName + " (stale-gen)"
		}
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

		// Cursor indicator: ▸ on the selected row, two-space pad on others
		// so the column alignment stays stable regardless of selection.
		cursor := "  "
		if i == a.cursor {
			cursor = "▸ "
		}

		lines = append(lines, fmt.Sprintf("%s%s %s  %s  %s%s  %s", cursor, dot, name, status, timeStr, tmuxStr, project))
	}

	summary := lipgloss.NewStyle().Foreground(ColorStatus).Render(
		fmt.Sprintf("\n[%d alive, %d stale, %d offline]", aliveCount, staleCount, offlineCount),
	)
	lines = append(lines, summary)

	hint := lipgloss.NewStyle().Foreground(lipgloss.Color("#666666")).Render(
		"  ↑/↓: navigate  Enter: capture pane  (Esc inside modal to close)",
	)
	lines = append(lines, hint)

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
