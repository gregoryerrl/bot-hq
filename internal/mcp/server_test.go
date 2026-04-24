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
		"hub_delete_agent",
		"hub_flag",
		"hub_send",
		"hub_read",
		"hub_agents",
		"hub_sessions",
		"hub_session_create",
		"hub_session_join",
		"hub_status",
		"hub_spawn",
		"hub_spawn_gemma",
		"hub_checkpoint",
		"hub_restore",
		"hub_issue_create",
		"hub_issue_list",
		"hub_issue_update",
		"claude_list",
		"claude_read",
		"claude_message",
		"claude_send",
		"claude_resume",
		"claude_stop",
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

func TestIssueCreateAndList(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	handlers := make(map[string]func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error))
	for _, td := range tools {
		handlers[td.Tool.Name] = td.Handler
	}
	ctx := context.Background()

	// Test creating an issue with all fields (including line_number)
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"reporter":    "coder-1",
		"severity":    "high",
		"title":       "Null pointer in handler",
		"description": "Crashes on nil input",
		"file_path":   "internal/hub/db.go",
		"line_number": float64(42),
	}
	result, err := handlers["hub_issue_create"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("create issue with line_number failed: %v", result.Content)
	}
	var issue1 map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &issue1); err != nil {
			t.Fatal(err)
		}
	}
	if issue1["title"] != "Null pointer in handler" {
		t.Errorf("expected title 'Null pointer in handler', got %v", issue1["title"])
	}
	if issue1["line_number"] != float64(42) {
		t.Errorf("expected line_number 42, got %v", issue1["line_number"])
	}
	if issue1["status"] != "open" {
		t.Errorf("expected status 'open', got %v", issue1["status"])
	}
	if issue1["assigned_to"] != "" {
		t.Errorf("expected empty assigned_to, got %v", issue1["assigned_to"])
	}
	if issue1["resolution"] != "" {
		t.Errorf("expected empty resolution, got %v", issue1["resolution"])
	}

	// Test creating an issue without optional fields (no line_number, no description, no file_path)
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"reporter": "brain",
		"severity": "low",
		"title":    "Minor typo in docs",
	}
	result, err = handlers["hub_issue_create"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("create issue without optionals failed: %v", result.Content)
	}
	var issue2 map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &issue2); err != nil {
			t.Fatal(err)
		}
	}
	// Verify line_number is nil/null when not provided
	if issue2["line_number"] != nil {
		t.Errorf("expected line_number nil when not provided, got %v", issue2["line_number"])
	}

	// Test listing all issues
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{}
	result, err = handlers["hub_issue_list"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("list issues failed: %v", result.Content)
	}
	var allIssues []map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &allIssues); err != nil {
			t.Fatal(err)
		}
	}
	if len(allIssues) != 2 {
		t.Fatalf("expected 2 issues, got %d", len(allIssues))
	}

	// Test listing with severity filter
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"severity": "high"}
	result, err = handlers["hub_issue_list"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	var highIssues []map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &highIssues); err != nil {
			t.Fatal(err)
		}
	}
	if len(highIssues) != 1 {
		t.Fatalf("expected 1 high issue, got %d", len(highIssues))
	}

	// Test listing with reporter filter
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"reporter": "brain"}
	result, err = handlers["hub_issue_list"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	var brainIssues []map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &brainIssues); err != nil {
			t.Fatal(err)
		}
	}
	if len(brainIssues) != 1 {
		t.Fatalf("expected 1 brain issue, got %d", len(brainIssues))
	}

	// Test listing with status filter
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"status": "open"}
	result, err = handlers["hub_issue_list"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	var openIssues []map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &openIssues); err != nil {
			t.Fatal(err)
		}
	}
	if len(openIssues) != 2 {
		t.Fatalf("expected 2 open issues, got %d", len(openIssues))
	}

	// Test empty result when no matches
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"severity": "critical"}
	result, err = handlers["hub_issue_list"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	var noIssues []map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &noIssues); err != nil {
			// null JSON decodes to nil slice, which is fine
			noIssues = nil
		}
	}
	if len(noIssues) != 0 {
		t.Fatalf("expected 0 issues for critical, got %d", len(noIssues))
	}

	// Verify line_number is nil in listed issue without line_number
	for _, iss := range allIssues {
		if iss["title"] == "Minor typo in docs" {
			if iss["line_number"] != nil {
				t.Errorf("expected nil line_number in listed issue, got %v", iss["line_number"])
			}
		}
		if iss["title"] == "Null pointer in handler" {
			if iss["line_number"] != float64(42) {
				t.Errorf("expected line_number 42 in listed issue, got %v", iss["line_number"])
			}
		}
	}
}

func TestIssueUpdate(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	handlers := make(map[string]func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error))
	for _, td := range tools {
		handlers[td.Tool.Name] = td.Handler
	}
	ctx := context.Background()

	// Create an issue first
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"reporter": "coder-1",
		"severity": "medium",
		"title":    "Fix login bug",
	}
	result, err := handlers["hub_issue_create"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	var created map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &created); err != nil {
			t.Fatal(err)
		}
	}
	issueID := created["id"].(string)

	// Update status to "in_progress"
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":     issueID,
		"status": "in_progress",
	}
	result, err = handlers["hub_issue_update"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("update status failed: %v", result.Content)
	}
	var updated map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &updated); err != nil {
			t.Fatal(err)
		}
	}
	if updated["status"] != "in_progress" {
		t.Errorf("expected status 'in_progress', got %v", updated["status"])
	}

	// Update assigned_to
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":          issueID,
		"assigned_to": "coder-2",
	}
	result, err = handlers["hub_issue_update"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("update assigned_to failed: %v", result.Content)
	}
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &updated); err != nil {
			t.Fatal(err)
		}
	}
	if updated["assigned_to"] != "coder-2" {
		t.Errorf("expected assigned_to 'coder-2', got %v", updated["assigned_to"])
	}

	// Update status to "fixed" with resolution
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":         issueID,
		"status":     "fixed",
		"resolution": "Added nil check in handler",
	}
	result, err = handlers["hub_issue_update"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("update to fixed failed: %v", result.Content)
	}
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &updated); err != nil {
			t.Fatal(err)
		}
	}
	if updated["status"] != "fixed" {
		t.Errorf("expected status 'fixed', got %v", updated["status"])
	}
	if updated["resolution"] != "Added nil check in handler" {
		t.Errorf("expected resolution 'Added nil check in handler', got %v", updated["resolution"])
	}

	// Verify updates persisted via list
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"status": "fixed"}
	result, err = handlers["hub_issue_list"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	var fixedIssues []map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &fixedIssues); err != nil {
			t.Fatal(err)
		}
	}
	if len(fixedIssues) != 1 {
		t.Fatalf("expected 1 fixed issue, got %d", len(fixedIssues))
	}
	if fixedIssues[0]["assigned_to"] != "coder-2" {
		t.Errorf("expected assigned_to persisted as 'coder-2', got %v", fixedIssues[0]["assigned_to"])
	}

	// Test update on non-existent ID
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":     "non-existent-id",
		"status": "fixed",
	}
	result, err = handlers["hub_issue_update"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if !result.IsError {
		t.Fatal("expected error for non-existent issue ID")
	}
}
