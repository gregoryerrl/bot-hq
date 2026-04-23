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
