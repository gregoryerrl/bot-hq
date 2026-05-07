package sessions

// Tests for retention/prune helpers (P-5 / phase-p.md §P-5 + OQ-5).
// All tests use BOT_HQ_SESSIONS_DIR override per R39 TEST-ISOLATION
// so prod ~/.bot-hq/sessions/ is never touched.

import (
	"os"
	"path/filepath"
	"sort"
	"testing"
	"time"
)

// writeManifestAt creates a session dir with a manifest pinned to the
// given start/end timestamps. Helper for retention-window tests.
func writeManifestAt(t *testing.T, id string, start, end time.Time) {
	t.Helper()
	m := Manifest{
		ID:      id,
		Project: "bot-hq",
		StartTS: start,
		EndTS:   end,
		Body:    "test manifest\n",
	}
	if err := WriteManifest(m); err != nil {
		t.Fatalf("write manifest %s: %v", id, err)
	}
}

// TestIsWithinRetention_ZeroDaysAlwaysWithin verifies the "disabled"
// semantics: retentionDays <= 0 means no window check.
func TestIsWithinRetention_ZeroDaysAlwaysWithin(t *testing.T) {
	setSessionsDir(t)
	// Manifest doesn't exist — retentionDays=0 still returns true.
	ok, err := IsWithinRetention("anything", 0, time.Now())
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if !ok {
		t.Errorf("retentionDays=0 should disable check (always true)")
	}
	ok, _ = IsWithinRetention("anything", -5, time.Now())
	if !ok {
		t.Errorf("retentionDays<0 should disable check (always true)")
	}
}

// TestIsWithinRetention_RecentSession returns true for a session
// within the window.
func TestIsWithinRetention_RecentSession(t *testing.T) {
	setSessionsDir(t)
	now := time.Date(2026, 5, 7, 12, 0, 0, 0, time.UTC)
	writeManifestAt(t, "2026-05-06-bot-hq", now.Add(-25*time.Hour), now.Add(-24*time.Hour))
	ok, err := IsWithinRetention("2026-05-06-bot-hq", 30, now)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if !ok {
		t.Errorf("session 1 day old should be within 30-day window")
	}
}

// TestIsWithinRetention_OldSession returns false for a session past
// the cutoff.
func TestIsWithinRetention_OldSession(t *testing.T) {
	setSessionsDir(t)
	now := time.Date(2026, 5, 7, 12, 0, 0, 0, time.UTC)
	writeManifestAt(t, "2026-04-01-bot-hq", now.Add(-37*24*time.Hour), now.Add(-36*24*time.Hour))
	ok, err := IsWithinRetention("2026-04-01-bot-hq", 30, now)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if ok {
		t.Errorf("session 36 days old should be outside 30-day window")
	}
}

// TestIsWithinRetention_MissingManifest returns false (out-of-window
// for safety) without erroring.
func TestIsWithinRetention_MissingManifest(t *testing.T) {
	setSessionsDir(t)
	ok, err := IsWithinRetention("nonexistent", 30, time.Now())
	if err != nil {
		t.Errorf("missing manifest should not error; got %v", err)
	}
	if ok {
		t.Errorf("missing manifest should be reported as out-of-window")
	}
}

// TestIsWithinRetention_FallsBackToStartTS verifies that an open
// (never-closed) manifest with EndTS=zero uses StartTS for the age
// computation.
func TestIsWithinRetention_FallsBackToStartTS(t *testing.T) {
	setSessionsDir(t)
	now := time.Date(2026, 5, 7, 12, 0, 0, 0, time.UTC)
	writeManifestAt(t, "2026-05-06-bot-hq", now.Add(-25*time.Hour), time.Time{})
	ok, err := IsWithinRetention("2026-05-06-bot-hq", 30, now)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if !ok {
		t.Errorf("EndTS=zero should fall back to StartTS for age check")
	}
}

