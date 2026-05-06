package webui

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// fakeHandlerServer wraps Server to return its handler for httptest. We
// don't use Start in tests — Start binds a real port; routes are exercised
// via the embedded mux.
func newTestServer(t *testing.T, root string) *Server {
	t.Helper()
	s := &Server{
		canonicalRoot: root,
	}
	mux := http.NewServeMux()
	s.registerRoutes(mux)
	s.httpServer = &http.Server{Handler: mux}
	return s
}

// callRoute issues an HTTP request against the server's mux and returns
// the status + body.
func callRoute(t *testing.T, s *Server, method, target string) (int, string) {
	t.Helper()
	req := httptest.NewRequest(method, target, nil)
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	body, _ := io.ReadAll(w.Result().Body)
	return w.Code, string(body)
}

func TestHandleFilesTree_RespectsSkipList(t *testing.T) {
	root := t.TempDir()
	// Canonical entries.
	mustMkdir(t, filepath.Join(root, "phase"))
	mustWrite(t, filepath.Join(root, "phase", "phase-n.md"), "# Phase N\n")
	mustMkdir(t, filepath.Join(root, "ratchets"))
	mustWrite(t, filepath.Join(root, "ratchets", "active.md"), "ratchets\n")
	// Non-canonical entries (should be skipped).
	mustWrite(t, filepath.Join(root, "hub.db"), "binary")
	mustWrite(t, filepath.Join(root, "live.log"), "log\n")
	mustMkdir(t, filepath.Join(root, "brian"))
	mustWrite(t, filepath.Join(root, "brian", "last_state.json"), "{}\n")
	mustMkdir(t, filepath.Join(root, "gates"))
	mustMkdir(t, filepath.Join(root, ".git"))

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}

	var payload struct {
		Tree []TreeNode `json:"tree"`
	}
	if err := json.Unmarshal([]byte(body), &payload); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, body)
	}

	names := map[string]bool{}
	for _, n := range payload.Tree {
		names[n.Name] = true
	}
	for _, want := range []string{"phase", "ratchets"} {
		if !names[want] {
			t.Errorf("expected canonical entry %q in tree, got names=%v", want, names)
		}
	}
	for _, skip := range []string{"hub.db", "live.log", "brian", "gates", ".git"} {
		if names[skip] {
			t.Errorf("expected skipped entry %q absent from tree, got names=%v", skip, names)
		}
	}
}

func TestHandleFileContent_ReadsCanonicalFile(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "phase"))
	mustWrite(t, filepath.Join(root, "phase", "phase-n.md"), "# Phase N v3\n")

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files/phase/phase-n.md")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	if !strings.Contains(body, "# Phase N v3") {
		t.Errorf("expected file content in body, got %q", body)
	}
}

func TestHandleFileContent_FormatJSON(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "ratchets"))
	mustWrite(t, filepath.Join(root, "ratchets", "active.md"), "ledger\n")

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files/ratchets/active.md?format=json")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	var p map[string]any
	if err := json.Unmarshal([]byte(body), &p); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, body)
	}
	if p["path"] != "ratchets/active.md" {
		t.Errorf("path = %v, want ratchets/active.md", p["path"])
	}
	if p["content"] != "ledger\n" {
		t.Errorf("content = %v, want 'ledger\\n'", p["content"])
	}
	if _, has := p["mtime"]; !has {
		t.Errorf("missing mtime")
	}
}

func TestHandleFileContent_RejectsSkippedPath(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "hub.db"), "binary")

	s := newTestServer(t, root)
	status, _ := callRoute(t, s, "GET", "/api/files/hub.db")
	if status == http.StatusOK {
		t.Errorf("expected non-200 for skipped path, got %d", status)
	}
}

func TestHandleFileContent_RejectsPathEscape(t *testing.T) {
	root := t.TempDir()

	s := newTestServer(t, root)
	status, _ := callRoute(t, s, "GET", "/api/files/../etc/passwd")
	if status == http.StatusOK {
		t.Errorf("expected non-200 for path-escape attempt, got %d", status)
	}
}

