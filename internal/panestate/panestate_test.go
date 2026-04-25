package panestate

import (
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestComputeActivity locks Phase-E-equivalence parametrically: every case
// passes paneActive == time.Time{} (no pane source wired), and the activity
// tier must match the heartbeat-only Phase E rule. F-core-a's case-i ratchet
// is falsifiable here — any leak from the new pane-tier branches into the
// paneActive=zero path breaks one of these existing assertions.
func TestComputeActivity(t *testing.T) {
	now := time.Now()
	var paneZero time.Time // explicit zero — Phase-E-equivalence path
	cases := []struct {
		name     string
		status   protocol.AgentStatus
		lastSeen time.Time
		want     AgentActivity
	}{
		{"working: 0s recency", protocol.StatusOnline, now, ActivityWorking},
		{"working: 4s recency", protocol.StatusOnline, now.Add(-4 * time.Second), ActivityWorking},
		{"online: 5s recency (boundary)", protocol.StatusOnline, now.Add(-5 * time.Second), ActivityOnline},
		{"online: 30s recency", protocol.StatusOnline, now.Add(-30 * time.Second), ActivityOnline},
		{"stale: 60s recency (boundary)", protocol.StatusOnline, now.Add(-60 * time.Second), ActivityStale},
		{"stale: 1h recency", protocol.StatusOnline, now.Add(-1 * time.Hour), ActivityStale},
		{"offline overrides recency=0s", protocol.StatusOffline, now, ActivityOffline},
		{"offline overrides recency=1h", protocol.StatusOffline, now.Add(-1 * time.Hour), ActivityOffline},
		{"legacy StatusWorking treated by recency only", protocol.StatusWorking, now.Add(-30 * time.Second), ActivityOnline},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := ComputeActivity(tc.status, tc.lastSeen, paneZero, now)
			if got != tc.want {
				t.Errorf("ComputeActivity(%v, recency=%v, paneZero) = %v, want %v", tc.status, now.Sub(tc.lastSeen), got, tc.want)
			}
		})
	}
}

// TestComputeActivityWithPane locks the OR-combination semantics F-core-b
// will activate. Independent-threshold per Push 4: heartbeat and pane tiers
// each compared against their own constants, OR'd at each tier boundary.
func TestComputeActivityWithPane(t *testing.T) {
	now := time.Now()
	cases := []struct {
		name       string
		status     protocol.AgentStatus
		lastSeen   time.Time
		paneActive time.Time
		want       AgentActivity
	}{
		// F1: paneActive zero, heartbeat fresh — Phase-E identity (sanity).
		{"F1: paneZero + fresh heartbeat → working", protocol.StatusOnline, now.Add(-2 * time.Second), time.Time{}, ActivityWorking},

		// F2: heartbeat stale beyond OnlineWindow, pane fresh — pane-tier rescues.
		// This is the msg 2646 false-stale failure mode under fix.
		{"F2: stale heartbeat + fresh pane → working (pane wins)", protocol.StatusOnline, now.Add(-2 * time.Minute), now.Add(-2 * time.Second), ActivityWorking},

		// F3: heartbeat fresh, pane stale — heartbeat-tier preserves cheap-ping behavior.
		{"F3: fresh heartbeat + stale pane → working (heartbeat wins)", protocol.StatusOnline, now.Add(-2 * time.Second), now.Add(-2 * time.Minute), ActivityWorking},

		// F4: status offline always wins regardless of either signal.
		{"F4: offline overrides both fresh signals", protocol.StatusOffline, now.Add(-2 * time.Second), now.Add(-2 * time.Second), ActivityOffline},

		// F5: both signals at exact boundary — strict-inequality semantic preserved
		// from Phase E (recency == Window is NOT in the lower tier).
		{"F5: heartbeat=5s + pane=5s (boundary equality) → online", protocol.StatusOnline, now.Add(-5 * time.Second), now.Add(-5 * time.Second), ActivityOnline},

		// F6: heartbeat past stale threshold but pane fresh — pane-tier rescues
		// from heartbeat-stale, demonstrating independent-threshold OR.
		{"F6: heartbeat past 60s + pane=2s → working (pane-tier OR-fires)", protocol.StatusOnline, now.Add(-61 * time.Second), now.Add(-2 * time.Second), ActivityWorking},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := ComputeActivity(tc.status, tc.lastSeen, tc.paneActive, now)
			if got != tc.want {
				t.Errorf("ComputeActivity(%v, hb=%v, pane=%v) = %v, want %v",
					tc.status, now.Sub(tc.lastSeen), now.Sub(tc.paneActive), got, tc.want)
			}
		})
	}
}

