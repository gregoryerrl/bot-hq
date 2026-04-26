package ui

import (
	"fmt"
	"time"

	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// paneModalScrollback is the number of scrollback lines to capture from the
// tmux pane. 500 trades a slightly larger payload for enough history that
// the modal is useful for catching up on output that scrolled off the
// visible viewport. Phase G v1 §2.2 (Rain C4).
const paneModalScrollback = 500

// paneModalAutoRefreshInterval drives the auto-follow tick when enabled.
// 1Hz balances responsiveness against tmux capture-pane shell-out cost.
const paneModalAutoRefreshInterval = time.Second

// PaneCaptureFunc abstracts tmux.CapturePane so tests can stub it.
type PaneCaptureFunc func(target string, lines int) (string, error)

// PaneModalClosed signals that the modal wants to be torn down.
type PaneModalClosed struct{}

// paneRefreshTick is the auto-follow timer message.
type paneRefreshTick struct{}

// PaneModal is a read-only overlay showing the captured content of a tmux
// pane. Capture-only by design — no key forwarding to the underlying pane
// in v1. Phase G v1 §2.2.
type PaneModal struct {
	target     string
	capture    PaneCaptureFunc
	viewport   viewport.Model
	autoFollow bool
	lastErr    error
	width      int
	height     int
}

// NewPaneModal constructs a modal bound to the given tmux target. capture
// is the function used to fetch pane content; production passes
// tmux.CapturePane, tests pass a stub.
func NewPaneModal(target string, capture PaneCaptureFunc) *PaneModal {
	vp := viewport.New(80, 20)
	vp.MouseWheelEnabled = true
	return &PaneModal{
		target:   target,
		capture:  capture,
		viewport: vp,
	}
}

// SetSize updates the modal's outer dimensions and resizes the inner
// viewport accordingly. Reserves 1 title row + 1 footer row + 2 border
// rows; horizontal padding accounts for the rounded border (2 cols).
func (m *PaneModal) SetSize(w, h int) {
	m.width = w
	m.height = h
	// 2-col right gutter inside border for breathing room (border = 2 cols,
	// gutter = 2 cols, innerW = w - 4).
	innerW := w - 4
	if innerW < 1 {
		innerW = 1
	}
	innerH := h - 4
	if innerH < 1 {
		innerH = 1
	}
	m.viewport.Width = innerW
	m.viewport.Height = innerH
}

// Refresh captures the pane content and reloads the viewport. On capture
// error, lastErr is set and the existing content remains (no flicker on
// transient tmux glitches).
func (m *PaneModal) Refresh() error {
	output, err := m.capture(m.target, paneModalScrollback)
	if err != nil {
		m.lastErr = err
		return err
	}
	m.lastErr = nil
	// Wrap to viewport width so over-long pane lines don't escape past the
	// rounded border and visually break it. lipgloss Style.Width(N).Render
	// wraps each newline-separated line independently; verified via scratch
	// test (multi-paragraph input → per-paragraph wrap, no concatenation).
	wrapped := lipgloss.NewStyle().Width(m.viewport.Width).Render(stripANSI(output))
	m.viewport.SetContent(wrapped)
	m.viewport.GotoBottom()
	return nil
}

// Update handles keypresses + the auto-follow tick.
func (m *PaneModal) Update(msg tea.Msg) (*PaneModal, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "esc":
			return m, func() tea.Msg { return PaneModalClosed{} }
		case "r":
			_ = m.Refresh()
			return m, nil
		case "f":
			m.autoFollow = !m.autoFollow
			if m.autoFollow {
				return m, paneRefreshTickCmd()
			}
			return m, nil
		default:
			// Forward viewport-scroll keys (PgUp/PgDn, arrows, etc.).
			var cmd tea.Cmd
			m.viewport, cmd = m.viewport.Update(msg)
			return m, cmd
		}
	case paneRefreshTick:
		if !m.autoFollow {
			// Race: user toggled off between ticks. Drop the tick.
			return m, nil
		}
		_ = m.Refresh()
		return m, paneRefreshTickCmd()
	}
	return m, nil
}

func paneRefreshTickCmd() tea.Cmd {
	return tea.Tick(paneModalAutoRefreshInterval, func(time.Time) tea.Msg {
		return paneRefreshTick{}
	})
}

// View renders the modal frame: title row, captured pane viewport, error
// footer (when set). Uses a rounded lipgloss border to visually separate
// the modal from the agents tab list underneath.
func (m *PaneModal) View() string {
	title := fmt.Sprintf("tmux:%s  [r] refresh  [f] follow:%s  [esc] close",
		m.target, onOff(m.autoFollow))
	titleStyled := lipgloss.NewStyle().Foreground(ColorSystem).Bold(true).Render(title)

	body := m.viewport.View()

	var footer string
	if m.lastErr != nil {
		footer = lipgloss.NewStyle().Foreground(ColorError).Render(
			"error: " + m.lastErr.Error(),
		)
	} else {
		footer = lipgloss.NewStyle().Foreground(ColorStatus).Render(
			fmt.Sprintf("(scrollback=%d  ↑/↓/PgUp/PgDn to scroll)", paneModalScrollback),
		)
	}

	box := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(ColorClive)
	if m.width > 2 {
		box = box.Width(m.width - 2)
	}
	return box.Render(lipgloss.JoinVertical(lipgloss.Left, titleStyled, body, footer))
}

// AutoFollow returns whether the modal is currently auto-refreshing. Test
// helper.
func (m *PaneModal) AutoFollow() bool { return m.autoFollow }

// Target returns the tmux target the modal is bound to. Test helper.
func (m *PaneModal) Target() string { return m.target }

// LastError returns the most recent capture error (or nil). Test helper.
func (m *PaneModal) LastError() error { return m.lastErr }

func onOff(b bool) string {
	if b {
		return "on"
	}
	return "off"
}
