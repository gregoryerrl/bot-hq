package ui

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// fakeSource implements panestate.AgentSource for ui-package tests without
// touching a real DB. Returns the embedded slice unchanged.
type fakeSource struct {
	agents []protocol.Agent
}

func (f *fakeSource) ListAgents(string) ([]protocol.Agent, error) {
	return f.agents, nil
}

func newPaneWithAgents(t *testing.T, agents []protocol.Agent) *panestate.Manager {
	t.Helper()
	mgr := panestate.NewManager(&fakeSource{agents: agents})
	if err := mgr.Refresh(); err != nil {
		t.Fatal(err)
	}
	return mgr
}

func TestParseCommand(t *testing.T) {
	tests := []struct {
		input   string
		target  string
		content string
	}{
		{"@brian hello", "brian", "hello"},
		{"@claude-abc stop", "claude-abc", "stop"},
		{"@live check status", "live", "check status"},
		{"hello world", "", "hello world"},
		{"spawn bcc-ad-manager", "", "spawn bcc-ad-manager"},
		{"@brian", "brian", ""},
	}

	for _, tt := range tests {
		target, content := parseCommand(tt.input)
		if target != tt.target {
			t.Errorf("parseCommand(%q) target = %q, want %q", tt.input, target, tt.target)
		}
		if content != tt.content {
			t.Errorf("parseCommand(%q) content = %q, want %q", tt.input, content, tt.content)
		}
	}
}

// TestHubTabViewIncludesStrip verifies that View() output contains alive-agent
// IDs after SetPane wires a populated panestate.Manager. Spec §5 commit 4 test.
func TestHubTabViewIncludesStrip(t *testing.T) {
	pane := newPaneWithAgents(t, []protocol.Agent{
		{ID: "brian-test", Name: "Brian", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now()},
		{ID: "rain-test", Name: "Rain", Type: protocol.AgentQA, Status: protocol.StatusOnline, LastSeen: time.Now()},
	})
	hub := NewHubTab()
	hub.SetPane(pane)
	hub.SetSize(120, 30)

	out := hub.View()
	if !strings.Contains(out, "brian-test") {
		t.Errorf("HubTab.View should contain brian-test in strip, got:\n%s", out)
	}
	if !strings.Contains(out, "rain-test") {
		t.Errorf("HubTab.View should contain rain-test in strip, got:\n%s", out)
	}
}

// TestHubTabViewWithoutPane verifies View() doesn't panic and produces no
// strip content when SetPane was never called.
func TestHubTabViewWithoutPane(t *testing.T) {
	hub := NewHubTab()
	hub.SetSize(120, 30)
	out := hub.View()
	// View must not panic and must include the input bar / separator scaffolding.
	if out == "" {
		t.Error("View should produce non-empty output even without pane")
	}
}

// TestHubTabViewHidesStaleAgents verifies that stale/offline agents are not
// rendered in the strip even when the pane snapshot includes them.
func TestHubTabViewHidesStaleAgents(t *testing.T) {
	stale := time.Now().Add(-2 * time.Minute) // older than OnlineWindow (60s)
	pane := newPaneWithAgents(t, []protocol.Agent{
		{ID: "alive-agent", Name: "Alive", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now()},
		{ID: "stale-agent", Name: "Stale", Type: protocol.AgentCoder, Status: protocol.StatusOnline, LastSeen: stale},
		{ID: "offline-agent", Name: "Offline", Type: protocol.AgentCoder, Status: protocol.StatusOffline, LastSeen: time.Now()},
	})
	hub := NewHubTab()
	hub.SetPane(pane)
	hub.SetSize(120, 30)

	out := hub.View()
	if !strings.Contains(out, "alive-agent") {
		t.Errorf("strip should contain alive-agent, got:\n%s", out)
	}
	if strings.Contains(out, "stale-agent") {
		t.Errorf("strip should hide stale-agent, got:\n%s", out)
	}
	if strings.Contains(out, "offline-agent") {
		t.Errorf("strip should hide offline-agent, got:\n%s", out)
	}
}

