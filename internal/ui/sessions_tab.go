package ui

import (
	"fmt"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// SessionsUpdated is a Bubbletea message sent when the session list changes.
type SessionsUpdated struct {
	Sessions []protocol.Session
}

// SessionSelected is a Bubbletea message sent when a session is selected/deselected for filtering.
type SessionSelected struct {
	SessionID string
}

// SessionsTab displays a list of active and recent sessions.
type SessionsTab struct {
	sessions []protocol.Session
	width    int
	height   int
	cursor   int
	selected string // selected session ID for filtering
}

// NewSessionsTab creates a new SessionsTab.
func NewSessionsTab() SessionsTab {
	return SessionsTab{}
}

// SetSize updates the tab's dimensions.
func (s *SessionsTab) SetSize(width, height int) {
	s.width = width
	s.height = height
}

// Update handles messages for the SessionsTab.
func (s SessionsTab) Update(msg tea.Msg) (SessionsTab, tea.Cmd) {
	switch msg := msg.(type) {
	case SessionsUpdated:
		s.sessions = msg.Sessions
		// Clamp cursor to valid range when sessions list shrinks
		if s.cursor >= len(s.sessions) {
			s.cursor = len(s.sessions) - 1
		}
		if s.cursor < 0 {
			s.cursor = 0
		}
	case tea.KeyMsg:
		switch msg.String() {
		case "j", "down":
			if s.cursor < len(s.sessions)-1 {
				s.cursor++
			}
		case "k", "up":
			if s.cursor > 0 {
				s.cursor--
			}
		case "enter":
			if len(s.sessions) > 0 && s.cursor < len(s.sessions) {
				sess := s.sessions[s.cursor]
				if s.selected == sess.ID {
					s.selected = ""
				} else {
					s.selected = sess.ID
				}
				return s, func() tea.Msg {
					return SessionSelected{SessionID: s.selected}
				}
			}
		case "esc":
			if s.selected != "" {
				s.selected = ""
				return s, func() tea.Msg {
					return SessionSelected{SessionID: ""}
				}
			}
		}
	}
	return s, nil
}

// View renders the SessionsTab.
func (s SessionsTab) View() string {
	if len(s.sessions) == 0 {
		return lipgloss.NewStyle().
			Foreground(ColorStatus).
			Render("No sessions yet.")
	}

	selectedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#BB77FF")).Bold(true)
	hint := lipgloss.NewStyle().Foreground(lipgloss.Color("#666666"))

	var lines []string
	activeCount := 0
	pausedCount := 0
	doneCount := 0

	for i, sess := range s.sessions {
		// Cursor indicator
		cursor := "  "
		if i == s.cursor {
			cursor = "▸ "
		}

		// Short ID (first 4 chars)
		shortID := sess.ID
		if len(shortID) > 4 {
			shortID = shortID[:4]
		}
		idStr := lipgloss.NewStyle().Foreground(ColorStatus).Render("#" + shortID)

		// Mode
		modeStr := lipgloss.NewStyle().Foreground(ColorClive).Render(
			fmt.Sprintf("%-12s", sess.Mode),
		)

		// Agents joined with arrow
		agentsStr := lipgloss.NewStyle().Foreground(lipgloss.Color("#FFFFFF")).Render(
			fmt.Sprintf("%-20s", strings.Join(sess.Agents, " ↔ ")),
		)

		// Purpose in quotes
		purpose := sess.Purpose
		if len(purpose) > 20 {
			purpose = purpose[:17] + "..."
		}
		purposeStr := lipgloss.NewStyle().Foreground(ColorSession).Render(
			fmt.Sprintf("\"%-20s\"", purpose),
		)

		// Status with color
		var statusColor lipgloss.Color
		switch sess.Status {
		case protocol.SessionActive:
			statusColor = ColorSystem
			activeCount++
		case protocol.SessionPaused:
			statusColor = ColorSession
			pausedCount++
		case protocol.SessionDone:
			statusColor = ColorStatus
			doneCount++
		}
		statusStr := lipgloss.NewStyle().Foreground(statusColor).Render(string(sess.Status))

		row := fmt.Sprintf("%s  %s  %s  %s  %s",
			idStr, modeStr, agentsStr, purposeStr, statusStr)

		// Highlight selected or cursor row
		if s.selected == sess.ID {
			row = selectedStyle.Render(fmt.Sprintf("%s[*] %s  %s  %s  %s  %s",
				cursor, "#"+shortID, fmt.Sprintf("%-12s", sess.Mode),
				fmt.Sprintf("%-20s", strings.Join(sess.Agents, " ↔ ")),
				fmt.Sprintf("\"%-20s\"", purpose), string(sess.Status)))
		} else if i == s.cursor {
			row = selectedStyle.Render(cursor) + row
		} else {
			row = cursor + row
		}

		lines = append(lines, row)
	}

	// Summary
	var parts []string
	if activeCount > 0 {
		parts = append(parts, fmt.Sprintf("%d active", activeCount))
	}
	if pausedCount > 0 {
		parts = append(parts, fmt.Sprintf("%d paused", pausedCount))
	}
	if doneCount > 0 {
		parts = append(parts, fmt.Sprintf("%d done", doneCount))
	}
	summary := lipgloss.NewStyle().Foreground(ColorStatus).Render(
		fmt.Sprintf("\n[%s]", strings.Join(parts, ", ")),
	)
	lines = append(lines, summary)

	// Help line
	if s.selected != "" {
		lines = append(lines, hint.Render("  ↑/↓: navigate  Enter: deselect  Esc: clear filter  Tab: switch tab"))
	} else {
		lines = append(lines, hint.Render("  ↑/↓: navigate  Enter: filter hub by session  Tab: switch tab"))
	}

	return strings.Join(lines, "\n")
}
