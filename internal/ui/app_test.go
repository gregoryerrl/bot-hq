package ui

import (
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func newTestDB(t *testing.T) *hub.DB {
	t.Helper()
	path := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

// TestNewAppSeedsFromRecentMessages locks the restart-flood fix:
// NewApp must single-push the last min(rowCount, 100) messages into hubTab
// and seed lastMsgID = max ID. Without this, the TUI pages 50 historical
// messages per tick and floods the viewport on every restart.
func TestNewAppSeedsFromRecentMessages(t *testing.T) {
	cases := []struct {
		name        string
		rowCount    int
		wantLen     int
		wantSeedGtZ bool
	}{
		{"empty DB", 0, 0, false},
		{"under cap", 30, 30, true},
		{"at cap", 100, 100, true},
		{"over cap", 250, 100, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			db := newTestDB(t)
			var maxID int64
			for i := 0; i < tc.rowCount; i++ {
				id, err := db.InsertMessage(protocol.Message{
					FromAgent: "user",
					Type:      protocol.MsgCommand,
					Content:   "msg",
				})
				if err != nil {
					t.Fatal(err)
				}
				if id > maxID {
					maxID = id
				}
			}
			app := NewApp(hub.Config{}, db, nil)
			if got := len(app.hubTab.messages); got != tc.wantLen {
				t.Errorf("len(hubTab.messages) = %d, want %d", got, tc.wantLen)
			}
			if tc.wantSeedGtZ {
				if app.lastMsgID <= 0 {
					t.Errorf("lastMsgID = %d, want > 0 when DB has rows", app.lastMsgID)
				}
				if app.lastMsgID != maxID {
					t.Errorf("lastMsgID = %d, want %d (max ID)", app.lastMsgID, maxID)
				}
			} else if app.lastMsgID != 0 {
				t.Errorf("lastMsgID = %d, want 0 when DB empty", app.lastMsgID)
			}
		})
	}
}
