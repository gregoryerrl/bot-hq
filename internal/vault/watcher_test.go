// Phase T T-10 cycle-3: vault watcher tests. R39 TEST-ISOLATION via
// t.TempDir() — no live filesystem touched.

package vault

import (
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// TestWatcher_DetectsChange validates the core mtime-advance detection: a
// file with a known baseline mtime fires the callback exactly once after
// being modified.
func TestWatcher_DetectsChange(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "rain.env")
	if err := os.WriteFile(path, []byte("DEEPSEEK_API_KEY=v1\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}

	var fired atomic.Int32
	var firedPath string
	var mu sync.Mutex
	w := NewWatcher([]string{path}, 50*time.Millisecond, func(p string) {
		fired.Add(1)
		mu.Lock()
		firedPath = p
		mu.Unlock()
	})

	w.Start()
	t.Cleanup(w.Stop)

	// Allow baseline capture, then advance mtime by overwriting + bumping mtime.
	time.Sleep(75 * time.Millisecond)
	future := time.Now().Add(2 * time.Second)
	if err := os.WriteFile(path, []byte("DEEPSEEK_API_KEY=v2\n"), 0o600); err != nil {
		t.Fatalf("rewrite: %v", err)
	}
	if err := os.Chtimes(path, future, future); err != nil {
		t.Fatalf("chtimes: %v", err)
	}

	// Wait for detection.
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) && fired.Load() == 0 {
		time.Sleep(20 * time.Millisecond)
	}
	if fired.Load() == 0 {
		t.Fatal("watcher did not detect mtime advance")
	}
	mu.Lock()
	gotPath := firedPath
	mu.Unlock()
	if gotPath != path {
		t.Errorf("fired with path = %q, want %q", gotPath, path)
	}
}

// TestWatcher_NoFalsePositiveOnUnchanged validates that an unchanged file
// across multiple poll cycles does NOT trigger the callback.
func TestWatcher_NoFalsePositiveOnUnchanged(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "rain.env")
	if err := os.WriteFile(path, []byte("DEEPSEEK_API_KEY=v1\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}

	var fired atomic.Int32
	w := NewWatcher([]string{path}, 25*time.Millisecond, func(string) {
		fired.Add(1)
	})

	w.Start()
	t.Cleanup(w.Stop)

	// Multiple poll intervals with no edit.
	time.Sleep(150 * time.Millisecond)

	if got := fired.Load(); got != 0 {
		t.Errorf("expected 0 callback fires for unchanged file; got %d", got)
	}
}

// TestWatcher_NoFireForMissingFile validates graceful handling of an
// initially absent path: no spurious fire, no panic.
func TestWatcher_NoFireForMissingFile(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "does-not-exist.env")

	var fired atomic.Int32
	w := NewWatcher([]string{missing}, 25*time.Millisecond, func(string) {
		fired.Add(1)
	})

	w.Start()
	t.Cleanup(w.Stop)

	time.Sleep(100 * time.Millisecond)
	if got := fired.Load(); got != 0 {
		t.Errorf("expected 0 fires for missing path; got %d", got)
	}
}

// TestWatcher_FireOnLateAppearance validates that a path that appears after
// the watcher starts (no initial baseline) fires the callback on first
// detection.
func TestWatcher_FireOnLateAppearance(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "rain.env")

	var fired atomic.Int32
	w := NewWatcher([]string{path}, 25*time.Millisecond, func(string) {
		fired.Add(1)
	})

	w.Start()
	t.Cleanup(w.Stop)

	// Path absent at baseline; create it after a poll cycle.
	time.Sleep(50 * time.Millisecond)
	if err := os.WriteFile(path, []byte("KEY=v1\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}

	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) && fired.Load() == 0 {
		time.Sleep(20 * time.Millisecond)
	}
	if fired.Load() == 0 {
		t.Fatal("watcher did not fire on late appearance")
	}
}

// TestWatcher_StopIsIdempotent validates Stop is safe to call multiple times
// (no panic, no goroutine leak).
func TestWatcher_StopIsIdempotent(t *testing.T) {
	w := NewWatcher([]string{}, 25*time.Millisecond, nil)
	w.Start()
	w.Stop()
	w.Stop() // second call must be a no-op, not a panic on closed channel.
}

// TestWatcher_StartIsIdempotent validates Start is safe to call multiple
// times (single goroutine, no leaks).
func TestWatcher_StartIsIdempotent(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "rain.env")
	if err := os.WriteFile(path, []byte("KEY=v1\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}

	var fired atomic.Int32
	w := NewWatcher([]string{path}, 25*time.Millisecond, func(string) {
		fired.Add(1)
	})

	w.Start()
	w.Start() // second call must be a no-op (would otherwise spawn a duplicate).
	t.Cleanup(w.Stop)

	// Trigger one change; verify the callback fires only once.
	time.Sleep(50 * time.Millisecond)
	future := time.Now().Add(2 * time.Second)
	if err := os.Chtimes(path, future, future); err != nil {
		t.Fatalf("chtimes: %v", err)
	}
	time.Sleep(150 * time.Millisecond)

	if got := fired.Load(); got != 1 {
		t.Errorf("expected exactly 1 fire (idempotent Start); got %d", got)
	}
}

// TestWatcher_CheckOnceManualTrigger validates the synchronous CheckOnce
// API (used by tests + manual diagnostics).
func TestWatcher_CheckOnceManualTrigger(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "rain.env")
	if err := os.WriteFile(path, []byte("KEY=v1\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}

	var fired atomic.Int32
	w := NewWatcher([]string{path}, time.Hour, func(string) {
		fired.Add(1)
	})

	// Manually capture baseline (Start would do this, but we want to skip the goroutine).
	w.captureBaseline()

	future := time.Now().Add(2 * time.Second)
	if err := os.Chtimes(path, future, future); err != nil {
		t.Fatalf("chtimes: %v", err)
	}

	w.CheckOnce()
	if got := fired.Load(); got != 1 {
		t.Errorf("CheckOnce expected 1 fire; got %d", got)
	}

	w.CheckOnce()
	if got := fired.Load(); got != 1 {
		t.Errorf("second CheckOnce on unchanged file expected no additional fire; got %d", got)
	}
}
