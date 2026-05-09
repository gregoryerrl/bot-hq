package clschema

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

// makeOldFile creates a file at path + sets its mtime backward by age.
func makeOldFile(t *testing.T, path string, age time.Duration) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(path, []byte("x"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	old := time.Now().Add(-age)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatalf("chtimes: %v", err)
	}
}

// ====== DefaultPolicy ======

func TestDefaultPolicy_sessions30d(t *testing.T) {
	p := DefaultPolicy()
	if ttl := p.SubdirTTL["sessions"]; ttl != 30*24*time.Hour {
		t.Errorf("sessions TTL = %v, want 720h (30d)", ttl)
	}
	if p.ArchiveDir != "archive" {
		t.Errorf("ArchiveDir = %q, want archive", p.ArchiveDir)
	}
}

// ====== ListArchivable ======

func TestListArchivable_emptyRootReturnsNil(t *testing.T) {
	root := t.TempDir()
	out, err := ListArchivable(root, DefaultPolicy())
	if err != nil {
		t.Fatalf("ListArchivable: %v", err)
	}
	if out != nil {
		t.Errorf("out = %v, want nil for empty root", out)
	}
}

func TestListArchivable_oldEntriesIncluded(t *testing.T) {
	root := t.TempDir()
	makeOldFile(t, filepath.Join(root, "sessions", "old-session-1"), 35*24*time.Hour)
	makeOldFile(t, filepath.Join(root, "sessions", "fresh-session"), 1*time.Hour)

	out, err := ListArchivable(root, DefaultPolicy())
	if err != nil {
		t.Fatalf("ListArchivable: %v", err)
	}
	if len(out) != 1 {
		t.Errorf("archivable count = %d, want 1", len(out))
	}
	if !filepath.IsAbs(out[0]) && !contains(out[0], "old-session-1") {
		t.Errorf("path = %q, want containing old-session-1", out[0])
	}
}

func TestListArchivable_emptyRootArgRejected(t *testing.T) {
	if _, err := ListArchivable("", DefaultPolicy()); err == nil {
		t.Error("expected error for empty root")
	}
}

func TestListArchivable_zeroTTLSkipped(t *testing.T) {
	root := t.TempDir()
	makeOldFile(t, filepath.Join(root, "snapshots", "ancient"), 365*24*time.Hour)
	policy := ArchivePolicy{
		SubdirTTL: map[string]time.Duration{
			"snapshots": 0, // retain-forever
		},
	}
	out, _ := ListArchivable(root, policy)
	if len(out) != 0 {
		t.Errorf("zero-TTL should skip; got %d entries", len(out))
	}
}

// ====== ArchiveOldSessions ======

func TestArchiveOldSessions_movesOldEntries(t *testing.T) {
	root := t.TempDir()
	makeOldFile(t, filepath.Join(root, "sessions", "old-1"), 35*24*time.Hour)
	makeOldFile(t, filepath.Join(root, "sessions", "old-2"), 60*24*time.Hour)
	makeOldFile(t, filepath.Join(root, "sessions", "fresh"), 1*time.Hour)

	count, err := ArchiveOldSessions(root, DefaultPolicy())
	if err != nil {
		t.Fatalf("ArchiveOldSessions: %v", err)
	}
	if count != 2 {
		t.Errorf("archived count = %d, want 2", count)
	}
	// Verify fresh stayed
	if _, err := os.Stat(filepath.Join(root, "sessions", "fresh")); err != nil {
		t.Errorf("fresh entry should not be archived: %v", err)
	}
	// Verify old moved to archive/
	if _, err := os.Stat(filepath.Join(root, "sessions", "archive", "old-1")); err != nil {
		t.Errorf("old-1 not in archive: %v", err)
	}
}

func TestArchiveOldSessions_emptyRootReturnsZero(t *testing.T) {
	root := t.TempDir()
	count, err := ArchiveOldSessions(root, DefaultPolicy())
	if err != nil {
		t.Fatalf("ArchiveOldSessions: %v", err)
	}
	if count != 0 {
		t.Errorf("count = %d, want 0", count)
	}
}

func TestArchiveOldSessions_skipsArchiveSubdirItself(t *testing.T) {
	root := t.TempDir()
	makeOldFile(t, filepath.Join(root, "sessions", "archive", "previously-archived"), 365*24*time.Hour)
	count, _ := ArchiveOldSessions(root, DefaultPolicy())
	if count != 0 {
		t.Errorf("archive-subdir should be skipped; got count=%d", count)
	}
}

// ====== ageOf helper ======

func TestAgeOf_returnsElapsedSinceMtime(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "test")
	makeOldFile(t, path, 5*time.Hour)
	age, err := ageOf(path)
	if err != nil {
		t.Fatalf("ageOf: %v", err)
	}
	if age < 4*time.Hour || age > 6*time.Hour {
		t.Errorf("age = %v, want ~5h ±1h", age)
	}
}

// helper
func contains(haystack, needle string) bool {
	for i := 0; i+len(needle) <= len(haystack); i++ {
		if haystack[i:i+len(needle)] == needle {
			return true
		}
	}
	return false
}
