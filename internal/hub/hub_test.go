package hub

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestHubStartIncrementsRebuildGen locks that NewHub bumps the rebuild
// generation once per call. Two consecutive NewHub calls against the same
// DB path should produce gen N then N+1. Phase G v1 #20.
func TestHubStartIncrementsRebuildGen(t *testing.T) {
	cfg := DefaultConfig()
	cfg.Hub.DBPath = filepath.Join(t.TempDir(), "rebuild_gen.db")

	h1, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}
	g1 := h1.RebuildGen
	if g1 != 1 {
		t.Errorf("first NewHub expected gen 1, got %d", g1)
	}
	h1.Stop()

	h2, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer h2.Stop()
	if h2.RebuildGen != 2 {
		t.Errorf("second NewHub expected gen 2, got %d", h2.RebuildGen)
	}
}

func TestHubDispatchToTmux(t *testing.T) {
	h := &Hub{}
	cmd := h.FormatTmuxMessage("claude-abc", protocol.Message{
		FromAgent: "live",
		Type:      protocol.MsgResponse,
		Content:   "JWT with refresh tokens",
	})
	if cmd == "" {
		t.Error("expected non-empty tmux command")
	}
}

func TestHubWSClientRegistration(t *testing.T) {
	cfg := DefaultConfig()
	cfg.Hub.DBPath = filepath.Join(t.TempDir(), "test.db")

	h, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Stop()

	ch := h.RegisterWSClient("live")
	if ch == nil {
		t.Fatal("expected non-nil channel")
	}

	// Send a message to the WS client
	go func() {
		h.dispatch(protocol.Message{
			FromAgent: "claude-abc",
			ToAgent:   "live",
			Type:      protocol.MsgResponse,
			Content:   "test message",
		})
	}()

	select {
	case msg := <-ch:
		if msg.Content != "test message" {
			t.Errorf("expected 'test message', got %q", msg.Content)
		}
	case <-time.After(2 * time.Second):
		t.Error("timed out waiting for WS dispatch")
	}

	h.UnregisterWSClient("live")
}

func TestHubBroadcast(t *testing.T) {
	cfg := DefaultConfig()
	cfg.Hub.DBPath = filepath.Join(t.TempDir(), "test.db")

	h, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Stop()

	ch1 := h.RegisterWSClient("client-1")
	ch2 := h.RegisterWSClient("client-2")

	// Broadcast (empty ToAgent)
	go func() {
		h.dispatch(protocol.Message{
			FromAgent: "sender",
			ToAgent:   "",
			Type:      protocol.MsgUpdate,
			Content:   "broadcast msg",
		})
	}()

	for _, ch := range []chan protocol.Message{ch1, ch2} {
		select {
		case msg := <-ch:
			if msg.Content != "broadcast msg" {
				t.Errorf("expected 'broadcast msg', got %q", msg.Content)
			}
		case <-time.After(2 * time.Second):
			t.Error("timed out waiting for broadcast")
		}
	}

	h.UnregisterWSClient("client-1")
	h.UnregisterWSClient("client-2")
}

