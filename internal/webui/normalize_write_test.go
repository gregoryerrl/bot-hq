package webui

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestHandleFileWrite_ProjectsYAMLNormalized: POST flat-form per-project
// YAML to /api/files/projects/<p>.yaml -> daemon normalizes + persists
// canonical nested form. Phase O drain #6 wiring verification.
func TestHandleFileWrite_ProjectsYAMLNormalized(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "projects"))
	s := newTestServerWithProposals(t, root)

	flatBody := strings.NewReader(`project_name: bot-hq
remote_url: git@github.com:foo/bar.git
push_requires_approval: true
branch_pattern: feat/*
commit_style: imperative-mood
`)

	req := httptest.NewRequest("POST", "/api/files/projects/bot-hq.yaml", flatBody)
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
	}

	persisted, err := os.ReadFile(filepath.Join(root, "projects", "bot-hq.yaml"))
	if err != nil {
		t.Fatalf("read persisted: %v", err)
	}
	persistedStr := string(persisted)

	// Canonical form expected.
	for _, want := range []string{
		"project_name: bot-hq",
		"branch:",
		"pattern: feat/*",
		"commit:",
		"style: imperative-mood",
		"gates:",
		"requiresApproval: true",
	} {
		if !strings.Contains(persistedStr, want) {
			t.Errorf("persisted output missing canonical key %q.\nGot:\n%s", want, persistedStr)
		}
	}

	// Flat keys must NOT have been persisted.
	for _, banned := range []string{"push_requires_approval:", "branch_pattern:", "commit_style:"} {
		if strings.Contains(persistedStr, banned) {
			t.Errorf("flat key %q still in persisted file (normalization not applied).\nGot:\n%s", banned, persistedStr)
		}
	}
}

// TestHandleFileWrite_NonProjectsYAMLPassesThrough: POST flat-shaped
// content under a non-projects path -> normalization NOT applied (only
// projects/*.yaml are normalized). Confirms the path-discriminator.
func TestHandleFileWrite_NonProjectsYAMLPassesThrough(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "phase"))
	s := newTestServerWithProposals(t, root)

	body := strings.NewReader("# anything\nkey: value\n")
	req := httptest.NewRequest("POST", "/api/files/phase/note.md", body)
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
	}
	persisted, _ := os.ReadFile(filepath.Join(root, "phase", "note.md"))
	if string(persisted) != "# anything\nkey: value\n" {
		t.Errorf("non-projects path was modified by normalization: %q", string(persisted))
	}
}

// TestHandleFileWrite_ProjectsYAMLBadInputRejected: malformed YAML at a
// projects/<p>.yaml path -> 400, file not written. Guards against
// persisting unparseable content.
func TestHandleFileWrite_ProjectsYAMLBadInputRejected(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "projects"))
	s := newTestServerWithProposals(t, root)

	bad := strings.NewReader("project_name: [unclosed\n")
	req := httptest.NewRequest("POST", "/api/files/projects/bot-hq.yaml", bad)
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusBadRequest {
		t.Errorf("status = %d, want 400 for malformed yaml; body=%s", w.Code, w.Body.String())
	}
	if _, err := os.Stat(filepath.Join(root, "projects", "bot-hq.yaml")); !os.IsNotExist(err) {
		t.Errorf("file should not exist after normalize-failure, got err=%v", err)
	}
}

// TestIsProjectsYAMLPath locks the path-discriminator. Direct unit
// coverage of the helper for grep-via-test.
func TestIsProjectsYAMLPath(t *testing.T) {
	cases := []struct {
		path string
		want bool
	}{
		{"projects/bot-hq.yaml", true},
		{"projects/bcc-ad-manager.yaml", true},
		{"projects/foo/bar.yaml", true}, // nested ok (future-compat)
		{"projects/notes.md", false},
		{"projects/bot-hq.yml", false}, // wrong extension
		{"rules/general.yaml", false},
		{"rules/agents/brian.yaml", false},
		{"phase/phase-n.md", false},
		{"", false},
	}
	for _, c := range cases {
		got := isProjectsYAMLPath(c.path)
		if got != c.want {
			t.Errorf("isProjectsYAMLPath(%q) = %v, want %v", c.path, got, c.want)
		}
	}
}
