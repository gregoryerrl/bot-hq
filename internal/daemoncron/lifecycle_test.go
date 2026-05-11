package daemoncron

import (
	"strings"
	"testing"
)

func TestBuildRosterPruneContent_Format(t *testing.T) {
	got := BuildRosterPruneContent(2, []string{"agent-a", "agent-b"})
	if !strings.HasPrefix(got, "[ROSTER-PRUNE] Removed 2 stale-offline") {
		t.Errorf("missing [ROSTER-PRUNE] prefix + count; got %q", got)
	}
	if !strings.Contains(got, "agent-a, agent-b") {
		t.Errorf("agent ids not embedded; got %q", got)
	}
}

func TestEmitRosterPrune_NoOpEmptyList(t *testing.T) {
	db := setupTestDB(t)
	EmitRosterPrune(db, nil)
	EmitRosterPrune(db, []string{})
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if strings.HasPrefix(m.Content, "[ROSTER-PRUNE]") {
			t.Errorf("empty id list should not emit; got %q", m.Content)
		}
	}
}

func TestEmitRosterPrune_FiresWithIDs(t *testing.T) {
	db := setupTestDB(t)
	EmitRosterPrune(db, []string{"x", "y", "z"})
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if strings.HasPrefix(m.Content, "[ROSTER-PRUNE] Removed 3") {
			if m.ToAgent != "rain" {
				t.Errorf("roster-prune should target rain; got %q", m.ToAgent)
			}
			if m.FromAgent != lifecycleAgentID {
				t.Errorf("roster-prune from_agent = %q, want %q (Z-9d system convention)", m.FromAgent, lifecycleAgentID)
			}
			return
		}
	}
	t.Error("expected roster-prune emit not found")
}
