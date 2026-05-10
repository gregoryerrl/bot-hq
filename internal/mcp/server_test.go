package mcp

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/sessions"
	"github.com/mark3labs/mcp-go/mcp"
)

func setupTestDB(t *testing.T) *hub.DB {
	t.Helper()
	// R39 TEST-ISOLATION: hub_register and hub_session_create write
	// session-cluster manifests under SessionsDir() when project is
	// supplied. Override the sessions root so MCP tests don't pollute
	// the real CL under ~/.bot-hq/sessions/. (Pre-W: TestHubRegister
	// with project="/projects/test" leaked nested manifest dirs into
	// the real CL across many test runs.)
	t.Setenv("BOT_HQ_SESSIONS_DIR", t.TempDir())

	// Phase Y-2: bot_hq_ipiv_* tools write under <CL_ROOT>/projects/
	// <project>/tasks/. Same isolation pattern — pin BOT_HQ_CL_ROOT to
	// a temp dir so IPIV tool tests don't pollute the real CL. Create
	// the projects/<test-project> subdir so IndexProject succeeds.
	clRoot := t.TempDir()
	t.Setenv("BOT_HQ_CL_ROOT", clRoot)
	if err := os.MkdirAll(filepath.Join(clRoot, "projects", "test-project"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(clRoot, "projects", "test-project.yaml"), []byte("project_name: test-project\n"), 0o644); err != nil {
		t.Fatal(err)
	}

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
		"hub_session_load",
		"hub_session_checkpoint",
		"hub_session_archive",
		"hub_broadcast",
		"hub_set_current_task",
		"hub_status",
		"hub_spawn",
		"hub_spawn_gemma",
		"hub_schedule_wake",
		"hub_cancel_wake",
		"hub_session_close",
		"hub_clear_halt",
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
		"bot_hq_context_load",
		"hub_session_finalize",
		"hub_session_lookback",
		"hub_session_summary",
		"bot_hq_ipiv_open",
		"bot_hq_ipiv_transition",
		"bot_hq_ipiv_set_artifact",
		"bot_hq_ipiv_complete",
		"bot_hq_ipiv_list",
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

// TestHubRegister_PreservesLauncherMeta locks the H6 fix's load-bearing
// invariant: Meta is owned by the launcher (e.g. internal/brian/brian.go,
// internal/rain/rain.go), and the MCP hub_register handler — typically called
// from the Claude STARTUP prompt AFTER the launcher has registered — must NOT
// clobber the launcher's Meta JSON. db.RegisterAgent is INSERT OR REPLACE, so
// the handler reads existing Meta and threads it through.
//
// Regression preventer (producer side). Without this test, a future refactor
// could "simplify" the handler back to constructing Meta="" and silently
// re-introduce H6 — fix would appear correct on first launcher boot, then
// regress on first Claude STARTUP hub_register call. Worst class of bug.
// Rain-pattern-8 + Brian-pattern-3.12 backstop: audit producer side here,
// audit consumer side via panestate_test.go's H6 tests.
func TestHubRegister_PreservesLauncherMeta(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)

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

	// Simulate the launcher's pre-registration with tmux_target Meta populated.
	launcherMeta := `{"tmux_target":"bot-hq-brian-1777154445"}`
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "brian",
		Name:   "Brian",
		Type:   protocol.AgentBrian,
		Status: protocol.StatusOnline,
		Meta:   launcherMeta,
	}); err != nil {
		t.Fatal(err)
	}

	// Claude STARTUP prompt re-registers via MCP. No meta param is passed
	// (matches the current STARTUP rule: hub_register id="brian" name="Brian"
	// type="brian"). Handler must preserve the launcher's Meta.
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":   "brian",
		"name": "Brian",
		"type": "brian",
	}
	result, err := registerHandler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("re-register failed: %v", result.Content)
	}

	after, err := db.GetAgent("brian")
	if err != nil {
		t.Fatal(err)
	}
	if after.Meta != launcherMeta {
		t.Errorf("Meta clobbered on re-register: got %q, want %q (H6 regression)", after.Meta, launcherMeta)
	}
}

