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
	// stripViewport scrolls the right-side session-strip column. Z-9e
	// upgrade: previously a static lipgloss render that clipped when more
	// than ~5 active sessions overflowed the column height. Now backed
	// by a real viewport.Model so the bottom cards stay reachable.
	stripViewport viewport.Model
	// stripActive routes arrow keys / PgUp / PgDn to stripViewport when
	// true, chat viewport when false. Toggled via Ctrl+R. Mouse wheel
	// always routes by cursor X position regardless of this flag.
	stripActive bool
	// chatColEnd is the rightmost X column the chat viewport occupies
	// (inclusive). Mouse messages with X > chatColEnd route to the strip
	// viewport. Recomputed on every resize.
	chatColEnd int
	input      textarea.Model
	width      int
	height     int
	focused    bool // true when the command input is focused
	sessions   []protocol.Session // Z-8e: active sessions for the strip column
	pane       *panestate.Manager // first-order activity strip source
	// followBottom sticks the chat viewport to the latest message. Default
	// true so hub initial render snaps to present. Disengages on user
	// scroll-up; re-engages on G / end.
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

	stripVp := viewport.New(30, 20)
	stripVp.MouseWheelEnabled = true

	return HubTab{
		viewport:      vp,
		stripViewport: stripVp,
		input:         ta,
		followBottom:  true,
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
		// Strip-card preview lines pull from msg history per-session, so
		// new messages can shift the strip too. Recompute its content.
		h.refreshStripContent()

	case SessionsUpdated:
		// Z-8e: track active sessions for the strip column render.
		h.sessions = msg.Sessions
		h.refreshStripContent()

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
				// Jump to present: snap chat to bottom and re-engage auto-follow.
				// Always operates on the chat pane regardless of stripActive —
				// "return to present" is a chat concept.
				h.viewport.GotoBottom()
				h.followBottom = true
			case "ctrl+r":
				// Z-9e: toggle arrow-key / PgUp / PgDn target between
				// chat viewport (default) and strip viewport. Mouse
				// wheel routes by cursor position regardless.
				if h.sessionStripColumnWidth() > 0 {
					h.stripActive = !h.stripActive
				}
			default:
				// Auto-focus input on any printable rune-bearing KeyMsg
				// OR on a bracketed-paste delivery. Z-9c: the check is
				// "msg carries at least one printable rune", NOT
				// "msg.String() is a single printable char" — tmux
				// send-keys (and fast typists) deliver multi-rune
				// batches in a single KeyMsg, and the pre-fix
				// `len(key)==1` test silently dropped them all.
				if isPrintableRuneMsg(msg) || msg.Paste {
					h.focused = true
					cmds = append(cmds, h.input.Focus())
					var cmd tea.Cmd
					h.input, cmd = h.input.Update(msg)
					cmds = append(cmds, cmd)
				} else if h.stripActive && h.sessionStripColumnWidth() > 0 {
					var cmd tea.Cmd
					h.stripViewport, cmd = h.stripViewport.Update(msg)
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
			// Z-9e: mouse wheel routes by cursor X position. Cursor over
			// strip column → strip viewport; otherwise → chat viewport.
			// chatColEnd is recomputed on every resize.
			if h.sessionStripColumnWidth() > 0 && msg.X > h.chatColEnd {
				var cmd tea.Cmd
				h.stripViewport, cmd = h.stripViewport.Update(msg)
				cmds = append(cmds, cmd)
			} else {
				var cmd tea.Cmd
				h.viewport, cmd = h.viewport.Update(msg)
				cmds = append(cmds, cmd)
				h.followBottom = h.viewport.AtBottom()
			}
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
	// Reserve: 1 indicator + inputRows. Z-9a dropped the bottom
	// activity-strip section (3 rows: top border + strip + bottom
	// border), freeing those rows for the chat viewport.
	reserved := 1 + inputRows
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

	// Z-9e: strip viewport sizing + mouse-routing X-boundary. chatColEnd
	// is the rightmost column owned by the chat (inclusive). Anything to
	// the right of it belongs to the strip column. When the strip
	// collapses (narrow terminal), chatColEnd extends to the full width
	// so MouseMsgs all route to chat.
	if stripW > 0 {
		// Strip viewport sits inside a left-bordered container (1 col
		// border + content). Account for the border when sizing the
		// inner viewport so it fits without horizontal clipping.
		innerStripW := stripW - 1
		if innerStripW < 1 {
			innerStripW = 1
		}
		h.stripViewport.Width = innerStripW
		h.stripViewport.Height = vpHeight
		h.chatColEnd = vpWidth // separator is at column vpWidth; strip begins after
		// If stripActive was set when the column was wider and the user
		// then shrank the terminal below the strip threshold, fall back
		// to chat-active.
	} else {
		h.stripViewport.Width = 0
		h.stripViewport.Height = 0
		h.chatColEnd = h.width
		h.stripActive = false
	}

	h.viewport.SetContent(h.renderMessages())
	if h.followBottom {
		h.viewport.GotoBottom()
	}
	h.refreshStripContent()
}

// refreshStripContent recomputes the strip viewport's content from the
// current sessions + messages slice. Caller is responsible for invoking
// after any state change that affects strip rendering (SessionsUpdated,
// MessageReceived, resize). Cheap — pure string build.
func (h *HubTab) refreshStripContent() {
	if h.stripViewport.Width <= 0 {
		return
	}
	// Pass a generous height so renderSessionStrips emits all cards;
	// the viewport handles scroll-clipping internally.
	h.stripViewport.SetContent(renderSessionStrips(h.sessions, h.messages, h.stripViewport.Width, 9999))
}

// View renders the HubTab. Layout:
//
//	┌─────────────┬─────────────┐
//	│ chat        │ session     │
//	│ viewport    │ strips      │
//	│             │ (Z-8e col)  │
//	├─────────────┴─────────────┤
//	│ indicator (full-width)    │
//	│ input textarea            │
//	└───────────────────────────┘
//
// Z-9a: bottom agent-dots activity strip dropped (Agents tab gone;
// plan-usage moved to top tab bar in app.View). The session-strip
// column on the right renders when terminal is wide enough; otherwise
// it collapses and the chat takes full width.
func (h HubTab) View() string {
	indicatorStyle := lipgloss.NewStyle().Width(h.width).Foreground(ColorStatus)
	var parts []string
	if h.sessionStripColumnWidth() > 0 {
		// Z-9e: show which pane arrow keys / PgUp / PgDn drive. Mouse
		// wheel routes by cursor position so this hint is only relevant
		// to keyboard users.
		if h.stripActive {
			parts = append(parts, "[strips] ctrl+r → chat")
		} else {
			parts = append(parts, "[chat] ctrl+r → strips")
		}
	}
	if !h.followBottom {
		parts = append(parts, "↓ scrolled up — press end to return")
	}
	indicator := indicatorStyle.Render(strings.Join(parts, "  "))

	// Z-8e: build the top-row split (chat viewport + session strip
	// column). Z-9e: strip column is now a real viewport (scrollable).
	// When terminal is too narrow the column collapses and chat takes
	// full width.
	stripW := h.sessionStripColumnWidth()
	var topRow string
	if stripW > 0 {
		stripHeight := h.viewport.Height
		stripColStyle := lipgloss.NewStyle().
			Border(lipgloss.NormalBorder(), false, false, false, true).
			BorderForeground(lipgloss.Color("#555555")).
			Width(stripW).
			Height(stripHeight)
		stripCol := stripColStyle.Render(h.stripViewport.View())
		topRow = lipgloss.JoinHorizontal(lipgloss.Top, h.viewport.View(), stripCol)
	} else {
		topRow = h.viewport.View()
	}

	return lipgloss.JoinVertical(lipgloss.Left,
		topRow,
		indicator,
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

// isPrintableRuneMsg reports whether the KeyMsg carries at least one
// printable rune (i.e., user typed text). Used to discriminate
// "typing into input" from control-key events like tab/escape/arrow
// keys when deciding whether to auto-focus the textarea.
//
// Z-9c: previous heuristic checked `len(msg.String()) == 1` which
// dropped any multi-rune KeyMsg batch (tmux send-keys, fast typing,
// pastes-without-bracketed-paste-flag). Now we look at the actual
// runes payload — any printable rune in the batch qualifies.
func isPrintableRuneMsg(msg tea.KeyMsg) bool {
	if msg.Type != tea.KeyRunes && msg.Type != tea.KeySpace {
		return false
	}
	for _, r := range msg.Runes {
		if r >= ' ' && r <= '~' {
			return true
		}
	}
	return false
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

