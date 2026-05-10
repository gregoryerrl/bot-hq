package gemma

import (
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// newTestGemma constructs an isolated *Gemma + fresh sqlite DB for tests.
func newTestGemma(t *testing.T) (*Gemma, *hub.DB) {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return New(db, hub.GemmaConfig{}), db
}