// TestHubRegister_NewAgentNoExistingMeta locks the green path for first-time
// registration via MCP (no prior launcher row). Handler must not error on the
// GetAgent miss and must register cleanly with Meta="".
func TestHubRegister_NewAgentNoExistingMeta(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)

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

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":   "fresh-agent",
		"name": "Fresh",
		"type": "coder",
	}
	result, err := registerHandler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("register failed: %v", result.Content)
	}

	after, err := db.GetAgent("fresh-agent")
	if err != nil {
		t.Fatal(err)
	}
	if after.Meta != "" {
		t.Errorf("fresh agent Meta = %q, want empty (no prior row to preserve from)", after.Meta)
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
	// Phase S S-4: even when `to: "bob"` parameter is passed, the
	// handler ignores it and sets ToAgent="" (broadcast). Bob still
	// receives via hub_read since broadcasts (to_agent='') match the
	// `WHERE to_agent=? OR to_agent=''` clause.
	if msgs[0].ToAgent != "" {
		t.Errorf("Phase S S-4: hub_send must force ToAgent=\"\" (PM removed); got %q", msgs[0].ToAgent)
	}
}

// TestHubSend_S4_PMRemoved locks the Phase S S-4 contract: hub_send
// MCP tool no longer routes via `to:` parameter — all messages
// broadcast (ToAgent always empty). Mention-detection via @<agent>
// in content replaces PM semantic.
func TestHubSend_S4_PMRemoved(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	handlers := make(map[string]func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error))
	for _, td := range tools {
		handlers[td.Tool.Name] = td.Handler
	}
	ctx := context.Background()

	cases := []struct {
		name    string
		toParam any // "to" param value (or nil to skip key)
	}{
		{"no-to-param", nil},
		{"to-param-empty", ""},
		{"to-param-passed-back-compat-ignored", "rain"},
		{"to-param-passed-self-ignored", "brian"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			args := map[string]any{
				"from":    "brian",
				"type":    "update",
				"content": "@rain test mention-routing post-PM-removal",
			}
			if tc.toParam != nil {
				args["to"] = tc.toParam
			}
			req := mcp.CallToolRequest{}
			req.Params.Arguments = args
			result, err := handlers["hub_send"](ctx, req)
			if err != nil {
				t.Fatal(err)
			}
			if result.IsError {
				t.Fatalf("hub_send failed: %v", result.Content)
			}
		})
	}

	// Verify all sent messages have ToAgent="" regardless of input.
	msgs, err := db.GetRecentMessages(10)
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range msgs {
		if m.FromAgent != "brian" {
			continue
		}
		if m.ToAgent != "" {
			t.Errorf("Phase S S-4: msg %d ToAgent=%q, want empty (PM removed)", m.ID, m.ToAgent)
		}
	}
}

// TestHubSend_T11_PasteDetectionBlocksApiKey locks the Phase T T-11
// cycle-3 contract: hub_send rejects content that contains an API-key
// pattern (sk-xxx) before persisting the message. Prevents the
// paste-to-hub recurrence class observed at msgs 17421 + 17460.
func TestHubSend_T11_PasteDetectionBlocksApiKey(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	handlers := make(map[string]func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error))
	for _, td := range tools {
		handlers[td.Tool.Name] = td.Handler
	}
	ctx := context.Background()

	preMsgs, err := db.GetRecentMessages(10)
	if err != nil {
		t.Fatal(err)
	}
	preCount := len(preMsgs)

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":    "user",
		"type":    "command",
		"content": "DEEPSEEK_API_KEY=sk-deadbeefcafebabedeadbeefcafebabe",
	}
	result, err := handlers["hub_send"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if !result.IsError {
		t.Fatalf("expected paste-detection block; got success result")
	}

	// Verify the block returned a useful remediation hint.
	textBlock, ok := result.Content[0].(mcp.TextContent)
	if !ok {
		t.Fatalf("expected TextContent in result; got %T", result.Content[0])
	}
	if !strings.Contains(textBlock.Text, "BLOCKED") {
		t.Errorf("block reason missing BLOCKED marker: %s", textBlock.Text)
	}
	if !strings.Contains(textBlock.Text, "FileVault") {
		t.Errorf("block reason missing FileVault remediation hint: %s", textBlock.Text)
	}

	// Verify NO new message was persisted (defensive screen pre-DB-write).
	postMsgs, err := db.GetRecentMessages(10)
	if err != nil {
		t.Fatal(err)
	}
	if len(postMsgs) != preCount {
		t.Errorf("paste-detection block leaked into DB: pre=%d post=%d", preCount, len(postMsgs))
	}
}

