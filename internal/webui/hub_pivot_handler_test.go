package webui

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// newTestServerWithDB constructs a Server backed by a real sqlite hub.DB
// at t.TempDir per R39 TEST-ISOLATION. Used for handlers that need
// db.InsertMessage (handleHubPivot, etc.).
func newTestServerWithDB(t *testing.T) *Server {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatalf("open hub db: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })

	s := &Server{
		db:            db,
		canonicalRoot: t.TempDir(),
	}
	mux := http.NewServeMux()
	s.registerRoutes(mux)
	s.httpServer = &http.Server{Handler: mux}
	return s
}

func postJSON(t *testing.T, s *Server, target string, body any) (int, string) {
	t.Helper()
	data, _ := json.Marshal(body)
	req := httptest.NewRequest("POST", target, bytes.NewReader(data))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	respBody, _ := io.ReadAll(w.Result().Body)
	return w.Code, string(respBody)
}

// TestHandleHubPivot_HappyPath: well-formed POST → 200, message inserted
// into hub.DB with [CONTEXT-SWITCH] prefix.
func TestHandleHubPivot_HappyPath(t *testing.T) {
	s := newTestServerWithDB(t)

	status, body := postJSON(t, s, "/api/hub-pivot", map[string]string{
		"agent":        "brian",
		"project":      "bcc-ad-manager",
		"prev_project": "bot-hq",
	})
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}

	var resp struct {
		OK    bool  `json:"ok"`
		MsgID int64 `json:"msg_id"`
	}
	if err := json.Unmarshal([]byte(body), &resp); err != nil {
		t.Fatalf("unmarshal response: %v; body=%s", err, body)
	}
	if !resp.OK {
		t.Errorf("ok = false; body=%s", body)
	}
	if resp.MsgID <= 0 {
		t.Errorf("msg_id = %d, want >0", resp.MsgID)
	}

	// Verify message landed in DB with expected content.
	msgs, err := s.db.GetRecentMessages(10)
	if err != nil {
		t.Fatalf("RecentMessages: %v", err)
	}
	if len(msgs) == 0 {
		t.Fatalf("no messages in DB after pivot insert")
	}
	got := msgs[0]
	if got.FromAgent != "brian" {
		t.Errorf("FromAgent = %q, want %q", got.FromAgent, "brian")
	}
	if !strings.HasPrefix(got.Content, "[CONTEXT-SWITCH]") {
		t.Errorf("content lacks [CONTEXT-SWITCH] prefix: %q", got.Content)
	}
	if !strings.Contains(got.Content, "brian") || !strings.Contains(got.Content, "bcc-ad-manager") {
		t.Errorf("content missing agent/project: %q", got.Content)
	}
}

// TestHandleHubPivot_NoPrevProject: prev_project optional → message
// uses simpler "pivoted to project X" framing.
func TestHandleHubPivot_NoPrevProject(t *testing.T) {
	s := newTestServerWithDB(t)

	status, body := postJSON(t, s, "/api/hub-pivot", map[string]string{
		"agent":   "rain",
		"project": "bot-hq",
	})
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}

	msgs, _ := s.db.GetRecentMessages(1)
	if len(msgs) == 0 {
		t.Fatalf("no messages")
	}
	if !strings.Contains(msgs[0].Content, "pivoted to project bot-hq") {
		t.Errorf("expected 'pivoted to project bot-hq' in content; got %q", msgs[0].Content)
	}
}

// TestHandleHubPivot_MissingAgent: empty agent → 400.
func TestHandleHubPivot_MissingAgent(t *testing.T) {
	s := newTestServerWithDB(t)
	status, body := postJSON(t, s, "/api/hub-pivot", map[string]string{
		"project": "bot-hq",
	})
	if status != http.StatusBadRequest {
		t.Errorf("status = %d, want 400; body=%s", status, body)
	}
}

// TestHandleHubPivot_MissingProject: empty project → 400.
func TestHandleHubPivot_MissingProject(t *testing.T) {
	s := newTestServerWithDB(t)
	status, body := postJSON(t, s, "/api/hub-pivot", map[string]string{
		"agent": "brian",
	})
	if status != http.StatusBadRequest {
		t.Errorf("status = %d, want 400; body=%s", status, body)
	}
}

// TestHandleHubPivot_BadJSON: malformed body → 400.
func TestHandleHubPivot_BadJSON(t *testing.T) {
	s := newTestServerWithDB(t)
	req := httptest.NewRequest("POST", "/api/hub-pivot", strings.NewReader("not json"))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusBadRequest {
		t.Errorf("status = %d, want 400", w.Code)
	}
}

// TestHandleHubPivot_WrongMethod: GET → 405.
func TestHandleHubPivot_WrongMethod(t *testing.T) {
	s := newTestServerWithDB(t)
	req := httptest.NewRequest("GET", "/api/hub-pivot", nil)
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusMethodNotAllowed {
		t.Errorf("status = %d, want 405", w.Code)
	}
}

// TestHandleHubPivot_NoDB: server constructed without DB → 503.
func TestHandleHubPivot_NoDB(t *testing.T) {
	s := &Server{canonicalRoot: t.TempDir()}
	mux := http.NewServeMux()
	s.registerRoutes(mux)
	s.httpServer = &http.Server{Handler: mux}

	status, _ := postJSON(t, s, "/api/hub-pivot", map[string]string{
		"agent":   "brian",
		"project": "bot-hq",
	})
	if status != http.StatusServiceUnavailable {
		t.Errorf("status = %d, want 503", status)
	}
}

// TestFormatPivotContent locks the message-body shape so peers can
// reliably grep / filter on the prefix.
func TestFormatPivotContent(t *testing.T) {
	cases := []struct {
		agent, project, prev string
		want                 string
	}{
		{"brian", "bot-hq", "", "[CONTEXT-SWITCH] agent brian pivoted to project bot-hq"},
		{"rain", "bcc-ad-manager", "bot-hq", "[CONTEXT-SWITCH] agent rain pivoted: bot-hq -> bcc-ad-manager"},
		{"emma", "bot-hq", "bot-hq", "[CONTEXT-SWITCH] agent emma pivoted to project bot-hq"}, // same prev = no-op framing
	}
	for _, c := range cases {
		got := formatPivotContent(c.agent, c.project, c.prev)
		if got != c.want {
			t.Errorf("formatPivotContent(%q, %q, %q) = %q, want %q",
				c.agent, c.project, c.prev, got, c.want)
		}
	}
}
