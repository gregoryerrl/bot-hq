package webui

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestWebuiContext_GetEmpty(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/api/webui-context", nil)
	rec := httptest.NewRecorder()
	s.handleWebuiContext(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status: got %d want 200", rec.Code)
	}
	var got WebuiContext
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.CurrentPath != "" || got.Project != "" {
		t.Errorf("expected empty context, got %+v", got)
	}
}

func TestWebuiContext_PostThenGet(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	body := `{"project":"bot-hq","currentPath":"projects/bot-hq/tasks.md","viewMode":"rendered"}`
	req := httptest.NewRequest(http.MethodPost, "/api/webui-context", strings.NewReader(body))
	rec := httptest.NewRecorder()
	s.handleWebuiContext(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("post status: got %d want 200", rec.Code)
	}

	req = httptest.NewRequest(http.MethodGet, "/api/webui-context", nil)
	rec = httptest.NewRecorder()
	s.handleWebuiContext(rec, req)
	var got WebuiContext
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.Project != "bot-hq" {
		t.Errorf("project: got %q want %q", got.Project, "bot-hq")
	}
	if got.CurrentPath != "projects/bot-hq/tasks.md" {
		t.Errorf("currentPath: got %q", got.CurrentPath)
	}
	if got.ViewMode != "rendered" {
		t.Errorf("viewMode: got %q", got.ViewMode)
	}
	if got.UpdatedAt.IsZero() {
		t.Errorf("updatedAt: server should stamp non-zero time")
	}
}

func TestWebuiContext_PostInvalidJSON(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodPost, "/api/webui-context", bytes.NewReader([]byte("not-json")))
	rec := httptest.NewRecorder()
	s.handleWebuiContext(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Errorf("status: got %d want 400", rec.Code)
	}
}

func TestWebuiContext_MethodNotAllowed(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodDelete, "/api/webui-context", nil)
	rec := httptest.NewRecorder()
	s.handleWebuiContext(rec, req)
	if rec.Code != http.StatusMethodNotAllowed {
		t.Errorf("status: got %d want 405", rec.Code)
	}
}

func TestBuildVoiceSystemInstruction_Empty(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	got := s.buildVoiceSystemInstruction()
	if got != defaultSystemInstruction {
		t.Errorf("empty context should return default unchanged")
	}
}

func TestBuildVoiceSystemInstruction_WithFocus(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	s.SetWebuiContext(WebuiContext{
		Project:     "bot-hq",
		CurrentPath: "projects/bot-hq/tasks.md",
		ViewMode:    "rendered",
	})
	got := s.buildVoiceSystemInstruction()
	if !strings.Contains(got, defaultSystemInstruction) {
		t.Errorf("output should include default systemInstruction")
	}
	for _, want := range []string{"USER VIEWING IN WEBUI", "project=bot-hq", "file=projects/bot-hq/tasks.md", "view=rendered"} {
		if !strings.Contains(got, want) {
			t.Errorf("output missing %q\nfull: %s", want, got)
		}
	}
}

func TestBuildVoiceSystemInstruction_PartialFocus(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	s.SetWebuiContext(WebuiContext{Project: "bot-hq"})
	got := s.buildVoiceSystemInstruction()
	if !strings.Contains(got, "project=bot-hq") {
		t.Errorf("project-only context should still inject")
	}
	if strings.Contains(got, "file=") {
		t.Errorf("project-only context should not include file=")
	}
}
