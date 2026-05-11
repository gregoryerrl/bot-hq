package emma

import (
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/daemoncron"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// newContextCapSystemMonitor builds an isolated SystemMonitor + temp-DB pair.
// Resets daemoncron's package-scoped plan-usage state so tests don't leak
// cooldown/halt-flag state across runs.
func newContextCapSystemMonitor(t *testing.T) (*SystemMonitor, *hub.DB) {
	t.Helper()
	daemoncron.ResetPlanUsageStateForTest()
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return NewSystemMonitor(db), db
}

// fakePaneSnap returns a paneSnapshotFn yielding a fixed slice each call.
// Tests build a fresh fake per scenario (no time-series semantics) since
// H-31 reads each tick independently — no baseline maintained across calls.
func fakePaneSnap(s []panestate.AgentSnapshot) paneSnapshotFn {
	return func() []panestate.AgentSnapshot { return s }
}

func countContextCapFlags(t *testing.T, db *hub.DB) int {
	t.Helper()
	msgs, err := db.GetRecentMessages(200)
	if err != nil {
		t.Fatal(err)
	}
	n := 0
	for _, m := range msgs {
		// Z-8b: daemon-cadence threshold flag emits as "system" (was "emma").
		if m.FromAgent == "system" && m.Type == protocol.MsgFlag &&
			strings.Contains(m.Content, "[CRITICAL]") &&
			strings.Contains(m.Content, "halt + checkpoint via H-15") {
			n++
		}
	}
	return n
}

// TestH31FlagFiresAt95 — UsagePct=95 + halt inactive + hysteresis-clear → one
// critical FLAG and halt_state set active.
func TestH31FlagFiresAt95(t *testing.T) {
	g, db := newContextCapSystemMonitor(t)
	g.paneSnapFn = fakePaneSnap([]panestate.AgentSnapshot{
		{ID: "brian", ContextPct: 95},
	})

	g.checkContextCap(time.Now())

	if got := countContextCapFlags(t, db); got != 1 {
		t.Fatalf("expected 1 critical context-cap flag at 95%%, got %d", got)
	}
	active, err := db.IsHalted()
	if err != nil {
		t.Fatal(err)
	}
	if !active {
		t.Errorf("halt_state must be active after 95%% fire")
	}
}

// TestH31NoFlagBelow95 — UsagePct=94 → no flag, no halt set. Locks the
// inclusive ≥95 threshold so a future off-by-one regression doesn't fire on
// 94% (which would alarm constantly under normal operation).
func TestH31NoFlagBelow95(t *testing.T) {
	g, db := newContextCapSystemMonitor(t)
	g.paneSnapFn = fakePaneSnap([]panestate.AgentSnapshot{
		{ID: "brian", ContextPct: 94},
	})

	g.checkContextCap(time.Now())

	if got := countContextCapFlags(t, db); got != 0 {
		t.Errorf("must not fire below 95%%; got %d flags", got)
	}
	active, err := db.IsHalted()
	if err != nil {
		t.Fatal(err)
	}
	if active {
		t.Errorf("halt_state must remain inactive when no flag fires")
	}
}

// TestH31HysteresisAndHaltSuppressDoubleFire — fire once at 95% on tick 1.
// Tick 2 still shows 95%+ but halt is now active → IsHaltActive gate skips
// the loop entirely. Even if halt were cleared, the per-agent hysteresis on
// `context-cap:<id>` would still suppress within the 30min window.
func TestH31HysteresisAndHaltSuppressDoubleFire(t *testing.T) {
	g, db := newContextCapSystemMonitor(t)
	g.paneSnapFn = fakePaneSnap([]panestate.AgentSnapshot{
		{ID: "brian", ContextPct: 96},
	})

	now := time.Now()
	g.checkContextCap(now)
	g.checkContextCap(now.Add(30 * time.Second))
	g.checkContextCap(now.Add(60 * time.Second))

	if got := countContextCapFlags(t, db); got != 1 {
		t.Errorf("expected 1 flag across 3 ticks (halt+hysteresis dual gate); got %d", got)
	}
	active, _ := db.IsHalted()
	if !active {
		t.Errorf("halt_state must remain active across silent re-ticks")
	}
}

// TestH31ResetBelow85Rearms — fire at 95, manually clear halt, drop to 84
// (resets per-agent hysteresis), back to 95 → second flag fires. Exercises
// the re-arm path used after a successful fresh-session restart that reset
// the squeeze AND a manual halt clear (e.g. via hub_clear_halt) — the
// halt-suppression gate alone would still block without that clear, which
// is the correct behavior under the halt-all-work convention.
func TestH31ResetBelow85Rearms(t *testing.T) {
	g, db := newContextCapSystemMonitor(t)
	now := time.Now()

	// Tick 1: squeeze hits 95.
	g.paneSnapFn = fakePaneSnap([]panestate.AgentSnapshot{
		{ID: "brian", ContextPct: 95},
	})
	g.checkContextCap(now)
	if got := countContextCapFlags(t, db); got != 1 {
		t.Fatalf("setup: expected 1 flag from first 95%% tick, got %d", got)
	}

	// Operator clears halt (e.g. fresh duo session online, or hub_clear_halt).
	if err := db.ClearHaltManually(); err != nil {
		t.Fatal(err)
	}

	// Tick 2: usage drops below reset threshold; hysteresis re-arms.
	g.paneSnapFn = fakePaneSnap([]panestate.AgentSnapshot{
		{ID: "brian", ContextPct: 70},
	})
	g.checkContextCap(now.Add(time.Minute))

	// Tick 3: squeeze returns to 95 → fresh fire.
	g.paneSnapFn = fakePaneSnap([]panestate.AgentSnapshot{
		{ID: "brian", ContextPct: 95},
	})
	g.checkContextCap(now.Add(2 * time.Minute))

	if got := countContextCapFlags(t, db); got != 2 {
		t.Errorf("expected 2 flags after reset+rearm cycle, got %d", got)
	}
}

// TestHaltClearsOnDuoReregister — set halt, advance brian/rain/clive
// last_seen past set_at via re-register, ClearHaltIfDuoReregistered fires.
func TestHaltClearsOnDuoReregister(t *testing.T) {
	_, db := newContextCapSystemMonitor(t)

	if err := db.SetHaltActive(hub.HaltCauseContextCap, "test halt", "emma"); err != nil {
		t.Fatal(err)
	}
	active, err := db.IsHalted()
	if err != nil {
		t.Fatal(err)
	}
	if !active {
		t.Fatal("setup: halt must be active")
	}

	// Sleep past the millisecond resolution of UnixMilli so subsequent
	// RegisterAgent writes are guaranteed to stamp a strictly-greater
	// last_seen than halt_state.set_at.
	time.Sleep(5 * time.Millisecond)

	// All three duo members re-register after set_at, advancing their
	// last_seen past the halt timestamp.
	for _, id := range hub.HaltStateDuo {
		if err := db.RegisterAgent(protocol.Agent{
			ID:     id,
			Name:   id,
			Type:   protocol.AgentBrian,
			Status: protocol.StatusOnline,
		}); err != nil {
			t.Fatal(err)
		}
	}

	cleared, err := db.ClearHaltIfDuoReregistered(hub.HaltStateDuo)
	if err != nil {
		t.Fatal(err)
	}
	if !cleared {
		t.Fatalf("expected halt to clear after duo re-register past set_at")
	}
	active, _ = db.IsHalted()
	if active {
		t.Errorf("halt_state must be inactive after duo-re-register clear")
	}
}

// TestHaltDoesNotClearWithPrunedDuoMember — currently-registered semantic:
// 2 of 3 duo members exist and advance past set_at; the third (clive) is
// missing from the agents table. The comparison set is the 2 registered
// members; both advanced → cleared. Locks the BRAIN R1 micro-refine
// (currently-registered, not registered-or-blocked).
func TestHaltDoesNotClearWithPrunedDuoMember(t *testing.T) {
	_, db := newContextCapSystemMonitor(t)

	if err := db.SetHaltActive(hub.HaltCauseContextCap, "test halt", "emma"); err != nil {
		t.Fatal(err)
	}

	time.Sleep(5 * time.Millisecond)

	// Only brian + rain register; clive stays absent (pruned/voice-down).
	for _, id := range []string{"brian", "rain"} {
		if err := db.RegisterAgent(protocol.Agent{
			ID:     id,
			Name:   id,
			Type:   protocol.AgentBrian,
			Status: protocol.StatusOnline,
		}); err != nil {
			t.Fatal(err)
		}
	}

	cleared, err := db.ClearHaltIfDuoReregistered(hub.HaltStateDuo)
	if err != nil {
		t.Fatal(err)
	}
	if !cleared {
		t.Fatalf("expected halt to clear when registered-duo-subset all advanced past set_at; got cleared=false")
	}
	active, _ := db.IsHalted()
	if active {
		t.Errorf("halt_state must be inactive after partial-duo-all-advanced clear")
	}
}

// TestHaltDoesNotClearWithEmptyComparisonSet — no duo member registered at
// all. Comparison set is empty; the contract says empty MUST NOT clear (an
// empty set is absence of evidence, not evidence of fresh-context arrival).
func TestHaltDoesNotClearWithEmptyComparisonSet(t *testing.T) {
	_, db := newContextCapSystemMonitor(t)

	if err := db.SetHaltActive(hub.HaltCauseContextCap, "test halt", "emma"); err != nil {
		t.Fatal(err)
	}

	cleared, err := db.ClearHaltIfDuoReregistered(hub.HaltStateDuo)
	if err != nil {
		t.Fatal(err)
	}
	if cleared {
		t.Errorf("empty comparison set must not clear; got cleared=true")
	}
	active, _ := db.IsHalted()
	if !active {
		t.Errorf("halt_state must remain active when no duo member is registered")
	}
}

// TestEmptyPaneSnapNoOp — paneSnapFn returns nil/empty; no flag, no halt.
// Locks the no-op invariant for the early-boot window before the first
// panestate.Refresh has populated the snapshot.
func TestEmptyPaneSnapNoOp(t *testing.T) {
	g, db := newContextCapSystemMonitor(t)
	g.paneSnapFn = fakePaneSnap(nil)

	g.checkContextCap(time.Now())

	if got := countContextCapFlags(t, db); got != 0 {
		t.Errorf("empty snapshot must not fire flags; got %d", got)
	}
}
