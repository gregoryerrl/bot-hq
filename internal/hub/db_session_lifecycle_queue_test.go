package hub

import (
	"path/filepath"
	"testing"
	"time"
)

func openTestDB(t *testing.T) *DB {
	t.Helper()
	db, err := OpenDB(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

func TestEnqueueSessionOp_HappyPath(t *testing.T) {
	db := openTestDB(t)
	id, err := db.EnqueueSessionOp(SessionLifecycleOp{
		Kind:            "open",
		SessionID:       "test-scope-abc123",
		Project:         "bot-hq",
		Scope:           "test scope",
		PointerListJSON: `["projects/bot-hq/vision.md"]`,
	})
	if err != nil {
		t.Fatal(err)
	}
	if id <= 0 {
		t.Errorf("expected positive id, got %d", id)
	}
	op, err := db.GetSessionOp(id)
	if err != nil {
		t.Fatal(err)
	}
	if op.Kind != "open" {
		t.Errorf("kind=%q want open", op.Kind)
	}
	if op.SessionID != "test-scope-abc123" {
		t.Errorf("session_id=%q want test-scope-abc123", op.SessionID)
	}
	if op.Project != "bot-hq" {
		t.Errorf("project=%q want bot-hq", op.Project)
	}
	if op.Status != "pending" {
		t.Errorf("status=%q want pending", op.Status)
	}
	if op.PointerListJSON != `["projects/bot-hq/vision.md"]` {
		t.Errorf("pointer_list_json=%q unexpected", op.PointerListJSON)
	}
}

func TestEnqueueSessionOp_RejectsInvalidKind(t *testing.T) {
	db := openTestDB(t)
	if _, err := db.EnqueueSessionOp(SessionLifecycleOp{Kind: "bogus", SessionID: "x"}); err == nil {
		t.Error("expected error for invalid kind")
	}
}

func TestClaimPendingSessionOps_AtomicallyMarksClaimed(t *testing.T) {
	db := openTestDB(t)
	// Enqueue 3 pending rows.
	for i := 0; i < 3; i++ {
		if _, err := db.EnqueueSessionOp(SessionLifecycleOp{
			Kind: "open", SessionID: "test", Project: "bot-hq",
		}); err != nil {
			t.Fatal(err)
		}
	}
	// First claim sees all 3.
	ops, err := db.ClaimPendingSessionOps(10)
	if err != nil {
		t.Fatal(err)
	}
	if len(ops) != 3 {
		t.Errorf("claim 1: got %d ops, want 3", len(ops))
	}
	for _, op := range ops {
		if op.ClaimedAt.IsZero() {
			t.Errorf("op %d not stamped claimed_at", op.ID)
		}
	}
	// Second claim should see 0 (all are claimed but still 'pending').
	ops2, err := db.ClaimPendingSessionOps(10)
	if err != nil {
		t.Fatal(err)
	}
	if len(ops2) != 0 {
		t.Errorf("claim 2: got %d ops, want 0 (already claimed)", len(ops2))
	}
}

func TestMarkSessionOpFired_TransitionsStatus(t *testing.T) {
	db := openTestDB(t)
	id, err := db.EnqueueSessionOp(SessionLifecycleOp{Kind: "open", SessionID: "x"})
	if err != nil {
		t.Fatal(err)
	}
	resultJSON := `{"session_id":"x","agents":["brian","rain"]}`
	if err := db.MarkSessionOpFired(id, "fired", resultJSON); err != nil {
		t.Fatal(err)
	}
	op, _ := db.GetSessionOp(id)
	if op.Status != "fired" {
		t.Errorf("status=%q want fired", op.Status)
	}
	if op.ResultJSON != resultJSON {
		t.Errorf("result_json=%q want %q", op.ResultJSON, resultJSON)
	}
	if op.FiredAt.IsZero() {
		t.Error("fired_at should be set")
	}
}

func TestMarkSessionOpFired_RejectsInvalidStatus(t *testing.T) {
	db := openTestDB(t)
	id, _ := db.EnqueueSessionOp(SessionLifecycleOp{Kind: "open", SessionID: "x"})
	if err := db.MarkSessionOpFired(id, "wat", "{}"); err == nil {
		t.Error("expected error for invalid status")
	}
}

func TestSessionLifecycleQueue_EndToEnd(t *testing.T) {
	// Simulate the Z-3d round-trip: subprocess enqueues, daemon claims,
	// daemon marks fired, subprocess polls + reads result.
	db := openTestDB(t)

	// Step 1: subprocess enqueue
	id, err := db.EnqueueSessionOp(SessionLifecycleOp{
		Kind:            "open",
		SessionID:       "e2e-test-aaaaaa",
		Project:         "bot-hq",
		Scope:           "e2e",
		PointerListJSON: `["p1","p2"]`,
	})
	if err != nil {
		t.Fatal(err)
	}

	// Step 2: subprocess starts polling (would happen in background).
	// We just observe pending state once.
	op, _ := db.GetSessionOp(id)
	if op.Status != "pending" {
		t.Errorf("step 2: status=%q want pending", op.Status)
	}

	// Step 3: daemon claims + processes.
	ops, _ := db.ClaimPendingSessionOps(10)
	if len(ops) != 1 || ops[0].ID != id {
		t.Fatalf("step 3: claim did not return our op (got %v)", ops)
	}
	result := `{"session_id":"e2e-test-aaaaaa","project":"bot-hq","scope":"e2e","agents":["brian","rain"]}`
	if err := db.MarkSessionOpFired(id, "fired", result); err != nil {
		t.Fatal(err)
	}

	// Step 4: subprocess sees fired + result.
	time.Sleep(10 * time.Millisecond) // simulate next poll
	op, _ = db.GetSessionOp(id)
	if op.Status != "fired" {
		t.Errorf("step 4: status=%q want fired", op.Status)
	}
	if op.ResultJSON != result {
		t.Errorf("step 4: result mismatch %q vs %q", op.ResultJSON, result)
	}
}