// TestHubSend_T11_CompactPipeContentPasses locks the inverse: normal
// peer-coord compact-pipe content (no API-key) is NOT blocked.
func TestHubSend_T11_CompactPipeContentPasses(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	handlers := make(map[string]func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error))
	for _, td := range tools {
		handlers[td.Tool.Name] = td.Handler
	}
	ctx := context.Background()

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":    "brian",
		"type":    "update",
		"content": "brian|update|standing-for-rain-BRAIN-2nd|cycle-closed",
	}
	result, err := handlers["hub_send"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("expected normal peer-coord content to pass; got error: %v", result.Content)
	}
}

// TestHubSend_S4_SchemaDropsToParam locks that the MCP tool schema
// for hub_send no longer declares a `to:` parameter. MCP clients
// reading the schema see only from / session_id / type / content.
func TestHubSend_S4_SchemaDropsToParam(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	for _, td := range tools {
		if td.Tool.Name != "hub_send" {
			continue
		}
		schemaJSON, err := json.Marshal(td.Tool.InputSchema)
		if err != nil {
			t.Fatal(err)
		}
		s := string(schemaJSON)
		if strings.Contains(s, `"to"`) {
			t.Errorf("Phase S S-4: hub_send schema must not declare `to:` parameter; got %s", s)
		}
		if !strings.Contains(s, `"from"`) || !strings.Contains(s, `"content"`) || !strings.Contains(s, `"type"`) {
			t.Errorf("hub_send schema missing required from/type/content; got %s", s)
		}
		return
	}
	t.Error("hub_send tool not found in BuildTools()")
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
		"from":     "brian",
		"agent_id": "brian",
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
		"agent_id": "brian",
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
		"from":     "brian",
		"agent_id": "brian",
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
		"from":     "brian",
		"agent_id": "brian",
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

	// Agent "brian" saves its own checkpoint
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":     "brian",
		"agent_id": "brian",
		"data":     `{"tasks":["t1","t2"]}`,
	}
	result, err := checkpointHandler(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("checkpoint failed: %v", result.Content)
	}

	// Different agent "coder-1" restores brian's checkpoint (should succeed)
	req = mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"agent_id": "brian",
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
		"reporter": "brian",
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
	req.Params.Arguments = map[string]any{"reporter": "brian"}
	result, err = handlers["hub_issue_list"](ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	var brianIssues []map[string]any
	if tc, ok := result.Content[0].(mcp.TextContent); ok {
		if err := json.Unmarshal([]byte(tc.Text), &brianIssues); err != nil {
			t.Fatal(err)
		}
	}
	if len(brianIssues) != 1 {
		t.Fatalf("expected 1 brian issue, got %d", len(brianIssues))
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

// decodeResultMap unmarshals a successful tool result's JSON text payload.
func decodeResultMap(t *testing.T, result *mcp.CallToolResult) map[string]any {
	t.Helper()
	if result.IsError {
		t.Fatalf("unexpected error result: %v", result.Content)
	}
	tc, ok := result.Content[0].(mcp.TextContent)
	if !ok {
		t.Fatalf("expected TextContent, got %T", result.Content[0])
	}
	out := map[string]any{}
	if err := json.Unmarshal([]byte(tc.Text), &out); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	return out
}

// TestSessionCloseStoresSnap locks the basic contract: hub_session_close
// upserts a session_ledger row keyed on agent_id with the supplied snap_text.
// Phase H slice 4 C4 (H-15).
func TestSessionCloseStoresSnap(t *testing.T) {
	db := setupTestDB(t)
	handler := findHandler(BuildTools(db), "hub_session_close")
	if handler == nil {
		t.Fatal("hub_session_close not found")
	}

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"agent_id":  "coder-abc",
		"snap_text": "SNAP v1: focus=H-15; cursor=db.go:468; next=tools wiring",
	}
	result, err := handler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	out := decodeResultMap(t, result)
	if out["status"] != "stored" {
		t.Errorf("status = %v, want stored", out["status"])
	}

	snap, err := db.GetLastSessionSnap("coder-abc")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(snap, "focus=H-15") {
		t.Errorf("ledger snap_text = %q, want substring focus=H-15", snap)
	}
}

// TestRegisterReturnsPriorSnap locks bootstrap consumption: when a prior
// session_ledger row exists for the agent, hub_register surfaces it as
// last_session_snap so cold-start callers can pre-load context. Phase H
// slice 4 C4 (H-15).
func TestRegisterReturnsPriorSnap(t *testing.T) {
	db := setupTestDB(t)
	const want = "SNAP v1: prior-session anchor"
	if err := db.StoreSessionClose("coder-prior", want); err != nil {
		t.Fatal(err)
	}

	handler := findHandler(BuildTools(db), "hub_register")
	if handler == nil {
		t.Fatal("hub_register not found")
	}
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":   "coder-prior",
		"name": "Coder Prior",
		"type": "coder",
	}
	result, err := handler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	out := decodeResultMap(t, result)
	if got := out["last_session_snap"]; got != want {
		t.Errorf("last_session_snap = %q, want %q", got, want)
	}

	// Negative branch: a never-closed agent registers with empty snap.
	req.Params.Arguments = map[string]any{
		"id":   "coder-fresh",
		"name": "Coder Fresh",
		"type": "coder",
	}
	result, err = handler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	out = decodeResultMap(t, result)
	if got := out["last_session_snap"]; got != "" {
		t.Errorf("first-time agent last_session_snap = %q, want empty", got)
	}
}

