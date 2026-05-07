package webui

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// TestListRecentEdits_OrderedByMtimeDesc: most-recently-modified file
// surfaces first; entries past limit are dropped.
func TestListRecentEdits_OrderedByMtimeDesc(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "phase"))
	mustMkdir(t, filepath.Join(root, "ratchets"))
	now := time.Now().UTC()
	older := now.Add(-2 * time.Hour)
	oldest := now.Add(-24 * time.Hour)
	writeAtMtime(t, filepath.Join(root, "phase", "old.md"), oldest)
	writeAtMtime(t, filepath.Join(root, "ratchets", "active.md"), older)
	writeAtMtime(t, filepath.Join(root, "discipline-log.md"), now)

	edits, err := ListRecentEdits(root, 5)
	if err != nil {
		t.Fatalf("ListRecentEdits: %v", err)
	}
	if len(edits) != 3 {
		t.Fatalf("len = %d (expected 3); edits = %+v", len(edits), edits)
	}
	if edits[0].Path != "discipline-log.md" {
		t.Errorf("[0] = %s, expected discipline-log.md", edits[0].Path)
	}
	if edits[1].Path != "ratchets/active.md" {
		t.Errorf("[1] = %s, expected ratchets/active.md", edits[1].Path)
	}
	if edits[2].Path != "phase/old.md" {
		t.Errorf("[2] = %s, expected phase/old.md", edits[2].Path)
	}
}

// TestListRecentEdits_LimitClamps: limit truncates to top-N.
func TestListRecentEdits_LimitClamps(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "phase"))
	now := time.Now().UTC()
	for i := 0; i < 5; i++ {
		mustWrite(t, filepath.Join(root, "phase", fileNameI(i)+".md"), "x")
		// Stagger mtimes so order is deterministic.
		writeAtMtime(t, filepath.Join(root, "phase", fileNameI(i)+".md"), now.Add(-time.Duration(i)*time.Minute))
	}
	edits, err := ListRecentEdits(root, 3)
	if err != nil {
		t.Fatalf("ListRecentEdits: %v", err)
	}
	if len(edits) != 3 {
		t.Errorf("limit=3 truncated to %d", len(edits))
	}
}

// TestListRecentEdits_SkipsRuntimeState: HIDE-list entries (hub.db,
// agent dirs, .git) never surface even if newest.
func TestListRecentEdits_SkipsRuntimeState(t *testing.T) {
	root := t.TempDir()
	now := time.Now().UTC()
	mustWrite(t, filepath.Join(root, "hub.db"), "binary")
	mustMkdir(t, filepath.Join(root, "brian"))
	mustWrite(t, filepath.Join(root, "brian", "last_state.json"), "{}")
	mustMkdir(t, filepath.Join(root, ".git"))
	mustWrite(t, filepath.Join(root, ".git", "HEAD"), "ref:")
	writeAtMtime(t, filepath.Join(root, "hub.db"), now)
	writeAtMtime(t, filepath.Join(root, "brian", "last_state.json"), now)
	mustWrite(t, filepath.Join(root, "discipline-log.md"), "log")
	writeAtMtime(t, filepath.Join(root, "discipline-log.md"), now.Add(-1*time.Hour))

	edits, err := ListRecentEdits(root, 10)
	if err != nil {
		t.Fatalf("ListRecentEdits: %v", err)
	}
	for _, e := range edits {
		if e.Path == "hub.db" || e.Path == "brian/last_state.json" || e.Path == ".git/HEAD" {
			t.Errorf("HIDE-list path surfaced: %s", e.Path)
		}
	}
	if len(edits) != 1 || edits[0].Path != "discipline-log.md" {
		t.Errorf("expected only discipline-log.md, got %+v", edits)
	}
}

// TestHandleRecentEdits_Endpoint: GET /api/recent-edits returns JSON
// {edits:[]} sorted by mtime desc.
func TestHandleRecentEdits_Endpoint(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "discipline-log.md"), "log")
	mustMkdir(t, filepath.Join(root, "phase"))
	mustWrite(t, filepath.Join(root, "phase", "phase-n.md"), "phase")
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/recent-edits")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	var p struct {
		Edits []RecentEdit `json:"edits"`
	}
	if err := json.Unmarshal([]byte(body), &p); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(p.Edits) < 2 {
		t.Errorf("expected >=2 edits, got %d", len(p.Edits))
	}
}

// TestHandleRecentEdits_LimitParam: ?limit=N respected within [1,100];
// out-of-range falls back to default 20.
func TestHandleRecentEdits_LimitParam(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "phase"))
	for i := 0; i < 5; i++ {
		mustWrite(t, filepath.Join(root, "phase", fileNameI(i)+".md"), "x")
	}
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/recent-edits?limit=2")
	if status != http.StatusOK {
		t.Fatalf("status = %d", status)
	}
	var p struct {
		Edits []RecentEdit `json:"edits"`
	}
	_ = json.Unmarshal([]byte(body), &p)
	if len(p.Edits) != 2 {
		t.Errorf("limit=2: got %d edits", len(p.Edits))
	}
}

// TestHandleRecentEdits_MethodNotAllowed: POST → 405.
func TestHandleRecentEdits_MethodNotAllowed(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	status, _ := callRoute(t, s, "POST", "/api/recent-edits")
	if status != http.StatusMethodNotAllowed {
		t.Errorf("expected 405 on POST, got %d", status)
	}
}

func fileNameI(i int) string {
	return string(rune('a' + i))
}

func writeAtMtime(t *testing.T, path string, mtime time.Time) {
	t.Helper()
	if err := os.WriteFile(path, []byte("x"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	if err := os.Chtimes(path, mtime, mtime); err != nil {
		t.Fatalf("chtimes: %v", err)
	}
}
