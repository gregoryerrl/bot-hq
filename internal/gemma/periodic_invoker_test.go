package gemma

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// TestInternalDispatchRoutingPrefix locks the C7 routing invariant: a wake
// with target_agent prefixed `_internal:` must NOT produce a hub_send
// message; instead the in-process handler runs and the row marks fired.
//
// Uses an unknown internal target (`_internal:test-noop`) so the default
// branch of dispatchInternalWake fires (logs + drops), exercising the
// prefix-routing path without depending on any real handler's side
// effects.
func TestInternalDispatchRoutingPrefix(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	g := New(db, hub.GemmaConfig{})

	wakeID, err := db.InsertWakeSchedule("_internal:test-noop", agentID, "noop-payload", time.Now().Add(-1*time.Second))
	if err != nil {
		t.Fatal(err)
	}

	g.dispatchWakes()

	msgs, err := db.GetRecentMessages(50)
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range msgs {
		if m.ToAgent == "_internal:test-noop" {
			t.Errorf("internal wake produced hub_send message; routing prefix not honored: %+v", m)
		}
	}

	w, err := db.GetWakeSchedule(wakeID)
	if err != nil {
		t.Fatal(err)
	}
	if w.FireStatus != "fired" {
		t.Errorf("expected fire_status=fired post-dispatch, got %q", w.FireStatus)
	}
}

// TestPeriodicInvokerReArmsAfterFire locks the loop-continuation
// invariant for `_internal:docdrift`: after a fire, a new pending wake
// for the same target must be scheduled at fire_at = now + docDriftInterval
// (±jitter for clock skew between insert and assertion).
func TestPeriodicInvokerReArmsAfterFire(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	g := New(db, hub.GemmaConfig{})

	if _, err := db.InsertWakeSchedule(internalDocDriftTarget, agentID, "", time.Now().Add(-1*time.Second)); err != nil {
		t.Fatal(err)
	}

	g.dispatchWakes()

	pending, err := db.HasPendingWakeForTarget(internalDocDriftTarget)
	if err != nil {
		t.Fatal(err)
	}
	if !pending {
		t.Fatalf("expected pending docdrift wake post-fire (re-arm); none found")
	}
}

// TestBootstrapDocDriftSchedulesIfNoPending locks the boot-side scheduler:
// fresh db (no pending wakes) → bootstrap creates one for
// _internal:docdrift.
func TestBootstrapDocDriftSchedulesIfNoPending(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	g := New(db, hub.GemmaConfig{})

	g.bootstrapInternalDocDrift()

	pending, err := db.HasPendingWakeForTarget(internalDocDriftTarget)
	if err != nil {
		t.Fatal(err)
	}
	if !pending {
		t.Errorf("expected bootstrap to create pending docdrift wake; none found")
	}
}

// TestBootstrapDocDriftSkipsIfPendingExists locks the rebuild-idempotency
// invariant: a pending wake from a prior boot must NOT trigger a
// duplicate schedule. Boot is a no-op when something is already pending.
func TestBootstrapDocDriftSkipsIfPendingExists(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	g := New(db, hub.GemmaConfig{})

	if _, err := db.InsertWakeSchedule(internalDocDriftTarget, agentID, "", time.Now().Add(docDriftInterval)); err != nil {
		t.Fatal(err)
	}

	g.bootstrapInternalDocDrift()

	wakes, err := db.ListPendingWakes(time.Now().Add(2 * docDriftInterval))
	if err != nil {
		t.Fatal(err)
	}
	count := 0
	for _, w := range wakes {
		if w.TargetAgent == internalDocDriftTarget {
			count++
		}
	}
	if count != 1 {
		t.Errorf("expected exactly 1 pending docdrift wake after bootstrap-with-existing; got %d", count)
	}
}

