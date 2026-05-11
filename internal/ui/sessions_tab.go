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
//
// Z-3 sessions-as-containers: sessions are scope-keyed (slug-uuid), not
// date-keyed. The per-session interactive view (input → BRAIN-duo of
// focused session; nested tabs for active sessions) is a Z-3-follow-up
// landing in a subsequent commit — Z-3 ships the substrate (manifest +
// MCP tools + bootstrap) and minimal UI awareness here; full TUI
// retargeting (per-session input box, agents-strip, focused-session
// state) is deferred to keep Z-3 within atomic-commit scope.
//
// Z-3-followup-2: default view shows only active sessions. Press 'a'
// to toggle "all" mode (include done/paused). Closed sessions stay in
// DB for retrospective query (hub_session_lookback / hub_session_summary)
// but the live Sessions tab focuses on what's currently in-flight per
// the user's expectation that "Sessions" = "current work."
type SessionsTab struct {
	sessions []protocol.Session // raw list as received from SessionsUpdated
	width    int
	height   int
	cursor   int
	selected string // selected session ID for filtering
	showAll  bool   // false = active-only (default); true = include done/paused
}

// visibleSessions returns the subset of s.sessions filtered per showAll.
func (s SessionsTab) visibleSessions() []protocol.Session {
	if s.showAll {
		return s.sessions
	}
	out := make([]protocol.Session, 0, len(s.sessions))
	for _, sess := range s.sessions {
		if sess.Status == protocol.SessionActive {
			out = append(out, sess)
		}
	}
	return out
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
		// Clamp cursor to visible-list bounds (active-only default).
		visible := s.visibleSessions()
		if s.cursor >= len(visible) {
			s.cursor = len(visible) - 1
		}
		if s.cursor < 0 {
			s.cursor = 0
		}
	case tea.KeyMsg:
		visible := s.visibleSessions()
		switch msg.String() {
		case "j", "down":
			if s.cursor < len(visible)-1 {
				s.cursor++
			}
		case "k", "up":
			if s.cursor > 0 {
				s.cursor--
			}
		case "a":
			// Toggle active-only vs all (Z-3-followup-2).
			s.showAll = !s.showAll
			s.cursor = 0
		case "enter":
			if len(visible) > 0 && s.cursor < len(visible) {
				sess := visible[s.cursor]
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

// View renders the SessionsTab. Default view: active sessions only.
// Press 'a' to toggle "all" mode (Z-3-followup-2).
func (s SessionsTab) View() string {
	visible := s.visibleSessions()
	if len(visible) == 0 {
		msg := "No active sessions. Talk to emma or use `hub_session_open` to start one."
		if s.showAll {
			msg = "No sessions in the database."
		}
		return lipgloss.NewStyle().
			Foreground(ColorStatus).
			Render(msg)
	}

	selectedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#BB77FF")).Bold(true)
	hint := lipgloss.NewStyle().Foreground(lipgloss.Color("#666666"))

	var lines []string
	activeCount := 0
	pausedCount := 0
	doneCount := 0

	for i, sess := range visible {
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
	mode := "active-only"
	if s.showAll {
		mode = "all sessions"
	}
	if s.selected != "" {
		lines = append(lines, hint.Render(fmt.Sprintf("  ↑/↓: navigate  Enter: deselect  Esc: clear filter  a: toggle filter  Tab: switch tab  (showing: %s)", mode)))
	} else {
		lines = append(lines, hint.Render(fmt.Sprintf("  ↑/↓: navigate  Enter: filter hub by session  a: toggle %s  Tab: switch tab", mode)))
	}

	return strings.Join(lines, "\n")
}
