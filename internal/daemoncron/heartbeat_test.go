package daemoncron

import (
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestHeartbeatContentTemplate_Format(t *testing.T) {
	got := heartbeatContentTemplate(15788)
	if !strings.HasPrefix(got, "[HEARTBEAT-LEDGER]") {
		t.Errorf("content should start with [HEARTBEAT-LEDGER]; got %q", got)
	}
	if !strings.Contains(got, "every 25 msgs") {
		t.Errorf("content should cite the 25-msg cadence; got %q", got)
	}
	if !strings.Contains(got, "latest-msg-id=15788") {
		t.Errorf("content should embed the passed msg-id; got %q", got)
	}
	if !strings.Contains(got, "R20 AgentState write opportunity") {
		t.Errorf("content should reference R20 AgentState; got %q", got)
	}
}

func TestRunHeartbeatLedgerSurface_NoMessages(t *testing.T) {
	ResetHeartbeatStateForTest()
	db := setupTestDB(t)
	c := New(db)
	// Empty DB — surface should silently no-op (no msgs to anchor).
	runHeartbeatLedgerSurface(c)
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if m.FromAgent == heartbeatAgentID {
			t.Errorf("empty DB should produce no heartbeat emit; got msg %d", m.ID)
		}
	}
}

func TestRunHeartbeatLedgerSurface_BelowThresholdNoEmit(t *testing.T) {
	ResetHeartbeatStateForTest()
	db := setupTestDB(t)
	c := New(db)
	// Insert 10 msgs (below 25 threshold) — no heartbeat emit.
	for i := 0; i < 10; i++ {
		_, _ = db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgUpdate,
			Content:   "test",
		})
	}
	runHeartbeatLedgerSurface(c)
	msgs, _ := db.GetRecentMessages(50)
	heartbeatCount := 0
	for _, m := range msgs {
		if m.FromAgent == heartbeatAgentID && strings.HasPrefix(m.Content, "[HEARTBEAT-LEDGER]") {
			heartbeatCount++
		}
	}
	if heartbeatCount != 0 {
		t.Errorf("expected 0 heartbeat emits below threshold; got %d", heartbeatCount)
	}
}

func TestRunHeartbeatLedgerSurface_ThresholdCrossEmits(t *testing.T) {
	ResetHeartbeatStateForTest()
	db := setupTestDB(t)
	c := New(db)
	// Insert 26 msgs — crosses the 25-msg threshold.
	for i := 0; i < 26; i++ {
		_, _ = db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgUpdate,
			Content:   "test",
		})
	}
	runHeartbeatLedgerSurface(c)
	msgs, _ := db.GetRecentMessages(50)
	heartbeatCount := 0
	for _, m := range msgs {
		if m.FromAgent == heartbeatAgentID && strings.HasPrefix(m.Content, "[HEARTBEAT-LEDGER]") {
			heartbeatCount++
		}
	}
	// Z-5h / Z-8b: single broadcast emit (was per-recipient PM × 2).
	if heartbeatCount != 1 {
		t.Errorf("expected 1 heartbeat broadcast at threshold cross; got %d", heartbeatCount)
	}
}

func TestRunHeartbeatLedgerSurface_BroadcastNotPM(t *testing.T) {
	// Z-5h / Z-8b: emits go out as broadcast (ToAgent="") so brian +
	// rain both see them via their pollLoops. Recipients recognize the
	// "[HEARTBEAT-LEDGER]" content prefix; from_agent="system".
	ResetHeartbeatStateForTest()
	db := setupTestDB(t)
	c := New(db)
	for i := 0; i < 26; i++ {
		_, _ = db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgUpdate,
			Content:   "test",
		})
	}
	runHeartbeatLedgerSurface(c)
	msgs, _ := db.GetRecentMessages(50)
	for _, m := range msgs {
		if m.FromAgent == heartbeatAgentID && strings.HasPrefix(m.Content, "[HEARTBEAT-LEDGER]") {
			if m.ToAgent != "" {
				t.Errorf("heartbeat emit ToAgent=%q, want '' (broadcast)", m.ToAgent)
			}
			if m.FromAgent != "system" {
				t.Errorf("heartbeat from_agent=%q, want 'system'", m.FromAgent)
			}
		}
	}
}

func TestRunHeartbeatLedgerSurface_DedupesWithinThreshold(t *testing.T) {
	ResetHeartbeatStateForTest()
	db := setupTestDB(t)
	c := New(db)
	// Insert 26 msgs; fire once.
	for i := 0; i < 26; i++ {
		_, _ = db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgUpdate,
			Content:   "test",
		})
	}
	runHeartbeatLedgerSurface(c)
	// Insert 5 more (below next-25-threshold from this lastFiredID).
	for i := 0; i < 5; i++ {
		_, _ = db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgUpdate,
			Content:   "more",
		})
	}
	runHeartbeatLedgerSurface(c)
	// Z-5h / Z-8b: single broadcast emit per threshold cross (was 2 PM emits).
	msgs, _ := db.GetRecentMessages(100)
	heartbeatCount := 0
	for _, m := range msgs {
		if m.FromAgent == heartbeatAgentID && strings.HasPrefix(m.Content, "[HEARTBEAT-LEDGER]") {
			heartbeatCount++
		}
	}
	if heartbeatCount != 1 {
		t.Errorf("expected dedupe to keep heartbeat count at 1 within threshold window; got %d", heartbeatCount)
	}
}
