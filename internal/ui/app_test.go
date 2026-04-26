package ui

import (
	"path/filepath"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
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

// TestKeyDispatchOnAgentsTab_CursorNav verifies App.Update routes tea.KeyMsg
// to the agents tab when activeTab=TabAgents. Regression: A2 added cursor
// handlers to AgentsTab.Update but app.go's KeyMsg default lacked a TabAgents
// branch, so every key (j/k/arrows/enter) was silently dropped at the App
// router. Unit tests on AgentsTab.Update couldn't catch this — the gap was
// one layer up at App.Update. This test exercises the parent router.
func TestKeyDispatchOnAgentsTab_CursorNav(t *testing.T) {
	db := newTestDB(t)
	for i, id := range []string{"a1", "a2", "a3"} {
		if err := db.RegisterAgent(protocol.Agent{
			ID:     id,
			Name:   "Agent " + id,
			Type:   protocol.AgentBrian,
			Status: protocol.StatusOnline,
		}); err != nil {
			t.Fatalf("register %d: %v", i, err)
		}
	}
	app := NewApp(hub.Config{}, db, nil)
	updated, _ := app.Update(tickMsg(time.Now()))
	app = updated.(App)
	app.activeTab = TabAgents

	if got := app.agentsTab.cursor; got != 0 {
		t.Fatalf("initial cursor = %d, want 0", got)
	}

	updated, _ = app.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("j")})
	app = updated.(App)
	if got := app.agentsTab.cursor; got != 1 {
		t.Errorf("after j: cursor = %d, want 1 (App.Update did not route key to AgentsTab)", got)
	}

	updated, _ = app.Update(tea.KeyMsg{Type: tea.KeyDown})
	app = updated.(App)
	if got := app.agentsTab.cursor; got != 2 {
		t.Errorf("after KeyDown: cursor = %d, want 2", got)
	}

	updated, _ = app.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("k")})
	app = updated.(App)
	if got := app.agentsTab.cursor; got != 1 {
		t.Errorf("after k: cursor = %d, want 1", got)
	}
}