// TestWrapTextPreservesParagraphBreaks locks that explicit `\n` between
// paragraphs round-trip through wrapping. The pre-fix behavior collapsed
// all newlines into spaces via strings.Fields, producing one wall of text.
func TestWrapTextPreservesParagraphBreaks(t *testing.T) {
	in := "line one\nline two"
	out := wrapText(in, 80)
	if !strings.Contains(out, "\n") {
		t.Fatalf("output lost paragraph break, got: %q", out)
	}
	if !strings.Contains(out, "line one") || !strings.Contains(out, "line two") {
		t.Fatalf("output dropped a paragraph, got: %q", out)
	}
	if strings.Count(out, "\n") != 1 {
		t.Errorf("expected exactly 1 newline, got %d in %q", strings.Count(out, "\n"), out)
	}
}

// TestWrapTextPreservesEmptyLines locks that blank lines between paragraphs
// (e.g. "a\n\nb") round-trip as empty segments, not collapsed.
func TestWrapTextPreservesEmptyLines(t *testing.T) {
	in := "para one\n\npara two"
	out := wrapText(in, 80)
	parts := strings.Split(out, "\n")
	if len(parts) != 3 {
		t.Fatalf("expected 3 segments (para1 / empty / para2), got %d: %q", len(parts), parts)
	}
	if parts[1] != "" {
		t.Errorf("middle segment must be empty, got %q", parts[1])
	}
}

// TestWrapTextBulletList locks that bullet lines stay on their own lines
// regardless of width — the most user-visible failure mode of the prior
// implementation.
func TestWrapTextBulletList(t *testing.T) {
	in := "- one\n- two\n- three"
	out := wrapText(in, 80)
	parts := strings.Split(out, "\n")
	if len(parts) != 3 {
		t.Fatalf("expected 3 bullet lines, got %d: %q", len(parts), parts)
	}
	for i, want := range []string{"one", "two", "three"} {
		if !strings.Contains(parts[i], want) {
			t.Errorf("part %d missing %q: %q", i, want, parts[i])
		}
	}
}

// TestWrapTextLongLineStillWraps locks that the within-paragraph wrap
// still happens at maxWidth — paragraph-split must not disable wrap.
func TestWrapTextLongLineStillWraps(t *testing.T) {
	in := "x x x x x x x x x x"
	out := wrapText(in, 5)
	if !strings.Contains(out, "\n") {
		t.Fatalf("expected wrap within paragraph, got: %q", out)
	}
}

// TestWrapTextNoNewlineRatchet locks that input without any `\n` produces
// the same shape as the pre-fix behavior for the common single-paragraph
// case (single line under maxWidth returns unchanged).
func TestWrapTextNoNewlineRatchet(t *testing.T) {
	in := "short message"
	out := wrapText(in, 80)
	if out != "short message" {
		t.Errorf("short single-line input changed unexpectedly: in=%q out=%q", in, out)
	}
}

// runeKey constructs a KeyMsg for a single rune (matches textarea's own
// test helpers in bubbles).
func runeKey(r rune) tea.KeyMsg {
	return tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{r}}
}

// typeString feeds each rune of s through the HubTab as KeyRunes events.
// Returns the updated HubTab.
func typeString(h HubTab, s string) HubTab {
	for _, r := range s {
		h, _ = h.Update(runeKey(r))
	}
	return h
}

