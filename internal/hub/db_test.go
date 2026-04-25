package hub

import (
	"path/filepath"
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