// TestPruneOlderThan_NoOp_ZeroDays verifies retentionDays<=0 is a
// safety no-op (no sessions are removed).
func TestPruneOlderThan_NoOp_ZeroDays(t *testing.T) {
	setSessionsDir(t)
	now := time.Now()
	writeManifestAt(t, "2025-01-01-bot-hq", now.Add(-365*24*time.Hour), now.Add(-364*24*time.Hour))
	pruned, err := PruneOlderThan(0, now)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if pruned != nil {
		t.Errorf("retentionDays=0 should not prune; got %v", pruned)
	}
	if _, err := os.Stat(filepath.Join(SessionsDir(), "2025-01-01-bot-hq")); err != nil {
		t.Errorf("session should still exist after no-op prune")
	}
}

// TestPruneOlderThan_RemovesOldSessions verifies the happy path: old
// sessions get deleted, recent ones kept.
func TestPruneOlderThan_RemovesOldSessions(t *testing.T) {
	setSessionsDir(t)
	now := time.Date(2026, 5, 7, 12, 0, 0, 0, time.UTC)
	writeManifestAt(t, "2026-05-06-bot-hq", now.Add(-25*time.Hour), now.Add(-24*time.Hour))    // recent
	writeManifestAt(t, "2026-05-04-bot-hq", now.Add(-72*time.Hour), now.Add(-71*time.Hour))    // recent
	writeManifestAt(t, "2026-04-01-bot-hq", now.Add(-37*24*time.Hour), now.Add(-36*24*time.Hour)) // old
	writeManifestAt(t, "2026-03-15-bot-hq", now.Add(-55*24*time.Hour), now.Add(-54*24*time.Hour)) // older

	pruned, err := PruneOlderThan(30, now)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	sort.Strings(pruned)
	want := []string{"2026-03-15-bot-hq", "2026-04-01-bot-hq"}
	if len(pruned) != len(want) {
		t.Fatalf("pruned len = %d, want %d; got %v", len(pruned), len(want), pruned)
	}
	for i, id := range pruned {
		if id != want[i] {
			t.Errorf("pruned[%d] = %q, want %q", i, id, want[i])
		}
	}
	// Recent sessions should still exist.
	for _, id := range []string{"2026-05-06-bot-hq", "2026-05-04-bot-hq"} {
		if _, err := os.Stat(filepath.Join(SessionsDir(), id)); err != nil {
			t.Errorf("recent session %s removed unexpectedly: %v", id, err)
		}
	}
	// Old sessions should be gone.
	for _, id := range want {
		if _, err := os.Stat(filepath.Join(SessionsDir(), id)); !os.IsNotExist(err) {
			t.Errorf("old session %s should be removed; stat err=%v", id, err)
		}
	}
}

// TestPruneOlderThan_SkipsUnparseableManifest verifies that a session
// dir with a corrupted manifest is preserved (conservative; operator
// investigates manually).
func TestPruneOlderThan_SkipsUnparseableManifest(t *testing.T) {
	setSessionsDir(t)
	now := time.Date(2026, 5, 7, 12, 0, 0, 0, time.UTC)
	dir := filepath.Join(SessionsDir(), "corrupt-id")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "manifest.md"), []byte("not yaml"), 0o644); err != nil {
		t.Fatalf("write corrupt: %v", err)
	}
	pruned, err := PruneOlderThan(30, now)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if len(pruned) != 0 {
		t.Errorf("corrupt manifest should be skipped; got pruned=%v", pruned)
	}
	if _, err := os.Stat(dir); err != nil {
		t.Errorf("corrupt session should be preserved; stat err=%v", err)
	}
}

// TestDefaultRetentionDays_ReasonableValue pins the default retention
// window so future-edits-changing-the-default break this ratchet test.
func TestDefaultRetentionDays_ReasonableValue(t *testing.T) {
	if DefaultRetentionDays < 7 || DefaultRetentionDays > 365 {
		t.Errorf("DefaultRetentionDays=%d outside reasonable range [7, 365]", DefaultRetentionDays)
	}
}
