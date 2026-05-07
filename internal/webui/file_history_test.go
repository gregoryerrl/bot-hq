package webui

// Tests for the file-history endpoint + helper (P-3 / phase-n.md:544).
// fileHistory drives the revert UI: lists per-dir-git commits touching
// a file so the user can pick a target SHA to revert to.

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestFileHistory_NoGit_ReturnsEmpty verifies the graceful path: when
// the file's top-dir has no .git/, fileHistory returns (nil, nil)
// rather than erroring — the frontend can show "no history yet".
func TestFileHistory_NoGit_ReturnsEmpty(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules"))
	mustWrite(t, filepath.Join(root, "rules", "general.yaml"), "key: value\n")

	commits, err := fileHistory(root, "rules/general.yaml", 50)
	if err != nil {
		t.Fatalf("fileHistory: %v", err)
	}
	if commits != nil {
		t.Errorf("expected nil commits when no .git exists, got %v", commits)
	}
}

// TestFileHistory_RootLevelFile_ReturnsEmpty confirms files at the
// canonical-store root (no top-dir) return nil — root-level files
// have no per-dir git per the canonical-git scoping in git.go.
func TestFileHistory_RootLevelFile_ReturnsEmpty(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "README.md"), "# README\n")

	commits, err := fileHistory(root, "README.md", 50)
	if err != nil {
		t.Fatalf("fileHistory: %v", err)
	}
	if commits != nil {
		t.Errorf("expected nil for root-level file, got %v", commits)
	}
}

// TestFileHistory_ListsCommits writes a file, commits it twice via
// the existing canonical-git plumbing, then asserts fileHistory
// returns both commits in reverse-chronological order with parsed
// fields populated.
func TestFileHistory_ListsCommits(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules"))
	relPath := "rules/general.yaml"
	abs := filepath.Join(root, relPath)
	mustWrite(t, abs, "v1: yes\n")

	if _, _, err := ensureCanonicalGit(root, relPath); err != nil {
		t.Fatalf("ensure git: %v", err)
	}
	if _, err := commitCanonicalChange(root, relPath, "user", "first commit"); err != nil {
		t.Fatalf("commit 1: %v", err)
	}
	if err := os.WriteFile(abs, []byte("v2: yes\n"), 0o644); err != nil {
		t.Fatalf("write v2: %v", err)
	}
	if _, err := commitCanonicalChange(root, relPath, "user", "second commit"); err != nil {
		t.Fatalf("commit 2: %v", err)
	}

	commits, err := fileHistory(root, relPath, 50)
	if err != nil {
		t.Fatalf("fileHistory: %v", err)
	}
	if len(commits) != 2 {
		t.Fatalf("commits len = %d, want 2; got %v", len(commits), commits)
	}
	if commits[0].Subject != "second commit" {
		t.Errorf("first entry should be most-recent (second commit); got %q", commits[0].Subject)
	}
	if commits[1].Subject != "first commit" {
		t.Errorf("second entry should be oldest (first commit); got %q", commits[1].Subject)
	}
	for i, c := range commits {
		if c.SHA == "" || len(c.SHA) < 7 {
			t.Errorf("commits[%d].SHA looks malformed: %q", i, c.SHA)
		}
		if c.Time == 0 {
			t.Errorf("commits[%d].Time = 0; expected unix seconds", i)
		}
		if c.Author == "" {
			t.Errorf("commits[%d].Author empty", i)
		}
	}
}

// TestFileHistory_LimitClamp verifies the limit param caps the result
// length even when more commits exist.
func TestFileHistory_LimitClamp(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules"))
	relPath := "rules/general.yaml"
	abs := filepath.Join(root, relPath)
	mustWrite(t, abs, "v: 0\n")
	if _, _, err := ensureCanonicalGit(root, relPath); err != nil {
		t.Fatalf("ensure git: %v", err)
	}
	for i := 0; i < 5; i++ {
		mustWrite(t, abs, "v: "+string(rune('0'+i))+"\n")
		if _, err := commitCanonicalChange(root, relPath, "user", "commit "+string(rune('0'+i))); err != nil {
			t.Fatalf("commit %d: %v", i, err)
		}
	}

	commits, err := fileHistory(root, relPath, 3)
	if err != nil {
		t.Fatalf("fileHistory: %v", err)
	}
	if len(commits) != 3 {
		t.Errorf("expected 3 commits with limit=3, got %d", len(commits))
	}
}

// TestHandleFileHistory_Endpoint verifies the HTTP handler returns the
// expected JSON shape on a happy-path file.
func TestHandleFileHistory_Endpoint(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules"))
	relPath := "rules/general.yaml"
	abs := filepath.Join(root, relPath)
	mustWrite(t, abs, "k: v\n")
	if _, _, err := ensureCanonicalGit(root, relPath); err != nil {
		t.Fatalf("ensure git: %v", err)
	}
	if _, err := commitCanonicalChange(root, relPath, "user", "init"); err != nil {
		t.Fatalf("commit: %v", err)
	}

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files/"+relPath+"/history")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	var resp map[string]any
	if err := json.Unmarshal([]byte(body), &resp); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	commits, ok := resp["commits"].([]any)
	if !ok {
		t.Fatalf("commits field missing or wrong type: %v", resp)
	}
	if len(commits) != 1 {
		t.Errorf("commits len = %d, want 1", len(commits))
	}
}

// TestHandleFileHistory_NoGit_ReturnsEmptyArray verifies the JSON
// response always has a `commits` array (never null) for ergonomic
// consumption by frontend code that treats null as a hard error.
func TestHandleFileHistory_NoGit_ReturnsEmptyArray(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules"))
	mustWrite(t, filepath.Join(root, "rules", "general.yaml"), "k: v\n")

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files/rules/general.yaml/history")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	if !strings.Contains(body, `"commits": []`) && !strings.Contains(body, `"commits":[]`) {
		t.Errorf("expected commits=[] for no-git case; body=%s", body)
	}
}

// TestHandleFileHistory_LimitParam confirms ?limit=N is honored.
func TestHandleFileHistory_LimitParam(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules"))
	relPath := "rules/general.yaml"
	abs := filepath.Join(root, relPath)
	if _, _, err := ensureCanonicalGit(root, relPath); err != nil {
		t.Fatalf("ensure git: %v", err)
	}
	for i := 0; i < 4; i++ {
		mustWrite(t, abs, "v: "+string(rune('a'+i))+"\n")
		if _, err := commitCanonicalChange(root, relPath, "user", "c"+string(rune('a'+i))); err != nil {
			t.Fatalf("commit %d: %v", i, err)
		}
	}
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files/rules/general.yaml/history?limit=2")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	var resp map[string]any
	if err := json.Unmarshal([]byte(body), &resp); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	commits := resp["commits"].([]any)
	if len(commits) != 2 {
		t.Errorf("commits len = %d, want 2 (limit clamp)", len(commits))
	}
}

// TestHandleFileHistory_MethodNotAllowed verifies non-GET requests are
// rejected with 405.
func TestHandleFileHistory_MethodNotAllowed(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	req := httptest.NewRequest("POST", "/api/files/rules/general.yaml/history", nil)
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusMethodNotAllowed {
		t.Errorf("status = %d, want 405", w.Code)
	}
}