// TestHubTabMultiLineSubmitOnEnter locks that ctrl+j (the universal
// terminal-supported newline binding) inserts a newline into the input
// buffer, then plain enter submits the full multi-line value via
// CommandSubmitted. shift+enter shares the same InsertNewline binding;
// ctrl+j is used in the test because bubbletea's KeyMsg encoding for
// shift+enter varies by terminal capability.
func TestHubTabMultiLineSubmitOnEnter(t *testing.T) {
	h := NewHubTab()
	h.SetSize(80, 24)
	h.focused = true
	h.input.Focus()

	h = typeString(h, "abc")

	// ctrl+j inserts a newline via the rebound InsertNewline binding.
	h, _ = h.Update(tea.KeyMsg{Type: tea.KeyCtrlJ})

	h = typeString(h, "def")

	// Plain enter submits.
	_, cmd := h.Update(tea.KeyMsg{Type: tea.KeyEnter})
	if cmd == nil {
		t.Fatal("expected CommandSubmitted cmd from plain enter, got nil")
	}
	out := cmd()
	cs, ok := out.(CommandSubmitted)
	if !ok {
		t.Fatalf("expected CommandSubmitted, got %T (%v)", out, out)
	}
	if !strings.Contains(cs.Text, "abc") || !strings.Contains(cs.Text, "def") {
		t.Errorf("submitted text missing parts: %q", cs.Text)
	}
	if !strings.Contains(cs.Text, "\n") {
		t.Errorf("submitted text missing newline (multi-line not preserved): %q", cs.Text)
	}
}

// TestHubTabPastePreservesNewlines locks that bracketed-paste delivery of
// '\n' (KeyEnter with Paste=true) inserts the newline into the buffer
// instead of triggering a submit.
func TestHubTabPastePreservesNewlines(t *testing.T) {
	h := NewHubTab()
	h.SetSize(80, 24)
	h.focused = true
	h.input.Focus()

	// Simulate pasting "x\ny" by sending each char as Paste=true.
	pasteX := tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'x'}, Paste: true}
	pasteNL := tea.KeyMsg{Type: tea.KeyEnter, Paste: true}
	pasteY := tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}, Paste: true}

	h, _ = h.Update(pasteX)
	var cmd tea.Cmd
	h, cmd = h.Update(pasteNL)
	if cmd != nil {
		// If a submit fired here, the paste was wrongly interpreted as enter.
		out := cmd()
		if _, ok := out.(CommandSubmitted); ok {
			t.Fatal("paste-flagged enter wrongly triggered CommandSubmitted")
		}
	}
	h, _ = h.Update(pasteY)

	val := h.input.Value()
	if !strings.Contains(val, "x") || !strings.Contains(val, "y") {
		t.Fatalf("paste lost characters: %q", val)
	}
	if !strings.Contains(val, "\n") {
		t.Errorf("paste lost newline: %q", val)
	}
}

// TestHubTabAccepts10kbInput locks that the textarea has no character
// limit (CharLimit=0). Long pastes must round-trip without truncation.
func TestHubTabAccepts10kbInput(t *testing.T) {
	h := NewHubTab()
	h.SetSize(80, 24)
	h.focused = true
	h.input.Focus()

	const size = 10_000
	long := strings.Repeat("a", size)
	h.input.SetValue(long)
	if got := len(h.input.Value()); got != size {
		t.Errorf("long input truncated: got len=%d, want %d", got, size)
	}
}

// TestHubTabPasteWhileUnfocusedAutoFocuses locks F1's behavior contract:
// when bracketed paste content arrives while the input is unfocused, the
// HubTab must auto-focus the input and forward the paste to the textarea
// rather than silently routing it to the viewport. Pre-fix, a multi-rune
// paste delivered as one KeyMsg{Paste:true} matched neither the "/"+"i"
// shortcut keys nor the single-printable-char branch (because the rune
// slice has length > 1) and was dropped into viewport.Update — observable
// to users as "bracketed paste didn't work" even though the bubbletea
// bracketed-paste pipeline itself was fine.
func TestHubTabPasteWhileUnfocusedAutoFocuses(t *testing.T) {
	h := NewHubTab()
	h.SetSize(80, 24)
	if h.focused {
		t.Fatalf("hub tab should start unfocused")
	}

	paste := tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("hello\nworld"), Paste: true}
	h, _ = h.Update(paste)

	if !h.focused {
		t.Errorf("paste arriving while unfocused must auto-focus the input")
	}
	if got := h.input.Value(); got != "hello\nworld" {
		t.Errorf("paste content should land in input buffer, got %q", got)
	}
}
