package panestate

import (
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestComputeActivity(t *testing.T) {
	now := time.Now()
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
			got := ComputeActivity(tc.status, tc.lastSeen, now)
			if got != tc.want {
				t.Errorf("ComputeActivity(%v, recency=%v) = %v, want %v", tc.status, now.Sub(tc.lastSeen), got, tc.want)
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
	if WorkingWindow != 5*time.Second {
		t.Errorf("WorkingWindow = %v, want 5s (spec §2)", WorkingWindow)
	}
	if OnlineWindow != 60*time.Second {
		t.Errorf("OnlineWindow = %v, want 60s (spec §2)", OnlineWindow)
	}
	if StaleAgentWindow != 7*24*time.Hour {
		t.Errorf("StaleAgentWindow = %v, want 7d (spec §3)", StaleAgentWindow)
	}
}

type fakeSource struct {
	agents []protocol.Agent
}

func (f *fakeSource) ListAgents(string) ([]protocol.Agent, error) {
	return f.agents, nil
}
