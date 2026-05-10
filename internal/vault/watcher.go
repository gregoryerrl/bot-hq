// Phase T T-10 cycle-3: vault file mtime watcher.
//
// Daemon-side polling watcher for FileVault paths. Detects user-side rotation
// (manual edit of `~/.bot-hq/agents/<agent>/.env`) and fires a callback on
// mtime advance. Callback emits a hub notification so the user gets feedback
// without having to "signal me when done" via hub channel — eliminates the
// paste-to-hub temptation class that recurred this cycle.
//
// Design: simple time.Ticker polling (not fsnotify) for stdlib-only leaf-
// package discipline + portability + low overhead (single-digit syscalls
// per minute per watched file). Polling interval defaults to 30s.

package vault

import (
	"os"
	"sync"
	"time"
)

// Watcher polls a list of files for mtime advance and fires a callback on
// change. The watcher is a dormant struct until Start; Stop blocks until
// the polling goroutine has exited.
type Watcher struct {
	paths    []string
	interval time.Duration
	onChange func(path string)

	mu     sync.Mutex
	mtimes map[string]time.Time

	startOnce sync.Once
	stopOnce  sync.Once
	stop      chan struct{}
	done      chan struct{}
}

// NewWatcher returns a Watcher over the given file paths. interval defaults
// to 30s when zero. onChange is called from the polling goroutine on each
// detected mtime advance; it must be safe for concurrent use if the caller
// reuses the same callback across multiple Watchers.
func NewWatcher(paths []string, interval time.Duration, onChange func(string)) *Watcher {
	if interval <= 0 {
		interval = 30 * time.Second
	}
	return &Watcher{
		paths:    append([]string(nil), paths...),
		interval: interval,
		onChange: onChange,
		mtimes:   make(map[string]time.Time),
		stop:     make(chan struct{}),
		done:     make(chan struct{}),
	}
}

// Start launches the polling goroutine. Initial mtimes are captured before
// the first tick so the first detected change is a real advance from the
// known baseline (no startup-trigger spurious fire). Subsequent calls to
// Start are no-ops.
func (w *Watcher) Start() {
	w.startOnce.Do(func() {
		w.captureBaseline()
		go w.loop()
	})
}

// Stop signals the polling goroutine to exit and blocks until it has done so.
// Subsequent calls to Stop are no-ops.
func (w *Watcher) Stop() {
	w.stopOnce.Do(func() {
		close(w.stop)
		<-w.done
	})
}

// CheckOnce runs a single poll synchronously. Exposed for tests + manual
// triggers; production callers use Start.
func (w *Watcher) CheckOnce() {
	w.checkOnce()
}

func (w *Watcher) loop() {
	defer close(w.done)
	ticker := time.NewTicker(w.interval)
	defer ticker.Stop()
	for {
		select {
		case <-w.stop:
			return
		case <-ticker.C:
			w.checkOnce()
		}
	}
}

func (w *Watcher) captureBaseline() {
	w.mu.Lock()
	defer w.mu.Unlock()
	for _, p := range w.paths {
		info, err := os.Stat(p)
		if err != nil {
			// Missing path → no baseline; first appearance counts as a change.
			continue
		}
		w.mtimes[p] = info.ModTime()
	}
}

func (w *Watcher) checkOnce() {
	for _, p := range w.paths {
		info, err := os.Stat(p)
		if err != nil {
			// Missing / unreadable → skip silently. A future appearance
			// counts as a change against the absent baseline.
			continue
		}
		w.mu.Lock()
		prev, ok := w.mtimes[p]
		changed := !ok || info.ModTime().After(prev)
		if changed {
			w.mtimes[p] = info.ModTime()
		}
		w.mu.Unlock()
		if changed && w.onChange != nil {
			w.onChange(p)
		}
	}
}
