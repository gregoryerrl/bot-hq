package gemma

import "github.com/gregoryerrl/bot-hq/internal/hub"

// NewWakeOnlyForTest builds a minimal Gemma whose only wired dependency is
// the DB + a stop channel. Bypasses Start()/Ollama so wake-dispatch tests
// don't need a live model server. Test-only export — visible to external
// test packages (e.g. gemma_test for the end-to-end MCP integration test
// landed in slice 3 C1.4).
func NewWakeOnlyForTest(db *hub.DB, stop chan struct{}) *Gemma {
	return &Gemma{db: db, stopCh: stop}
}

// RunWakeDispatchLoopForTest re-exports the unexported wakeDispatchLoop so
// gemma_test goroutines can drive it directly. Same loop the production
// Start() goroutine runs.
func (g *Gemma) RunWakeDispatchLoopForTest() { g.wakeDispatchLoop() }
