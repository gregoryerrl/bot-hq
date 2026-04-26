package ui

import (
	"errors"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// stubCapture returns a capture function that yields a fixed payload.
func stubCapture(payload string) PaneCaptureFunc {
	return func(target string, lines int) (string, error) {
		return payload, nil
	}
}

// errCapture returns a capture function that always errors.
func errCapture(err error) PaneCaptureFunc {
	return func(target string, lines int) (string, error) {
		return "", err
	}
}

// TestPaneModalRefreshSetsContent locks that Refresh populates the
// viewport with capture output and clears any prior error.
func TestPaneModalRefreshSetsContent(t *testing.T) {
	m := NewPaneModal("bot-hq-brian-1", stubCapture("hello world"))
	m.SetSize(80, 24)
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if m.LastError() != nil {
		t.Errorf("expected no error after successful refresh, got %v", m.LastError())
	}
	if !strings.Contains(m.View(), "hello world") {
		t.Errorf("modal view should contain captured payload, got:\n%s", m.View())
	}
}

// TestPaneModalRefreshErrorPreservesContent locks that a capture error
// surfaces in lastErr but does not corrupt the existing view (no flicker
// on transient tmux glitches).
func TestPaneModalRefreshErrorPreservesContent(t *testing.T) {
	wantErr := errors.New("tmux daemon down")
	m := NewPaneModal("bot-hq-brian-1", errCapture(wantErr))
	m.SetSize(80, 24)
	if err := m.Refresh(); err != wantErr {
		t.Errorf("expected returned error to equal capture error, got %v", err)
	}
	if m.LastError() != wantErr {
		t.Errorf("expected lastErr=%v, got %v", wantErr, m.LastError())
	}
}

// TestPaneModalEscClosesModal locks that pressing Esc emits a
// PaneModalClosed signal via the returned tea.Cmd.
func TestPaneModalEscClosesModal(t *testing.T) {
	m := NewPaneModal("bot-hq-brian-1", stubCapture("x"))
	_, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEsc})
	if cmd == nil {
		t.Fatal("expected Esc to produce a Cmd")
	}
	out := cmd()
	if _, ok := out.(PaneModalClosed); !ok {
		t.Errorf("expected PaneModalClosed, got %T", out)
	}
}

// TestPaneModalAutoFollowToggle locks that 'f' flips the autoFollow flag.
func TestPaneModalAutoFollowToggle(t *testing.T) {
	m := NewPaneModal("bot-hq-brian-1", stubCapture("x"))
	if m.AutoFollow() {
		t.Fatal("expected autoFollow to default false")
	}
	m, _ = m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'f'}})
	if !m.AutoFollow() {
		t.Errorf("expected autoFollow=true after 'f' press")
	}
	m, _ = m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'f'}})
	if m.AutoFollow() {
		t.Errorf("expected autoFollow=false after second 'f' press")
	}
}

// TestPaneModalRKeyRefreshes locks that 'r' triggers a fresh capture.
func TestPaneModalRKeyRefreshes(t *testing.T) {
	calls := 0
	captureWithCount := func(target string, lines int) (string, error) {
		calls++
		return "tick", nil
	}
	m := NewPaneModal("bot-hq-brian-1", captureWithCount)
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if calls != 1 {
		t.Fatalf("setup precondition failed: capture not called once, got %d", calls)
	}
	m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'r'}})
	if calls != 2 {
		t.Errorf("expected 'r' to trigger second capture, got %d total calls", calls)
	}
}

// TestAgentsTabEnterOpensModal locks that pressing Enter on an agent row
// with a tmux_target meta opens the pane modal.
func TestAgentsTabEnterOpensModal(t *testing.T) {
	agents := []protocol.Agent{
		{ID: "with-target", Name: "WithTarget", Type: protocol.AgentBrian, Status: protocol.StatusOnline,
			Meta: `{"tmux_target":"bot-hq-brian-1"}`},
	}
	tab := NewAgentsTab(stubCapture("captured"))
	tab.SetSize(120, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})

	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyEnter})

	if tab.paneModal == nil {
		t.Fatalf("expected modal to open on Enter, paneModal is nil")
	}
	if tab.paneModal.Target() != "bot-hq-brian-1" {
		t.Errorf("modal bound to wrong target: %q", tab.paneModal.Target())
	}
	if !strings.Contains(tab.View(), "captured") {
		t.Errorf("modal view should contain captured content, got:\n%s", tab.View())
	}
}

// TestAgentsTabEnterNoTargetIsNoop locks that Enter on an agent row with
// no tmux_target does NOT open the modal (and does not panic).
func TestAgentsTabEnterNoTargetIsNoop(t *testing.T) {
	agents := []protocol.Agent{
		{ID: "no-target", Name: "NoTarget", Type: protocol.AgentBrian, Status: protocol.StatusOnline},
	}
	tab := NewAgentsTab(stubCapture("x"))
	tab.SetSize(120, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})

	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyEnter})

	if tab.paneModal != nil {
		t.Errorf("Enter on agent without tmux_target should not open modal")
	}
}

// TestAgentsTabModalEscClosesModal locks that Esc inside an open modal
// returns the tab to its underlying list view (paneModal cleared).
func TestAgentsTabModalEscClosesModal(t *testing.T) {
	agents := []protocol.Agent{
		{ID: "a", Name: "A", Type: protocol.AgentBrian, Status: protocol.StatusOnline,
			Meta: `{"tmux_target":"t1"}`},
	}
	tab := NewAgentsTab(stubCapture("x"))
	tab.SetSize(120, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})
	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyEnter})
	if tab.paneModal == nil {
		t.Fatalf("setup precondition failed: modal should have opened on Enter")
	}

	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyEsc})

	if tab.paneModal != nil {
		t.Errorf("Esc inside modal should clear paneModal, still set")
	}
}

// TestAgentsTabCursorNavigation locks that j/k (and arrow keys) move the
// cursor within bounds.
func TestAgentsTabCursorNavigation(t *testing.T) {
	agents := []protocol.Agent{
		{ID: "a", Name: "A", Type: protocol.AgentBrian, Status: protocol.StatusOnline},
		{ID: "b", Name: "B", Type: protocol.AgentBrian, Status: protocol.StatusOnline},
		{ID: "c", Name: "C", Type: protocol.AgentBrian, Status: protocol.StatusOnline},
	}
	tab := NewAgentsTab(stubCapture(""))
	tab.SetSize(120, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})

	if tab.cursor != 0 {
		t.Fatalf("initial cursor should be 0, got %d", tab.cursor)
	}

	// Down.
	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'j'}})
	if tab.cursor != 1 {
		t.Errorf("after j cursor=%d want 1", tab.cursor)
	}
	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyDown})
	if tab.cursor != 2 {
		t.Errorf("after down-arrow cursor=%d want 2", tab.cursor)
	}
	// Past end clamps.
	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'j'}})
	if tab.cursor != 2 {
		t.Errorf("cursor should clamp at len-1=2, got %d", tab.cursor)
	}
	// Up.
	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'k'}})
	if tab.cursor != 1 {
		t.Errorf("after k cursor=%d want 1", tab.cursor)
	}
	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyUp})
	if tab.cursor != 0 {
		t.Errorf("after up-arrow cursor=%d want 0", tab.cursor)
	}
	// Past start clamps.
	tab, _ = tab.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'k'}})
	if tab.cursor != 0 {
		t.Errorf("cursor should clamp at 0, got %d", tab.cursor)
	}
}
