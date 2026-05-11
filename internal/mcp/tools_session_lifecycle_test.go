package mcp

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/mark3labs/mcp-go/mcp"
)

func TestAllocateSessionID_Shape(t *testing.T) {
	id, err := AllocateSessionID("Z-3 Sessions As Containers")
	if err != nil {
		t.Fatal(err)
	}
	// Expect: lowercase slug-uuid pattern. Slug portion is sanitized.
	if !strings.HasPrefix(id, "z-3-sessions-as-containers-") {
		t.Errorf("unexpected slug prefix in %q", id)
	}
	// uuid suffix is 6 chars from the alphabet
	parts := strings.Split(id, "-")
	last := parts[len(parts)-1]
	if len(last) != 6 {
		t.Errorf("uuid suffix should be 6 chars; got %q (len %d)", last, len(last))
	}
	for _, c := range last {
		if !strings.ContainsRune(sessionIDUUIDAlphabet, c) {
			t.Errorf("uuid char %q not in alphabet %q", c, sessionIDUUIDAlphabet)
		}
	}
}

func TestAllocateSessionID_RejectsEmpty(t *testing.T) {
	if _, err := AllocateSessionID(""); err == nil {
		t.Error("expected error on empty scope")
	}
	if _, err := AllocateSessionID("   "); err == nil {
		t.Error("expected error on whitespace-only scope")
	}
	if _, err := AllocateSessionID("---"); err == nil {
		t.Error("expected error on all-hyphens scope")
	}
}

func TestAllocateSessionID_LongScopeTruncated(t *testing.T) {
	long := strings.Repeat("abcde-", 20) // 120 chars
	id, err := AllocateSessionID(long)
	if err != nil {
		t.Fatal(err)
	}
	// Slug portion ≤ 40; total = 40 + 1 (hyphen) + 6 (uuid) = 47
	if len(id) > 47 {
		t.Errorf("expected truncated id ≤47 chars; got %q (len %d)", id, len(id))
	}
}

func TestAllocateSessionID_Uniqueness(t *testing.T) {
	seen := make(map[string]bool)
	for i := 0; i < 50; i++ {
		id, err := AllocateSessionID("test")
		if err != nil {
			t.Fatal(err)
		}
		if seen[id] {
			t.Errorf("collision on %q after %d attempts", id, i)
		}
		seen[id] = true
	}
}

func TestSessionOpenHook_Concurrent(t *testing.T) {
	// Fault-tree F8: concurrent hub_session_open should not collide
	// on session-id thanks to crypto/rand uuid + mutex. Race the
	// allocation directly.
	var wg sync.WaitGroup
	results := make(chan string, 20)
	for i := 0; i < 20; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			id, _ := AllocateSessionID("race")
			results <- id
		}()
	}
	wg.Wait()
	close(results)
	seen := make(map[string]bool)
	for id := range results {
		if seen[id] {
			t.Errorf("concurrent collision on %q", id)
		}
		seen[id] = true
	}
}

func TestHubSessionOpen_RequiresProjectExist(t *testing.T) {
	canonRoot := t.TempDir()
	t.Setenv("BOT_HQ_HOME", canonRoot)

	db := setupTestDB(t)
	tool := hubSessionOpen(db)

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"project":    "nonexistent",
		"scope_name": "test-scope",
	}
	res, err := tool.Handler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if !res.IsError {
		t.Errorf("expected IsError=true for missing project")
	}
}

