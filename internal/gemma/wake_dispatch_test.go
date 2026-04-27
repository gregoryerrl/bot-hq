package gemma

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// newWakeTestGemma returns an in-memory Gemma wired to a fresh DB. We bypass
// New()/Start() so the test stays free of Ollama + the goroutines we don't
// want firing alongside dispatchWakes. Only the db field is needed.
func newWakeTestGemma(t *testing.T) (*Gemma, *hub.DB) {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	g := &Gemma{db: db, stopCh: make(chan struct{})}
	return g, db
}

// TestDispatchWakesFiresAndFlipsState locks the slice-3 C1 (#7) success path:
// a pending wake whose fire_at is in the past gets a hub_send (from='emma',
// type='command') and the row transitions pending → fired with fired_at set.
func TestDispatchWakesFiresAndFlipsState(t *testing.T) {
	g, db := newWakeTestGemma(t)

	id, err := db.InsertWakeSchedule("brian", "rain", "wake-up: re-test 3", time.Now().Add(-1*time.Second))
	if err != nil {
		t.Fatal(err)
	}

	g.dispatchWakes()

	// Row state: pending → fired with fired_at set.
	row, err := db.GetWakeSchedule(id)
	if err != nil {
		t.Fatal(err)
	}
	if row.FireStatus != hub.WakeStatusFired {
		t.Errorf("status: got %q want fired", row.FireStatus)
	}
	if row.FiredAt.IsZero() {
		t.Error("fired_at not set after dispatch")
	}

	// Hub message landed for the target with from=emma, type=command.
	msgs, err := db.ReadMessages("brian", 0, 50)
	if err != nil {
		t.Fatal(err)
	}
	var found *protocol.Message
	for i := range msgs {
		if msgs[i].FromAgent == agentID && msgs[i].Content == "wake-up: re-test 3" {
			found = &msgs[i]
		}
	}
	if found == nil {
		t.Fatalf("dispatched message not found in hub log; got %d msgs for brian", len(msgs))
	}
	if found.Type != protocol.MsgCommand {
		t.Errorf("dispatched message type: got %q want command", found.Type)
	}
	if found.ToAgent != "brian" {
		t.Errorf("dispatched message to: got %q want brian", found.ToAgent)
	}
}

// TestDispatchWakesSkipsFutureRows locks that fire_at-in-the-future rows are
// not dispatched on the current tick — Emma's clock-driven gating, not just
// "fire everything pending."
func TestDispatchWakesSkipsFutureRows(t *testing.T) {
	g, db := newWakeTestGemma(t)
	futureID, _ := db.InsertWakeSchedule("brian", "rain", "later", time.Now().Add(time.Hour))

	g.dispatchWakes()

	row, _ := db.GetWakeSchedule(futureID)
	if row.FireStatus != hub.WakeStatusPending {
		t.Errorf("future row status: got %q want pending", row.FireStatus)
	}
}

// TestDispatchWakesSkipsCancelled locks the cancel-then-dispatch race: a row
// already cancelled before its fire_at must not produce a hub_send.
func TestDispatchWakesSkipsCancelled(t *testing.T) {
	g, db := newWakeTestGemma(t)
	id, _ := db.InsertWakeSchedule("brian", "rain", "p", time.Now().Add(-1*time.Second))
	if _, err := db.CancelWake(id); err != nil {
		t.Fatal(err)
	}

	g.dispatchWakes()

	// No hub message produced for brian.
	msgs, _ := db.ReadMessages("brian", 0, 50)
	for _, m := range msgs {
		if m.FromAgent == agentID {
			t.Errorf("cancelled wake produced hub_send: %+v", m)
		}
	}
	row, _ := db.GetWakeSchedule(id)
	if row.FireStatus != hub.WakeStatusCancelled {
		t.Errorf("cancelled row drifted: got %q want cancelled", row.FireStatus)
	}
}

// TestDispatchWakesIdempotentWithinTick locks that the same row only fires
// once even if dispatchWakes is called repeatedly — ListPendingWakes excludes
// terminal-state rows so the second call sees nothing.
func TestDispatchWakesIdempotentWithinTick(t *testing.T) {
	g, db := newWakeTestGemma(t)
	db.InsertWakeSchedule("brian", "rain", "once", time.Now().Add(-1*time.Second))

	g.dispatchWakes()
	g.dispatchWakes()

	msgs, _ := db.ReadMessages("brian", 0, 50)
	count := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && m.Content == "once" {
			count++
		}
	}
	if count != 1 {
		t.Errorf("expected exactly 1 dispatch, got %d", count)
	}
}
