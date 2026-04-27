package gemma

import (
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// countDeliveryGapMsgs returns how many [DELIVERY-GAP] alerts Emma has
// emitted to the hub. Used by both positive and negative test cases.
func countDeliveryGapMsgs(t *testing.T, db *hub.DB) int {
	t.Helper()
	msgs, err := db.GetRecentMessages(200)
	if err != nil {
		t.Fatal(err)
	}
	n := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && strings.Contains(m.Content, "[DELIVERY-GAP]") {
			n++
		}
	}
	return n
}

// TestAuditDeliveryGapFiresWhenPendingExceedsAge locks the v1 trigger:
// a message_queue row whose `created` is older than deliveryGapAge
// ago AND status='pending' produces a [DELIVERY-GAP] alert. The
// virtual-now pattern lets us age the queue row deterministically.
func TestAuditDeliveryGapFiresWhenPendingExceedsAge(t *testing.T) {
	g, db := newTestGemma(t)

	if err := db.EnqueueMessage(42, "rain", "bot-hq-rain", "[brian] test msg"); err != nil {
		t.Fatal(err)
	}

	pending, _ := db.GetPendingMessages()
	if len(pending) != 1 {
		t.Fatalf("expected 1 pending row, got %d", len(pending))
	}
	queuedAt := pending[0].Created

	// Virtual now: 90s after the row was created — past the 60s
	// deliveryGapAge threshold.
	virtualNow := queuedAt.Add(90 * time.Second)
	g.auditDeliveryGapAt(virtualNow)

	if got := countDeliveryGapMsgs(t, db); got != 1 {
		t.Errorf("expected 1 [DELIVERY-GAP] alert at age=90s; got %d", got)
	}
}

// TestAuditDeliveryGapSuppressesYoungRows locks the v1 negative: a
// pending row younger than deliveryGapAge does NOT flag.
func TestAuditDeliveryGapSuppressesYoungRows(t *testing.T) {
	g, db := newTestGemma(t)

	if err := db.EnqueueMessage(42, "rain", "bot-hq-rain", "[brian] test msg"); err != nil {
		t.Fatal(err)
	}
	pending, _ := db.GetPendingMessages()
	queuedAt := pending[0].Created

	// Virtual now: 30s after creation — below 60s threshold.
	virtualNow := queuedAt.Add(30 * time.Second)
	g.auditDeliveryGapAt(virtualNow)

	if got := countDeliveryGapMsgs(t, db); got != 0 {
		t.Errorf("young pending rows must not flag; got %d alerts at age=30s", got)
	}
}

// TestAuditDeliveryGapHysteresis locks the per-row dedup contract:
// repeated audits of the same stalled row do NOT re-fire.
func TestAuditDeliveryGapHysteresis(t *testing.T) {
	g, db := newTestGemma(t)
	db.EnqueueMessage(99, "brian", "bot-hq-brian", "[rain] test")

	pending, _ := db.GetPendingMessages()
	queuedAt := pending[0].Created
	virtualNow := queuedAt.Add(120 * time.Second)

	g.auditDeliveryGapAt(virtualNow)
	g.auditDeliveryGapAt(virtualNow)
	g.auditDeliveryGapAt(virtualNow)

	if got := countDeliveryGapMsgs(t, db); got != 1 {
		t.Errorf("hysteresis violation: 3 audits of same stalled row produced %d alerts (want 1)", got)
	}
}

// TestAuditDeliveryGapRefiresOnNewQueueRow locks that the dedup is
// per-row (queue-id), not per-agent. A new pending row to the same
// agent, also stalled, must produce a new alert.
func TestAuditDeliveryGapRefiresOnNewQueueRow(t *testing.T) {
	g, db := newTestGemma(t)
	db.EnqueueMessage(1, "rain", "bot-hq-rain", "first")
	pending, _ := db.GetPendingMessages()
	first := pending[0]
	virtualNow := first.Created.Add(90 * time.Second)
	g.auditDeliveryGapAt(virtualNow)

	// Second pending row for same target.
	db.EnqueueMessage(2, "rain", "bot-hq-rain", "second")
	g.auditDeliveryGapAt(virtualNow.Add(time.Second))

	if got := countDeliveryGapMsgs(t, db); got != 2 {
		t.Errorf("new queue row to same agent should re-fire; got %d alerts (want 2)", got)
	}
}

// TestAuditDeliveryGapTrackerPrunesDrainedRows locks the
// flag-tracker pruning behavior: once a queue row exits pending
// (drained or exhausted), the dedup entry is removed so the tracker
// doesn't grow unbounded over a long-running session.
func TestAuditDeliveryGapTrackerPrunesDrainedRows(t *testing.T) {
	g, db := newTestGemma(t)
	db.EnqueueMessage(1, "rain", "bot-hq-rain", "first")
	pending, _ := db.GetPendingMessages()
	qmID := pending[0].ID
	virtualNow := pending[0].Created.Add(90 * time.Second)

	g.auditDeliveryGapAt(virtualNow)

	g.deliveryFlagMu.Lock()
	if _, ok := g.deliveryFlagTracker[qmID]; !ok {
		g.deliveryFlagMu.Unlock()
		t.Fatalf("tracker should contain qmID after first fire")
	}
	g.deliveryFlagMu.Unlock()

	// Drain the row (status='delivered').
	if err := db.UpdateQueueStatus(qmID, "delivered", 1); err != nil {
		t.Fatal(err)
	}
	g.auditDeliveryGapAt(virtualNow.Add(time.Second))

	g.deliveryFlagMu.Lock()
	defer g.deliveryFlagMu.Unlock()
	if _, ok := g.deliveryFlagTracker[qmID]; ok {
		t.Errorf("tracker should prune entries for non-pending rows; %d still present", qmID)
	}
}

// TestDeliveryGapAgeConstant locks the v1 threshold so accidental
// edits to deliveryGapAge are caught by tests rather than only at
// production deploy.
func TestDeliveryGapAgeConstant(t *testing.T) {
	if deliveryGapAge != 60*time.Second {
		t.Errorf("deliveryGapAge = %v, want 60s (slice-5 v1 lean per Rain BRAIN 4071)", deliveryGapAge)
	}
}
