package hub

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
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

// TestRecordDispatchDecisionEnvGate locks the no-op contract: when the
// env gate is unset, recordDispatchDecision must not create the diag
// directory or any files. Production default = no instrumentation.
func TestRecordDispatchDecisionEnvGate(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)
	t.Setenv(dispatchDecisionEnvVar, "")

	recordDispatchDecision(dispatchDecisionRecord{MsgID: 1, ToAgent: "rain", Outcome: "sent"})

	if _, err := os.Stat(filepath.Join(home, "diag")); !os.IsNotExist(err) {
		t.Errorf("diag dir should not exist when env gate unset; stat err = %v", err)
	}
}

// TestRecordDispatchDecisionWritesJSONL locks the JSONL append shape when
// the env gate is set: one record per line, RFC3339Nano timestamp,
// canonical field names, decision + outcome together.
func TestRecordDispatchDecisionWritesJSONL(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)
	t.Setenv(dispatchDecisionEnvVar, "1")

	recordDispatchDecision(dispatchDecisionRecord{
		MsgID:      42,
		ToAgent:    "coder-abc",
		TmuxTarget: "bot-hq-coder-abc:0.0",
		Ready:      true,
		LastLine:   "❯",
		Outcome:    "sent",
	})
	recordDispatchDecision(dispatchDecisionRecord{
		MsgID:      43,
		ToAgent:    "coder-abc",
		TmuxTarget: "bot-hq-coder-abc:0.0",
		Ready:      true,
		LastLine:   "❯",
		Outcome:    "send_keys_err",
		Err:        "tmux: connect failed",
	})

	path := filepath.Join(home, "diag", "dispatch-decisions.jsonl")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("expected JSONL at %s, err: %v", path, err)
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected 2 JSONL lines, got %d: %q", len(lines), data)
	}
	for i, line := range lines {
		var rec dispatchDecisionRecord
		if err := json.Unmarshal([]byte(line), &rec); err != nil {
			t.Errorf("line %d not valid JSON: %v (%q)", i, err, line)
		}
		if rec.TS == "" {
			t.Errorf("line %d missing ts", i)
		}
	}
	// Spot-check err propagation.
	if !strings.Contains(string(data), `"outcome":"send_keys_err"`) {
		t.Errorf("send_keys_err outcome missing from JSONL: %q", data)
	}
	if !strings.Contains(string(data), `"err":"tmux: connect failed"`) {
		t.Errorf("err field missing from JSONL: %q", data)
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

func TestIsReady(t *testing.T) {
	h := &Hub{}

	// Realistic capture of an idle trio agent pane: prompt-box bordered by
	// `──` lines, INSERT-mode footer below. The `✻ Crunched` line is the
	// most-recent turn's static summary glyph — must not flag busy.
	idleTrioPane := strings.Join([]string{
		"⏺ Brian's drive. Watching.",
		"",
		"✻ Crunched for 3s",
		"",
		"────────────────────────",
		"❯ ",
		"────────────────────────",
		"  -- INSERT -- ⏵⏵ bypass permissions on (shift+tab to cycle)",
		"",
	}, "\n")

	// Active synthesizing — `✶` glyph in the live spinner frame.
	activeSynthPane := strings.Join([]string{
		"⏺ Working through the change set.",
		"",
		"✶ Synthesizing… (49s · ↓ 2.9k tokens)",
		"",
		"────────────────────────",
		"❯ ",
		"────────────────────────",
		"  -- INSERT -- ⏵⏵ bypass permissions on (shift+tab to cycle)",
	}, "\n")

	// Active bash tool — `Running…` indented under the `⏺ Bash(...)` line.
	activeBashPane := strings.Join([]string{
		"⏺ Bash(cd ~/Projects/bot-hq && go test ./...)",
		"  ⎿  Running…",
		"",
		"────────────────────────",
		"❯ ",
		"────────────────────────",
		"  -- INSERT -- ⏵⏵ bypass permissions on (shift+tab to cycle)",
	}, "\n")

	// Regression-lock for line-prefix discipline: agent reply text contains
	// "Running…" as substring (e.g. quoting a log line). Must NOT false-busy
	// because the substring is not at line-start (after trim).
	quotedRunningPane := strings.Join([]string{
		"⏺ Investigating the failure log:",
		`  "[queue] Message 4123 to rain — Running… retry pending"`,
		"  Diagnosis complete.",
		"",
		"────────────────────────",
		"❯ ",
		"────────────────────────",
	}, "\n")

	tests := []struct {
		name   string
		output string
		want   bool
	}{
		// Pre-inversion test cases — kept for backwards-compat baseline.
		{"shell prompt $", "some output\n$ ", true},
		{"zsh prompt ❯", "some output\n❯", true},
		{"generic prompt >", "some output\n>", true},
		// Inversion philosophy: heuristic-miss → fail-safe-to-ready. Both of
		// the following were `false` under the old isAtPrompt (last-line check
		// failed because content didn't end with a known prompt suffix). Under
		// isReady, no busy-marker present → ready=true. The shift IS the fix.
		{"trailing newline only", "some output\n", true},
		{"unknown content no busy markers", "Processing your request\n  working on it", true},
		{"empty pane", "", true},
		{"prompt after output", "some output\n$ ", true},

		// New cases scoped to the H-22-bis incident + Rain's regression-locks.
		{"idle trio pane with INSERT footer + ✻ summary glyph", idleTrioPane, true},
		{"active synthesizing — ✶ glyph above prompt box", activeSynthPane, false},
		{"active bash — Running… line-prefix above prompt box", activeBashPane, false},
		{"quoted Running… inside agent reply (substring not prefix)", quotedRunningPane, true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := h.isReady(tt.output)
			if got != tt.want {
				t.Errorf("isReady(%q) = %v, want %v", tt.output, got, tt.want)
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
