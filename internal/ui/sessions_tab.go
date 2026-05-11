package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// SessionsUpdated is a Bubbletea message sent when the session list changes.
type SessionsUpdated struct {
	Sessions []protocol.Session
}

// SessionSelected is a Bubbletea message sent when a session is selected/deselected.
// Z-8f: post-restructure this is a hint that the Sessions tab drilled
// into (or out of) a container view. Empty SessionID = drilled out.
type SessionSelected struct {
	SessionID string
}

// SessionsTab displays a list of active and recent sessions, with a
// drilled-in container view when the user presses Enter on a row.
//
// Z-8f: pre-Z-8f the Sessions tab was a passive list — pressing Enter
// flipped the Hub tab into a session-filter view. Post-Z-8f the
// Sessions tab owns its own container view: agent strip + filtered
// stream + session-scoped input. Esc returns to the list. The Hub tab
// is strictly the main-hub view (Z-8e) and no longer reuses its
// viewport for session content.
type SessionsTab struct {
	sessions []protocol.Session // raw list as received from SessionsUpdated
	width    int
	height   int
	cursor   int
	showAll  bool // false = active-only (default); true = include done/paused

	// Z-8f: drilled-in container view state. When drilled is true the
	// tab renders the container (viewport + input) instead of the
	// list. Esc pops back to the list.
	drilled           bool
	drilledSessionID  string
	containerVP       viewport.Model
	containerInput    textarea.Model
	containerFocused  bool
	containerMessages []protocol.Message // full message stream; filtered at render
	containerFollow   bool
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
	ta := textarea.New()
	ta.Placeholder = "Talk to this session's duo (brian + rain)..."
	ta.CharLimit = 0
	ta.ShowLineNumbers = false
	ta.SetHeight(inputRows)
	ta.KeyMap.InsertNewline = key.NewBinding(
		key.WithKeys("enter", "ctrl+m", "shift+enter", "ctrl+j", "alt+enter"),
		key.WithHelp("shift+enter", "newline"),
	)

	vp := viewport.New(80, 20)
	vp.MouseWheelEnabled = true

	return SessionsTab{
		containerVP:     vp,
		containerInput:  ta,
		containerFollow: true,
	}
}

// SetSize updates the tab's dimensions.
func (s *SessionsTab) SetSize(width, height int) {
	s.width = width
	s.height = height
	s.resizeContainer()
}

// resizeContainer adjusts the container viewport + input for current
// width/height. Only relevant when drilled; safe to call always.
func (s *SessionsTab) resizeContainer() {
	// Reserve: 1 header + 1 separator + inputRows + 1 hint.
	reserved := 3 + inputRows
	vpHeight := s.height - reserved
	if vpHeight < 1 {
		vpHeight = 1
	}
	s.containerVP.Width = s.width
	s.containerVP.Height = vpHeight
	s.containerInput.SetWidth(s.width - 4)
	s.containerInput.SetHeight(inputRows)
	if s.drilled {
		s.containerVP.SetContent(s.renderContainerMessages())
		if s.containerFollow {
			s.containerVP.GotoBottom()
		}
	}
}

// Update handles messages for the SessionsTab.
func (s SessionsTab) Update(msg tea.Msg) (SessionsTab, tea.Cmd) {
	switch msg := msg.(type) {
	case SessionsUpdated:
		s.sessions = msg.Sessions
		visible := s.visibleSessions()
		if s.cursor >= len(visible) {
			s.cursor = len(visible) - 1
		}
		if s.cursor < 0 {
			s.cursor = 0
		}
		// If drilled-in session is no longer in the list, pop back.
		if s.drilled {
			found := false
			for _, sess := range s.sessions {
				if sess.ID == s.drilledSessionID {
					found = true
					break
				}
			}
			if !found {
				s.exitContainer()
			}
		}

	case MessageReceived:
		// Z-8f: container view tracks the full message stream so it
		// can filter on render. Append-only — no filtering at this
		// stage so future drill-ins see complete history.
		s.containerMessages = append(s.containerMessages, msg.Message)
		if s.drilled && msg.Message.SessionID == s.drilledSessionID {
			s.containerVP.SetContent(s.renderContainerMessages())
			if s.containerFollow {
				s.containerVP.GotoBottom()
			}
		}

	case tea.KeyMsg:
		if s.drilled {
			return s.updateContainer(msg)
		}
		return s.updateList(msg)
	}
	return s, nil
}

