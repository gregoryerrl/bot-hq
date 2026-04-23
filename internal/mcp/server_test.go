package mcp

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/mark3labs/mcp-go/mcp"
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

func TestToolsRegistered(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)

	expected := []string{
		"hub_register",
		"hub_unregister",
		"hub_send",
		"hub_read",
		"hub_agents",
		"hub_sessions",
		"hub_session_create",
		"hub_session_join",
		"hub_status",
		"hub_spawn",
		"hub_checkpoint",
		"hub_restore",
	}

	if len(tools) != len(expected) {
		t.Fatalf("expected %d tools, got %d", len(expected), len(tools))
	}

	names := make(map[string]bool)
	for _, td := range tools {
		names[td.Tool.Name] = true
	}

	for _, name := range expected {
		if !names[name] {
			t.Errorf("missing tool: %s", name)
		}
	}
}

func TestHubRegisterTool(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)

	// Find the hub_register tool
	var registerHandler func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error)
	for _, td := range tools {
		if td.Tool.Name == "hub_register" {
			registerHandler = td.Handler
			break
		}
	}
	if registerHandler == nil {
		t.Fatal("hub_register tool not found")
	}

	// Call the handler
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":      "test-agent",
		"name":    "Test Agent",
		"type":    "coder",
		"project": "/projects/test",
	}

	result, err := registerHandler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("unexpected error: %v", result.Content)
	}

	// Verify the agent is in the database
	agent, err := db.GetAgent("test-agent")
	if err != nil {
		t.Fatal(err)
	}
	if agent.Name != "Test Agent" {
		t.Errorf("expected name 'Test Agent', got %q", agent.Name)
	}
	if agent.Type != protocol.AgentCoder {
		t.Errorf("expected type coder, got %s", agent.Type)
	}
	if agent.Status != protocol.StatusOnline {
		t.Errorf("expected status online, got %s", agent.Status)
	}
	if agent.Project != "/projects/test" {
		t.Errorf("expected project '/projects/test', got %q", agent.Project)
	}
}

func TestHubSendAndReadTools(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)

	// Find handlers
	handlers := make(map[string]func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error))
	for _, td := range tools {
		handlers[td.Tool.Name] = td.Handler
	}

	ctx := context.Background()

	// Register two agents
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":   "alice",
		"name": "Alice",
		"type": "coder",
	}
	result, err := handlers["hub_register"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("register alice failed: %v", result.Content)
	}

	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":   "bob",
		"name": "Bob",
		"type": "voice",
	}
	result, err = handlers["hub_register"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("register bob failed: %v", result.Content)
	}

	// Send a message from alice to bob
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":    "alice",
		"to":      "bob",
		"type":    "question",
		"content": "Hello Bob!",
	}
	result, err = handlers["hub_send"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("send failed: %v", result.Content)
	}

	// Verify send returned a message_id
	var sendResult map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &sendResult); err != nil {
			t.Fatal(err)
		}
		if sendResult["status"] != "sent" {
			t.Errorf("expected status 'sent', got %v", sendResult["status"])
		}
	}

	// Read messages for bob
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"agent_id": "bob",
	}
	result, err = handlers["hub_read"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("read failed: %v", result.Content)
	}

	// Parse the messages
	var msgs []protocol.Message
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &msgs); err != nil {
			t.Fatal(err)
		}
	}

	if len(msgs) != 1 {
		t.Fatalf("expected 1 message, got %d", len(msgs))
	}
	if msgs[0].Content != "Hello Bob!" {
		t.Errorf("expected 'Hello Bob!', got %q", msgs[0].Content)
	}
	if msgs[0].FromAgent != "alice" {
		t.Errorf("expected from 'alice', got %q", msgs[0].FromAgent)
	}
}

func findHandler(tools []ToolDef, name string) func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error) {
	for _, td := range tools {
		if td.Tool.Name == name {
			return td.Handler
		}
	}
	return nil
}

func TestCheckpointSelfOnly(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	handler := findHandler(tools, "hub_checkpoint")
	if handler == nil {
		t.Fatal("hub_checkpoint tool not found")
	}
	ctx := context.Background()

	// Self-checkpoint should succeed
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":     "brain",
		"agent_id": "brain",
		"data":     `{"state":"ok"}`,
	}
	result, err := handler(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("self-checkpoint should succeed, got error: %v", result.Content)
	}

	// Cross-agent checkpoint should be rejected
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":     "coder-1",
		"agent_id": "brain",
		"data":     `{"hijack":true}`,
	}
	result, err = handler(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if !result.IsError {
		t.Fatal("cross-agent checkpoint should be rejected")
	}
	tc, ok := result.Content[0].(mcp.TextContent)
	if !ok || !strings.Contains(tc.Text, "agents can only checkpoint their own state") {
		t.Errorf("expected self-only error message, got %v", result.Content)
	}
}

func TestCheckpointSizeLimit(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	handler := findHandler(tools, "hub_checkpoint")
	if handler == nil {
		t.Fatal("hub_checkpoint tool not found")
	}
	ctx := context.Background()

	// Build data just over 1MB
	bigValue := strings.Repeat("x", 1_000_001-len(`{"big":""}`))
	bigData := `{"big":"` + bigValue + `"}`

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":     "brain",
		"agent_id": "brain",
		"data":     bigData,
	}
	result, err := handler(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if !result.IsError {
		t.Fatal("checkpoint over 1MB should be rejected")
	}
	tc, ok := result.Content[0].(mcp.TextContent)
	if !ok || !strings.Contains(tc.Text, "checkpoint data exceeds 1MB limit") {
		t.Errorf("expected size limit error, got %v", result.Content)
	}

	// Data exactly at 1MB should succeed
	exactValue := strings.Repeat("x", 1_000_000-len(`{"ok":""}`))
	exactData := `{"ok":"` + exactValue + `"}`

	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":     "brain",
		"agent_id": "brain",
		"data":     exactData,
	}
	result, err = handler(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("checkpoint at exactly 1MB should succeed, got: %v", result.Content)
	}
}

func TestRestoreCrossAgent(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	checkpointHandler := findHandler(tools, "hub_checkpoint")
	restoreHandler := findHandler(tools, "hub_restore")
	if checkpointHandler == nil || restoreHandler == nil {
		t.Fatal("checkpoint/restore tools not found")
	}
	ctx := context.Background()

	// Agent "brain" saves its own checkpoint
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":     "brain",
		"agent_id": "brain",
		"data":     `{"tasks":["t1","t2"]}`,
	}
	result, err := checkpointHandler(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("checkpoint failed: %v", result.Content)
	}

	// Different agent "coder-1" restores brain's checkpoint (should succeed)
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"agent_id": "brain",
	}
	result, err = restoreHandler(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("cross-agent restore should succeed, got error: %v", result.Content)
	}

	var restored map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &restored); err != nil {
			t.Fatal(err)
		}
	}
	if restored["status"] != "restored" {
		t.Errorf("expected status 'restored', got %v", restored["status"])
	}
	if restored["data"] != `{"tasks":["t1","t2"]}` {
		t.Errorf("unexpected restored data: %v", restored["data"])
	}
}
