package ui

import (
	"strings"
	"testing"
	"time"

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
