package hub

// Tests for pending_actions queue helpers (P-9 / phase-n.md:818).
// All tests use setupTestDB (R39 TEST-ISOLATION) — no prod hub.db
// touched.

import (
	"strings"
	"testing"
)

// TestInsertPendingAction_AssignsID + happy-path round-trip.
func TestInsertPendingAction_AssignsID(t *testing.T) {
	db := setupTestDB(t)
	id, err := db.InsertPendingAction("rain", "hr-broadcast", "Important review needed", 100)
	if err != nil {
		t.Fatalf("insert: %v", err)
	}
	if id <= 0 {
		t.Errorf("insert returned non-positive id %d", id)
	}
}

// TestInsertPendingAction_TruncatesLongSummary verifies the cap.
func TestInsertPendingAction_TruncatesLongSummary(t *testing.T) {
	db := setupTestDB(t)
	long := strings.Repeat("x", 500)
	id, err := db.InsertPendingAction("rain", "hr-broadcast", long, 0)
	if err != nil {
		t.Fatalf("insert: %v", err)
	}
	actions, err := db.ListPendingActions(50, false)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	var found bool
	for _, a := range actions {
		if a.ID == id {
			found = true
			if !strings.Contains(a.Summary, "…") {
				t.Errorf("long summary should be truncated; got len=%d", len(a.Summary))
			}
			if len(a.Summary) > pendingSummaryMaxLen+10 {
				t.Errorf("summary len = %d, want ≤ %d+ellipsis", len(a.Summary), pendingSummaryMaxLen)
			}
		}
	}
	if !found {
		t.Errorf("inserted action not found in list")
	}
}

// TestListPendingActions_FiltersByStatus default-pending only.
func TestListPendingActions_FiltersByStatus(t *testing.T) {
	db := setupTestDB(t)
	id1, _ := db.InsertPendingAction("rain", "hr-broadcast", "first", 1)
	_, _ = db.InsertPendingAction("brian", "hr-broadcast", "second", 2)
	if _, err := db.AckPendingAction(id1); err != nil {
		t.Fatalf("ack: %v", err)
	}
	pending, err := db.ListPendingActions(50, false)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(pending) != 1 {
		t.Errorf("expected 1 pending after ack; got %d (%v)", len(pending), pending)
	}
	all, err := db.ListPendingActions(50, true)
	if err != nil {
		t.Fatalf("list all: %v", err)
	}
	if len(all) != 2 {
		t.Errorf("expected 2 with includeAcked; got %d", len(all))
	}
}

// TestListPendingActions_ReverseChrono newest first.
func TestListPendingActions_ReverseChrono(t *testing.T) {
	db := setupTestDB(t)
	_, _ = db.InsertPendingAction("rain", "hr", "first", 1)
	_, _ = db.InsertPendingAction("brian", "hr", "second", 2)
	_, _ = db.InsertPendingAction("rain", "hr", "third", 3)
	got, err := db.ListPendingActions(50, false)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(got) != 3 {
		t.Fatalf("len = %d, want 3", len(got))
	}
	if got[0].Summary != "third" {
		t.Errorf("first entry should be newest; got %q", got[0].Summary)
	}
}

// TestListPendingActions_LimitClamp + default + hard-cap.
func TestListPendingActions_LimitClamp(t *testing.T) {
	db := setupTestDB(t)
	for i := 0; i < 5; i++ {
		_, _ = db.InsertPendingAction("rain", "hr", "msg", int64(i))
	}
	got, err := db.ListPendingActions(2, false)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(got) != 2 {
		t.Errorf("limit=2 → %d hits, want 2", len(got))
	}
	got, err = db.ListPendingActions(0, false)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(got) != 5 {
		t.Errorf("limit=0 (default 50) → %d, want 5", len(got))
	}
}

// TestCountPendingActions matches list-len.
func TestCountPendingActions(t *testing.T) {
	db := setupTestDB(t)
	if n, _ := db.CountPendingActions(); n != 0 {
		t.Errorf("empty count = %d, want 0", n)
	}
	_, _ = db.InsertPendingAction("rain", "hr", "a", 1)
	id2, _ := db.InsertPendingAction("brian", "hr", "b", 2)
	if n, _ := db.CountPendingActions(); n != 2 {
		t.Errorf("count after 2 inserts = %d, want 2", n)
	}
	_, _ = db.AckPendingAction(id2)
	if n, _ := db.CountPendingActions(); n != 1 {
		t.Errorf("count after 1 ack = %d, want 1", n)
	}
}

// TestAckPendingAction_Idempotent: ack'ing an already-ack'd row is
// reported as not-updated; ack'ing nonexistent row likewise.
func TestAckPendingAction_Idempotent(t *testing.T) {
	db := setupTestDB(t)
	id, _ := db.InsertPendingAction("rain", "hr", "test", 1)
	ok, err := db.AckPendingAction(id)
	if err != nil || !ok {
		t.Fatalf("first ack should succeed; ok=%v err=%v", ok, err)
	}
	ok2, err := db.AckPendingAction(id)
	if err != nil {
		t.Fatalf("second ack err: %v", err)
	}
	if ok2 {
		t.Errorf("second ack should report ok=false (already ack'd)")
	}
	ok3, err := db.AckPendingAction(99999)
	if err != nil {
		t.Fatalf("missing-id ack err: %v", err)
	}
	if ok3 {
		t.Errorf("missing-id ack should report ok=false")
	}
}
