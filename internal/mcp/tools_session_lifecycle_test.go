package mcp

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

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

func TestHubSessionOpen_EnqueuesWhenHookAbsent(t *testing.T) {
	// Z-3d: when hook is absent, hub_session_open creates the session
	// container + manifest synchronously, then enqueues a row in
	// session_lifecycle_queue and polls until it sees 'fired' or times
	// out. In this test we simulate the daemon's queue ticker by firing
	// MarkSessionOpFired ourselves from a background goroutine.
	canonRoot := t.TempDir()
	t.Setenv("BOT_HQ_HOME", canonRoot)
	t.Setenv("BOT_HQ_SESSIONS_DIR", filepath.Join(canonRoot, "sessions"))

	projDir := filepath.Join(canonRoot, "projects")
	if err := os.MkdirAll(projDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "test-proj.yaml"), []byte("project_name: test-proj\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	SetSessionOpenHook(nil)
	db := setupTestDB(t)

	// Background "daemon" that drains the queue.
	done := make(chan struct{})
	go func() {
		defer close(done)
		deadline := time.Now().Add(5 * time.Second)
		for time.Now().Before(deadline) {
			ops, _ := db.ClaimPendingSessionOps(10)
			for _, op := range ops {
				result := `{"session_id":"` + op.SessionID + `","project":"` + op.Project + `","scope":"` + op.Scope + `","agents":["brian","rain"]}`
				_ = db.MarkSessionOpFired(op.ID, "fired", result)
			}
			if len(ops) > 0 {
				return
			}
			time.Sleep(100 * time.Millisecond)
		}
	}()

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
	<-done
	if res.IsError {
		t.Fatalf("unexpected error: %s", res.Content[0].(mcp.TextContent).Text)
	}
	var info SessionOpenInfo
	if err := json.Unmarshal([]byte(res.Content[0].(mcp.TextContent).Text), &info); err != nil {
		t.Fatalf("json unmarshal: %v; raw=%q", err, res.Content[0].(mcp.TextContent).Text)
	}
	if info.SessionID == "" {
		t.Error("expected non-empty session_id")
	}
	if len(info.Agents) != 2 || info.Agents[0] != "brian" || info.Agents[1] != "rain" {
		t.Errorf("agents=%v want [brian rain] (from simulated queue-drainer)", info.Agents)
	}
	// Verify session skeleton exists on disk.
	skeleton := filepath.Join(os.Getenv("BOT_HQ_SESSIONS_DIR"), info.SessionID)
	for _, sub := range []string{"brian", "rain", "tasks", "manifest.md"} {
		if _, err := os.Stat(filepath.Join(skeleton, sub)); err != nil {
			t.Errorf("expected %s/%s to exist: %v", info.SessionID, sub, err)
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
