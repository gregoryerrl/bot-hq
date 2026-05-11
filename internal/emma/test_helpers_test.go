package emma

import (
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// newTestSystemMonitor constructs an isolated *SystemMonitor + fresh sqlite DB
// for tests. Z-9d: renamed from newTestEmma when the Emma type split into
// SystemMonitor (daemon-cadence) + Subprocess (tmux Claude Code).
func newTestSystemMonitor(t *testing.T) (*SystemMonitor, *hub.DB) {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return NewSystemMonitor(db), db
}
