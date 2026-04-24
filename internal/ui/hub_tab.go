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
	messages      []protocol.Message
	viewport      viewport.Model
	input         textinput.Model
	width         int
	height        int
	focused       bool // true when the command input is focused
	sessionFilter string
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
			key := msg.String()
			switch key {
			case "/", "i":
				h.focused = true
				cmds = append(cmds, h.input.Focus())
			default:
				// Auto-focus input on any printable character
				if len(key) == 1 && key >= " " && key <= "~" {
					h.focused = true
					cmds = append(cmds, h.input.Focus())
					// Forward the typed character to the input
					var cmd tea.Cmd
					h.input, cmd = h.input.Update(msg)
					cmds = append(cmds, cmd)
				} else {
					var cmd tea.Cmd
					h.viewport, cmd = h.viewport.Update(msg)
					cmds = append(cmds, cmd)
				}
			}
		}

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

// SetSessionFilter filters the hub to only show messages from a specific session.
// Pass an empty string to clear the filter.
func (h *HubTab) SetSessionFilter(sessionID string) {
	h.sessionFilter = sessionID
	h.viewport.SetContent(h.renderMessages())
	h.viewport.GotoBottom()
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
		if h.sessionFilter != "" && msg.SessionID != h.sessionFilter {
			continue
		}
		lines = append(lines, h.formatMessage(msg))
	}
	return strings.Join(lines, "\n")
}

// formatMessage renders a single message as "[HH:MM:SS] from → to: content"
// with each agent name colored independently.
func (h HubTab) formatMessage(msg protocol.Message) string {
	timestamp := msg.Created.Format("15:04:05")

	if msg.Type == protocol.MsgFlag {
		flagStyle := lipgloss.NewStyle().Foreground(ColorError).Bold(true)
		fromStyle := lipgloss.NewStyle().Foreground(agentColor(msg.FromAgent))
		tsStyle := lipgloss.NewStyle().Foreground(ColorStatus)
		return fmt.Sprintf("%s %s %s %s",
			tsStyle.Render("["+timestamp+"]"),
			flagStyle.Render("⚑ FLAG"),
			fromStyle.Render(msg.FromAgent+":"),
			flagStyle.Render(msg.Content),
		)
	}

	tsStyle := lipgloss.NewStyle().Foreground(ColorStatus)
	arrowStyle := lipgloss.NewStyle().Foreground(ColorStatus)

	fromColor := agentColor(msg.FromAgent)
	fromStyle := lipgloss.NewStyle().Foreground(fromColor)
	toName := "*"
	if msg.ToAgent != "" {
		toName = msg.ToAgent
	}
	toStyle := lipgloss.NewStyle().Foreground(agentColor(toName))

	// Message content matches the author's color
	msgColor := fromColor
	if msg.Type == protocol.MsgError {
		msgColor = ColorError
	}

	// Build the prefix to calculate its visible width for wrapping
	prefix := fmt.Sprintf("[%s] %s → %s: ", timestamp, msg.FromAgent, toName)
	prefixLen := len(prefix)

	// Wrap content to fit viewport width
	content := msg.Content
	if h.width > 0 && prefixLen+len(content) > h.width {
		content = wrapText(content, h.width-prefixLen)
	}

	msgStyle := lipgloss.NewStyle().Foreground(msgColor)

	return fmt.Sprintf("%s %s %s %s%s %s",
		tsStyle.Render("["+timestamp+"]"),
		fromStyle.Render(msg.FromAgent),
		arrowStyle.Render("→"),
		toStyle.Render(toName),
		arrowStyle.Render(":"),
		msgStyle.Render(content),
	)
}

// parseCommand extracts an @agent target from user input.
// Returns (target, content). If no @mention, target is empty.
func parseCommand(input string) (string, string) {
	input = strings.TrimSpace(input)
	if !strings.HasPrefix(input, "@") {
		return "", input
	}
	parts := strings.SplitN(input, " ", 2)
	target := strings.TrimPrefix(parts[0], "@")
	content := ""
	if len(parts) > 1 {
		content = parts[1]
	}
	return target, content
}

// agentColorPalette is a set of distinct colors for dynamically assigned agent colors.
var agentColorPalette = []lipgloss.Color{
	lipgloss.Color("#A855F7"), // purple
	lipgloss.Color("#EC4899"), // pink
	lipgloss.Color("#14B8A6"), // teal
	lipgloss.Color("#F59E0B"), // amber
	lipgloss.Color("#8B5CF6"), // violet
	lipgloss.Color("#10B981"), // emerald
	lipgloss.Color("#F472B6"), // rose
	lipgloss.Color("#06B6D4"), // cyan
}

// agentColor returns a consistent color for a given agent name.
// Known agents get fixed colors; others get a hash-based color from the palette.
func agentColor(name string) lipgloss.Color {
	lower := strings.ToLower(name)
	switch {
	case lower == "system" || lower == "hub":
		return ColorSystem
	case lower == "brian":
		return ColorBrian
	case lower == "live":
		return ColorClive
	case lower == "rain":
		return ColorRain
	case lower == "user":
		return lipgloss.Color("#FFFFFF") // white
	case lower == "discord":
		return ColorDiscord
	case lower == "*":
		return ColorStatus
	}

	// Hash-based color for coder agents etc.
	var hash uint32
	for _, c := range name {
		hash = hash*31 + uint32(c)
	}
	return agentColorPalette[hash%uint32(len(agentColorPalette))]
}

// wrapText wraps text to fit within maxWidth, breaking at spaces.
func wrapText(text string, maxWidth int) string {
	if maxWidth <= 0 {
		return text
	}
	var lines []string
	words := strings.Fields(text)
	var current string
	for _, word := range words {
		if current == "" {
			current = word
		} else if len(current)+1+len(word) > maxWidth {
			lines = append(lines, current)
			current = word
		} else {
			current += " " + word
		}
	}
	if current != "" {
		lines = append(lines, current)
	}
	if len(lines) <= 1 {
		return text
	}
	// Indent continuation lines to align with the first line's content
	indent := strings.Repeat(" ", 2)
	result := lines[0]
	for _, line := range lines[1:] {
		result += "\n" + indent + line
	}
	return result
}

// messageColor determines the display color for a message based on the sender.
func (h HubTab) messageColor(msg protocol.Message) lipgloss.Color {
	if msg.Type == protocol.MsgError {
		return ColorError
	}
	return agentColor(msg.FromAgent)
}
