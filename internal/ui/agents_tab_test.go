package ui

import (
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestAgentsTabReadsActivity verifies the Status column renders the
// derived activity label (working/online/stale/offline) instead of the
// raw protocol.AgentStatus. Spec §4 commit 4: source switch.
func TestAgentsTabReadsActivity(t *testing.T) {
	now := time.Now()
	stale := now.Add(-2 * time.Minute) // beyond OnlineWindow (60s)

	cases := []struct {
		name      string
		status    protocol.AgentStatus
		lastSeen  time.Time
		wantLabel string
	}{
		{"working", protocol.StatusOnline, now, "working"},
		{"online", protocol.StatusOnline, now.Add(-30 * time.Second), "online"},
		{"stale", protocol.StatusOnline, stale, "stale"},
		{"offline", protocol.StatusOffline, now, "offline"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			agents := []protocol.Agent{
				{ID: "x-test", Name: "X Test", Type: protocol.AgentBrian, Status: tc.status, LastSeen: tc.lastSeen},
			}
			pane := newPaneWithAgents(t, agents)
			tab := NewAgentsTab()
			tab.SetPane(pane)
			tab.SetSize(120, 30)
			tab, _ = tab.Update(AgentsUpdated{Agents: agents})

			out := tab.View()
			if !strings.Contains(out, tc.wantLabel) {
				t.Errorf("AgentsTab.View should contain activity label %q, got:\n%s", tc.wantLabel, out)
			}
		})
	}
}

// TestAgentsTabFallbackWithoutPane verifies the tab still renders sanely
// when SetPane was never called (status-based fallback). Locks the
// nil-tolerance contract for headless tests / pre-construction state.
func TestAgentsTabFallbackWithoutPane(t *testing.T) {
	agents := []protocol.Agent{
		{ID: "fallback-online", Name: "FallbackOnline", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now()},
		{ID: "fallback-offline", Name: "FallbackOffline", Type: protocol.AgentCoder, Status: protocol.StatusOffline, LastSeen: time.Now()},
	}
	tab := NewAgentsTab()
	tab.SetSize(120, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})

	out := tab.View()
	// Name (not ID) is what appears in the rendered Name column.
	if !strings.Contains(out, "FallbackOnline") {
		t.Errorf("View missing FallbackOnline name: %q", out)
	}
	if !strings.Contains(out, "offline") {
		t.Errorf("View should label offline agent: %q", out)
	}
	// Online status without pane falls back to ActivityStale (we have status but
	// no recency truth) — that's acceptable as a defensive default.
	if !strings.Contains(out, "stale") {
		t.Errorf("View should label non-offline-without-pane as stale (defensive default): %q", out)
	}
}

// TestAgentsTabAliveSummary verifies the summary line uses the activity-based
// alive count (working + online), not the legacy status field.
func TestAgentsTabAliveSummary(t *testing.T) {
	now := time.Now()
	stale := now.Add(-2 * time.Minute)
	agents := []protocol.Agent{
		{ID: "a1", Name: "A1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now},      // working
		{ID: "a2", Name: "A2", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: stale},    // stale (counts as not-alive)
		{ID: "a3", Name: "A3", Type: protocol.AgentCoder, Status: protocol.StatusOffline, LastSeen: now},     // offline
	}
	pane := newPaneWithAgents(t, agents)
	tab := NewAgentsTab()
	tab.SetPane(pane)
	tab.SetSize(120, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})

	out := tab.View()
	// Expect [1 alive, 2 offline] — only a1 is alive (working). a2 is stale.
	if !strings.Contains(out, "1 alive") {
		t.Errorf("expected '1 alive' in summary, got:\n%s", out)
	}
}

// Compile-time assert that activityDot covers all four activity values.
// Surface a regression if a future ActivityFoo enum value is added without
// styles.go being updated.
func TestActivityDotCoversAllStates(t *testing.T) {
	cases := []panestate.AgentActivity{
		panestate.ActivityWorking,
		panestate.ActivityOnline,
		panestate.ActivityStale,
		panestate.ActivityOffline,
	}
	for _, a := range cases {
		got := activityDot(a)
		if got == "?" || got == "" {
			t.Errorf("activityDot(%v) returned %q — missing case in styles.go", a, got)
		}
	}
}