// TestSessionCloseOverwritesPrior locks upsert semantics: a second
// hub_session_close for the same agent_id wins; only the latest SNAP is
// surfaced on next register. Phase H slice 4 C4 (H-15).
func TestSessionCloseOverwritesPrior(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	closeHandler := findHandler(tools, "hub_session_close")
	registerHandler := findHandler(tools, "hub_register")
	if closeHandler == nil || registerHandler == nil {
		t.Fatal("required tools not found")
	}

	for _, snap := range []string{"first", "second"} {
		req := mcp.CallToolRequest{}
		req.Params.Arguments = map[string]any{
			"agent_id":  "coder-up",
			"snap_text": snap,
		}
		result, err := closeHandler(context.Background(), req)
		if err != nil {
			t.Fatal(err)
		}
		if result.IsError {
			t.Fatalf("close failed: %v", result.Content)
		}
	}

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"id":   "coder-up",
		"name": "Coder Up",
		"type": "coder",
	}
	result, err := registerHandler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	out := decodeResultMap(t, result)
	if got := out["last_session_snap"]; got != "second" {
		t.Errorf("last_session_snap = %q, want %q (upsert second-write-wins)", got, "second")
	}
}

// TestHubSessionCreate_WritesManifestWhenProjectProvided locks the Phase N
// v3a writer-flow wiring contract: hub_session_create with a project param
// writes a session-cluster manifest at SessionsDir()/<YYYY-MM-DD-project>/
// manifest.md alongside the in-db session record, and refreshes the rolling
// index. Without this test, a future refactor could "simplify" the handler
// back to in-db-only and silently re-introduce the writer-flow gap that
// motivated v3a (per discipline-log §2026-05-06T(post-v2-close-pre-v3-open)
// joint entry R31 OVER-CLAIM phase-close-arc-snapshot-class anchor).
func TestHubSessionCreate_WritesManifestWhenProjectProvided(t *testing.T) {
	t.Setenv("BOT_HQ_SESSIONS_DIR", t.TempDir())
	db := setupTestDB(t)

	handler := findHandler(BuildTools(db), "hub_session_create")
	if handler == nil {
		t.Fatal("hub_session_create not found")
	}

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"mode":    "implement",
		"purpose": "Phase N v3a writer-flow wiring",
		"agents":  "brian,rain",
		"project": "bot-hq",
	}
	result, err := handler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("unexpected error: %v", result.Content)
	}

	out := decodeResultMap(t, result)
	if out["status"] != "created" {
		t.Errorf("status = %v, want created", out["status"])
	}
	clusterID, ok := out["session_cluster_id"].(string)
	if !ok || clusterID == "" {
		t.Fatalf("session_cluster_id missing or non-string: %v", out["session_cluster_id"])
	}
	if !strings.HasSuffix(clusterID, "-bot-hq") {
		t.Errorf("session_cluster_id = %q, want suffix '-bot-hq'", clusterID)
	}

	// Verify manifest file actually exists at the expected path.
	manifestPath := sessions.ManifestPath(clusterID)
	data, err := os.ReadFile(manifestPath)
	if err != nil {
		t.Fatalf("manifest not written at %s: %v", manifestPath, err)
	}
	body := string(data)
	if !strings.Contains(body, "id: "+clusterID) {
		t.Errorf("manifest body missing id field: %s", body)
	}
	if !strings.Contains(body, "project: bot-hq") {
		t.Errorf("manifest body missing project field: %s", body)
	}
	if !strings.Contains(body, "Phase N v3a writer-flow wiring") {
		t.Errorf("manifest body missing purpose: %s", body)
	}
}

