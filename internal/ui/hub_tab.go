package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// inputRows is the visible row count for the command textarea. It expands
// up to textarea.MaxHeight internally for very long pastes/messages, but
// the rendered footprint stays at this fixed size.
const inputRows = 3

// MessageReceived is a Bubbletea message for when a new hub message arrives.
type MessageReceived struct {
	Message protocol.Message
}

// CommandSubmitted is emitted when the user presses Enter in the command input.
type CommandSubmitted struct {
	Text string
}

// HubTab displays a scrollable, color-coded message feed with a command input.
//
// Input is a textarea (not textinput) so users can compose multi-line
// messages and paste long content without truncation. Keybindings are
// flipped from textarea's defaults: enter submits, shift+enter / ctrl+j /
// alt+enter insert a newline. CharLimit is left at textarea's default 0
// (unlimited).
type HubTab struct {
	messages      []protocol.Message
	viewport      viewport.Model
	input         textarea.Model
	width         int
	height        int
	focused       bool // true when the command input is focused
	sessionFilter string
	pane          *panestate.Manager // first-order activity strip source
	// followBottom sticks the viewport to the latest message. Default true so
	// hub initial render snaps to present (fixes post-restart "rendered mid-
	// conversation"). Disengages on user scroll-up; re-engages on G / end.
	followBottom bool
}

// SetPane wires a panestate.Manager so HubTab.View can render the activity
// strip above the input bar. App calls this after construction. Phase E
// commit 4 wiring; Phase F may collapse the message-driven path entirely.
func (h *HubTab) SetPane(p *panestate.Manager) {
	h.pane = p
}