func TestHandleRules_ResolvesGeneralAndProject(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules"))
	mustMkdir(t, filepath.Join(root, "rules", "projects"))
	mustWrite(t, filepath.Join(root, "rules", "general.yaml"), "tone:\n  reply: \"general-reply\"\n  eod: \"general-eod\"\n")
	mustWrite(t, filepath.Join(root, "rules", "projects", "test-proj.yaml"), "tone:\n  reply: \"project-reply\"\n")

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/rules?project=test-proj")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	var resolved map[string]any
	if err := json.Unmarshal([]byte(body), &resolved); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, body)
	}
	tone, ok := resolved["tone"].(map[string]any)
	if !ok {
		t.Fatalf("tone missing or wrong type: %v", resolved["tone"])
	}
	if tone["reply"] != "project-reply" {
		t.Errorf("tone.reply = %v, want project-reply (project wins)", tone["reply"])
	}
	if tone["eod"] != "general-eod" {
		t.Errorf("tone.eod = %v, want general-eod (inherits)", tone["eod"])
	}
}

func TestHandleRules_LoadsAgentLayer(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules", "agents"))
	mustWrite(t, filepath.Join(root, "rules", "agents", "brian.yaml"), "role: \"HANDS\"\n")

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/rules?agent=brian")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	var resolved map[string]any
	if err := json.Unmarshal([]byte(body), &resolved); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, body)
	}
	agentLayer, ok := resolved["agent"].(map[string]any)
	if !ok {
		t.Fatalf("agent layer missing: %v", resolved)
	}
	if agentLayer["role"] != "HANDS" {
		t.Errorf("agent.role = %v, want HANDS", agentLayer["role"])
	}
}

func TestHandleSessions_EmptyIndex(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/sessions")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	var p map[string]any
	if err := json.Unmarshal([]byte(body), &p); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, body)
	}
	if p["index"] != "" {
		t.Errorf("index = %v, want empty (no sessions yet)", p["index"])
	}
}

func TestHandleCliveActivity_NoDB(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	// db not set — should return 503 gracefully
	status, _ := callRoute(t, s, "GET", "/api/clive/activity")
	if status != http.StatusServiceUnavailable {
		t.Errorf("status = %d, want 503 when db nil", status)
	}
}

func TestStaticHandler_ServesIndex(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	if !strings.Contains(body, "bot-hq Clive workspace") {
		t.Errorf("expected index.html title in body, got first 200 chars: %s", body[:min(200, len(body))])
	}
}

func TestResolveCanonicalPath_RejectsDotfile(t *testing.T) {
	root := t.TempDir()
	if _, err := resolveCanonicalPath(root, ".git/HEAD"); err == nil {
		t.Errorf("expected error for dotfile path, got nil")
	}
}

func TestResolveCanonicalPath_RejectsEscape(t *testing.T) {
	root := t.TempDir()
	if _, err := resolveCanonicalPath(root, "../../etc/passwd"); err == nil {
		t.Errorf("expected error for escape path, got nil")
	}
}

func TestResolveCanonicalPath_AcceptsNormal(t *testing.T) {
	root := t.TempDir()
	abs, err := resolveCanonicalPath(root, "phase/phase-n.md")
	if err != nil {
		t.Fatalf("resolveCanonicalPath: %v", err)
	}
	want := filepath.Join(root, "phase", "phase-n.md")
	if abs != want {
		t.Errorf("abs = %q, want %q", abs, want)
	}
}

// helpers

func mustMkdir(t *testing.T, p string) {
	t.Helper()
	if err := os.MkdirAll(p, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", p, err)
	}
}

func mustWrite(t *testing.T, p, content string) {
	t.Helper()
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatalf("write %s: %v", p, err)
	}
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
