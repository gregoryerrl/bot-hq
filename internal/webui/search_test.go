package webui

import (
	"encoding/json"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
)

// TestSearchCanonicalStore_BasicMatch: substring case-insensitive
// matches one line in a single file.
func TestSearchCanonicalStore_BasicMatch(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "discipline-log.md"),
		"first line\nMatching FOO line here\nthird line\n")
	results, err := SearchCanonicalStore(root, "foo", 10)
	if err != nil {
		t.Fatalf("Search: %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("len = %d (want 1); results = %+v", len(results), results)
	}
	r := results[0]
	if r.Path != "discipline-log.md" || r.Line != 2 {
		t.Errorf("got %+v", r)
	}
	if !strings.Contains(r.Snippet, "FOO") {
		t.Errorf("snippet missing FOO: %q", r.Snippet)
	}
}

// TestSearchCanonicalStore_MultipleFiles: matches surface across files
// in canonical-store walk order; limit clamps total.
func TestSearchCanonicalStore_MultipleFiles(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "phase"))
	mustWrite(t, filepath.Join(root, "discipline-log.md"), "alpha BAR beta\n")
	mustWrite(t, filepath.Join(root, "phase", "phase-n.md"), "BAR here\nand BAR again\n")
	results, err := SearchCanonicalStore(root, "bar", 10)
	if err != nil {
		t.Fatalf("Search: %v", err)
	}
	if len(results) != 3 {
		t.Errorf("len = %d (want 3); results = %+v", len(results), results)
	}
}

// TestSearchCanonicalStore_LimitClamps: stops walking once limit hit.
func TestSearchCanonicalStore_LimitClamps(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "discipline-log.md"),
		"BAZ\nBAZ\nBAZ\nBAZ\nBAZ\n")
	results, err := SearchCanonicalStore(root, "baz", 2)
	if err != nil {
		t.Fatalf("Search: %v", err)
	}
	if len(results) != 2 {
		t.Errorf("limit=2: got %d", len(results))
	}
}

// TestSearchCanonicalStore_SkipsRuntimeState: HIDE-list entries
// (hub.db, agent dirs) never surface even if they contain the query.
func TestSearchCanonicalStore_SkipsRuntimeState(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "hub.db"), "QUUX in hub.db\n")
	mustMkdir(t, filepath.Join(root, "brian"))
	mustWrite(t, filepath.Join(root, "brian", "last_state.json"), "QUUX in agent\n")
	mustWrite(t, filepath.Join(root, "discipline-log.md"), "QUUX in canonical\n")
	results, err := SearchCanonicalStore(root, "quux", 10)
	if err != nil {
		t.Fatalf("Search: %v", err)
	}
	for _, r := range results {
		if r.Path == "hub.db" || strings.HasPrefix(r.Path, "brian/") {
			t.Errorf("HIDE-list path surfaced: %s", r.Path)
		}
	}
	if len(results) != 1 || results[0].Path != "discipline-log.md" {
		t.Errorf("expected only discipline-log.md, got %+v", results)
	}
}

// TestSearchCanonicalStore_EmptyQuery: empty query → empty results.
func TestSearchCanonicalStore_EmptyQuery(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "discipline-log.md"), "anything\n")
	results, err := SearchCanonicalStore(root, "", 10)
	if err != nil {
		t.Fatalf("Search: %v", err)
	}
	if results != nil {
		t.Errorf("empty query: got %+v", results)
	}
}

// TestSearchCanonicalStore_LongLineSnippetTruncated: matched line
// longer than searchSnippetMax → snippet truncated with ellipsis.
func TestSearchCanonicalStore_LongLineSnippetTruncated(t *testing.T) {
	root := t.TempDir()
	long := strings.Repeat("x", 300) + "needle" + strings.Repeat("y", 300)
	mustWrite(t, filepath.Join(root, "discipline-log.md"), long+"\n")
	results, err := SearchCanonicalStore(root, "needle", 10)
	if err != nil {
		t.Fatalf("Search: %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("len = %d", len(results))
	}
	if !strings.HasSuffix(results[0].Snippet, "…") {
		t.Errorf("expected ellipsis suffix on long line; snippet=%q", results[0].Snippet)
	}
}

// TestHandleSearch_Endpoint: basic 200 + results.
func TestHandleSearch_Endpoint(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "discipline-log.md"), "needle\n")
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/search?q=needle")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	var p struct {
		Results []SearchResult `json:"results"`
	}
	_ = json.Unmarshal([]byte(body), &p)
	if len(p.Results) != 1 {
		t.Errorf("len = %d", len(p.Results))
	}
}

// TestHandleSearch_QueryTooShort: q with <2 chars → 400.
func TestHandleSearch_QueryTooShort(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	status, _ := callRoute(t, s, "GET", "/api/search?q=a")
	if status != http.StatusBadRequest {
		t.Errorf("expected 400, got %d", status)
	}
}

// TestHandleSearch_LimitParam: ?limit=N respected.
func TestHandleSearch_LimitParam(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "discipline-log.md"),
		"matchme\nmatchme\nmatchme\nmatchme\nmatchme\n")
	s := newTestServer(t, root)
	_, body := callRoute(t, s, "GET", "/api/search?q=matchme&limit=2")
	var p struct {
		Results []SearchResult `json:"results"`
	}
	_ = json.Unmarshal([]byte(body), &p)
	if len(p.Results) != 2 {
		t.Errorf("limit=2: got %d", len(p.Results))
	}
}

// TestHandleSearch_MethodNotAllowed: POST → 405.
func TestHandleSearch_MethodNotAllowed(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	status, _ := callRoute(t, s, "POST", "/api/search?q=foo")
	if status != http.StatusMethodNotAllowed {
		t.Errorf("expected 405, got %d", status)
	}
}
