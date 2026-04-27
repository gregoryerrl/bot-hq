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

// TestQueueRowCreatedRoundTrip locks that GetPendingMessages returns
// rows whose Created field is non-zero and recent — i.e. the SQL
// scan correctly populates time.Time from the TIMESTAMP column.
//
// Regression lock for the H-22-bis post-deploy bug: prior code did
// `time.Parse(time.DateTime, created)` against driver-formatted RFC3339
// strings, swallowed the parse error into `_`, left Created at the
// zero value. Production `now.Sub(time.Time{})` overflowed to
// math.MaxInt64 nanoseconds, so the auditor fired with a 292-year
// "age" sentinel. The unit tests at the time used virtual-now anchored
// to whatever the read returned (zero) so they passed.
//
// This test forces a real round-trip: insert via EnqueueMessage (which
// fires SQLite CURRENT_TIMESTAMP server-side), read back via
// GetPendingMessages, assert Created is sane.
func TestQueueRowCreatedRoundTrip(t *testing.T) {
	_, db := newTestGemma(t)

	before := time.Now().Add(-2 * time.Second)
	if err := db.EnqueueMessage(7, "rain", "bot-hq-rain", "[brian] roundtrip"); err != nil {
		t.Fatal(err)
	}
	after := time.Now().Add(2 * time.Second)

	pending, err := db.GetPendingMessages()
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 1 {
		t.Fatalf("expected 1 pending row, got %d", len(pending))
	}
	got := pending[0].Created
	if got.IsZero() {
		t.Fatalf("Created is zero — SQL scan failed to populate time.Time (regression: H-22-bis format-mismatch swallow)")
	}
	if got.Before(before) || got.After(after) {
		t.Errorf("Created = %v, want within [%v, %v] (clock-coherent round-trip)", got, before, after)
	}
}

// TestAuditDeliveryGapPositiveFireOnRealClock is the ratchet-7
// integration test: a real SQL row + real time.Now() audit produce a
// real [DELIVERY-GAP] alert with a sane age string. Sister of the
// virtual-now tests above — those locked logic, this locks
// SQL-round-trip correctness end-to-end.
//
// Without this, virtual-now tests pass even when Created is zeroed,
// because their `virtualNow := pending[0].Created.Add(...)` arithmetic
// stays self-consistent inside the year-0001 anchor. Production fires
// math.MaxInt64 sentinels.
func TestAuditDeliveryGapPositiveFireOnRealClock(t *testing.T) {
	g, db := newTestGemma(t)

	if err := db.EnqueueMessage(11, "rain", "bot-hq-rain", "[brian] real-clock"); err != nil {
		t.Fatal(err)
	}

	// Backdate created column so the row exceeds deliveryGapAge under
	// real time.Now(). EnqueueMessage uses server-side CURRENT_TIMESTAMP,
	// so we override with the test helper.
	if err := db.SetQueueRowCreatedForTest(11, time.Now().Add(-90*time.Second)); err != nil {
		t.Fatal(err)
	}

	g.auditDeliveryGap()

	msgs, err := db.GetRecentMessages(50)
	if err != nil {
		t.Fatal(err)
	}
	var alert string
	for _, m := range msgs {
		if m.FromAgent == agentID && strings.Contains(m.Content, "[DELIVERY-GAP]") {
			alert = m.Content
			break
		}
	}
	if alert == "" {
		t.Fatal("expected one [DELIVERY-GAP] alert from real-clock audit; got none")
	}

	// Sanity-check the age substring: must NOT be the math.MaxInt64
	// nanosecond sentinel that surfaced in production.
	if strings.Contains(alert, "2562047h") {
		t.Errorf("alert contains MaxInt64 duration sentinel — Created is zero (regression): %s", alert)
	}
	// And must report a plausible age (≥60s, ≤24h) — anchored in
	// real-clock arithmetic, not virtual.
	if !strings.Contains(alert, "pending for 1m") && !strings.Contains(alert, "pending for 2m") {
		t.Errorf("alert age looks wrong; got %q (expected ~90s real-clock window)", alert)
	}
}