// TestKeyDispatchOnAgentsTab_EnterOpensModal verifies the Enter path through
// App.Update reaches AgentsTab and opens the pane modal when the cursor is
// on an agent with a tmux_target. Pairs with the cursor-nav test to cover
// both navigation and modal-trigger surfaces of the Slice 1 regression.
func TestKeyDispatchOnAgentsTab_EnterOpensModal(t *testing.T) {
	db := newTestDB(t)
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "tmux-agent",
		Name:   "Tmux Agent",
		Type:   protocol.AgentBrian,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"bot-hq:0.1"}`,
	}); err != nil {
		t.Fatal(err)
	}
	app := NewApp(hub.Config{}, db, nil)
	app.agentsTab = NewAgentsTab(func(target string, lines int) (string, error) {
		return "captured pane content for " + target, nil
	})
	if app.pane != nil {
		app.agentsTab.SetPane(app.pane)
	}
	updated, _ := app.Update(tickMsg(time.Now()))
	app = updated.(App)
	app.activeTab = TabAgents

	if app.agentsTab.paneModal != nil {
		t.Fatal("paneModal non-nil before Enter")
	}

	updated, _ = app.Update(tea.KeyMsg{Type: tea.KeyEnter})
	app = updated.(App)
	if app.agentsTab.paneModal == nil {
		t.Errorf("paneModal nil after Enter (App.Update did not route Enter to AgentsTab)")
	}
}

// TestKeyDispatchInverse verifies that keys do NOT reach the agents tab when
// activeTab is something else (e.g. TabHub). Catches future regressions
// where someone wires keys to all tabs unconditionally.
func TestKeyDispatchInverse(t *testing.T) {
	db := newTestDB(t)
	for _, id := range []string{"a1", "a2", "a3"} {
		if err := db.RegisterAgent(protocol.Agent{
			ID:     id,
			Name:   "Agent " + id,
			Type:   protocol.AgentBrian,
			Status: protocol.StatusOnline,
		}); err != nil {
			t.Fatal(err)
		}
	}
	app := NewApp(hub.Config{}, db, nil)
	updated, _ := app.Update(tickMsg(time.Now()))
	app = updated.(App)
	app.activeTab = TabHub

	startCursor := app.agentsTab.cursor

	updated, _ = app.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("j")})
	app = updated.(App)
	if got := app.agentsTab.cursor; got != startCursor {
		t.Errorf("agentsTab.cursor advanced to %d while activeTab=TabHub; want %d (key leaked across tab boundary)", got, startCursor)
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

// TestAppNumberKeysAutoFocusHubInput locks Path C: numeric keys (1-4) no
// longer jump to a specific tab. They auto-focus the hub input and type
// literally, like any other printable. Tab cycling via tab/shift+tab is
// the only path between tabs.
func TestAppNumberKeysAutoFocusHubInput(t *testing.T) {
	for _, ch := range []string{"1", "2", "3", "4"} {
		t.Run("key="+ch, func(t *testing.T) {
			app := NewApp(hub.Config{}, nil, nil)
			app.activeTab = TabHub
			startTab := app.activeTab

			updated, _ := app.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune(ch)})
			app = updated.(App)

			if !app.hubTab.focused {
				t.Errorf("key %q: hubTab.focused = false, want true (auto-focus on printable)", ch)
			}
			if got := app.hubTab.input.Value(); got != ch {
				t.Errorf("key %q: input.Value() = %q, want %q", ch, got, ch)
			}
			if app.activeTab != startTab {
				t.Errorf("key %q: activeTab changed from %v to %v (number keys must NOT jump tabs)", ch, startTab, app.activeTab)
			}
		})
	}
}

// TestAppQAutoFocusesHubInput locks Path C: q is no longer a quit alias.
// It auto-focuses the hub input and types literally. Quit is ctrl+c only.
func TestAppQAutoFocusesHubInput(t *testing.T) {
	app := NewApp(hub.Config{}, nil, nil)
	app.activeTab = TabHub

	updated, cmd := app.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("q")})
	app = updated.(App)

	if !app.hubTab.focused {
		t.Errorf("hubTab.focused = false, want true (q must auto-focus, not quit)")
	}
	if got := app.hubTab.input.Value(); got != "q" {
		t.Errorf("input.Value() = %q, want %q (q must type literally)", got, "q")
	}
	if cmd != nil {
		if msg := cmd(); msg != nil {
			if _, ok := msg.(tea.QuitMsg); ok {
				t.Errorf("q returned tea.QuitMsg — q must NOT quit under Path C")
			}
		}
	}
}

// TestAppCtrlCStillQuits is the sanity check that the q-quit removal didn't
// break ctrl+c quit. ctrl+c remains the sole quit binding under Path C.
func TestAppCtrlCStillQuits(t *testing.T) {
	app := NewApp(hub.Config{}, nil, nil)

	_, cmd := app.Update(tea.KeyMsg{Type: tea.KeyCtrlC})
	if cmd == nil {
		t.Fatal("ctrl+c returned nil cmd, want a quit command")
	}
	msg := cmd()
	if _, ok := msg.(tea.QuitMsg); !ok {
		t.Errorf("ctrl+c cmd returned %T, want tea.QuitMsg", msg)
	}
}

// TestAppTabCyclingUnchanged locks that tab/shift+tab still cycle through
// all 4 tabs and back to the start. The Path C trim (removing 1/2/3/4)
// must not regress this surface.
func TestAppTabCyclingUnchanged(t *testing.T) {
	app := NewApp(hub.Config{}, nil, nil)
	app.activeTab = TabHub

	expect := []Tab{TabAgents, TabSessions, TabSettings, TabHub}
	for i, want := range expect {
		updated, _ := app.Update(tea.KeyMsg{Type: tea.KeyTab})
		app = updated.(App)
		if app.activeTab != want {
			t.Errorf("tab cycle step %d: activeTab = %v, want %v", i+1, app.activeTab, want)
		}
	}

	// Reverse cycle via shift+tab.
	expect = []Tab{TabSettings, TabSessions, TabAgents, TabHub}
	for i, want := range expect {
		updated, _ := app.Update(tea.KeyMsg{Type: tea.KeyShiftTab})
		app = updated.(App)
		if app.activeTab != want {
			t.Errorf("shift+tab cycle step %d: activeTab = %v, want %v", i+1, app.activeTab, want)
		}
	}
}

