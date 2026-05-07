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

func TestSubscribeWebuiContext_FiresOnRealChange(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	ch, unsub := s.SubscribeWebuiContext()
	defer unsub()
	s.SetWebuiContext(WebuiContext{Project: "bot-hq", CurrentPath: "phase/phase-p.md"})
	select {
	case got := <-ch:
		if got.CurrentPath != "phase/phase-p.md" {
			t.Errorf("got path %q want phase/phase-p.md", got.CurrentPath)
		}
	default:
		t.Fatal("expected ctx update on subscriber chan")
	}
}

func TestSubscribeWebuiContext_SilentOnNoop(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	s.SetWebuiContext(WebuiContext{Project: "bot-hq", CurrentPath: "x.md"})
	ch, unsub := s.SubscribeWebuiContext()
	defer unsub()
	// Re-set with same project/path/viewMode (UpdatedAt differs but is
	// excluded from the diff). Should NOT fan out — Clive doesn't need
	// a context-update message every time the frontend re-POSTs the
	// same focus on a stale-resync.
	s.SetWebuiContext(WebuiContext{Project: "bot-hq", CurrentPath: "x.md"})
	select {
	case got := <-ch:
		t.Fatalf("unexpected ctx update on no-op write: %+v", got)
	default:
	}
}

func TestSubscribeWebuiContext_MultipleSubsAllReceive(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	ch1, unsub1 := s.SubscribeWebuiContext()
	defer unsub1()
	ch2, unsub2 := s.SubscribeWebuiContext()
	defer unsub2()
	s.SetWebuiContext(WebuiContext{Project: "p", CurrentPath: "f.md"})
	for i, ch := range []<-chan WebuiContext{ch1, ch2} {
		select {
		case got := <-ch:
			if got.CurrentPath != "f.md" {
				t.Errorf("sub %d: got %q want f.md", i, got.CurrentPath)
			}
		default:
			t.Errorf("sub %d: expected ctx update", i)
		}
	}
}

func TestSubscribeWebuiContext_UnsubStopsDelivery(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	ch, unsub := s.SubscribeWebuiContext()
	unsub()
	// chan should be closed after unsub.
	select {
	case _, ok := <-ch:
		if ok {
			t.Fatal("chan should be closed after unsub")
		}
	default:
		t.Fatal("expected chan to be closed (recv-ready)")
	}
	// SetWebuiContext should not panic even with no subscribers.
	s.SetWebuiContext(WebuiContext{Project: "p", CurrentPath: "f.md"})
}

func TestFormatFocusContext_EmptyReturnsEmpty(t *testing.T) {
	if got := formatFocusContext(WebuiContext{}); got != "" {
		t.Errorf("empty ctx should return empty, got %q", got)
	}
}

func TestFormatFocusContext_AuthoritativeFraming(t *testing.T) {
	got := formatFocusContext(WebuiContext{Project: "bot-hq", CurrentPath: "phase/phase-p.md"})
	for _, want := range []string{
		"USER VIEWING IN WEBUI",
		"project=bot-hq",
		"file=phase/phase-p.md",
		"authoritative",
		"do not say you can't see",
	} {
		if !strings.Contains(got, want) {
			t.Errorf("missing %q in:\n%s", want, got)
		}
	}
}