func (s SessionsTab) updateList(msg tea.KeyMsg) (SessionsTab, tea.Cmd) {
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
		s.showAll = !s.showAll
		s.cursor = 0
	case "enter":
		if len(visible) > 0 && s.cursor < len(visible) {
			sess := visible[s.cursor]
			s.enterContainer(sess.ID)
			return s, func() tea.Msg {
				return SessionSelected{SessionID: sess.ID}
			}
		}
	}
	return s, nil
}

func (s SessionsTab) updateContainer(msg tea.KeyMsg) (SessionsTab, tea.Cmd) {
	var cmds []tea.Cmd
	if s.containerFocused {
		switch {
		case msg.String() == "enter" && !msg.Paste:
			val := s.containerInput.Value()
			if val != "" {
				s.containerInput.Reset()
				sid := s.drilledSessionID
				cmds = append(cmds, func() tea.Msg {
					return CommandSubmitted{Text: val, SessionID: sid}
				})
			}
		case msg.String() == "esc":
			s.containerFocused = false
			s.containerInput.Blur()
		default:
			var cmd tea.Cmd
			s.containerInput, cmd = s.containerInput.Update(msg)
			cmds = append(cmds, cmd)
		}
		return s, tea.Batch(cmds...)
	}
	switch msg.String() {
	case "esc":
		// Pop out of container back to list.
		s.exitContainer()
		return s, func() tea.Msg { return SessionSelected{SessionID: ""} }
	case "end":
		s.containerVP.GotoBottom()
		s.containerFollow = true
	default:
		// Z-9c: detect printable via the runes payload, not via
		// len(msg.String()) — tmux send-keys batches multiple runes
		// into a single KeyMsg which the pre-fix check silently
		// dropped (see hub_tab.go isPrintableRuneMsg comment).
		if isPrintableRuneMsg(msg) || msg.Paste {
			s.containerFocused = true
			cmds = append(cmds, s.containerInput.Focus())
			var cmd tea.Cmd
			s.containerInput, cmd = s.containerInput.Update(msg)
			cmds = append(cmds, cmd)
		} else {
			var cmd tea.Cmd
			s.containerVP, cmd = s.containerVP.Update(msg)
			cmds = append(cmds, cmd)
			s.containerFollow = s.containerVP.AtBottom()
		}
	}
	return s, tea.Batch(cmds...)
}

// ContainerFocused reports whether the drilled-in container's input
// is focused. Used by app.go to decide whether to route keystrokes
// (including ctrl+c handling) to the container.
func (s SessionsTab) ContainerFocused() bool {
	return s.drilled && s.containerFocused
}

// Drilled reports whether the tab is in drilled-in (container) mode.
func (s SessionsTab) Drilled() bool {
	return s.drilled
}

// SeedContainerHistory replaces any already-known messages for the
// drilled session with the supplied DB-sourced list and re-renders
// the viewport. Z-9c: app.go calls this on SessionSelected so the
// container view shows full session history even when the boot-time
// message-seed window didn't reach back to older sessions.
//
// Other-session rows in containerMessages are preserved (a tab may
// have learned about them via MessageReceived fan-out).
func (s *SessionsTab) SeedContainerHistory(msgs []protocol.Message) {
	if !s.drilled || s.drilledSessionID == "" {
		s.containerMessages = append(s.containerMessages, msgs...)
		return
	}
	others := make([]protocol.Message, 0, len(s.containerMessages))
	for _, m := range s.containerMessages {
		if m.SessionID != s.drilledSessionID {
			others = append(others, m)
		}
	}
	s.containerMessages = append(others, msgs...)
	s.containerVP.SetContent(s.renderContainerMessages())
	if s.containerFollow {
		s.containerVP.GotoBottom()
	}
}

func (s *SessionsTab) enterContainer(sessionID string) {
	s.drilled = true
	s.drilledSessionID = sessionID
	s.containerFollow = true
	s.containerFocused = false
	s.containerInput.Reset()
	s.resizeContainer()
	s.containerVP.GotoBottom()
}

