package hub

import (
	"database/sql"
	"fmt"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func setupTestDB(t *testing.T) *DB {
	t.Helper()
	path := filepath.Join(t.TempDir(), "test.db")
	db, err := OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

// insertAgentWithLastSeen registers an agent then backdates last_seen via
// direct SQL — needed by C5 prune tests since RegisterAgent always stamps
// last_seen = now.
func insertAgentWithLastSeen(t *testing.T, db *DB, id string, status protocol.AgentStatus, lastSeen time.Time) {
	t.Helper()
	if err := db.RegisterAgent(protocol.Agent{
		ID:     id,
		Name:   id,
		Type:   protocol.AgentCoder,
		Status: status,
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := db.conn.Exec(
		`UPDATE agents SET last_seen = ? WHERE id = ?`,
		lastSeen.UnixMilli(), id,
	); err != nil {
		t.Fatal(err)
	}
}

// TestPruneRemovesStaleOffline locks the slice 3 C5 (H-25) primary
// invariant: an offline agent whose last_seen is older than threshold is
// pruned + its ID returned for audit.
func TestPruneRemovesStaleOffline(t *testing.T) {
	db := setupTestDB(t)
	old := time.Now().Add(-48 * time.Hour)
	insertAgentWithLastSeen(t, db, "stale-1", protocol.StatusOffline, old)

	ids, err := db.PruneStaleOfflineAgents(24 * time.Hour)
	if err != nil {
		t.Fatal(err)
	}
	if len(ids) != 1 || ids[0] != "stale-1" {
		t.Errorf("expected pruned ids = [stale-1], got %v", ids)
	}
	if _, err := db.GetAgent("stale-1"); err == nil {
		t.Errorf("agent stale-1 still present after prune")
	}
}

// TestPruneSparesOnline locks the live-agent protection invariant:
// agents with status='online' or 'working' are NEVER pruned regardless of
// last_seen age. The status filter alone gates eligibility.
func TestPruneSparesOnline(t *testing.T) {
	db := setupTestDB(t)
	old := time.Now().Add(-48 * time.Hour)
	insertAgentWithLastSeen(t, db, "online-old", protocol.StatusOnline, old)
	insertAgentWithLastSeen(t, db, "working-old", protocol.StatusWorking, old)

	ids, err := db.PruneStaleOfflineAgents(24 * time.Hour)
	if err != nil {
		t.Fatal(err)
	}
	if len(ids) != 0 {
		t.Errorf("expected 0 pruned ids, got %v", ids)
	}
	if _, err := db.GetAgent("online-old"); err != nil {
		t.Errorf("online-old should be retained: %v", err)
	}
	if _, err := db.GetAgent("working-old"); err != nil {
		t.Errorf("working-old should be retained: %v", err)
	}
}

// TestPruneSparesRecentOffline locks the threshold-respecting invariant:
// recently-offline agents (within threshold) survive prune. Reclamation
// only targets long-dead rows.
func TestPruneSparesRecentOffline(t *testing.T) {
	db := setupTestDB(t)
	recent := time.Now().Add(-1 * time.Hour)
	insertAgentWithLastSeen(t, db, "recent-offline", protocol.StatusOffline, recent)

	ids, err := db.PruneStaleOfflineAgents(24 * time.Hour)
	if err != nil {
		t.Fatal(err)
	}
	if len(ids) != 0 {
		t.Errorf("expected 0 pruned ids, got %v", ids)
	}
	if _, err := db.GetAgent("recent-offline"); err != nil {
		t.Errorf("recent-offline should be retained: %v", err)
	}
}

func TestRegisterAndGetAgent(t *testing.T) {
	db := setupTestDB(t)

	agent := protocol.Agent{
		ID:      "claude-abc",
		Name:    "Claude ABC",
		Type:    protocol.AgentCoder,
		Status:  protocol.StatusOnline,
		Project: "/projects/test",
	}

	if err := db.RegisterAgent(agent); err != nil {
		t.Fatal(err)
	}

	got, err := db.GetAgent("claude-abc")
	if err != nil {
		t.Fatal(err)
	}
	if got.Name != "Claude ABC" {
		t.Errorf("expected name 'Claude ABC', got %q", got.Name)
	}
	if got.Type != protocol.AgentCoder {
		t.Errorf("expected type coder, got %s", got.Type)
	}
}

// TestRebuildGenMigrationIdempotent locks that re-running migrate against
// an already-migrated DB does not double-add the rebuild_gen column or
// otherwise error. Phase G v1 #20.
func TestRebuildGenMigrationIdempotent(t *testing.T) {
	db := setupTestDB(t)
	if err := db.migrate(); err != nil {
		t.Errorf("second migrate() should be no-op, got: %v", err)
	}
	if err := db.migrate(); err != nil {
		t.Errorf("third migrate() should still be no-op, got: %v", err)
	}
}

// TestAddColumnIfMissingRejectsBadIdentifier locks the SQL identifier guard
// added in slice 2 C3. The guard is unreachable from current call sites
// (all literal constants), but a future contributor could reintroduce
// injection by passing dynamic input — the guard fails fast instead.
func TestAddColumnIfMissingRejectsBadIdentifier(t *testing.T) {
	db := setupTestDB(t)
	cases := []struct {
		name        string
		table, col  string
		wantErrFrag string
	}{
		{"sql injection in table", "agents; DROP TABLE messages--", "ok", "invalid table"},
		{"sql injection in column", "agents", "x; DROP TABLE messages--", "invalid column"},
		{"empty table", "", "ok", "invalid table"},
		{"leading digit table", "1agents", "ok", "invalid table"},
		{"space in column", "agents", "bad name", "invalid column"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := db.addColumnIfMissing(tc.table, tc.col, "TEXT")
			if err == nil {
				t.Fatalf("expected error for table=%q col=%q, got nil", tc.table, tc.col)
			}
			if !strings.Contains(err.Error(), tc.wantErrFrag) {
				t.Errorf("err = %q, want substring %q", err.Error(), tc.wantErrFrag)
			}
		})
	}
}

// TestSnapJSONColumnPresent locks that the messages table carries the
// snap_json column post-migrate, and that re-running migrate is a no-op
// (no double-add). Phase G v1 slice 2 C2.
func TestSnapJSONColumnPresent(t *testing.T) {
	db := setupTestDB(t)
	if !columnExists(t, db, "messages", "snap_json") {
		t.Fatalf("snap_json column missing on messages after migrate()")
	}
	// Idempotent re-run.
	if err := db.migrate(); err != nil {
		t.Fatalf("re-migrate err: %v", err)
	}
	if !columnExists(t, db, "messages", "snap_json") {
		t.Fatalf("snap_json column missing after second migrate()")
	}
}

func columnExists(t *testing.T, db *DB, table, column string) bool {
	t.Helper()
	rows, err := db.conn.Query("PRAGMA table_info(" + table + ")")
	if err != nil {
		t.Fatalf("pragma: %v", err)
	}
	defer rows.Close()
	for rows.Next() {
		var cid int
		var name, typ string
		var notnull, pk int
		var dflt sql.NullString
		if err := rows.Scan(&cid, &name, &typ, &notnull, &dflt, &pk); err != nil {
			t.Fatalf("scan: %v", err)
		}
		if name == column {
			return true
		}
	}
	return false
}

// TestIncrementRebuildGen locks that the gen monotonically increases
// across calls and persists between DB reopens via the settings row.
func TestIncrementRebuildGen(t *testing.T) {
	path := filepath.Join(t.TempDir(), "gen.db")
	db1, err := OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	g1, err := db1.IncrementRebuildGen()
	if err != nil {
		t.Fatal(err)
	}
	if g1 != 1 {
		t.Errorf("first increment expected 1, got %d", g1)
	}
	if got := db1.CurrentRebuildGen(); got != 1 {
		t.Errorf("CurrentRebuildGen after first increment = %d, want 1", got)
	}
	g2, err := db1.IncrementRebuildGen()
	if err != nil {
		t.Fatal(err)
	}
	if g2 != 2 {
		t.Errorf("second increment expected 2, got %d", g2)
	}
	db1.Close()

	// Reopen — gen should persist via settings row.
	db2, err := OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	defer db2.Close()
	g3, err := db2.IncrementRebuildGen()
	if err != nil {
		t.Fatal(err)
	}
	if g3 != 3 {
		t.Errorf("post-reopen increment expected 3 (continues from 2), got %d", g3)
	}
}

// TestRegisterAgentStampsRebuildGen locks that RegisterAgent stamps the
// current rebuild_gen onto the row, and that legacy rows registered before
// the first IncrementRebuildGen carry gen=0.
func TestRegisterAgentStampsRebuildGen(t *testing.T) {
	db := setupTestDB(t)

	// Register before any IncrementRebuildGen call → gen 0 (legacy).
	if err := db.RegisterAgent(protocol.Agent{
		ID: "legacy", Name: "Legacy", Type: protocol.AgentCoder, Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}
	got, _ := db.GetAgent("legacy")
	if got.RebuildGen != 0 {
		t.Errorf("legacy agent expected gen 0, got %d", got.RebuildGen)
	}

	// After increment, new registrations stamp the current gen.
	if _, err := db.IncrementRebuildGen(); err != nil {
		t.Fatal(err)
	}
	if err := db.RegisterAgent(protocol.Agent{
		ID: "fresh", Name: "Fresh", Type: protocol.AgentBrian, Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}
	got, _ = db.GetAgent("fresh")
	if got.RebuildGen != 1 {
		t.Errorf("fresh agent expected gen 1, got %d", got.RebuildGen)
	}

	// ListAgents carries the gen too.
	agents, _ := db.ListAgents("")
	for _, a := range agents {
		if a.ID == "fresh" && a.RebuildGen != 1 {
			t.Errorf("ListAgents fresh row gen = %d, want 1", a.RebuildGen)
		}
		if a.ID == "legacy" && a.RebuildGen != 0 {
			t.Errorf("ListAgents legacy row gen = %d, want 0", a.RebuildGen)
		}
	}
}

// TestRegisterAgentReadsLatestGenAcrossDBInstances locks that an agent
// registered through a second DB handle (simulating the MCP subcommand
// process, which opens its own *DB and never calls IncrementRebuildGen)
// stamps the current gen from the settings table — not a stale per-process
// cache. Regression test for the post-rebuild #7 bug where MCP-registered
// agents were stamped gen=0 while the hub's settings row was already at 1.
func TestRegisterAgentReadsLatestGenAcrossDBInstances(t *testing.T) {
	path := filepath.Join(t.TempDir(), "cross.db")

	hubDB, err := OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	defer hubDB.Close()

	mcpDB, err := OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	defer mcpDB.Close()

	// Hub process bumps the gen at startup.
	if _, err := hubDB.IncrementRebuildGen(); err != nil {
		t.Fatal(err)
	}

	// MCP process registers an agent. Must observe gen=1 even though it
	// never called IncrementRebuildGen on its own handle.
	if got := mcpDB.CurrentRebuildGen(); got != 1 {
		t.Fatalf("mcpDB.CurrentRebuildGen = %d, want 1 (settings authoritative)", got)
	}
	if err := mcpDB.RegisterAgent(protocol.Agent{
		ID: "mcp-agent", Name: "MCP", Type: protocol.AgentBrian, Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}

	// Read back via the hub handle — confirms storage, not just cache.
	got, err := hubDB.GetAgent("mcp-agent")
	if err != nil {
		t.Fatal(err)
	}
	if got.RebuildGen != 1 {
		t.Errorf("mcp-agent RebuildGen = %d, want 1 (cross-process write)", got.RebuildGen)
	}
}

func TestListAgents(t *testing.T) {
	db := setupTestDB(t)

	db.RegisterAgent(protocol.Agent{ID: "a1", Name: "A1", Type: protocol.AgentCoder, Status: protocol.StatusOnline})
	db.RegisterAgent(protocol.Agent{ID: "a2", Name: "A2", Type: protocol.AgentVoice, Status: protocol.StatusOnline})
	db.RegisterAgent(protocol.Agent{ID: "a3", Name: "A3", Type: protocol.AgentCoder, Status: protocol.StatusOffline})

	agents, err := db.ListAgents("")
	if err != nil {
		t.Fatal(err)
	}
	if len(agents) != 3 {
		t.Errorf("expected 3 agents, got %d", len(agents))
	}

	online, err := db.ListAgents("online")
	if err != nil {
		t.Fatal(err)
	}
	if len(online) != 2 {
		t.Errorf("expected 2 online agents, got %d", len(online))
	}
}

// TestInsertMessagePopulatesSnapJSON locks the send-path SNAP extraction
// hook: messages with a well-formed SNAP block get snap_json populated with
// canonical JSON; messages without a block get empty string; malformed
// blocks are tolerated (empty + log warn, no insert failure).
// Phase G v1 slice 2 C3.
func TestInsertMessagePopulatesSnapJSON(t *testing.T) {
	db := setupTestDB(t)
	db.RegisterAgent(protocol.Agent{ID: "brian", Name: "B", Type: protocol.AgentBrian, Status: protocol.StatusOnline})

	cases := []struct {
		name       string
		content    string
		wantEmpty  bool
		wantSubstr string
	}{
		{
			name: "well-formed SNAP populates JSON",
			content: "body text\n\nSNAP:\n" +
				"Branches: bot-hq:main@abc\n" +
				"Agents:   brian(idle)\n" +
				"Pending:  none\n" +
				"Next:     ship",
			wantSubstr: `"branches":["bot-hq:main@abc"]`,
		},
		{
			name:      "no SNAP block leaves snap_json empty",
			content:   "just a regular message with no footer",
			wantEmpty: true,
		},
		{
			name:      "malformed SNAP tolerates and stores empty",
			content:   "SNAP:\nBranches: x\nAgents: y", // truncated block
			wantEmpty: true,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			id, err := db.InsertMessage(protocol.Message{
				FromAgent: "brian",
				Type:      protocol.MsgUpdate,
				Content:   tc.content,
			})
			if err != nil {
				t.Fatalf("InsertMessage: %v", err)
			}
			var got string
			if err := db.conn.QueryRow(`SELECT snap_json FROM messages WHERE id = ?`, id).Scan(&got); err != nil {
				t.Fatalf("scan snap_json: %v", err)
			}
			if tc.wantEmpty && got != "" {
				t.Errorf("snap_json = %q, want empty", got)
			}
			if !tc.wantEmpty && !strings.Contains(got, tc.wantSubstr) {
				t.Errorf("snap_json = %q, want substring %q", got, tc.wantSubstr)
			}
		})
	}
}

func TestInsertAndReadMessages(t *testing.T) {
	db := setupTestDB(t)

	db.RegisterAgent(protocol.Agent{ID: "sender", Name: "S", Type: protocol.AgentCoder, Status: protocol.StatusOnline})
	db.RegisterAgent(protocol.Agent{ID: "receiver", Name: "R", Type: protocol.AgentVoice, Status: protocol.StatusOnline})

	id, err := db.InsertMessage(protocol.Message{
		FromAgent: "sender",
		ToAgent:   "receiver",
		Type:      protocol.MsgQuestion,
		Content:   "Hello?",
		Created:   time.Now(),
	})
	if err != nil {
		t.Fatal(err)
	}
	if id <= 0 {
		t.Errorf("expected positive message ID, got %d", id)
	}

	msgs, err := db.ReadMessages("receiver", 0, 50)
	if err != nil {
		t.Fatal(err)
	}
	if len(msgs) != 1 {
		t.Fatalf("expected 1 message, got %d", len(msgs))
	}
	if msgs[0].Content != "Hello?" {
		t.Errorf("expected 'Hello?', got %q", msgs[0].Content)
	}
}

// TestReadMessagesTailContract locks the restart-context-recovery contract:
// sinceID<=0 returns the latest N rows in chronological order, sinceID>0 returns
// the next-after-sinceID rows in chronological order, sinceID==lastID returns
// empty (polling stability). Without these guarantees, fresh-start callers
// (agent boot, hub_read with no since_id) get oldest-first historical traffic
// instead of recent context.
func TestReadMessagesTailContract(t *testing.T) {
	db := setupTestDB(t)
	db.RegisterAgent(protocol.Agent{ID: "sender", Name: "S", Type: protocol.AgentCoder, Status: protocol.StatusOnline})

	const total = 100
	var maxID int64
	for i := 1; i <= total; i++ {
		id, err := db.InsertMessage(protocol.Message{
			FromAgent: "sender",
			Type:      protocol.MsgUpdate,
			Content:   "msg",
		})
		if err != nil {
			t.Fatal(err)
		}
		if id > maxID {
			maxID = id
		}
	}

	t.Run("sinceID=0 returns latest N chronological", func(t *testing.T) {
		msgs, err := db.ReadMessages("", 0, 50)
		if err != nil {
			t.Fatal(err)
		}
		if len(msgs) != 50 {
			t.Fatalf("len=%d, want 50", len(msgs))
		}
		if msgs[0].ID != maxID-49 {
			t.Errorf("first ID=%d, want %d (latest 50 starting from maxID-49)", msgs[0].ID, maxID-49)
		}
		if msgs[len(msgs)-1].ID != maxID {
			t.Errorf("last ID=%d, want %d (chronological order ends at maxID)", msgs[len(msgs)-1].ID, maxID)
		}
		for i := 1; i < len(msgs); i++ {
			if msgs[i].ID <= msgs[i-1].ID {
				t.Errorf("not chronological at i=%d: %d <= %d", i, msgs[i].ID, msgs[i-1].ID)
			}
		}
	})

	t.Run("sinceID=K returns next-after-K chronological", func(t *testing.T) {
		msgs, err := db.ReadMessages("", 50, 50)
		if err != nil {
			t.Fatal(err)
		}
		if len(msgs) != 50 {
			t.Fatalf("len=%d, want 50", len(msgs))
		}
		if msgs[0].ID != 51 {
			t.Errorf("first ID=%d, want 51 (next after sinceID=50)", msgs[0].ID)
		}
		if msgs[len(msgs)-1].ID != 100 {
			t.Errorf("last ID=%d, want 100", msgs[len(msgs)-1].ID)
		}
	})

	t.Run("sinceID=lastID returns empty (polling stability)", func(t *testing.T) {
		msgs, err := db.ReadMessages("", maxID, 50)
		if err != nil {
			t.Fatal(err)
		}
		if len(msgs) != 0 {
			t.Errorf("len=%d, want 0 (no spurious replay at watermark)", len(msgs))
		}
	})

	t.Run("sinceID<0 defensive: treated as tail", func(t *testing.T) {
		msgs, err := db.ReadMessages("", -5, 50)
		if err != nil {
			t.Fatal(err)
		}
		if len(msgs) != 50 {
			t.Fatalf("len=%d, want 50", len(msgs))
		}
		if msgs[len(msgs)-1].ID != maxID {
			t.Errorf("last ID=%d, want %d (tail mode on negative sinceID)", msgs[len(msgs)-1].ID, maxID)
		}
	})
}

// TestReadMessagesTailBoundaryExactN locks behavior when DB row count equals
// the requested limit. Boundary case for the restart-context-recovery contract.
func TestReadMessagesTailBoundaryExactN(t *testing.T) {
	db := setupTestDB(t)
	db.RegisterAgent(protocol.Agent{ID: "sender", Name: "S", Type: protocol.AgentCoder, Status: protocol.StatusOnline})

	const n = 50
	for i := 1; i <= n; i++ {
		if _, err := db.InsertMessage(protocol.Message{FromAgent: "sender", Type: protocol.MsgUpdate, Content: "msg"}); err != nil {
			t.Fatal(err)
		}
	}

	msgs, err := db.ReadMessages("", 0, n)
	if err != nil {
		t.Fatal(err)
	}
	if len(msgs) != n {
		t.Fatalf("len=%d, want %d (return all when row count == limit)", len(msgs), n)
	}
	if msgs[0].ID != 1 || msgs[len(msgs)-1].ID != int64(n) {
		t.Errorf("range=[%d,%d], want [1,%d]", msgs[0].ID, msgs[len(msgs)-1].ID, n)
	}
}

func TestCreateAndGetSession(t *testing.T) {
	db := setupTestDB(t)

	sess := protocol.Session{
		ID:      "sess-1",
		Mode:    protocol.ModeBrainstorm,
		Purpose: "fix login bug",
		Agents:  []string{"claude-abc", "live"},
		Status:  protocol.SessionActive,
	}

	if err := db.CreateSession(sess); err != nil {
		t.Fatal(err)
	}

	got, err := db.GetSession("sess-1")
	if err != nil {
		t.Fatal(err)
	}
	if got.Purpose != "fix login bug" {
		t.Errorf("expected purpose 'fix login bug', got %q", got.Purpose)
	}
	if len(got.Agents) != 2 {
		t.Errorf("expected 2 agents, got %d", len(got.Agents))
	}
}

func TestUpdateHook(t *testing.T) {
	db := setupTestDB(t)

	received := make(chan protocol.Message, 1)
	db.OnMessage(func(msg protocol.Message) {
		received <- msg
	})

	db.InsertMessage(protocol.Message{
		FromAgent: "test",
		ToAgent:   "other",
		Type:      protocol.MsgUpdate,
		Content:   "hook test",
		Created:   time.Now(),
	})

	select {
	case msg := <-received:
		if msg.Content != "hook test" {
			t.Errorf("expected 'hook test', got %q", msg.Content)
		}
	case <-time.After(2 * time.Second):
		t.Error("timed out waiting for update hook")
	}
}

func TestSaveAndGetCheckpoint(t *testing.T) {
	db := setupTestDB(t)

	data := `{"active_tasks":["task-1"],"context":"some state"}`
	if err := db.SaveCheckpoint("brian", data); err != nil {
		t.Fatal(err)
	}

	cp, err := db.GetCheckpoint("brian")
	if err != nil {
		t.Fatal(err)
	}
	if cp.AgentID != "brian" {
		t.Errorf("expected agent_id 'brian', got %q", cp.AgentID)
	}
	if cp.Data != data {
		t.Errorf("expected data %q, got %q", data, cp.Data)
	}
	if cp.Version != 1 {
		t.Errorf("expected version 1, got %d", cp.Version)
	}
	if cp.Created.IsZero() {
		t.Error("expected non-zero created time")
	}
}

func TestSaveCheckpointVersionIncrement(t *testing.T) {
	db := setupTestDB(t)

	db.SaveCheckpoint("brian", `{"v":1}`)
	cp1, _ := db.GetCheckpoint("brian")

	time.Sleep(10 * time.Millisecond)
	db.SaveCheckpoint("brian", `{"v":2}`)
	cp2, _ := db.GetCheckpoint("brian")

	if cp2.Version != 2 {
		t.Errorf("expected version 2, got %d", cp2.Version)
	}
	if cp2.Data != `{"v":2}` {
		t.Errorf("expected updated data, got %q", cp2.Data)
	}
	if !cp2.Created.Equal(cp1.Created) {
		t.Errorf("expected created to stay the same: got %v vs %v", cp1.Created, cp2.Created)
	}
	if !cp2.Updated.After(cp1.Updated) {
		t.Errorf("expected updated to advance")
	}
}

func TestGetCheckpointNotFound(t *testing.T) {
	db := setupTestDB(t)

	_, err := db.GetCheckpoint("nonexistent")
	if err == nil {
		t.Error("expected error for non-existent checkpoint")
	}
}

func TestDeleteCheckpoint(t *testing.T) {
	db := setupTestDB(t)

	db.SaveCheckpoint("brian", `{"x":1}`)
	if err := db.DeleteCheckpoint("brian"); err != nil {
		t.Fatal(err)
	}

	_, err := db.GetCheckpoint("brian")
	if err == nil {
		t.Error("expected error after deleting checkpoint")
	}
}

func TestSaveCheckpointInvalidJSON(t *testing.T) {
	db := setupTestDB(t)

	err := db.SaveCheckpoint("brian", "not json")
	if err == nil {
		t.Error("expected error for invalid JSON data")
	}
}

func TestUpdateAgentLastSeen(t *testing.T) {
	db := setupTestDB(t)
	agent := protocol.Agent{
		ID:      "lastseen-test",
		Name:    "Last Seen Test",
		Type:    protocol.AgentBrian,
		Status:  protocol.StatusOnline,
		Project: "/projects/test",
	}
	if err := db.RegisterAgent(agent); err != nil {
		t.Fatal(err)
	}
	initial, err := db.GetAgent("lastseen-test")
	if err != nil {
		t.Fatal(err)
	}

	// Wait for an observably newer timestamp at ms resolution.
	time.Sleep(5 * time.Millisecond)

	if err := db.UpdateAgentLastSeen("lastseen-test"); err != nil {
		t.Fatal(err)
	}

	after, err := db.GetAgent("lastseen-test")
	if err != nil {
		t.Fatal(err)
	}

	if !after.LastSeen.After(initial.LastSeen) {
		t.Errorf("LastSeen did not advance: initial=%v after=%v", initial.LastSeen, after.LastSeen)
	}
	// Status untouched — locks against the bug-pattern where status writes leaked
	// into recency updates (cf. claude_stop no-offline-flip discussion).
	if after.Status != protocol.StatusOnline {
		t.Errorf("Status mutated: got %q want %q", after.Status, protocol.StatusOnline)
	}
	if after.Project != "/projects/test" {
		t.Errorf("Project mutated: got %q want %q", after.Project, "/projects/test")
	}
	if after.Name != "Last Seen Test" {
		t.Errorf("Name mutated: got %q want %q", after.Name, "Last Seen Test")
	}
}

func TestUpdateAgentLastSeenUnknownID(t *testing.T) {
	db := setupTestDB(t)
	// Unknown ID → UPDATE matches zero rows, no error.
	if err := db.UpdateAgentLastSeen("nonexistent"); err != nil {
		t.Errorf("unexpected error for unknown id: %v", err)
	}
}

// Bug #4 cleanup: ReconcileCoderGhosts must flip ONLY coder agents, ONLY
// when status=online, ONLY when their paired session is stopped. Three
// conjoined predicates — easy to break one in a refactor without noticing.
// Test cases lock each predicate independently.
func TestReconcileCoderGhosts_FlipsStoppedSessionAgents(t *testing.T) {
	db := setupTestDB(t)

	// Two ghosts: coder + online + stopped session → should flip
	for _, id := range []string{"ghost1", "ghost2"} {
		if err := db.InsertClaudeSession(ClaudeSession{
			ID: id, Project: "/tmp", TmuxTarget: "cc-" + id,
			Mode: "managed", Status: "stopped",
		}); err != nil {
			t.Fatal(err)
		}
		if err := db.RegisterAgent(protocol.Agent{
			ID: id, Name: id, Type: protocol.AgentCoder, Status: protocol.StatusOnline,
		}); err != nil {
			t.Fatal(err)
		}
	}

	// Healthy coder: coder + online + running session → must NOT flip
	if err := db.InsertClaudeSession(ClaudeSession{
		ID: "healthy", Project: "/tmp", TmuxTarget: "cc-healthy",
		Mode: "managed", Status: "running",
	}); err != nil {
		t.Fatal(err)
	}
	if err := db.RegisterAgent(protocol.Agent{
		ID: "healthy", Name: "Healthy", Type: protocol.AgentCoder, Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}

	// Non-coder agent with stopped session: type predicate guards this →
	// must NOT flip. (Contrived: voice/brian/discord don't normally have
	// claude_sessions rows, but the SQL must guard regardless.)
	if err := db.InsertClaudeSession(ClaudeSession{
		ID: "voice1", Project: "/tmp", TmuxTarget: "cc-voice1",
		Mode: "attached", Status: "stopped",
	}); err != nil {
		t.Fatal(err)
	}
	if err := db.RegisterAgent(protocol.Agent{
		ID: "voice1", Name: "V1", Type: protocol.AgentVoice, Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}

	// Already-offline coder with stopped session: status predicate guards
	// this → no-op (already offline, not a ghost).
	if err := db.InsertClaudeSession(ClaudeSession{
		ID: "alreadyoff", Project: "/tmp", TmuxTarget: "cc-alreadyoff",
		Mode: "managed", Status: "stopped",
	}); err != nil {
		t.Fatal(err)
	}
	if err := db.RegisterAgent(protocol.Agent{
		ID: "alreadyoff", Name: "Off", Type: protocol.AgentCoder, Status: protocol.StatusOffline,
	}); err != nil {
		t.Fatal(err)
	}

	n, err := db.ReconcileCoderGhosts()
	if err != nil {
		t.Fatalf("ReconcileCoderGhosts error: %v", err)
	}
	if n != 2 {
		t.Errorf("flipped count: got %d, want 2 (ghost1 + ghost2)", n)
	}

	// Verify each predicate independently
	for _, id := range []string{"ghost1", "ghost2"} {
		a, _ := db.GetAgent(id)
		if a.Status != protocol.StatusOffline {
			t.Errorf("%s: status got %q, want offline", id, a.Status)
		}
	}
	if a, _ := db.GetAgent("healthy"); a.Status != protocol.StatusOnline {
		t.Errorf("healthy coder flipped erroneously: status=%q (running session must not be touched)", a.Status)
	}
	if a, _ := db.GetAgent("voice1"); a.Status != protocol.StatusOnline {
		t.Errorf("voice agent flipped erroneously: status=%q (type predicate failed)", a.Status)
	}
	if a, _ := db.GetAgent("alreadyoff"); a.Status != protocol.StatusOffline {
		t.Errorf("alreadyoff coder: status=%q (should still be offline)", a.Status)
	}

	// Idempotency: second call should flip zero rows.
	n2, err := db.ReconcileCoderGhosts()
	if err != nil {
		t.Fatalf("second call error: %v", err)
	}
	if n2 != 0 {
		t.Errorf("second call flipped %d rows, want 0 (not idempotent)", n2)
	}
}

// Empty DB: no rows to reconcile, must not error and must return 0.
func TestReconcileCoderGhosts_EmptyDB(t *testing.T) {
	db := setupTestDB(t)
	n, err := db.ReconcileCoderGhosts()
	if err != nil {
		t.Errorf("unexpected error on empty DB: %v", err)
	}
	if n != 0 {
		t.Errorf("flipped count on empty DB: got %d, want 0", n)
	}
}

// D2 cleanup: a coder agent with no claude_session row but a last_seen
// older than staleCoderCutoff is also a ghost — pre-bug-#4 leak rows
// have no session marker but never get re-touched, so they accumulate
// at status=online forever. Stale path must catch them.
func TestReconcileCoderGhosts_FlipsStaleCoder(t *testing.T) {
	db := setupTestDB(t)

	if err := db.RegisterAgent(protocol.Agent{
		ID: "stalecoder", Name: "Stale", Type: protocol.AgentCoder, Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}
	// Backdate last_seen well past the cutoff (8 days ago).
	stale := time.Now().Add(-8 * 24 * time.Hour).UnixMilli()
	if _, err := db.conn.Exec(`UPDATE agents SET last_seen = ? WHERE id = ?`, stale, "stalecoder"); err != nil {
		t.Fatal(err)
	}

	// Fresh coder with no session: must NOT flip (last_seen recent, no
	// session marker — the regular path doesn't touch it either).
	if err := db.RegisterAgent(protocol.Agent{
		ID: "freshcoder", Name: "Fresh", Type: protocol.AgentCoder, Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}

	// Stale non-coder: type predicate must guard against this.
	if err := db.RegisterAgent(protocol.Agent{
		ID: "stalevoice", Name: "Voice", Type: protocol.AgentVoice, Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := db.conn.Exec(`UPDATE agents SET last_seen = ? WHERE id = ?`, stale, "stalevoice"); err != nil {
		t.Fatal(err)
	}

	n, err := db.ReconcileCoderGhosts()
	if err != nil {
		t.Fatalf("ReconcileCoderGhosts error: %v", err)
	}
	if n != 1 {
		t.Errorf("flipped count: got %d, want 1 (stalecoder only)", n)
	}
	if a, _ := db.GetAgent("stalecoder"); a.Status != protocol.StatusOffline {
		t.Errorf("stalecoder: status got %q, want offline", a.Status)
	}
	if a, _ := db.GetAgent("freshcoder"); a.Status != protocol.StatusOnline {
		t.Errorf("freshcoder flipped erroneously: status=%q (last_seen recent)", a.Status)
	}
	if a, _ := db.GetAgent("stalevoice"); a.Status != protocol.StatusOnline {
		t.Errorf("stalevoice flipped erroneously: status=%q (type predicate failed)", a.Status)
	}
}

// --- Phase H slice 3 C1 (#7) wake_schedule ---

func TestWakeScheduleMigrationCreatesTableAndIndex(t *testing.T) {
	db := setupTestDB(t)
	// Table exists (PRAGMA table_info returns rows).
	rows, err := db.conn.Query(`PRAGMA table_info(wake_schedule)`)
	if err != nil {
		t.Fatalf("pragma: %v", err)
	}
	cols := map[string]bool{}
	for rows.Next() {
		var cid int
		var name, typ string
		var notnull, pk int
		var dflt sql.NullString
		if err := rows.Scan(&cid, &name, &typ, &notnull, &dflt, &pk); err != nil {
			t.Fatalf("scan: %v", err)
		}
		cols[name] = true
	}
	rows.Close()
	for _, want := range []string{"id", "target_agent", "fire_at", "payload", "created_by", "created_at", "fired_at", "fire_status"} {
		if !cols[want] {
			t.Errorf("wake_schedule missing column %q", want)
		}
	}
	// Index exists under either the partial-index name or the fallback name.
	idxRows, err := db.conn.Query(`SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='wake_schedule'`)
	if err != nil {
		t.Fatalf("sqlite_master: %v", err)
	}
	defer idxRows.Close()
	found := false
	for idxRows.Next() {
		var name string
		if err := idxRows.Scan(&name); err != nil {
			t.Fatal(err)
		}
		if name == "idx_wake_schedule_pending_fire_at" || name == "idx_wake_schedule_status_fire_at" {
			found = true
		}
	}
	if !found {
		t.Errorf("wake_schedule index missing — expected partial or fallback variant")
	}
	// Idempotent.
	if err := db.migrate(); err != nil {
		t.Fatalf("re-migrate: %v", err)
	}
}

func TestSqliteSupportsPartialIndex(t *testing.T) {
	cases := map[string]bool{
		"3.45.1": true,
		"3.8.0":  true,
		"3.8.5":  true,
		"3.7.17": false,
		"3.7.0":  false,
		"2.9.9":  false,
		"4.0.0":  true,
		"":       false,
		"foo":    false,
	}
	for ver, want := range cases {
		if got := sqliteSupportsPartialIndex(ver); got != want {
			t.Errorf("sqliteSupportsPartialIndex(%q) = %v, want %v", ver, got, want)
		}
	}
}

func TestWakeScheduleInsertAndList(t *testing.T) {
	db := setupTestDB(t)
	now := time.Now()

	// Two pending wakes — one due, one future.
	dueID, err := db.InsertWakeSchedule("brian", "rain", "wake-up payload", now.Add(-1*time.Second))
	if err != nil {
		t.Fatalf("insert due: %v", err)
	}
	if _, err := db.InsertWakeSchedule("rain", "brian", "future payload", now.Add(1*time.Hour)); err != nil {
		t.Fatalf("insert future: %v", err)
	}

	pending, err := db.ListPendingWakes(now)
	if err != nil {
		t.Fatalf("list pending: %v", err)
	}
	if len(pending) != 1 {
		t.Fatalf("ListPendingWakes returned %d rows, want 1 (only the due wake)", len(pending))
	}
	if pending[0].ID != dueID {
		t.Errorf("expected due wake id=%d, got id=%d", dueID, pending[0].ID)
	}
	if pending[0].TargetAgent != "brian" || pending[0].Payload != "wake-up payload" {
		t.Errorf("row contents drifted: target=%q payload=%q", pending[0].TargetAgent, pending[0].Payload)
	}
	if pending[0].FireStatus != WakeStatusPending {
		t.Errorf("status got %q, want pending", pending[0].FireStatus)
	}
}

func TestWakeScheduleStateMachine(t *testing.T) {
	db := setupTestDB(t)
	now := time.Now()

	mkPending := func(t *testing.T) int64 {
		t.Helper()
		id, err := db.InsertWakeSchedule("brian", "rain", "p", now.Add(-1*time.Second))
		if err != nil {
			t.Fatal(err)
		}
		return id
	}

	// pending → fired
	id := mkPending(t)
	ok, err := db.MarkWakeFired(id)
	if err != nil || !ok {
		t.Fatalf("MarkWakeFired: ok=%v err=%v", ok, err)
	}
	w, _ := db.GetWakeSchedule(id)
	if w.FireStatus != WakeStatusFired {
		t.Errorf("status after fire: %q", w.FireStatus)
	}
	if w.FiredAt.IsZero() {
		t.Error("fired_at should be set after MarkWakeFired")
	}
	// Re-firing an already-fired row is a no-op.
	if ok, _ := db.MarkWakeFired(id); ok {
		t.Error("MarkWakeFired on fired row should report false (terminal)")
	}

	// pending → failed
	id = mkPending(t)
	ok, err = db.MarkWakeFailed(id)
	if err != nil || !ok {
		t.Fatalf("MarkWakeFailed: ok=%v err=%v", ok, err)
	}
	w, _ = db.GetWakeSchedule(id)
	if w.FireStatus != WakeStatusFailed {
		t.Errorf("status after fail: %q", w.FireStatus)
	}
	if !w.FiredAt.IsZero() {
		t.Error("fired_at should remain zero on failed (per state-machine doc)")
	}

	// pending → cancelled, idempotent
	id = mkPending(t)
	ok, err = db.CancelWake(id)
	if err != nil || !ok {
		t.Fatalf("CancelWake first call: ok=%v err=%v", ok, err)
	}
	ok, err = db.CancelWake(id)
	if err != nil {
		t.Fatalf("CancelWake idempotent err: %v", err)
	}
	if ok {
		t.Error("CancelWake second call should report false (already cancelled, no transition)")
	}

	// Cancel after fire is also a no-op (terminal).
	id = mkPending(t)
	if _, err := db.MarkWakeFired(id); err != nil {
		t.Fatal(err)
	}
	ok, err = db.CancelWake(id)
	if err != nil || ok {
		t.Errorf("CancelWake on fired row: ok=%v err=%v (want false, nil)", ok, err)
	}
}

func TestWakeScheduleListExcludesTerminal(t *testing.T) {
	db := setupTestDB(t)
	now := time.Now()
	pendID, _ := db.InsertWakeSchedule("brian", "rain", "p", now.Add(-1*time.Second))
	firedID, _ := db.InsertWakeSchedule("brian", "rain", "f", now.Add(-1*time.Second))
	cancID, _ := db.InsertWakeSchedule("brian", "rain", "c", now.Add(-1*time.Second))
	if _, err := db.MarkWakeFired(firedID); err != nil {
		t.Fatal(err)
	}
	if _, err := db.CancelWake(cancID); err != nil {
		t.Fatal(err)
	}
	rows, err := db.ListPendingWakes(now)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || rows[0].ID != pendID {
		t.Errorf("ListPendingWakes after terminal flips: got %+v, want only id=%d", rows, pendID)
	}
}

func TestWakeScheduleCancelMissingID(t *testing.T) {
	db := setupTestDB(t)
	_, err := db.CancelWake(99999)
	if err != sql.ErrNoRows {
		t.Errorf("CancelWake on missing id: err=%v want sql.ErrNoRows", err)
	}
}

// --- Phase H slice 3 C3 (#2): atomic register-return watermark ---

// TestHubRegisterReturnsCurrentMaxMsgID locks the basic contract: after
// inserting N messages, RegisterAgentWithWatermark returns N as the watermark
// and persists it to last_seen_msg_id on the agent row.
func TestHubRegisterReturnsCurrentMaxMsgID(t *testing.T) {
	db := setupTestDB(t)
	const n = 5
	var lastID int64
	for i := 0; i < n; i++ {
		id, err := db.InsertMessage(protocol.Message{
			FromAgent: "tester",
			Type:      protocol.MsgUpdate,
			Content:   "msg",
		})
		if err != nil {
			t.Fatal(err)
		}
		lastID = id
	}

	wm, _, err := db.RegisterAgentWithWatermark(protocol.Agent{
		ID: "wm_basic", Name: "WM", Type: protocol.AgentBrian, Status: protocol.StatusOnline,
	})
	if err != nil {
		t.Fatal(err)
	}
	if wm != lastID {
		t.Errorf("watermark = %d, want %d (MAX(messages.id))", wm, lastID)
	}

	var stored int64
	if err := db.conn.QueryRow(`SELECT last_seen_msg_id FROM agents WHERE id = ?`, "wm_basic").Scan(&stored); err != nil {
		t.Fatal(err)
	}
	if stored != lastID {
		t.Errorf("last_seen_msg_id stored = %d, want %d", stored, lastID)
	}
}

// TestHubRegisterAtomicWithMaxMsgID exercises the race-window contract:
// concurrent InsertMessage + RegisterAgentWithWatermark must leave no message
// with id <= watermark unaccounted for. The returned watermark must be a
// valid prefix-id of the messages table — every msg.id <= watermark exists.
//
// SQLite write serialization (MaxOpenConns=1 + busy_timeout) plus the tx
// boundary in RegisterAgentWithWatermark guarantee this; the test confirms
// no committed message slips between SELECT MAX and the watermark commit.
func TestHubRegisterAtomicWithMaxMsgID(t *testing.T) {
	db := setupTestDB(t)
	// Pre-load some messages so the registers race with both pre-existing
	// and concurrently-inserted rows.
	for i := 0; i < 10; i++ {
		if _, err := db.InsertMessage(protocol.Message{FromAgent: "pre", Type: protocol.MsgUpdate, Content: "p"}); err != nil {
			t.Fatal(err)
		}
	}

	const inserters = 4
	const insertsPerWorker = 25
	const registers = 10

	var wg sync.WaitGroup
	startCh := make(chan struct{})

	wg.Add(inserters)
	for w := 0; w < inserters; w++ {
		go func() {
			defer wg.Done()
			<-startCh
			for i := 0; i < insertsPerWorker; i++ {
				if _, err := db.InsertMessage(protocol.Message{FromAgent: "race", Type: protocol.MsgUpdate, Content: "r"}); err != nil {
					t.Errorf("insert: %v", err)
					return
				}
			}
		}()
	}

	type wmResult struct {
		watermark int64
	}
	wmCh := make(chan wmResult, registers)
	wg.Add(registers)
	for r := 0; r < registers; r++ {
		go func(idx int) {
			defer wg.Done()
			<-startCh
			wm, _, err := db.RegisterAgentWithWatermark(protocol.Agent{
				ID:     fmt.Sprintf("wm_race_%d", idx),
				Name:   "WM",
				Type:   protocol.AgentCoder,
				Status: protocol.StatusOnline,
			})
			if err != nil {
				t.Errorf("register: %v", err)
				return
			}
			wmCh <- wmResult{watermark: wm}
		}(r)
	}

	close(startCh)
	wg.Wait()
	close(wmCh)

	// Every returned watermark must point at a message that exists in the
	// table — i.e. no register call returned a watermark <= MAX-at-that-time
	// while a concurrent insert was in-flight uncommitted.
	for r := range wmCh {
		if r.watermark <= 0 {
			t.Errorf("watermark = %d, want > 0 (pre-loaded msgs exist)", r.watermark)
			continue
		}
		var exists int
		err := db.conn.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = ?`, r.watermark).Scan(&exists)
		if err != nil {
			t.Fatal(err)
		}
		if exists != 1 {
			t.Errorf("watermark %d does not point at a real message row", r.watermark)
		}
	}
}

// TestHubRegisterRerunAdvancesWatermark exercises the rebuild semantics: an
// agent re-registering after additional messages have landed gets a strictly
// greater watermark, advancing the silent-discard horizon.
func TestHubRegisterRerunAdvancesWatermark(t *testing.T) {
	db := setupTestDB(t)
	for i := 0; i < 3; i++ {
		if _, err := db.InsertMessage(protocol.Message{FromAgent: "x", Type: protocol.MsgUpdate, Content: "a"}); err != nil {
			t.Fatal(err)
		}
	}
	wm1, _, err := db.RegisterAgentWithWatermark(protocol.Agent{
		ID: "wm_rerun", Name: "WM", Type: protocol.AgentBrian, Status: protocol.StatusOnline,
	})
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 4; i++ {
		if _, err := db.InsertMessage(protocol.Message{FromAgent: "x", Type: protocol.MsgUpdate, Content: "b"}); err != nil {
			t.Fatal(err)
		}
	}
	wm2, _, err := db.RegisterAgentWithWatermark(protocol.Agent{
		ID: "wm_rerun", Name: "WM", Type: protocol.AgentBrian, Status: protocol.StatusOnline,
	})
	if err != nil {
		t.Fatal(err)
	}
	if wm2 <= wm1 {
		t.Errorf("rerun watermark = %d, want > %d (first register)", wm2, wm1)
	}
}

// TestHaltStateMultiCauseAPI locks the slice-5 C1 (H-32+H-33) cause-keyed
// halt_state surface: independent SetHaltActive(cause), ClearHalt(cause),
// IsHalted, GetHaltCause. Each cause must coexist independently and a
// per-cause clear must not touch unrelated rows.
func TestHaltStateMultiCauseAPI(t *testing.T) {
	db := setupTestDB(t)

	if h, _ := db.IsHalted(); h {
		t.Fatalf("fresh DB must not be halted")
	}

	if err := db.SetHaltActive(HaltCauseContextCap, "ctx 95%", "emma"); err != nil {
		t.Fatal(err)
	}
	if err := db.SetHaltActive(HaltCausePlanCap, "plan 96%", "emma"); err != nil {
		t.Fatal(err)
	}
	if h, _ := db.IsHalted(); !h {
		t.Errorf("IsHalted must be true with two causes active")
	}

	ctxRow, ok, err := db.GetHaltCause(HaltCauseContextCap)
	if err != nil {
		t.Fatal(err)
	}
	if !ok || ctxRow.Reason != "ctx 95%" {
		t.Errorf("context-cap row = %+v ok=%v, want reason=%q", ctxRow, ok, "ctx 95%")
	}
	planRow, ok, err := db.GetHaltCause(HaltCausePlanCap)
	if err != nil {
		t.Fatal(err)
	}
	if !ok || planRow.Reason != "plan 96%" {
		t.Errorf("plan-cap row = %+v ok=%v, want reason=%q", planRow, ok, "plan 96%")
	}

	// Clear context-cap only — plan-cap row must survive.
	if err := db.ClearHalt(HaltCauseContextCap); err != nil {
		t.Fatal(err)
	}
	if _, ok, _ := db.GetHaltCause(HaltCauseContextCap); ok {
		t.Errorf("ClearHalt(context-cap) must delete that row")
	}
	if _, ok, _ := db.GetHaltCause(HaltCausePlanCap); !ok {
		t.Errorf("ClearHalt(context-cap) must NOT touch plan-cap row")
	}
	if h, _ := db.IsHalted(); !h {
		t.Errorf("IsHalted must still be true with plan-cap active")
	}

	// Manual clear wipes everything.
	if err := db.ClearHaltManually(); err != nil {
		t.Fatal(err)
	}
	if h, _ := db.IsHalted(); h {
		t.Errorf("ClearHaltManually must wipe all rows")
	}
}

// TestSetHaltActiveIdempotentUpsert locks the upsert semantic — repeated
// fires for the same cause refresh set_at/reason without producing a
// duplicate row.
func TestSetHaltActiveIdempotentUpsert(t *testing.T) {
	db := setupTestDB(t)
	if err := db.SetHaltActive(HaltCausePlanCap, "first", "emma"); err != nil {
		t.Fatal(err)
	}
	time.Sleep(2 * time.Millisecond)
	if err := db.SetHaltActive(HaltCausePlanCap, "second", "emma"); err != nil {
		t.Fatal(err)
	}
	row, ok, err := db.GetHaltCause(HaltCausePlanCap)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatalf("upsert must leave the cause active")
	}
	if row.Reason != "second" {
		t.Errorf("upsert must overwrite reason; got %q", row.Reason)
	}
}

// TestClearHaltIfTrioReregisteredScopedToContextCap locks the H-33
// scoping: the auto-clear path now only deletes the context-cap row,
// even when the plan-cap row is also active. Plan-cap clears organically
// via window-rollover or poll-shows-decay, not via re-register.
func TestClearHaltIfTrioReregisteredScopedToContextCap(t *testing.T) {
	db := setupTestDB(t)

	if err := db.SetHaltActive(HaltCauseContextCap, "ctx", "emma"); err != nil {
		t.Fatal(err)
	}
	if err := db.SetHaltActive(HaltCausePlanCap, "plan", "emma"); err != nil {
		t.Fatal(err)
	}
	time.Sleep(5 * time.Millisecond)

	for _, id := range HaltStateTrio {
		if err := db.RegisterAgent(protocol.Agent{
			ID: id, Name: id, Type: protocol.AgentBrian, Status: protocol.StatusOnline,
		}); err != nil {
			t.Fatal(err)
		}
	}
	cleared, err := db.ClearHaltIfTrioReregistered(HaltStateTrio)
	if err != nil {
		t.Fatal(err)
	}
	if !cleared {
		t.Fatalf("expected context-cap clear after trio re-register past set_at")
	}
	if _, ok, _ := db.GetHaltCause(HaltCauseContextCap); ok {
		t.Errorf("context-cap row must be deleted")
	}
	if _, ok, _ := db.GetHaltCause(HaltCausePlanCap); !ok {
		t.Errorf("plan-cap row must SURVIVE trio re-register (H-33 scoping)")
	}
}

// TestHaltStateMigrationFromLegacySchema locks the slice-5 C1 idempotent
// migration: a DB pre-seeded with the legacy (id=1, active flag) schema
// holding an active row must end up on the new (cause PRIMARY KEY)
// schema with the active row migrated to cause='context-cap'.
func TestHaltStateMigrationFromLegacySchema(t *testing.T) {
	dir := t.TempDir()
	dbPath := filepath.Join(dir, "test.db")

	// Phase 1: simulate a legacy DB by opening sqlite directly + writing
	// the old schema with an active row, then closing.
	legacy, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := legacy.Exec(`
		CREATE TABLE halt_state (
			id      INTEGER PRIMARY KEY CHECK (id = 1),
			active  INTEGER NOT NULL DEFAULT 0,
			set_by  TEXT NOT NULL DEFAULT '',
			set_at  INTEGER NOT NULL DEFAULT 0,
			reason  TEXT NOT NULL DEFAULT ''
		);
		INSERT INTO halt_state (id, active, set_by, set_at, reason)
		VALUES (1, 1, 'emma', 1700000000000, 'legacy ctx fire');
	`); err != nil {
		t.Fatal(err)
	}
	legacy.Close()

	// Phase 2: open via the production OpenDB, which runs migrate() and
	// MUST detect + rewrite the legacy schema in place.
	db, err := OpenDB(dbPath)
	if err != nil {
		t.Fatalf("OpenDB on legacy schema: %v", err)
	}
	t.Cleanup(func() { db.Close() })

	row, ok, err := db.GetHaltCause(HaltCauseContextCap)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatalf("legacy active row must migrate to cause=context-cap")
	}
	if row.Reason != "legacy ctx fire" {
		t.Errorf("migrated reason = %q, want 'legacy ctx fire'", row.Reason)
	}
	if row.SetBy != "emma" {
		t.Errorf("migrated set_by = %q, want 'emma'", row.SetBy)
	}

	// Phase 3: re-running migrate (simulated by closing+reopening) must
	// be a no-op — idempotent. The cause row must persist.
	db.Close()
	db2, err := OpenDB(dbPath)
	if err != nil {
		t.Fatalf("re-open after migration: %v", err)
	}
	defer db2.Close()
	if _, ok, _ := db2.GetHaltCause(HaltCauseContextCap); !ok {
		t.Errorf("context-cap row must survive re-run of migrate (idempotent)")
	}
}

// TestHaltStateMigrationLegacyInactive locks the inactive-row case: a
// legacy DB with active=0 must end up on the new schema with NO rows.
func TestHaltStateMigrationLegacyInactive(t *testing.T) {
	dir := t.TempDir()
	dbPath := filepath.Join(dir, "test.db")
	legacy, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := legacy.Exec(`
		CREATE TABLE halt_state (
			id      INTEGER PRIMARY KEY CHECK (id = 1),
			active  INTEGER NOT NULL DEFAULT 0,
			set_by  TEXT NOT NULL DEFAULT '',
			set_at  INTEGER NOT NULL DEFAULT 0,
			reason  TEXT NOT NULL DEFAULT ''
		);
		INSERT INTO halt_state (id, active) VALUES (1, 0);
	`); err != nil {
		t.Fatal(err)
	}
	legacy.Close()

	db, err := OpenDB(dbPath)
	if err != nil {
		t.Fatalf("OpenDB on legacy inactive schema: %v", err)
	}
	t.Cleanup(func() { db.Close() })

	if h, _ := db.IsHalted(); h {
		t.Errorf("legacy active=0 must migrate to zero-row halt_state; IsHalted=true")
	}
}
