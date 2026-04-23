package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/textinput"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// MessageReceived is a Bubbletea message for when a new hub message arrives.
type MessageReceived struct {
	Message protocol.Message
}

// CommandSubmitted is emitted when the user presses Enter in the command input.
type CommandSubmitted struct {
	Text string
}

// HubTab displays a scrollable, color-coded message feed with a command input.
type HubTab struct {
	messages []protocol.Message
	viewport viewport.Model
	input    textinput.Model
	width    int
	height   int
	focused  bool // true when the command input is focused
}

// NewHubTab creates a new HubTab with default dimensions.
func NewHubTab() HubTab {
	ti := textinput.New()
	ti.Placeholder = "Type a command (@agent message, spawn project, etc.)..."
	ti.CharLimit = 500

	vp := viewport.New(80, 20)
	vp.MouseWheelEnabled = true

	return HubTab{
		viewport: vp,
		input:    ti,
	}
}

// Init satisfies the tea.Model-like interface for composability.
func (h HubTab) Init() tea.Cmd {
	return nil
}

// Update handles messages for the HubTab.
func (h HubTab) Update(msg tea.Msg) (HubTab, tea.Cmd) {
	var cmds []tea.Cmd

	switch msg := msg.(type) {
	case MessageReceived:
		h.messages = append(h.messages, msg.Message)
		h.viewport.SetContent(h.renderMessages())
		// Auto-scroll to bottom when a new message arrives
		h.viewport.GotoBottom()

	case tea.KeyMsg:
		if h.focused {
			switch msg.String() {
			case "enter":
				val := h.input.Value()
				if val != "" {
					h.input.Reset()
					cmds = append(cmds, func() tea.Msg {
						return CommandSubmitted{Text: val}
					})
				}
			case "esc":
				h.focused = false
				h.input.Blur()
			default:
				var cmd tea.Cmd
				h.input, cmd = h.input.Update(msg)
				cmds = append(cmds, cmd)
			}
		} else {
			switch msg.String() {
			case "/", "i":
				h.focused = true
				cmds = append(cmds, h.input.Focus())
			default:
				var cmd tea.Cmd
				h.viewport, cmd = h.viewport.Update(msg)
				cmds = append(cmds, cmd)
			}
		}

	case tea.WindowSizeMsg:
		h.width = msg.Width
		h.height = msg.Height
		h.resize()

	case tea.MouseMsg:
		if !h.focused {
			var cmd tea.Cmd
			h.viewport, cmd = h.viewport.Update(msg)
			cmds = append(cmds, cmd)
		}
	}

	return h, tea.Batch(cmds...)
}

// SetSize updates the HubTab's dimensions and resizes internal components.
func (h *HubTab) SetSize(width, height int) {
	h.width = width
	h.height = height
	h.resize()
}

// resize recalculates viewport and input dimensions.
func (h *HubTab) resize() {
	// Reserve 3 lines: 1 for separator, 1 for input, 1 for padding
	inputHeight := 3
	vpHeight := h.height - inputHeight
	if vpHeight < 1 {
		vpHeight = 1
	}

	h.viewport.Width = h.width
	h.viewport.Height = vpHeight
	h.input.Width = h.width - 4 // Account for prompt and padding

	h.viewport.SetContent(h.renderMessages())
}

// View renders the HubTab.
func (h HubTab) View() string {
	separator := lipgloss.NewStyle().
		Width(h.width).
		Foreground(lipgloss.Color("#555555")).
		Render(strings.Repeat("─", h.width))

	return lipgloss.JoinVertical(lipgloss.Left,
		h.viewport.View(),
		separator,
		h.input.View(),
	)
}

// renderMessages builds the full string content for the viewport from all messages.
func (h HubTab) renderMessages() string {
	if len(h.messages) == 0 {
		empty := lipgloss.NewStyle().
			Foreground(ColorStatus).
			Render("No messages yet. Waiting for agents to connect...")
		return empty
	}

	var lines []string
	for _, msg := range h.messages {
		lines = append(lines, h.formatMessage(msg))
	}
	return strings.Join(lines, "\n")
}

// formatMessage renders a single message as "[HH:MM:SS] from → to: content"
// with color based on agent type and message type.
func (h HubTab) formatMessage(msg protocol.Message) string {
	timestamp := msg.Created.Format("15:04:05")

	var arrow string
	if msg.ToAgent != "" {
		arrow = fmt.Sprintf("%s → %s", msg.FromAgent, msg.ToAgent)
	} else {
		arrow = fmt.Sprintf("%s → *", msg.FromAgent)
	}

	color := h.messageColor(msg)
	style := lipgloss.NewStyle().Foreground(color)
	tsStyle := lipgloss.NewStyle().Foreground(ColorStatus)

	return fmt.Sprintf("%s %s %s",
		tsStyle.Render("["+timestamp+"]"),
		style.Render(arrow+":"),
		style.Render(msg.Content),
	)
}

// messageColor determines the display color for a message based on its type
// and the sender's identity.
func (h HubTab) messageColor(msg protocol.Message) lipgloss.Color {
	// Error messages are always red
	if msg.Type == protocol.MsgError {
		return ColorError
	}

	// Handshake and session-related
	if msg.Type == protocol.MsgHandshake {
		return ColorSession
	}

	// Status updates
	if msg.Type == protocol.MsgUpdate {
		return ColorStatus
	}

	// Color by agent name/type hints
	from := strings.ToLower(msg.FromAgent)
	switch {
	case from == "system" || from == "hub":
		return ColorSystem
	case strings.Contains(from, "live") || strings.Contains(from, "voice"):
		return ColorLive
	case strings.Contains(from, "coder") || strings.Contains(from, "claude"):
		return ColorCoder
	case strings.Contains(from, "discord") || strings.Contains(from, "brain"):
		return ColorDiscord
	}

	// Default based on message type
	switch msg.Type {
	case protocol.MsgCommand:
		return ColorLive
	case protocol.MsgResult, protocol.MsgResponse:
		return ColorCoder
	case protocol.MsgQuestion:
		return ColorSession
	default:
		return ColorStatus
	}
}
