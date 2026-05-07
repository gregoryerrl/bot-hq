package daemoncron

import (
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// registerTestAgent inserts an agent (last_seen = real now). Tests
// drive aging via virtualNow ahead of register-time per Cron's
// SetNowFunc — emulates gemma_test.go pattern (virtualNow := time.
// Now().Add(staleThreshold + time.Minute)).
func registerTestAgent(t *testing.T, c *Cron, id string, currentTask string) {
	t.Helper()
	if err := c.db.RegisterAgent(protocol.Agent{
		ID:     id,
		Name:   id,
		Type:   protocol.AgentCoder,
		Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatalf("register %s: %v", id, err)
	}
	if currentTask != "" {
		if err := c.db.SetAgentCurrentTask(id, currentTask); err != nil {
			t.Fatalf("set current task %s: %v", id, err)
		}
	}
}

func countStaleCoderEmits(t *testing.T, c *Cron) int {
	t.Helper()
	msgs, err := c.db.GetRecentMessages(100)
	if err != nil {
		t.Fatalf("recent: %v", err)
	}
	count := 0
	for _, m := range msgs {
		if m.FromAgent == staleAgentID && strings.HasPrefix(m.Content, "[STALE-CODER]") {
			count++
		}
	}
	return count
}

func TestRunStaleCoderSurface_AgentBelowThreshold_NoEmit(t *testing.T) {
	ResetStaleStateForTest()
	db := setupTestDB(t)
	c := New(db)
	registerTestAgent(t, c, "fresh-coder", "")
	// virtualNow within threshold (5min) — LastSeen-aged 5min < 30min.
	c.SetNowFunc(func() time.Time { return time.Now().Add(5 * time.Minute) })

	runStaleCoderSurface(c)

	if got := countStaleCoderEmits(t, c); got != 0 {
		t.Errorf("expected 0 stale emits below threshold; got %d", got)
	}
}

func TestRunStaleCoderSurface_AgentAboveThreshold_Emits(t *testing.T) {
	ResetStaleStateForTest()
	db := setupTestDB(t)
	c := New(db)
	registerTestAgent(t, c, "stale-coder", "")
	// virtualNow past staleThreshold — LastSeen-aged 60min > 30min.
	c.SetNowFunc(func() time.Time { return time.Now().Add(60 * time.Minute) })

	runStaleCoderSurface(c)

	if got := countStaleCoderEmits(t, c); got != 1 {
		t.Errorf("expected 1 stale emit above threshold; got %d", got)
	}
}

func TestRunStaleCoderSurface_IntentionalIdleShortCircuit(t *testing.T) {
	ResetStaleStateForTest()
	db := setupTestDB(t)
	c := New(db)
	registerTestAgent(t, c, "idle-coder", "smoking S-1a batch")
	c.SetNowFunc(func() time.Time { return time.Now().Add(60 * time.Minute) })

	runStaleCoderSurface(c)

	if got := countStaleCoderEmits(t, c); got != 0 {
		t.Errorf("expected 0 stale emits for intentional-idle (current_task non-empty); got %d", got)
	}
}

func TestRunStaleCoderSurface_LastSeenDedupe(t *testing.T) {
	ResetStaleStateForTest()
	db := setupTestDB(t)
	c := New(db)
	registerTestAgent(t, c, "stale-coder-3", "")
	c.SetNowFunc(func() time.Time { return time.Now().Add(60 * time.Minute) })

	// First fire.
	runStaleCoderSurface(c)
	first := countStaleCoderEmits(t, c)
	if first != 1 {
		t.Fatalf("expected 1 emit on first run; got %d", first)
	}

	// Second fire WITHOUT advancing LastSeen → same incident → dedupe.
	runStaleCoderSurface(c)
	if got := countStaleCoderEmits(t, c); got != 1 {
		t.Errorf("LastSeen-equal dedupe should suppress 2nd emit; got %d", got)
	}
}

func TestRunStaleCoderSurface_HaltStateSuppression(t *testing.T) {
	ResetStaleStateForTest()
	db := setupTestDB(t)
	c := New(db)
	registerTestAgent(t, c, "stale-during-halt", "")
	c.SetNowFunc(func() time.Time { return time.Now().Add(60 * time.Minute) })

	// Set halt state.
	if err := db.SetHaltActive("test-halt", "halt-test", "test"); err != nil {
		t.Fatalf("SetHaltActive: %v", err)
	}

	runStaleCoderSurface(c)

	if got := countStaleCoderEmits(t, c); got != 0 {
		t.Errorf("halt-state should suppress stale emits; got %d", got)
	}
}

func TestRunStaleCoderSurface_UserHaltDirectiveSuppression(t *testing.T) {
	ResetStaleStateForTest()
	db := setupTestDB(t)
	c := New(db)
	registerTestAgent(t, c, "stale-during-user-halt", "")
	c.SetNowFunc(func() time.Time { return time.Now().Add(60 * time.Minute) })

	// User HALT directive.
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: "user",
		Type:      protocol.MsgCommand,
		Content:   "HALT",
	}); err != nil {
		t.Fatalf("insert user halt: %v", err)
	}

	runStaleCoderSurface(c)

	if got := countStaleCoderEmits(t, c); got != 0 {
		t.Errorf("user HALT directive should suppress stale emits; got %d", got)
	}
}

func TestRunStaleCoderSurface_RecipientCorrect(t *testing.T) {
	ResetStaleStateForTest()
	db := setupTestDB(t)
	c := New(db)
	registerTestAgent(t, c, "recipient-test", "")
	c.SetNowFunc(func() time.Time { return time.Now().Add(60 * time.Minute) })

	runStaleCoderSurface(c)

	msgs, _ := db.GetRecentMessages(50)
	for _, m := range msgs {
		if m.FromAgent == staleAgentID && strings.HasPrefix(m.Content, "[STALE-CODER]") {
			if m.ToAgent != "rain" {
				t.Errorf("stale-coder PM should target rain; got %q", m.ToAgent)
			}
			return
		}
	}
}

func TestRunStaleCoderSurface_SelfSkipped(t *testing.T) {
	ResetStaleStateForTest()
	db := setupTestDB(t)
	c := New(db)
	registerTestAgent(t, c, "emma", "")
	c.SetNowFunc(func() time.Time { return time.Now().Add(60 * time.Minute) })

	runStaleCoderSurface(c)

	if got := countStaleCoderEmits(t, c); got != 0 {
		t.Errorf("emma should be self-skipped; got %d emits", got)
	}
}
