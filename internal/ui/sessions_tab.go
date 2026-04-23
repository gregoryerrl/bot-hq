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

// SessionsTab displays a list of active and recent sessions.
type SessionsTab struct {
	sessions []protocol.Session
	width    int
	height   int
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

	var lines []string
	activeCount := 0
	pausedCount := 0
	doneCount := 0

	for _, sess := range s.sessions {
		// Short ID (first 4 chars)
		shortID := sess.ID
		if len(shortID) > 4 {
			shortID = shortID[:4]
		}
		idStr := lipgloss.NewStyle().Foreground(ColorStatus).Render("#" + shortID)

		// Mode
		modeStr := lipgloss.NewStyle().Foreground(ColorLive).Render(
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

		lines = append(lines, fmt.Sprintf("%s  %s  %s  %s  %s",
			idStr, modeStr, agentsStr, purposeStr, statusStr))
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

	return strings.Join(lines, "\n")
}
