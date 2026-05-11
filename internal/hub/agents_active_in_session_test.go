package hub

import (
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestAgentsActiveInSession verifies Z-5c per-session presence
// derivation. agents.session_id is last-write-wins (lies under
// concurrent sessions), so the truth source is messages.from_agent.
func TestAgentsActiveInSession(t *testing.T) {
	db := setupTestDB(t)

	// Register 3 agents.
	for _, id := range []string{"brian", "rain", "emma"} {
		if err := db.RegisterAgent(protocol.Agent{
			ID:     id,
			Name:   id,
			Type:   protocol.AgentBrian,
			Status: protocol.StatusOnline,
		}); err != nil {
			t.Fatal(err)
		}
	}

	// Session A: brian + rain post. emma posts on main hub.
	insert := func(from, sessionID string) {
		if _, err := db.InsertMessage(protocol.Message{
			FromAgent: from,
			Type:      "update",
			Content:   "hello",
			SessionID: sessionID,
		}); err != nil {
			t.Fatal(err)
		}
	}
	insert("brian", "session-A")
	insert("rain", "session-A")
	insert("emma", "")
	insert("brian", "session-B") // active in B too

	gotIDs := func(agents []protocol.Agent) map[string]bool {
		s := make(map[string]bool, len(agents))
		for _, a := range agents {
			s[a.ID] = true
		}
		return s
	}

	t.Run("session-A returns brian + rain", func(t *testing.T) {
		agents, err := db.AgentsActiveInSession("session-A", time.Hour)
		if err != nil {
			t.Fatal(err)
		}
		ids := gotIDs(agents)
		if !ids["brian"] || !ids["rain"] {
			t.Errorf("want brian + rain, got %v", ids)
		}
		if ids["emma"] {
			t.Errorf("emma should not be in session-A, got %v", ids)
		}
	})

	t.Run("session-B returns only brian", func(t *testing.T) {
		agents, err := db.AgentsActiveInSession("session-B", time.Hour)
		if err != nil {
			t.Fatal(err)
		}
		ids := gotIDs(agents)
		if !ids["brian"] || len(ids) != 1 {
			t.Errorf("want only brian, got %v", ids)
		}
	})

	t.Run("main hub returns emma", func(t *testing.T) {
		agents, err := db.AgentsActiveInSession("", time.Hour)
		if err != nil {
			t.Fatal(err)
		}
		ids := gotIDs(agents)
		if !ids["emma"] || len(ids) != 1 {
			t.Errorf("want only emma on main hub, got %v", ids)
		}
	})

	t.Run("recentWindow excludes old messages", func(t *testing.T) {
		// Backdate brian's session-A message to 2h ago.
		if _, err := db.conn.Exec(
			`UPDATE messages SET created = ? WHERE from_agent = ? AND session_id = ?`,
			time.Now().Add(-2*time.Hour).UnixMilli(), "brian", "session-A",
		); err != nil {
			t.Fatal(err)
		}
		agents, err := db.AgentsActiveInSession("session-A", time.Hour)
		if err != nil {
			t.Fatal(err)
		}
		ids := gotIDs(agents)
		if ids["brian"] {
			t.Errorf("brian's 2h-old message should be excluded by 1h window, got %v", ids)
		}
		if !ids["rain"] {
			t.Errorf("rain should still be present (recent), got %v", ids)
		}
	})

	t.Run("recentWindow=0 returns all-time", func(t *testing.T) {
		// brian's session-A message is still backdated 2h. With
		// recentWindow=0, it should be included again.
		agents, err := db.AgentsActiveInSession("session-A", 0)
		if err != nil {
			t.Fatal(err)
		}
		ids := gotIDs(agents)
		if !ids["brian"] || !ids["rain"] {
			t.Errorf("recentWindow=0 should include brian + rain, got %v", ids)
		}
	})
}
