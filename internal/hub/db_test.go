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
