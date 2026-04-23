package mcp

import (
	"context"
	"encoding/json"
	"path/filepath"
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
