package webui

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// fixtureMultiProject creates two projects under root, each with one
// architecture/ file and one decisions/ file, so cross-project queries
// have something to group across.
func fixtureMultiProject(t *testing.T, root string) {
	t.Helper()
	for _, p := range []string{"bot-hq", "alpha"} {
		dir := filepath.Join(root, "projects", p)
		if err := os.MkdirAll(filepath.Join(dir, "architecture"), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.MkdirAll(filepath.Join(dir, "decisions"), 0o755); err != nil {
			t.Fatal(err)
		}
		mustWrite(t, filepath.Join(root, "projects", p+".yaml"), "project_name: "+p+"\nremote_url: \"\"\n")
		mustWrite(t, filepath.Join(dir, "architecture", "doc.md"), "# "+p+" arch\n")
		mustWrite(t, filepath.Join(dir, "decisions", "doc.md"), "# "+p+" decision\n")
	}
}

func TestComputeCrossProject_GroupsFilesByProject(t *testing.T) {
	root := t.TempDir()
	fixtureMultiProject(t, root)

	resp, err := computeCrossProject(root, "architecture")
	if err != nil {
		t.Fatal(err)
	}
	if resp.Class != "architecture" {
		t.Errorf("class = %q, want architecture", resp.Class)
	}
	if resp.Total != 2 {
		t.Errorf("total = %d, want 2 (one per project)", resp.Total)
	}
	if len(resp.Projects) != 2 {
		t.Fatalf("projects count = %d, want 2; got %+v", len(resp.Projects), resp.Projects)
	}
	for _, p := range resp.Projects {
		if p.Count != 1 {
			t.Errorf("project %s count = %d, want 1", p.Project, p.Count)
		}
		if len(p.Files) != 1 {
			t.Errorf("project %s files = %d, want 1", p.Project, len(p.Files))
		}
		if p.Files[0].Class != "architecture" {
			t.Errorf("project %s file class = %q, want architecture", p.Project, p.Files[0].Class)
		}
	}
	if resp.Projects[0].Project != "alpha" || resp.Projects[1].Project != "bot-hq" {
		t.Errorf("projects not alphabetically sorted: %+v", resp.Projects)
	}
}

func TestComputeCrossProject_ClassWithNoMatches(t *testing.T) {
	root := t.TempDir()
	fixtureMultiProject(t, root)

	resp, err := computeCrossProject(root, "nonexistent-class")
	if err != nil {
		t.Fatal(err)
	}
	if resp.Total != 0 {
		t.Errorf("total = %d, want 0", resp.Total)
	}
	for _, p := range resp.Projects {
		if p.Count != 0 {
			t.Errorf("project %s count = %d, want 0", p.Project, p.Count)
		}
	}
}

func TestComputeCrossProject_EmptyClassErrors(t *testing.T) {
	_, err := computeCrossProject(t.TempDir(), "")
	if err == nil {
		t.Error("expected error for empty class")
	}
}

func TestCrossProjectCache_LookupMissEmpty(t *testing.T) {
	c := newCrossProjectCache()
	if _, ok := c.lookup("anything"); ok {
		t.Error("expected miss on empty cache")
	}
}

func TestCrossProjectCache_LookupStoreRoundTrip(t *testing.T) {
	c := newCrossProjectCache()
	c.store("architecture", crossProjectResponse{Class: "architecture", Total: 5})
	got, ok := c.lookup("architecture")
	if !ok {
		t.Fatal("expected hit after store")
	}
	if got.Total != 5 {
		t.Errorf("total = %d, want 5", got.Total)
	}
}

func TestCrossProjectCache_Expires(t *testing.T) {
	c := newCrossProjectCache()
	c.entries["architecture"] = crossProjectCacheEntry{
		expires: time.Now().Add(-time.Second),
		data:    crossProjectResponse{Class: "architecture", Total: 99},
	}
	if _, ok := c.lookup("architecture"); ok {
		t.Error("expected miss on expired entry")
	}
}

func TestHandleCrossProject_RequiresClass(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/cross-project")
	if status != http.StatusBadRequest {
		t.Errorf("status = %d body=%s, want 400", status, body)
	}
}

func TestHandleCrossProject_EndToEnd(t *testing.T) {
	root := t.TempDir()
	fixtureMultiProject(t, root)
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/cross-project?class=architecture")
	if status != http.StatusOK {
		t.Fatalf("status = %d body=%s", status, body)
	}
	var resp crossProjectResponse
	if err := json.Unmarshal([]byte(body), &resp); err != nil {
		t.Fatal(err)
	}
	if resp.Total != 2 {
		t.Errorf("total = %d, want 2", resp.Total)
	}
	if resp.Class != "architecture" {
		t.Errorf("class = %q, want architecture", resp.Class)
	}
}

func TestHandleCrossProject_CacheHitDoesNotRewalk(t *testing.T) {
	root := t.TempDir()
	fixtureMultiProject(t, root)
	s := newTestServer(t, root)

	// First request populates cache.
	status, _ := callRoute(t, s, "GET", "/api/cross-project?class=architecture")
	if status != http.StatusOK {
		t.Fatalf("first request status = %d", status)
	}
	// Mutate the fixture so a re-walk would produce a different result.
	if err := os.Remove(filepath.Join(root, "projects", "bot-hq", "architecture", "doc.md")); err != nil {
		t.Fatal(err)
	}
	// Second request — should hit cache and still return 2 (stale ok).
	status, body := callRoute(t, s, "GET", "/api/cross-project?class=architecture")
	if status != http.StatusOK {
		t.Fatalf("second status = %d", status)
	}
	var resp crossProjectResponse
	if err := json.Unmarshal([]byte(body), &resp); err != nil {
		t.Fatal(err)
	}
	if resp.Total != 2 {
		t.Errorf("cache-hit total = %d, want 2 (cached pre-deletion)", resp.Total)
	}
}
