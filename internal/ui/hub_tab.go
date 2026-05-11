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

// CommandSubmitted is emitted when the user presses Enter in a
// command input. Z-8f added SessionID so the container input in the
// Sessions tab can tag its outbound message with the session-scope.
// Empty SessionID = main hub submit (broadcast).
type CommandSubmitted struct {
	Text      string
	SessionID string
}

// HubTab displays the main hub view: cross-session-relevant traffic on
// the left (user-to-emma chat + broadcasts + elevations from any
// session) and per-session strip column on the right (one card per
// active session with recent message previews).
//
// Z-8e separates Hub from Container: pre-Z-8e the Hub tab was reused
// as both the global view and the per-session-filtered view (via
// SetSessionFilter from the Sessions tab). Post-Z-8e the per-session
// view lives in the Sessions tab as a drilled-in container view, and
// the Hub tab strictly shows main-hub traffic via PassesMainHubView.
//
// Input is a textarea (not textinput) so users can compose multi-line
// messages and paste long content without truncation. Keybindings are
// flipped from textarea's defaults: enter submits, shift+enter / ctrl+j /
// alt+enter insert a newline. CharLimit is left at textarea's default 0
// (unlimited).
type HubTab struct {
	messages []protocol.Message
	viewport viewport.Model
	input    textarea.Model
	width    int
	height   int
	focused  bool // true when the command input is focused
	sessions []protocol.Session // Z-8e: active sessions for the strip column
	pane     *panestate.Manager // first-order activity strip source
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
	ta.Placeholder = "Talk to emma (open/close sessions, route cross-session, brainstorm new project)..."
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

	case SessionsUpdated:
		// Z-8e: track active sessions for the strip column render.
		h.sessions = msg.Sessions

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

// SetSessionFilter is retained as a no-op for back-compat with callers
// not yet ported to Z-8f (Sessions tab container view). Z-8e dropped
// the per-session filter from the Hub tab — main hub now strictly
// shows rows where PassesMainHubView. Callers wanting a session view
// should use the Sessions tab drill-in.
func (h *HubTab) SetSessionFilter(_ string) {
	// no-op: Z-8e
}

// sessionStripColumnWidth returns the right-column width for session
// strips: 30% of total width, clamped to [24, 40] cols. Narrower
// terminals collapse the column (returns 0 → strip skipped in View).
func (h HubTab) sessionStripColumnWidth() int {
	if h.width < 80 {
		return 0
	}
	w := h.width * 30 / 100
	if w < 24 {
		w = 24
	}
	if w > 40 {
		w = 40
	}
	return w
}

// SessionFilter is retained as a no-op for back-compat with callers
// that haven't yet been ported to the Z-8e Hub-vs-Container split.
// The Hub tab no longer filters by session — Sessions tab owns the
// drilled-in container view (Z-8f). Always returns "".
func (h HubTab) SessionFilter() string {
	return ""
}

// resize recalculates viewport and input dimensions. When the user is in
// follow-bottom mode (initial render or actively tracking latest), snap to
// bottom after the resize so a terminal resize doesn't strand them mid-feed.
// When the user has scrolled up, preserve their scroll position.
func (h *HubTab) resize() {
	// Reserve: 1 indicator + 1 strip top border + 1 strip + 1 strip bottom border + inputRows.
	// Total = 4 + inputRows (unchanged from prior separator+indicator+strip+padding layout).
	reserved := 4 + inputRows
	vpHeight := h.height - reserved
	if vpHeight < 1 {
		vpHeight = 1
	}

	// Z-8e: chat viewport takes left ~70% width when the strip column
	// renders; full width when terminal is too narrow for the strip.
	stripW := h.sessionStripColumnWidth()
	vpWidth := h.width
	if stripW > 0 {
		vpWidth = h.width - stripW - 1 // -1 for the vertical separator
		if vpWidth < 20 {
			vpWidth = 20
		}
	}

	h.viewport.Width = vpWidth
	h.viewport.Height = vpHeight
	h.input.SetWidth(h.width - 4) // Account for prompt and padding
	h.input.SetHeight(inputRows)

	h.viewport.SetContent(h.renderMessages())
	if h.followBottom {
		h.viewport.GotoBottom()
	}
}

// View renders the HubTab. Layout:
//
//	┌─────────────┬─────────────┐
//	│ chat        │ session     │
//	│ viewport    │ strips      │
//	│             │ (Z-8e col)  │
//	├─────────────┴─────────────┤
//	│ indicator (full-width)    │
//	│ agent dots strip          │
//	│ input textarea            │
//	└───────────────────────────┘
//
// The session-strip column renders when terminal is wide enough
// (sessionStripColumnWidth > 0); otherwise it collapses and the chat
// takes full width. Bottom slots (indicator/strip/input) span the
// full width regardless.
func (h HubTab) View() string {
	indicatorStyle := lipgloss.NewStyle().Width(h.width).Foreground(ColorStatus)
	indicatorText := ""
	if !h.followBottom {
		indicatorText = "↓ scrolled up — press end to return"
	}
	indicator := indicatorStyle.Render(indicatorText)

	agentStripContent := ""
	if h.pane != nil {
		agentStripContent = renderStrip(h.pane.Snapshot(), h.pane.HubSnapshot(), h.width)
	}
	agentStripStyle := lipgloss.NewStyle().
		Border(lipgloss.NormalBorder(), true, false, true, false).
		BorderForeground(lipgloss.Color("#555555")).
		Width(h.width).
		Foreground(ColorStatus)
	agentStrip := agentStripStyle.Render(agentStripContent)

	// Z-8e: build the top-row split (chat viewport + session strip
	// column). When terminal is too narrow the column collapses and
	// chat takes full width.
	stripW := h.sessionStripColumnWidth()
	var topRow string
	if stripW > 0 {
		// Render the strip column with the same height as the chat
		// viewport so the row aligns cleanly.
		stripHeight := h.viewport.Height
		stripColStyle := lipgloss.NewStyle().
			Border(lipgloss.NormalBorder(), false, false, false, true).
			BorderForeground(lipgloss.Color("#555555")).
			Width(stripW).
			Height(stripHeight)
		stripCol := stripColStyle.Render(renderSessionStrips(h.sessions, h.messages, stripW, stripHeight))
		topRow = lipgloss.JoinHorizontal(lipgloss.Top, h.viewport.View(), stripCol)
	} else {
		topRow = h.viewport.View()
	}

	return lipgloss.JoinVertical(lipgloss.Left,
		topRow,
		indicator,
		agentStrip,
		h.input.View(),
	)
}

// renderMessages builds the full string content for the viewport from
// the main-hub-view subset of all messages: broadcasts + elevations
// (Z-8c PassesMainHubView). Session-scoped non-elevated chatter is
// hidden from the main hub view.
func (h HubTab) renderMessages() string {
	if len(h.messages) == 0 {
		empty := lipgloss.NewStyle().
			Foreground(ColorStatus).
			Render("No messages yet. Waiting for agents to connect...")
		return empty
	}

	var lines []string
	for _, msg := range h.messages {
		if !protocol.PassesMainHubView(msg) {
			continue
		}
		lines = append(lines, h.formatMessage(msg))
	}
	if len(lines) == 0 {
		return lipgloss.NewStyle().
			Foreground(ColorStatus).
			Render("No main-hub traffic yet. Session-scoped activity lives in the Sessions tab; elevated signals ([HR]/Flag) will surface here.")
	}
	return strings.Join(lines, "\n")
}

// formatMessage renders a single message as "[HH:MM:SS] [session] from: content".
// Z-5i removed the "→ to:" arrow rendering: Phase S S-4 already dropped
// PM semantics from the wire protocol; the prior arrow display
// implied a PM-class targeting that hasn't existed since S-4. User-
// mockup format is "[Session-id] sender: content" — sender colored
// per agentColor, session-tag in aggregated view only.
func (h HubTab) formatMessage(msg protocol.Message) string {
	timestamp := msg.Created.Format("15:04:05")
	sessionTag := h.sessionTagFor(msg) // empty when filter is set

	if msg.Type == protocol.MsgFlag {
		flagStyle := lipgloss.NewStyle().Foreground(ColorError).Bold(true)
		fromStyle := lipgloss.NewStyle().Foreground(agentColor(msg.FromAgent))
		tsStyle := lipgloss.NewStyle().Foreground(ColorStatus)
		if sessionTag != "" {
			return fmt.Sprintf("%s %s %s %s %s",
				tsStyle.Render("["+timestamp+"]"),
				tsStyle.Render(sessionTag),
				flagStyle.Render("⚑ FLAG"),
				fromStyle.Render(msg.FromAgent+":"),
				flagStyle.Render(msg.Content),
			)
		}
		return fmt.Sprintf("%s %s %s %s",
			tsStyle.Render("["+timestamp+"]"),
			flagStyle.Render("⚑ FLAG"),
			fromStyle.Render(msg.FromAgent+":"),
			flagStyle.Render(msg.Content),
		)
	}

	tsStyle := lipgloss.NewStyle().Foreground(ColorStatus)

	fromColor := agentColor(msg.FromAgent)
	fromStyle := lipgloss.NewStyle().Foreground(fromColor)

	// Message content matches the author's color
	msgColor := fromColor
	if msg.Type == protocol.MsgError {
		msgColor = ColorError
	}

	// Build the prefix (including optional session tag) to calculate its
	// visible width for wrapping.
	var prefix string
	if sessionTag != "" {
		prefix = fmt.Sprintf("[%s] %s %s: ", timestamp, sessionTag, msg.FromAgent)
	} else {
		prefix = fmt.Sprintf("[%s] %s: ", timestamp, msg.FromAgent)
	}
	prefixLen := len(prefix)

	// Wrap content to fit viewport width
	content := msg.Content
	if h.width > 0 && prefixLen+len(content) > h.width {
		content = wrapText(content, h.width-prefixLen)
	}

	msgStyle := lipgloss.NewStyle().Foreground(msgColor)

	if sessionTag != "" {
		return fmt.Sprintf("%s %s %s: %s",
			tsStyle.Render("["+timestamp+"]"),
			tsStyle.Render(sessionTag),
			fromStyle.Render(msg.FromAgent),
			msgStyle.Render(content),
		)
	}
	return fmt.Sprintf("%s %s: %s",
		tsStyle.Render("["+timestamp+"]"),
		fromStyle.Render(msg.FromAgent),
		msgStyle.Render(content),
	)
}

// sessionTagFor returns the short session-attribution tag for a message
// in the aggregated view, or "" when the Hub is filtered to a single
// session (where the tag would be redundant on every row).
//
//   - filter set       → "" (no tag)
//   - msg.SessionID==""→ "[main]"
//   - else             → "[<scope4>·<uuid>]" — first 4 chars of the
//     scope-slug + '·' + the trailing uuid-suffix (everything after the
//     last '-'). Combining both halves keeps the tag readable (scope
//     hint) and distinct (uuid-suffix has high entropy), avoiding
//     collisions across sessions with shared scope-prefixes like
//     "captain-hook-A-…" vs "captain-hook-B-…".
//
// Falls back to the raw id when no '-' is present (legacy date-keyed
// IDs are rare post Z-3 but keep the path defensive).
func (h HubTab) sessionTagFor(msg protocol.Message) string {
	// Z-8e: Hub tab no longer filters by session; tag is always
	// rendered to disambiguate broadcast/main vs session-scoped
	// elevated rows in the same viewport.
	if msg.SessionID == "" {
		return "[main]"
	}
	id := msg.SessionID
	idx := strings.LastIndex(id, "-")
	if idx <= 0 || idx >= len(id)-1 {
		// No '-' or trailing '-' — use first 6 chars as best-effort.
		if len(id) > 6 {
			return "[" + id[:6] + "]"
		}
		return "[" + id + "]"
	}
	scope := id[:idx]
	uuid := id[idx+1:]
	scopeShort := scope
	if len(scopeShort) > 4 {
		scopeShort = scopeShort[:4]
	}
	return "[" + scopeShort + "·" + uuid + "]"
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
	case lower == "clive":
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