func TestManagerRefresh(t *testing.T) {
	fake := &fakeSource{
		agents: []protocol.Agent{
			{ID: "a1", Name: "Agent 1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now()},
			{ID: "a2", Name: "Agent 2", Type: protocol.AgentCoder, Status: protocol.StatusOffline, LastSeen: time.Now().Add(-1 * time.Hour)},
		},
	}
	m := NewManager(fake)
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	snap := m.Snapshot()
	if len(snap) != 2 {
		t.Fatalf("len(snapshot) = %d, want 2", len(snap))
	}
	if snap[0].Activity != ActivityWorking {
		t.Errorf("snap[0].Activity = %v, want ActivityWorking", snap[0].Activity)
	}
	if snap[1].Activity != ActivityOffline {
		t.Errorf("snap[1].Activity = %v, want ActivityOffline", snap[1].Activity)
	}
}

func TestManagerSnapshotFreshness(t *testing.T) {
	// After Refresh runs against new DB state, Snapshot reflects the new state.
	// Locks against tabs holding stale []AgentSnapshot copies instead of *Manager references.
	fake := &fakeSource{
		agents: []protocol.Agent{
			{ID: "a1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now().Add(-1 * time.Hour)},
		},
	}
	m := NewManager(fake)
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].Activity; got != ActivityStale {
		t.Errorf("first snapshot got %v, want ActivityStale", got)
	}

	fake.agents[0].LastSeen = time.Now()
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].Activity; got != ActivityWorking {
		t.Errorf("second snapshot got %v, want ActivityWorking after Refresh", got)
	}
}

func TestManagerSnapshotIsCopy(t *testing.T) {
	// Snapshot must return a copy; mutating it must not affect Manager state.
	fake := &fakeSource{
		agents: []protocol.Agent{
			{ID: "a1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now()},
		},
	}
	m := NewManager(fake)
	m.Refresh()
	s1 := m.Snapshot()
	s1[0].ID = "mutated"
	s2 := m.Snapshot()
	if s2[0].ID != "a1" {
		t.Errorf("Manager state was mutated by external snapshot mutation: got %q", s2[0].ID)
	}
}

func TestThresholds(t *testing.T) {
	// Lock the spec'd window values so tuning changes are intentional.
	// F-core-a Push 4 lock: heartbeat- and pane-tier constants are independent.
	// Initial pane values match heartbeat for Phase-E-equivalence at F-core-a;
	// tunable independently once F-core-b activates pane sourcing.
	if HeartbeatWorkingWindow != 5*time.Second {
		t.Errorf("HeartbeatWorkingWindow = %v, want 5s", HeartbeatWorkingWindow)
	}
	if HeartbeatOnlineWindow != 60*time.Second {
		t.Errorf("HeartbeatOnlineWindow = %v, want 60s", HeartbeatOnlineWindow)
	}
	if PaneWorkingWindow != 5*time.Second {
		t.Errorf("PaneWorkingWindow = %v, want 5s (initial = heartbeat per Phase-E-equivalence)", PaneWorkingWindow)
	}
	if PaneOnlineWindow != 60*time.Second {
		t.Errorf("PaneOnlineWindow = %v, want 60s (initial = heartbeat per Phase-E-equivalence)", PaneOnlineWindow)
	}
	if StaleAgentWindow != 7*24*time.Hour {
		t.Errorf("StaleAgentWindow = %v, want 7d", StaleAgentWindow)
	}
}

type fakeSource struct {
	agents []protocol.Agent
}

func (f *fakeSource) ListAgents(string) ([]protocol.Agent, error) {
	return f.agents, nil
}