func TestHubSessionOpen_CreatesSessionWithoutHook(t *testing.T) {
	// Hook absent: tool still allocates session container + writes
	// manifest, returns degraded-mode result with empty agents.
	canonRoot := t.TempDir()
	t.Setenv("BOT_HQ_HOME", canonRoot)
	t.Setenv("BOT_HQ_SESSIONS_DIR", filepath.Join(canonRoot, "sessions"))

	// Create a fake project file so validation passes.
	projDir := filepath.Join(canonRoot, "projects")
	if err := os.MkdirAll(projDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "test-proj.yaml"), []byte("project_name: test-proj\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	// Clear hook to test degraded mode.
	SetSessionOpenHook(nil)

	db := setupTestDB(t)
	tool := hubSessionOpen(db)
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"project":      "test-proj",
		"scope_name":   "scope test alpha",
		"pointer_list": []any{"projects/test-proj/README.md"},
	}
	res, err := tool.Handler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if res.IsError {
		t.Fatalf("unexpected error from no-hook path: %s", res.Content[0].(mcp.TextContent).Text)
	}
	// Parse the JSON response
	var info SessionOpenInfo
	if err := json.Unmarshal([]byte(res.Content[0].(mcp.TextContent).Text), &info); err != nil {
		t.Fatalf("json unmarshal: %v; raw=%q", err, res.Content[0].(mcp.TextContent).Text)
	}
	t.Logf("info=%+v", info)
	if info.SessionID == "" {
		t.Error("expected non-empty session_id")
	}
	if info.Project != "test-proj" {
		t.Errorf("project=%q want test-proj", info.Project)
	}
	if info.Scope != "scope test alpha" {
		t.Errorf("scope=%q want 'scope test alpha'", info.Scope)
	}
	// Verify session skeleton exists. Skeleton lives under
	// BOT_HQ_SESSIONS_DIR (the sessions package's resolution).
	// setupTestDB overrides BOT_HQ_SESSIONS_DIR; we read the current
	// env value rather than recompute from canonRoot.
	skeleton := filepath.Join(os.Getenv("BOT_HQ_SESSIONS_DIR"), info.SessionID)
	for _, sub := range []string{"brian", "rain", "tasks", "manifest.md"} {
		if _, err := os.Stat(filepath.Join(skeleton, sub)); err != nil {
			t.Errorf("expected %s/%s to exist: %v", info.SessionID, sub, err)
			entries, _ := os.ReadDir(skeleton)
			for _, e := range entries {
				t.Logf("  found: %s", e.Name())
			}
		}
	}
}

func TestHubSessionOpen_HookInvokedAndPersistsThreadID(t *testing.T) {
	canonRoot := t.TempDir()
	t.Setenv("BOT_HQ_HOME", canonRoot)
	t.Setenv("BOT_HQ_SESSIONS_DIR", filepath.Join(canonRoot, "sessions"))

	projDir := filepath.Join(canonRoot, "projects")
	if err := os.MkdirAll(projDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "p.yaml"), []byte("project_name: p\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	called := false
	SetSessionOpenHook(func(r SessionOpenRequest) (*SessionOpenInfo, error) {
		called = true
		return &SessionOpenInfo{
			SessionID:       r.SessionID,
			Project:         r.Project,
			Scope:           r.Scope,
			DiscordThreadID: "discord-thread-xyz",
			Agents:          []string{"brian", "rain"},
		}, nil
	})
	t.Cleanup(func() { SetSessionOpenHook(nil) })

	db := setupTestDB(t)
	tool := hubSessionOpen(db)
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"project":    "p",
		"scope_name": "alpha",
	}
	res, err := tool.Handler(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if res.IsError {
		t.Errorf("unexpected error: %s", res.Content[0].(mcp.TextContent).Text)
	}
	if !called {
		t.Error("expected hook to be invoked")
	}
	var info SessionOpenInfo
	if err := json.Unmarshal([]byte(res.Content[0].(mcp.TextContent).Text), &info); err != nil {
		t.Fatal(err)
	}
	if info.DiscordThreadID != "discord-thread-xyz" {
		t.Errorf("discord_thread_id=%q want discord-thread-xyz", info.DiscordThreadID)
	}
	// Verify the manifest got the discord_thread_id round-trip
	manifestPath := filepath.Join(os.Getenv("BOT_HQ_SESSIONS_DIR"), info.SessionID, "manifest.md")
	data, err := os.ReadFile(manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "discord_thread_id: discord-thread-xyz") {
		t.Errorf("manifest missing discord_thread_id roundtrip: %s", string(data))
	}
}