// TestHubSessionCreate_NoManifestWhenProjectMissing locks the regression-
// preventer contract: hub_session_create without a project param keeps the
// pre-v3a behavior (in-db-only, no manifest file). Defends against
// accidental writes when a caller omits project.
func TestHubSessionCreate_NoManifestWhenProjectMissing(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("BOT_HQ_SESSIONS_DIR", dir)
	db := setupTestDB(t)

	handler := findHandler(BuildTools(db), "hub_session_create")
	if handler == nil {
		t.Fatal("hub_session_create not found")
	}

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"mode":    "implement",
		"purpose": "no-project test",
	}
	result, err := handler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if result.IsError {
		t.Fatalf("unexpected error: %v", result.Content)
	}

	out := decodeResultMap(t, result)
	if _, has := out["session_cluster_id"]; has {
		t.Errorf("session_cluster_id should be absent when project missing, got %v", out["session_cluster_id"])
	}

	// Verify no manifest dirs were created in the sessions root.
	entries, _ := os.ReadDir(dir)
	for _, e := range entries {
		if e.IsDir() {
			t.Errorf("unexpected session dir created: %s", e.Name())
		}
	}
}

// TestHubRegister_WritesIdempotentManifest locks the Phase N v3a writer-flow
// wiring on the hub_register path: registering an agent with a project param
// writes (or refreshes) the today-session-cluster manifest, idempotently
// merging the agent into the agents list across repeated registrations.
func TestHubRegister_WritesIdempotentManifest(t *testing.T) {
	t.Setenv("BOT_HQ_SESSIONS_DIR", t.TempDir())
	db := setupTestDB(t)

	handler := findHandler(BuildTools(db), "hub_register")
	if handler == nil {
		t.Fatal("hub_register not found")
	}

	// Register agent A
	req1 := mcp.CallToolRequest{}
	req1.Params.Arguments = map[string]any{
		"id":      "agent-a",
		"name":    "Agent A",
		"type":    "coder",
		"project": "phase-n-v3-test",
	}
	result1, err := handler(context.Background(), req1)
	if err != nil {
		t.Fatal(err)
	}
	out1 := decodeResultMap(t, result1)
	clusterID, ok := out1["session_cluster_id"].(string)
	if !ok || clusterID == "" {
		t.Fatalf("first register: session_cluster_id missing: %v", out1["session_cluster_id"])
	}

	// Register agent B for same project — should merge into same cluster
	req2 := mcp.CallToolRequest{}
	req2.Params.Arguments = map[string]any{
		"id":      "agent-b",
		"name":    "Agent B",
		"type":    "coder",
		"project": "phase-n-v3-test",
	}
	result2, err := handler(context.Background(), req2)
	if err != nil {
		t.Fatal(err)
	}
	out2 := decodeResultMap(t, result2)
	if got := out2["session_cluster_id"]; got != clusterID {
		t.Errorf("second register cluster_id = %v, want %s (same-day merge)", got, clusterID)
	}

	// Verify both agents are in the manifest.
	m, err := sessions.ReadManifest(clusterID)
	if err != nil {
		t.Fatalf("read manifest: %v", err)
	}
	if len(m.Agents) != 2 {
		t.Errorf("manifest agents = %v, want 2 (agent-a + agent-b)", m.Agents)
	}
}
