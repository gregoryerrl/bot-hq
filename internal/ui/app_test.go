package ui

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
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

// TestNewAppBackfillIsChronological locks the boot-fix iteration order:
// hubTab.messages must end up oldest→newest, matching the chronological
// order GetRecentMessages returns. A reversed iteration was the cause of
// the visible "jump on restart" — newest msgs landed at top, oldest at
// bottom, and the auto-scroll-to-bottom anchored on the wrong end.
func TestNewAppBackfillIsChronological(t *testing.T) {
	db := newTestDB(t)
	var ids []int64
	for i := 0; i < 5; i++ {
		id, err := db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgCommand,
			Content:   "msg",
		})
		if err != nil {
			t.Fatal(err)
		}
		ids = append(ids, id)
	}
	app := NewApp(hub.Config{}, db, nil)
	if len(app.hubTab.messages) != 5 {
		t.Fatalf("len(messages) = %d, want 5", len(app.hubTab.messages))
	}
	for i := 0; i < 5; i++ {
		if app.hubTab.messages[i].ID != ids[i] {
			t.Errorf("messages[%d].ID = %d, want %d (chronological order broken)",
				i, app.hubTab.messages[i].ID, ids[i])
		}
	}
}

// TestNewAppLastIDMatchesNewest locks that lastMsgID equals the newest
// inserted message's ID after backfill, regardless of iteration direction.
// Independent contract from TestNewAppBackfillIsChronological so a future
// refactor can't drop one without surfacing the other.
func TestNewAppLastIDMatchesNewest(t *testing.T) {
	db := newTestDB(t)
	var newest int64
	for i := 0; i < 5; i++ {
		id, err := db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgCommand,
			Content:   "msg",
		})
		if err != nil {
			t.Fatal(err)
		}
		newest = id
	}
	app := NewApp(hub.Config{}, db, nil)
	if app.lastMsgID != newest {
		t.Errorf("lastMsgID = %d, want %d (newest)", app.lastMsgID, newest)
	}
}

// TestNewAppConstructsPanestateManager verifies App owns a non-nil
// panestate.Manager when a DB is provided. Locks against accidental
// regression where the field is dropped or left nil.
func TestNewAppConstructsPanestateManager(t *testing.T) {
	db := newTestDB(t)
	app := NewApp(hub.Config{}, db, nil)
	if app.pane == nil {
		t.Fatal("App.pane is nil with non-nil db")
	}
}

// TestNewAppPaneNilWithNilDB verifies App.pane is nil when DB is nil.
// App must tolerate a nil DB (e.g. for headless render tests).
func TestNewAppPaneNilWithNilDB(t *testing.T) {
	app := NewApp(hub.Config{}, nil, nil)
	if app.pane != nil {
		t.Error("App.pane should be nil with nil db")
	}
}

// TestAppTickRefreshesPanestate verifies that App.Update(tickMsg) calls
// pane.Refresh, populating the snapshot from the current DB list.
func TestAppTickRefreshesPanestate(t *testing.T) {
	db := newTestDB(t)
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "tick-agent",
		Name:   "Tick Agent",
		Type:   protocol.AgentBrian,
		Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}
	app := NewApp(hub.Config{}, db, nil)
	if app.pane == nil {
		t.Fatal("pane nil")
	}
	// Pre-tick: snapshot should be empty.
	if got := len(app.pane.Snapshot()); got != 0 {
		t.Errorf("pre-tick snapshot len = %d, want 0", got)
	}

	updated, _ := app.Update(tickMsg(time.Now()))
	app = updated.(App)

	snap := app.pane.Snapshot()
	if len(snap) == 0 {
		t.Fatal("post-tick snapshot is empty; pane.Refresh did not fire")
	}
	found := false
	for _, s := range snap {
		if s.ID == "tick-agent" {
			found = true
			if s.Activity == panestate.ActivityOffline {
				t.Errorf("registered agent should not be offline; got %v", s.Activity)
			}
		}
	}
	if !found {
		t.Errorf("registered agent not present in snapshot")
	}
}

// TestAppTickPropagatesAgentsToTab verifies the AgentsTab still receives the
// agent slice via AgentsUpdated after the panestate plumbing change. Locks
// the "behavior at user-visible level: zero" guarantee for commit 3.
func TestAppTickPropagatesAgentsToTab(t *testing.T) {
	db := newTestDB(t)
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "tab-agent",
		Name:   "Tab Agent",
		Type:   protocol.AgentBrian,
		Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}
	app := NewApp(hub.Config{}, db, nil)

	updated, _ := app.Update(tickMsg(time.Now()))
	app = updated.(App)

	if got := len(app.agentsTab.agents); got != 1 {
		t.Errorf("agentsTab.agents len = %d, want 1", got)
	}
	if got := app.agentsTab.agents[0].ID; got != "tab-agent" {
		t.Errorf("agentsTab.agents[0].ID = %q, want tab-agent", got)
	}
}

// TestPanestateSnapshotFreshness verifies that after a DB last_seen update,
// the next tick's snapshot reflects the new state. Locks against tabs
// holding stale snapshot copies — surfaces if Manager.Refresh stops
// re-querying or stops recomputing activity.
func TestPanestateSnapshotFreshness(t *testing.T) {
	db := newTestDB(t)
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "fresh-agent",
		Name:   "Fresh Agent",
		Type:   protocol.AgentBrian,
		Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}

	app := NewApp(hub.Config{}, db, nil)
	updated, _ := app.Update(tickMsg(time.Now()))
	app = updated.(App)

	first := app.pane.Snapshot()
	var firstSeen time.Time
	for _, s := range first {
		if s.ID == "fresh-agent" {
			firstSeen = s.LastSeen
		}
	}

	time.Sleep(10 * time.Millisecond)
	if err := db.UpdateAgentLastSeen("fresh-agent"); err != nil {
		t.Fatal(err)
	}

	updated, _ = app.Update(tickMsg(time.Now()))
	app = updated.(App)

	second := app.pane.Snapshot()
	for _, s := range second {
		if s.ID == "fresh-agent" {
			if !s.LastSeen.After(firstSeen) {
				t.Errorf("snapshot LastSeen did not advance after UpdateAgentLastSeen: first=%v second=%v", firstSeen, s.LastSeen)
			}
			return
		}
	}
	t.Fatal("agent missing from second snapshot")
}