func TestHubDispatchToCoderAgent(t *testing.T) {
	cfg := DefaultConfig()
	cfg.Hub.DBPath = filepath.Join(t.TempDir(), "test.db")

	h, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Stop()

	// Register a coder agent with a tmux target in meta
	h.DB.RegisterAgent(protocol.Agent{
		ID:     "claude-abc",
		Name:   "Claude ABC",
		Type:   protocol.AgentCoder,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"cc-abc123"}`,
	})

	// Dispatch a message to the coder — should not panic or error
	msg := protocol.Message{
		FromAgent: "user",
		ToAgent:   "claude-abc",
		Type:      protocol.MsgCommand,
		Content:   "hello claude",
	}
	h.dispatch(msg)
	// No panic = success
}

func TestMessageQueueEnqueue(t *testing.T) {
	db := setupTestDB(t)

	// Enqueue a message
	err := db.EnqueueMessage(42, "claude-abc", "cc-abc123", "[user] hello claude")
	if err != nil {
		t.Fatal(err)
	}

	// Verify it's pending
	pending, err := db.GetPendingMessages()
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 1 {
		t.Fatalf("expected 1 pending message, got %d", len(pending))
	}
	if pending[0].MessageID != 42 {
		t.Errorf("expected message_id 42, got %d", pending[0].MessageID)
	}
	if pending[0].TargetAgent != "claude-abc" {
		t.Errorf("expected target_agent 'claude-abc', got %q", pending[0].TargetAgent)
	}
	if pending[0].TmuxTarget != "cc-abc123" {
		t.Errorf("expected tmux_target 'cc-abc123', got %q", pending[0].TmuxTarget)
	}
	if pending[0].FormattedText != "[user] hello claude" {
		t.Errorf("expected formatted_text '[user] hello claude', got %q", pending[0].FormattedText)
	}
	if pending[0].Status != "pending" {
		t.Errorf("expected status 'pending', got %q", pending[0].Status)
	}
}

func TestMessageQueueDelivery(t *testing.T) {
	db := setupTestDB(t)

	// Enqueue a message
	err := db.EnqueueMessage(1, "claude-abc", "cc-abc123", "[user] test")
	if err != nil {
		t.Fatal(err)
	}

	// Mark as delivered
	pending, _ := db.GetPendingMessages()
	if len(pending) != 1 {
		t.Fatalf("expected 1 pending, got %d", len(pending))
	}

	err = db.UpdateQueueStatus(pending[0].ID, "delivered", 1)
	if err != nil {
		t.Fatal(err)
	}

	// Verify no more pending messages
	pending, err = db.GetPendingMessages()
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 0 {
		t.Errorf("expected 0 pending messages after delivery, got %d", len(pending))
	}
}

func TestMessageQueueMaxAttempts(t *testing.T) {
	db := setupTestDB(t)

	// Enqueue a message
	err := db.EnqueueMessage(99, "claude-xyz", "cc-xyz789", "[user] retry test")
	if err != nil {
		t.Fatal(err)
	}

	pending, _ := db.GetPendingMessages()
	if len(pending) != 1 {
		t.Fatalf("expected 1 pending, got %d", len(pending))
	}

	// Simulate max attempts reached — mark as failed
	err = db.UpdateQueueStatus(pending[0].ID, "failed", pending[0].MaxAttempts+1)
	if err != nil {
		t.Fatal(err)
	}

	// Verify no pending messages (failed is not pending)
	pending, err = db.GetPendingMessages()
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 0 {
		t.Errorf("expected 0 pending after max attempts, got %d", len(pending))
	}
}

// TestEmitRetryExhaustAlertInsertsBridgedMessage locks the slice-5
// H-22-bis bridge: hub's retry-exhaust event emits a synthetic
// protocol.Message that flows through Emma's sentinel pipeline (which
// reads protocol.Message content, not stdout logs). FromAgent must be
// the non-registered "hub" so Emma's source-filter does not suppress it.
func TestEmitRetryExhaustAlertInsertsBridgedMessage(t *testing.T) {
	db := setupTestDB(t)
	h := &Hub{DB: db}

	h.emitRetryExhaustAlert(42, "coder-abc123", 30)

	msgs, err := db.GetRecentMessages(10)
	if err != nil {
		t.Fatal(err)
	}
	var bridged *protocol.Message
	for i := range msgs {
		if msgs[i].FromAgent == retryExhaustFromAgent {
			bridged = &msgs[i]
			break
		}
	}
	if bridged == nil {
		t.Fatalf("expected synthetic bridge message from %q, got %d messages: %v", retryExhaustFromAgent, len(msgs), msgs)
	}
	want := "[queue] Message 42 to coder-abc123 failed after 30 attempts"
	if bridged.Content != want {
		t.Errorf("bridge content = %q, want %q", bridged.Content, want)
	}
	if bridged.Type != protocol.MsgUpdate {
		t.Errorf("bridge type = %q, want %q", bridged.Type, protocol.MsgUpdate)
	}
}

// TestRetryExhaustFromAgentNotRegistrable locks the source-filter
// interaction: bridge emit must use a from_agent that is never present
// in the agents table, otherwise Emma's source-filter (which excludes
// registered agents to defeat queueFailPattern prose FPs) would also
// suppress the bridge emit. Concretely: confirm "hub" is not registered
// at the time of an emit by leaving the agents table empty.
func TestRetryExhaustFromAgentNotRegistrable(t *testing.T) {
	db := setupTestDB(t)
	h := &Hub{DB: db}

	h.emitRetryExhaustAlert(7, "rain", 30)

	agents, err := db.ListAgents("")
	if err != nil {
		t.Fatal(err)
	}
	for _, a := range agents {
		if a.ID == retryExhaustFromAgent {
			t.Fatalf("retryExhaustFromAgent %q must not be in agents table; emit would be suppressed by Emma's source-filter", retryExhaustFromAgent)
		}
	}
}

func TestMessageQueueCleanup(t *testing.T) {
	db := setupTestDB(t)

	// Enqueue and immediately deliver
	err := db.EnqueueMessage(1, "claude-abc", "cc-abc", "[user] old msg")
	if err != nil {
		t.Fatal(err)
	}
	pending, _ := db.GetPendingMessages()
	db.UpdateQueueStatus(pending[0].ID, "delivered", 1)

	// Cleanup with 0 duration should remove it
	err = db.CleanDeliveredMessages(0)
	if err != nil {
		t.Fatal(err)
	}
}

func TestIsAtPrompt(t *testing.T) {
	h := &Hub{}

	tests := []struct {
		name   string
		output string
		want   bool
	}{
		{"shell prompt $", "some output\n$ ", true},
		{"zsh prompt ❯", "some output\n❯", true},
		{"generic prompt >", "some output\n>", true},
		{"trailing newline only", "some output\n", false},
		{"busy agent", "Processing your request...\n  working on it", false},
		{"empty pane", "", true},
		{"prompt after output", "some output\n$ ", true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := h.isAtPrompt(tt.output)
			if got != tt.want {
				t.Errorf("isAtPrompt(%q) = %v, want %v", tt.output, got, tt.want)
			}
		})
	}
}

func TestHubNewAndStop(t *testing.T) {
	cfg := DefaultConfig()
	cfg.Hub.DBPath = filepath.Join(t.TempDir(), "test.db")

	h, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}

	if h.DB == nil {
		t.Error("expected non-nil DB")
	}

	if err := h.Stop(); err != nil {
		t.Errorf("unexpected error on Stop: %v", err)
	}
}
