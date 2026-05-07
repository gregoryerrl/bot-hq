package webui

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

// TestExternalDirFiles covers the resolver helper directly: missing dir
// returns (nil, nil); present dir surfaces files with External=true and
// the "external/<project>/<basename>" path scheme.
func TestExternalDirFiles_MissingDirReturnsEmpty(t *testing.T) {
	nodes, err := externalDirFiles("/nonexistent/path-1234", "myproj", ".md")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(nodes) != 0 {
		t.Errorf("nodes = %d, want 0 for missing dir", len(nodes))
	}
}

func TestExternalDirFiles_SurfacesWithExternalFlag(t *testing.T) {
	dir := t.TempDir()
	mustWrite(t, filepath.Join(dir, "alpha.md"), "alpha\n")
	mustWrite(t, filepath.Join(dir, "beta.md"), "beta\n")
	mustWrite(t, filepath.Join(dir, "skip.txt"), "skip\n") // ext filter

	nodes, err := externalDirFiles(dir, "myproj", ".md")
	if err != nil {
		t.Fatal(err)
	}
	if len(nodes) != 2 {
		t.Fatalf("nodes = %d, want 2; got %+v", len(nodes), nodes)
	}
	for _, n := range nodes {
		if !n.External {
			t.Errorf("node %s External=false, want true", n.Name)
		}
	}
	if nodes[0].Path != "external/myproj/alpha.md" {
		t.Errorf("path = %s, want external/myproj/alpha.md", nodes[0].Path)
	}
	if nodes[1].Path != "external/myproj/beta.md" {
		t.Errorf("path = %s, want external/myproj/beta.md", nodes[1].Path)
	}
}

// TestHandleExternalFile_RegisteredProjectReadOK and friends exercise the
// /api/external-file endpoint behavior. We can't relocate $HOME for the
// real on-disk read, so these tests use a sub-test approach with a
// temp HOME so docsRoot resolves into a controlled path.
func TestHandleExternalFile_EndToEnd(t *testing.T) {
	// Set up a temp HOME with the registered-project + docs structure
	// matching what handleExternalFile expects.
	homeTmp := t.TempDir()
	t.Setenv("HOME", homeTmp)

	canonRoot := filepath.Join(homeTmp, ".bot-hq-canon")
	mustMkdir(t, canonRoot)
	mustMkdir(t, filepath.Join(canonRoot, "projects"))
	mustWrite(t, filepath.Join(canonRoot, "projects", "myproj.yaml"), "project_name: myproj\n")

	docsDir := filepath.Join(homeTmp, "Projects", "myproj", "docs")
	mustMkdir(t, docsDir)
	mustWrite(t, filepath.Join(docsDir, "guide.md"), "external content\n")

	s := newTestServerWithProposals(t, canonRoot)

	t.Run("registered project read returns content", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodGet, "/api/external-file/myproj/guide.md?format=json", nil)
		rr := httptest.NewRecorder()
		s.handleExternalFile(rr, req)
		if rr.Code != http.StatusOK {
			t.Fatalf("status = %d, body = %s", rr.Code, rr.Body.String())
		}
		var payload map[string]any
		if err := json.Unmarshal(rr.Body.Bytes(), &payload); err != nil {
			t.Fatal(err)
		}
		if got, _ := payload["content"].(string); got != "external content\n" {
			t.Errorf("content = %q", got)
		}
		if ext, _ := payload["external"].(bool); !ext {
			t.Errorf("external = %v, want true", ext)
		}
	})

	t.Run("unknown project returns 404", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodGet, "/api/external-file/notregistered/guide.md", nil)
		rr := httptest.NewRecorder()
		s.handleExternalFile(rr, req)
		if rr.Code != http.StatusNotFound {
			t.Errorf("status = %d, want 404", rr.Code)
		}
	})

	t.Run("path-traversal attempt rejected", func(t *testing.T) {
		// Plant a file outside the docs/ subdir.
		secret := filepath.Join(homeTmp, "Projects", "myproj", "secret.md")
		mustWrite(t, secret, "secret\n")
		req := httptest.NewRequest(http.MethodGet, "/api/external-file/myproj/../secret.md", nil)
		rr := httptest.NewRecorder()
		s.handleExternalFile(rr, req)
		// Either 400 (rejected by guard) or 404 (path normalization). Both
		// are acceptable; what's NOT acceptable is 200 + secret content.
		if rr.Code == http.StatusOK {
			body := rr.Body.String()
			if filepath.Base(body) == "secret\n" || (len(body) > 0 && body[:6] == "secret") {
				t.Errorf("traversal succeeded: body = %q", body)
			}
		}
	})

	t.Run("missing file returns 404", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodGet, "/api/external-file/myproj/no-such.md", nil)
		rr := httptest.NewRecorder()
		s.handleExternalFile(rr, req)
		if rr.Code != http.StatusNotFound {
			t.Errorf("status = %d, want 404", rr.Code)
		}
	})

	t.Run("POST returns 405", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodPost, "/api/external-file/myproj/guide.md", nil)
		rr := httptest.NewRecorder()
		s.handleExternalFile(rr, req)
		if rr.Code != http.StatusMethodNotAllowed {
			t.Errorf("status = %d, want 405", rr.Code)
		}
	})

	t.Run("missing project segment returns 400", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodGet, "/api/external-file/", nil)
		rr := httptest.NewRecorder()
		s.handleExternalFile(rr, req)
		if rr.Code != http.StatusBadRequest {
			t.Errorf("status = %d, want 400", rr.Code)
		}
	})
}

// TestResolveProjectExternalDocs_PicksUpDocsDir verifies the destination
// resolver wires HOME → ~/Projects/<p>/docs/ correctly.
func TestResolveProjectExternalDocs_PicksUpDocsDir(t *testing.T) {
	homeTmp := t.TempDir()
	t.Setenv("HOME", homeTmp)

	docsDir := filepath.Join(homeTmp, "Projects", "myproj", "docs")
	mustMkdir(t, docsDir)
	mustWrite(t, filepath.Join(docsDir, "alpha.md"), "alpha\n")

	nodes, err := resolveProjectExternalDocs("", "myproj")
	if err != nil {
		t.Fatal(err)
	}
	if len(nodes) != 1 {
		t.Fatalf("nodes = %d, want 1", len(nodes))
	}
	if !nodes[0].External {
		t.Errorf("External = false, want true")
	}
	if nodes[0].Path != "external/myproj/alpha.md" {
		t.Errorf("path = %s", nodes[0].Path)
	}
}

// Suppress unused-import lint when running just this file in isolation.
var _ = os.Stat
