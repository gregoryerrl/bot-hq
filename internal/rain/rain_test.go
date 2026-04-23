package rain

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func setupTestDB(t *testing.T) *hub.DB {
	t.Helper()
	path := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

// 1. TestNew_DefaultWorkDir — New(db, "") → workDir should be ~/Projects
func TestNew_DefaultWorkDir(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "")

	home, err := os.UserHomeDir()
	if err != nil {
		// If UserHomeDir fails, fallback is os.TempDir()
		home = os.TempDir()
	}
	expected := filepath.Join(home, "Projects")

	if r.workDir != expected {
		t.Errorf("expected workDir %q, got %q", expected, r.workDir)
	}
}

// 2. TestNew_CustomWorkDir — New(db, "/tmp/foo") → workDir = "/tmp/foo"
func TestNew_CustomWorkDir(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/foo")

	if r.workDir != "/tmp/foo" {
		t.Errorf("expected workDir %q, got %q", "/tmp/foo", r.workDir)
	}
}

// 3. TestNew_FieldsInitialized — stopCh not nil, running=false, lastMsgID=0
func TestNew_FieldsInitialized(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	if r.stopCh == nil {
		t.Error("expected stopCh to be non-nil")
	}
	if r.running {
		t.Error("expected running to be false")
	}
	if r.lastMsgID != 0 {
		t.Errorf("expected lastMsgID 0, got %d", r.lastMsgID)
	}
}

// 4. TestIsRunning_Default — false on fresh instance
func TestIsRunning_Default(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	if r.IsRunning() {
		t.Error("expected IsRunning() to return false on fresh instance")
	}
}

// 5. TestStop_NotRunning_NoOp — call Stop() on fresh instance, no panic
func TestStop_NotRunning_NoOp(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	// Should not panic
	r.Stop()

	if r.IsRunning() {
		t.Error("expected IsRunning() to return false after Stop() on fresh instance")
	}
}

// 6. TestFormatRainNudge_BasicFormat — verify "[Hub message from X]: Y" structure
func TestFormatRainNudge_BasicFormat(t *testing.T) {
	result := formatRainNudge("brain", "Please review the code")

	if !strings.Contains(result, "[Hub message from brain]: Please review the code") {
		t.Errorf("expected nudge to contain message header, got %q", result)
	}
	if !strings.Contains(result, `to="brain"`) {
		t.Errorf("expected nudge to contain routing instruction with to=\"brain\", got %q", result)
	}
	if !strings.Contains(result, "hub_send") {
		t.Errorf("expected nudge to mention hub_send, got %q", result)
	}
	if !strings.Contains(result, "hub_flag") {
		t.Errorf("expected nudge to mention hub_flag, got %q", result)
	}
}

// 7. TestFormatRainNudge_EmptyContent — handles empty string
func TestFormatRainNudge_EmptyContent(t *testing.T) {
	result := formatRainNudge("brain", "")

	if !strings.Contains(result, "[Hub message from brain]: ") {
		t.Errorf("expected nudge to handle empty content, got %q", result)
	}
	// Should not panic and should still contain routing instructions
	if !strings.Contains(result, `to="brain"`) {
		t.Errorf("expected routing instructions even with empty content, got %q", result)
	}
}

// 8. TestFormatRainNudge_SpecialChars — quotes, newlines in content
func TestFormatRainNudge_SpecialChars(t *testing.T) {
	content := "He said \"hello\"\nand then\ttabs"
	result := formatRainNudge("user", content)

	if !strings.Contains(result, `"hello"`) {
		t.Errorf("expected nudge to preserve quotes, got %q", result)
	}
	if !strings.Contains(result, "\n") {
		t.Errorf("expected nudge to preserve newlines, got %q", result)
	}
	if !strings.Contains(result, "\t") {
		t.Errorf("expected nudge to preserve tabs, got %q", result)
	}
	if !strings.Contains(result, `to="user"`) {
		t.Errorf("expected routing to user, got %q", result)
	}
}

// 9. TestWriteMCPConfig_JSONStructure — create Rain with t.TempDir() workDir,
// call writeMCPConfig(), read and parse the JSON file, verify structure.
func TestWriteMCPConfig_JSONStructure(t *testing.T) {
	db := setupTestDB(t)
	tmpDir := t.TempDir()
	r := New(db, tmpDir)

	if err := r.writeMCPConfig(); err != nil {
		t.Fatal(err)
	}

	configPath := filepath.Join(tmpDir, ".bot-hq-rain-mcp.json")
	data, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatalf("failed to read config file: %v", err)
	}

	var config map[string]any
	if err := json.Unmarshal(data, &config); err != nil {
		t.Fatalf("failed to parse JSON: %v", err)
	}

	mcpServers, ok := config["mcpServers"].(map[string]any)
	if !ok {
		t.Fatal("expected mcpServers key in config")
	}

	botHQ, ok := mcpServers["bot-hq"].(map[string]any)
	if !ok {
		t.Fatal("expected bot-hq key in mcpServers")
	}

	if _, ok := botHQ["command"]; !ok {
		t.Error("expected command field in bot-hq config")
	}

	args, ok := botHQ["args"]
	if !ok {
		t.Fatal("expected args field in bot-hq config")
	}

	argsList, ok := args.([]any)
	if !ok {
		t.Fatalf("expected args to be an array, got %T", args)
	}

	if len(argsList) != 1 || argsList[0] != "mcp" {
		t.Errorf("expected args=[\"mcp\"], got %v", argsList)
	}

	// Verify file permissions
	info, err := os.Stat(configPath)
	if err != nil {
		t.Fatal(err)
	}
	perm := info.Mode().Perm()
	if perm != 0600 {
		t.Errorf("expected file permission 0600, got %04o", perm)
	}
}

// 10. TestProcessNewMessages_SkipsSelf — insert a message from "rain" to "rain",
// call processNewMessages, verify no SendCommand attempt.
func TestProcessNewMessages_SkipsSelf(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, t.TempDir())

	// Register rain agent so messages can be addressed to it
	db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
	})

	// Insert a message from rain to rain
	_, err := db.InsertMessage(protocol.Message{
		FromAgent: "rain",
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   "self-message that should be skipped",
		Created:   time.Now(),
	})
	if err != nil {
		t.Fatal(err)
	}

	// processNewMessages should skip messages from self.
	// Since Rain is not running (no tmux), SendCommand would return an error.
	// If the skip logic works, SendCommand is never called and no error occurs.
	// We verify by checking that lastMsgID advances (message was seen) but
	// no panic/error from trying to send to a non-existent tmux session.
	r.processNewMessages()

	if r.lastMsgID == 0 {
		t.Error("expected lastMsgID to advance after processing messages")
	}
}
