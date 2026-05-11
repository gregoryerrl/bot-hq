package emma

import (
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// newTestEmma constructs an isolated *Emma + fresh sqlite DB for tests.
func newTestEmma(t *testing.T) (*Emma, *hub.DB) {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return New(db, hub.EmmaConfig{}), db
}
