package webui

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// callRouteWithBody is a body-bearing variant of callRoute for POST tests.
func callRouteWithBody(t *testing.T, s *Server, method, target string, body []byte) (int, string) {
	t.Helper()
	req := httptest.NewRequest(method, target, bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	respBody, _ := io.ReadAll(w.Result().Body)
	return w.Code, string(respBody)
}

// TestHandleProjectRegister_Success: POST creates the yaml file with
// canonical starter content + returns 201.
func TestHandleProjectRegister_Success(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	body, _ := json.Marshal(map[string]string{
		"name":       "myproj",
		"remote_url": "https://example.com/r.git",
	})
	status, respBody := callRouteWithBody(t, s, "POST", "/api/projects", body)
	if status != http.StatusCreated {
		t.Fatalf("status = %d, body=%s", status, respBody)
	}
	yamlPath := filepath.Join(root, "projects", "myproj.yaml")
	data, err := os.ReadFile(yamlPath)
	if err != nil {
		t.Fatalf("read yaml: %v", err)
	}
	content := string(data)
	if !strings.Contains(content, `project_name: "myproj"`) {
		t.Errorf("missing project_name: %s", content)
	}
	if !strings.Contains(content, `remote_url: "https://example.com/r.git"`) {
		t.Errorf("missing remote_url: %s", content)
	}
}

// TestHandleProjectRegister_EmptyRemoteURL: omitting remote_url emits an
// empty placeholder (template still valid).
func TestHandleProjectRegister_EmptyRemoteURL(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	body, _ := json.Marshal(map[string]string{"name": "noremote"})
	status, _ := callRouteWithBody(t, s, "POST", "/api/projects", body)
	if status != http.StatusCreated {
		t.Fatalf("status = %d", status)
	}
	data, _ := os.ReadFile(filepath.Join(root, "projects", "noremote.yaml"))
	if !strings.Contains(string(data), `remote_url: ""`) {
		t.Errorf("missing empty remote_url placeholder: %s", string(data))
	}
}

// TestHandleProjectRegister_InvalidName: schema regex rejects bad names
// (uppercase, dots, leading-digit, too-long, etc.) with 400.
func TestHandleProjectRegister_InvalidName(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	cases := []string{
		"BadCase",
		"with.dot",
		"1leadingdigit",
		"with/slash",
		"with space",
		"",
		"a", // too short (regex requires 2+)
		strings.Repeat("a", 65),
	}
	for _, n := range cases {
		t.Run(n, func(t *testing.T) {
			body, _ := json.Marshal(map[string]string{"name": n})
			status, _ := callRouteWithBody(t, s, "POST", "/api/projects", body)
			if status != http.StatusBadRequest {
				t.Errorf("name %q: expected 400, got %d", n, status)
			}
		})
	}
}

// TestHandleProjectRegister_ReservedBotHQ: 'bot-hq' is reserved → 400.
func TestHandleProjectRegister_ReservedBotHQ(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	body, _ := json.Marshal(map[string]string{"name": "bot-hq"})
	status, _ := callRouteWithBody(t, s, "POST", "/api/projects", body)
	if status != http.StatusBadRequest {
		t.Errorf("expected 400 on reserved name, got %d", status)
	}
}

// TestHandleProjectRegister_Collision: existing yaml → 409 + no overwrite.
func TestHandleProjectRegister_Collision(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	dir := filepath.Join(root, "projects")
	mustMkdir(t, dir)
	existing := filepath.Join(dir, "alreadythere.yaml")
	mustWrite(t, existing, "preserved-content\n")
	body, _ := json.Marshal(map[string]string{"name": "alreadythere"})
	status, _ := callRouteWithBody(t, s, "POST", "/api/projects", body)
	if status != http.StatusConflict {
		t.Errorf("expected 409 on collision, got %d", status)
	}
	data, _ := os.ReadFile(existing)
	if string(data) != "preserved-content\n" {
		t.Errorf("existing file overwritten: %s", string(data))
	}
}

// TestHandleProjectRegister_PostThenList: registered project shows up in
// subsequent GET /api/projects.
func TestHandleProjectRegister_PostThenList(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	body, _ := json.Marshal(map[string]string{"name": "addedproj"})
	status, _ := callRouteWithBody(t, s, "POST", "/api/projects", body)
	if status != http.StatusCreated {
		t.Fatalf("post status = %d", status)
	}
	listStatus, listBody := callRoute(t, s, "GET", "/api/projects")
	if listStatus != http.StatusOK {
		t.Fatalf("list status = %d", listStatus)
	}
	if !strings.Contains(listBody, "addedproj") {
		t.Errorf("registered project missing from list: %s", listBody)
	}
}

// TestHandleProjectRegister_BadJSON: malformed body → 400.
func TestHandleProjectRegister_BadJSON(t *testing.T) {
	root := t.TempDir()
	s := newTestServer(t, root)
	status, _ := callRouteWithBody(t, s, "POST", "/api/projects", []byte("not-json"))
	if status != http.StatusBadRequest {
		t.Errorf("expected 400 on bad json, got %d", status)
	}
}
