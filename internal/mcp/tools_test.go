package mcp

import (
	"context"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/mark3labs/mcp-go/mcp"
)

func TestExtractAgentID_Priority(t *testing.T) {
	cases := []struct {
		name string
		args map[string]any
		want string
	}{
		{"from wins over agent_id", map[string]any{"from": "a", "agent_id": "b", "id": "c"}, "a"},
		{"agent_id wins over id", map[string]any{"agent_id": "b", "id": "c"}, "b"},
		{"id when alone", map[string]any{"id": "c"}, "c"},
		{"empty when none present", map[string]any{}, ""},
		{"empty string fields skipped", map[string]any{"from": "", "agent_id": "x"}, "x"},
		{"non-string fields ignored", map[string]any{"from": 123, "id": "z"}, "z"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			req := mcp.CallToolRequest{}
			req.Params.Arguments = tc.args
			if got := extractAgentID(req); got != tc.want {
				t.Errorf("extractAgentID(%v) = %q, want %q", tc.args, got, tc.want)
			}
		})
	}
}

// Each middleware test uses a unique agent ID so the package-global throttle
// cache (lastSeenWriteCache) doesn't cross-contaminate between tests.

func TestWithLastSeen_UpdatesAgentRow(t *testing.T) {
	db := setupTestDB(t)
	id := "ws_alice"
	if err := db.RegisterAgent(protocol.Agent{ID: id, Name: "Alice", Type: protocol.AgentBrian, Status: protocol.StatusOnline}); err != nil {
		t.Fatal(err)
	}
	initial, _ := db.GetAgent(id)

	time.Sleep(5 * time.Millisecond)

	called := false
	inner := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		called = true
		return mcp.NewToolResultText("ok"), nil
	}
	wrapped := withLastSeen(db, inner)

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"from": id}
	if _, err := wrapped(context.Background(), req); err != nil {
		t.Fatal(err)
	}

	if !called {
		t.Error("inner handler was not called")
	}
	after, _ := db.GetAgent(id)
	if !after.LastSeen.After(initial.LastSeen) {
		t.Errorf("LastSeen did not advance: initial=%v after=%v", initial.LastSeen, after.LastSeen)
	}
	if after.Status != protocol.StatusOnline {
		t.Errorf("Status mutated: got %q", after.Status)
	}
}

func TestWithLastSeen_NoAgentIDPassesThrough(t *testing.T) {
	db := setupTestDB(t)

	called := false
	inner := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		called = true
		return mcp.NewToolResultText("ok"), nil
	}
	wrapped := withLastSeen(db, inner)

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"prompt": "hello"} // claude_send-style, no agent
	if _, err := wrapped(context.Background(), req); err != nil {
		t.Fatal(err)
	}

	if !called {
		t.Error("inner handler was not called")
	}
}

func TestWithLastSeen_Throttle(t *testing.T) {
	db := setupTestDB(t)
	id := "ws_bob"
	if err := db.RegisterAgent(protocol.Agent{ID: id, Name: "Bob", Type: protocol.AgentBrian, Status: protocol.StatusOnline}); err != nil {
		t.Fatal(err)
	}

	inner := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		return mcp.NewToolResultText("ok"), nil
	}
	wrapped := withLastSeen(db, inner)

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"from": id}

	if _, err := wrapped(context.Background(), req); err != nil {
		t.Fatal(err)
	}
	first, _ := db.GetAgent(id)

	time.Sleep(50 * time.Millisecond) // well under 1s throttle window

	if _, err := wrapped(context.Background(), req); err != nil {
		t.Fatal(err)
	}
	second, _ := db.GetAgent(id)

	if !second.LastSeen.Equal(first.LastSeen) {
		t.Errorf("Throttle did not suppress write: first=%v second=%v", first.LastSeen, second.LastSeen)
	}
}

func TestWithLastSeen_ThrottleExpires(t *testing.T) {
	db := setupTestDB(t)
	id := "ws_carol"
	if err := db.RegisterAgent(protocol.Agent{ID: id, Name: "Carol", Type: protocol.AgentBrian, Status: protocol.StatusOnline}); err != nil {
		t.Fatal(err)
	}

	inner := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		return mcp.NewToolResultText("ok"), nil
	}
	wrapped := withLastSeen(db, inner)

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"from": id}

	if _, err := wrapped(context.Background(), req); err != nil {
		t.Fatal(err)
	}
	first, _ := db.GetAgent(id)

	// Force the throttle window to expire by backdating the cache entry.
	lastSeenWriteCache.Store(id, time.Now().Add(-2*time.Second))

	time.Sleep(5 * time.Millisecond) // ensure observable timestamp delta at ms resolution

	if _, err := wrapped(context.Background(), req); err != nil {
		t.Fatal(err)
	}
	second, _ := db.GetAgent(id)

	if !second.LastSeen.After(first.LastSeen) {
		t.Errorf("Throttle expiry did not allow write: first=%v second=%v", first.LastSeen, second.LastSeen)
	}
}

func TestBuildTools_AllExpectedKeysExtractable(t *testing.T) {
	// Per-tool extraction matrix: every tool whose params include a caller-context
	// agent must expose one of commonAgentIDKeys. Anonymous read/spawn tools and
	// session_id-scoped tools are excluded with reasoning — see exclusions below.
	//
	// This test locks against a future regression where a caller-context tool
	// is added without one of {from, agent_id, id} in its schema, silently
	// falling outside the middleware's extraction set.
	excluded := map[string]string{
		// One-shot, no agent context.
		"claude_send":   "one-shot --print mode, no agent caller",
		"claude_resume": "session resume, operates on session_id, not agent",

		// Anonymous reads: caller identity is irrelevant to the operation.
		"hub_agents":     "lists agents, no caller-context update needed",
		"hub_sessions":   "lists sessions, anonymous read",
		"hub_issue_list": "lists issues, anonymous read",
		"claude_list":    "lists claude sessions, anonymous read",

		// Spawn tools: caller anonymous; spawned agent registers itself separately.
		"hub_spawn":       "spawns a new coder agent; caller's identity is incidental",
		"hub_spawn_gemma": "spawns a gemma scout; caller's identity is incidental",

		// Session-scoped tools: operate on session_id which may be linked to an
		// agent via agents.meta.tmux_target. Resolving that mapping adds
		// complexity for marginal gain — deferred to Phase F per spec §6.
		"claude_read":    "operates on session_id (Phase F may resolve to agent)",
		"claude_message": "operates on session_id (Phase F may resolve to agent)",
		"claude_stop":    "operates on session_id (Phase F may resolve to agent)",

		// Anonymous create operations.
		"hub_session_create": "creates a session; caller not bound to an agent yet",
		"hub_issue_create":   "creates an issue; caller anonymous in current schema",
	}

	db := setupTestDB(t)
	tools := BuildTools(db)

	for _, td := range tools {
		if reason := excluded[td.Tool.Name]; reason != "" {
			continue
		}
		schema := td.Tool.InputSchema.Properties
		hasKey := false
		for _, k := range commonAgentIDKeys {
			if _, ok := schema[k]; ok {
				hasKey = true
				break
			}
		}
		if !hasKey {
			t.Errorf("tool %q has no agent-id param (missing all of %v); add to excluded set with reason if intentional, or add an extractable key to the tool's schema", td.Tool.Name, commonAgentIDKeys)
		}
	}
}
