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
	stale := now.Add(-2 * time.Minute) // beyond HeartbeatOnlineWindow (60s)

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
			tab := NewAgentsTab(noPaneCapture)
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
	tab := NewAgentsTab(noPaneCapture)
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

// TestAgentsTabSummaryBuckets verifies the summary line surfaces all three
// activity-derived buckets — alive (working + online), stale, and offline.
// Stale agents must surface as their own bucket, not get absorbed into offline.
func TestAgentsTabSummaryBuckets(t *testing.T) {
	now := time.Now()
	stale := now.Add(-2 * time.Minute)
	agents := []protocol.Agent{
		{ID: "a1", Name: "A1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now},   // working
		{ID: "a2", Name: "A2", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: stale}, // stale
		{ID: "a3", Name: "A3", Type: protocol.AgentCoder, Status: protocol.StatusOffline, LastSeen: now},  // offline
	}
	pane := newPaneWithAgents(t, agents)
	tab := NewAgentsTab(noPaneCapture)
	tab.SetPane(pane)
	tab.SetSize(120, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})

	out := tab.View()
	// Expect [1 alive, 1 stale, 1 offline] — a1 alive (working), a2 stale, a3 offline.
	for _, want := range []string{"1 alive", "1 stale", "1 offline"} {
		if !strings.Contains(out, want) {
			t.Errorf("expected %q in summary, got:\n%s", want, out)
		}
	}
}

// TestAgentsTabStaleGenSuffix locks Phase G v1 #20 agents-tab UX:
// stale-gen agents stay visible in the agents tab but with a "(stale-gen)"
// suffix appended to their name so the user can see and prune them.
func TestAgentsTabStaleGenSuffix(t *testing.T) {
	now := time.Now()
	agents := []protocol.Agent{
		{ID: "current", Name: "Current", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now, RebuildGen: 2},
		{ID: "stalegen", Name: "StaleGen", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now, RebuildGen: 1},
	}
	mgr := panestate.NewManager(&fakeSource{agents: agents, rebuildGen: 2}, noPaneCapture)
	if err := mgr.Refresh(); err != nil {
		t.Fatal(err)
	}
	tab := NewAgentsTab(noPaneCapture)
	tab.SetPane(mgr)
	tab.SetSize(120, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})

	out := tab.View()
	if !strings.Contains(out, "Current") {
		t.Errorf("agents tab should render current-gen agent name, got:\n%s", out)
	}
	if !strings.Contains(out, "StaleGen") {
		t.Errorf("agents tab should still render stale-gen agent name, got:\n%s", out)
	}
	if !strings.Contains(out, "(stale-gen)") {
		t.Errorf("stale-gen agent should carry (stale-gen) suffix, got:\n%s", out)
	}
	// Current-gen agent should not get the suffix.
	currentLine := ""
	for _, line := range strings.Split(out, "\n") {
		if strings.Contains(line, "Current") && !strings.Contains(line, "StaleGen") {
			currentLine = line
			break
		}
	}
	if currentLine != "" && strings.Contains(currentLine, "(stale-gen)") {
		t.Errorf("current-gen agent line should NOT carry (stale-gen) suffix: %q", currentLine)
	}
}

// TestAgentsTabRendersContextPctColumn locks slice 5 C1 (H-32) per-row
// context-% column. Sources from panestate snapshot's ContextPct (a
// rename of the slice-4 UsagePct). Three-row scenario covers the three
// surface states: known squeeze, known low, and unknown (-1) → " --%".
func TestAgentsTabRendersContextPctColumn(t *testing.T) {
	now := time.Now()
	agents := []protocol.Agent{
		{ID: "hot", Name: "Hot", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now,
			Meta: `{"tmux_target":"hot:0.0"}`},
		{ID: "cool", Name: "Cool", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now,
			Meta: `{"tmux_target":"cool:0.0"}`},
		{ID: "ghost", Name: "Ghost", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now},
	}
	mgr := panestate.NewManager(&fakeSource{agents: agents}, func(target string, _ int) (string, error) {
		switch target {
		case "hot:0.0":
			return "5% until auto-compact", nil // → ContextPct = 95
		case "cool:0.0":
			return "70% until auto-compact", nil // → ContextPct = 30
		}
		return "", nil
	})
	// Two ticks: first seeds the cache (LastPaneActivity=zero); second
	// promotes ContextPct so the agents-tab snapshot reflects it.
	if err := mgr.Refresh(); err != nil {
		t.Fatal(err)
	}
	// Refresh again to lock the seeded cache values into the snapshot
	// (first-tick contract is "no prior frame to compare", but ContextPct
	// is published on the first tick already).
	if err := mgr.Refresh(); err != nil {
		t.Fatal(err)
	}

	tab := NewAgentsTab(noPaneCapture)
	tab.SetPane(mgr)
	tab.SetSize(160, 30)
	tab, _ = tab.Update(AgentsUpdated{Agents: agents})
	out := tab.View()

	if !strings.Contains(out, "95%") {
		t.Errorf("hot agent (95%% context) must surface in column; got:\n%s", out)
	}
	if !strings.Contains(out, "30%") {
		t.Errorf("cool agent (30%% context) must surface in column; got:\n%s", out)
	}
	if !strings.Contains(out, " --%") {
		t.Errorf("ghost agent (no tmux_target → -1) must render as ' --%%' cell; got:\n%s", out)
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