// NewHubTab creates a new HubTab with default dimensions.
func NewHubTab() HubTab {
	ta := textarea.New()
	ta.Placeholder = "Type a command (@agent message, spawn project, etc.)..."
	ta.CharLimit = 0 // unlimited; long pastes must round-trip without truncation
	ta.ShowLineNumbers = false
	ta.SetHeight(inputRows)
	// Newline keys: shift+enter (capable terminals), ctrl+j / alt+enter
	// (universal fallback), and "enter" itself — but only when forwarded
	// to the textarea. HubTab.Update intercepts non-paste enter keystrokes
	// as submit BEFORE forwarding, so plain enter on a real keypress
	// submits while pasted '\n' (delivered as Paste=true enter KeyMsgs by
	// bubbletea's bracketed-paste handler) inserts the newline as the user
	// expects.
	ta.KeyMap.InsertNewline = key.NewBinding(
		key.WithKeys("enter", "ctrl+m", "shift+enter", "ctrl+j", "alt+enter"),
		key.WithHelp("shift+enter", "newline"),
	)

	vp := viewport.New(80, 20)
	vp.MouseWheelEnabled = true

	return HubTab{
		viewport:     vp,
		input:        ta,
		followBottom: true,
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
		// Auto-scroll only if user is following the bottom. If they scrolled
		// up to read history, don't snap them back on each new message.
		if h.followBottom {
			h.viewport.GotoBottom()
		}

	case tea.KeyMsg:
		if h.focused {
			switch {
			case msg.String() == "enter" && !msg.Paste:
				// Real enter keystroke = submit. Pasted '\n' (Paste=true)
				// falls through to the textarea so it lands as a newline
				// in the buffer.
				val := h.input.Value()
				if val != "" {
					h.input.Reset()
					cmds = append(cmds, func() tea.Msg {
						return CommandSubmitted{Text: val}
					})
				}
			case msg.String() == "esc":
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
			case "end":
				// Jump to present: snap to bottom and re-engage auto-follow.
				h.viewport.GotoBottom()
				h.followBottom = true
			default:
				// Auto-focus input on any printable character OR on a
				// bracketed-paste delivery. Without the Paste branch a
				// multi-rune paste arriving while unfocused would route to
				// the viewport (silently dropped) instead of being captured
				// as input — observably "bracketed paste didn't work" even
				// though the bubbletea bracketed-paste path itself was fine.
				printable := len(key) == 1 && key >= " " && key <= "~"
				if printable || msg.Paste {
					h.focused = true
					cmds = append(cmds, h.input.Focus())
					var cmd tea.Cmd
					h.input, cmd = h.input.Update(msg)
					cmds = append(cmds, cmd)
				} else {
					var cmd tea.Cmd
					h.viewport, cmd = h.viewport.Update(msg)
					cmds = append(cmds, cmd)
					// Recompute follow state after viewport scroll. User
					// scrolled to bottom = follow re-engages; otherwise off.
					h.followBottom = h.viewport.AtBottom()
				}
			}
		}

	case tea.MouseMsg:
		if !h.focused {
			var cmd tea.Cmd
			h.viewport, cmd = h.viewport.Update(msg)
			cmds = append(cmds, cmd)
			h.followBottom = h.viewport.AtBottom()
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
// Pass an empty string to clear the filter. Re-engages auto-follow on the new
// view since switching filter is the user asking to see "latest of this slice."
func (h *HubTab) SetSessionFilter(sessionID string) {
	h.sessionFilter = sessionID
	h.viewport.SetContent(h.renderMessages())
	h.viewport.GotoBottom()
	h.followBottom = true
}

// resize recalculates viewport and input dimensions. When the user is in
// follow-bottom mode (initial render or actively tracking latest), snap to
// bottom after the resize so a terminal resize doesn't strand them mid-feed.
// When the user has scrolled up, preserve their scroll position.
func (h *HubTab) resize() {
	// Reserve: 1 separator + 1 strip + inputRows for textarea + 1 padding.
	reserved := 3 + inputRows
	vpHeight := h.height - reserved
	if vpHeight < 1 {
		vpHeight = 1
	}

	h.viewport.Width = h.width
	h.viewport.Height = vpHeight
	h.input.SetWidth(h.width - 4) // Account for prompt and padding
	h.input.SetHeight(inputRows)

	h.viewport.SetContent(h.renderMessages())
	if h.followBottom {
		h.viewport.GotoBottom()
	}
}

// View renders the HubTab. Layout (top to bottom):
//
//	viewport       — scrollable message feed
//	separator      — dividing line
//	strip          — per-agent activity dots (Phase E commit 4)
//	input          — command input
//
// Strip line is reserved even when no agents are alive so the input bar
// position stays stable across strip-empty/non-empty transitions.
func (h HubTab) View() string {
	separator := lipgloss.NewStyle().
		Width(h.width).
		Foreground(lipgloss.Color("#555555")).
		Render(strings.Repeat("─", h.width))

	stripContent := ""
	if h.pane != nil {
		stripContent = renderStrip(h.pane.Snapshot())
	}
	stripStyle := lipgloss.NewStyle().Width(h.width).Foreground(ColorStatus)
	strip := stripStyle.Render(stripContent)

	return lipgloss.JoinVertical(lipgloss.Left,
		h.viewport.View(),
		separator,
		strip,
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

// wrapText wraps text to fit within maxWidth, breaking at spaces, while
// preserving original `\n` paragraph structure. Bullet lists, headers,
// blank lines, and any other newline-shaped formatting round-trip through
// wrapping unchanged.
//
// Continuation lines (within a paragraph after the first wrap point, and
// the first line of every paragraph after the first) are indented by 2
// spaces. The first line of the first paragraph is not indented.
func wrapText(text string, maxWidth int) string {
	if maxWidth <= 0 {
		return text
	}
	indent := strings.Repeat(" ", 2)
	paragraphs := strings.Split(text, "\n")
	var out []string
	for pi, p := range paragraphs {
		if p == "" {
			out = append(out, "")
			continue
		}
		words := strings.Fields(p)
		if len(words) == 0 {
			out = append(out, "")
			continue
		}
		var lines []string
		current := words[0]
		for _, w := range words[1:] {
			if len(current)+1+len(w) > maxWidth {
				lines = append(lines, current)
				current = w
			} else {
				current += " " + w
			}
		}
		lines = append(lines, current)
		first := lines[0]
		if pi > 0 {
			first = indent + first
		}
		out = append(out, first)
		for _, l := range lines[1:] {
			out = append(out, indent+l)
		}
	}
	return strings.Join(out, "\n")
}

// messageColor determines the display color for a message based on the sender.
func (h HubTab) messageColor(msg protocol.Message) lipgloss.Color {
	if msg.Type == protocol.MsgError {
		return ColorError
	}
	return agentColor(msg.FromAgent)
}