func (s *SessionsTab) exitContainer() {
	s.drilled = false
	s.drilledSessionID = ""
	s.containerFocused = false
	s.containerInput.Blur()
}

// renderContainerMessages renders the session-filtered stream for the
// drilled-in container view.
func (s SessionsTab) renderContainerMessages() string {
	if s.drilledSessionID == "" {
		return ""
	}
	tsStyle := lipgloss.NewStyle().Foreground(ColorStatus)
	var lines []string
	for _, msg := range s.containerMessages {
		if msg.SessionID != s.drilledSessionID {
			continue
		}
		timestamp := msg.Created.Format("15:04:05")
		fromStyle := lipgloss.NewStyle().Foreground(agentColor(msg.FromAgent))
		msgStyle := lipgloss.NewStyle().Foreground(agentColor(msg.FromAgent))
		if msg.Type == protocol.MsgError {
			msgStyle = lipgloss.NewStyle().Foreground(ColorError)
		}
		content := msg.Content
		if s.width > 0 {
			prefixLen := len(timestamp) + len(msg.FromAgent) + 5
			if prefixLen+len(content) > s.width {
				content = wrapText(content, s.width-prefixLen)
			}
		}
		lines = append(lines, fmt.Sprintf("%s %s: %s",
			tsStyle.Render("["+timestamp+"]"),
			fromStyle.Render(msg.FromAgent),
			msgStyle.Render(content),
		))
	}
	if len(lines) == 0 {
		return lipgloss.NewStyle().Foreground(ColorStatus).Render("No messages in this session yet.")
	}
	return strings.Join(lines, "\n")
}

// View renders the SessionsTab. Drilled-in mode renders the container
// view; otherwise the list. Default list view: active sessions only.
// Press 'a' to toggle "all" mode (Z-3-followup-2).
func (s SessionsTab) View() string {
	if s.drilled {
		return s.viewContainer()
	}
	return s.viewList()
}

func (s SessionsTab) viewContainer() string {
	headerStyle := lipgloss.NewStyle().Foreground(ColorSession).Bold(true).Width(s.width)
	hintStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#666666")).Width(s.width)
	sepStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#555555")).Width(s.width)

	header := headerStyle.Render(fmt.Sprintf("▸ Session: %s    [Esc: back to list]", s.drilledSessionID))
	sep := sepStyle.Render(strings.Repeat("─", s.width))
	hint := hintStyle.Render("  type to send to this session's duo  ·  Esc twice to leave")

	return lipgloss.JoinVertical(lipgloss.Left,
		header,
		sep,
		s.containerVP.View(),
		hint,
		s.containerInput.View(),
	)
}

func (s SessionsTab) viewList() string {
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
		cursor := "  "
		if i == s.cursor {
			cursor = "▸ "
		}

		shortID := sess.ID
		if len(shortID) > 4 {
			shortID = shortID[:4]
		}
		idStr := lipgloss.NewStyle().Foreground(ColorStatus).Render("#" + shortID)

		modeStr := lipgloss.NewStyle().Foreground(ColorClive).Render(
			fmt.Sprintf("%-12s", sess.Mode),
		)

		agentsStr := lipgloss.NewStyle().Foreground(lipgloss.Color("#FFFFFF")).Render(
			fmt.Sprintf("%-20s", strings.Join(sess.Agents, " ↔ ")),
		)

		purpose := sess.Purpose
		if len(purpose) > 20 {
			purpose = purpose[:17] + "..."
		}
		purposeStr := lipgloss.NewStyle().Foreground(ColorSession).Render(
			fmt.Sprintf("\"%-20s\"", purpose),
		)

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

		if i == s.cursor {
			row = selectedStyle.Render(cursor) + row
		} else {
			row = cursor + row
		}

		lines = append(lines, row)
	}

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

	mode := "active-only"
	if s.showAll {
		mode = "all sessions"
	}
	lines = append(lines, hint.Render(fmt.Sprintf("  ↑/↓: navigate  Enter: drill into container  a: toggle %s  Tab: switch tab", mode)))

	return strings.Join(lines, "\n")
}
