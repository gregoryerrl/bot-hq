package emma

import "github.com/gregoryerrl/bot-hq/internal/hub"

// NewWakeOnlyForTest builds a minimal SystemMonitor whose only wired
// dependency is the DB + a stop channel. Bypasses Start() so wake-
// dispatch tests don't need the rest of the daemon-cadence goroutines
// to be running. Test-only export — visible to external test packages.
func NewWakeOnlyForTest(db *hub.DB, stop chan struct{}) *SystemMonitor {
	return &SystemMonitor{db: db, stopCh: stop}
}

// RunWakeDispatchLoopForTest re-exports the unexported wakeDispatchLoop so
// external test goroutines can drive it directly. Same loop the production
// Start() goroutine runs.
func (g *SystemMonitor) RunWakeDispatchLoopForTest() { g.wakeDispatchLoop() }
