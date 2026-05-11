package webui

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// hubTestServer returns a Server backed by a temp-dir hub.DB (hub.OpenDB
// creates a file path, not in-memory) and initialized SSE state.
func hubTestServer(t *testing.T) (*Server, *hub.DB) {
	t.Helper()
	db, err := hub.OpenDB(filepath.Join(t.TempDir(), "hub.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	s := &Server{db: db}
	s.initSSE()
	s.initHubSSE()
	return s, db
}

func TestHandleHubMessages_GET_FiltersBySession(t *testing.T) {
	s, db := hubTestServer(t)
	// Seed: 3 messages spanning 2 sessions + main hub.
	for _, m := range []protocol.Message{
		{FromAgent: "brian", Type: "update", Content: "from session A", SessionID: "session-A"},
		{FromAgent: "rain", Type: "update", Content: "from session A too", SessionID: "session-A"},
		{FromAgent: "brian", Type: "update", Content: "from session B", SessionID: "session-B"},
		{FromAgent: "user", Type: "command", Content: "hi emma", SessionID: ""},
	} {
		if _, err := db.InsertMessage(m); err != nil {
			t.Fatal(err)
		}
	}

	cases := []struct {
		name      string
		query     string
		wantCount int
	}{
		{"no filter returns all 4", "", 4},
		{"session-A returns 2", "?session_id=session-A", 2},
		{"session-B returns 1", "?session_id=session-B", 1},
		{"main hub returns 1", "?session_id=", 1},
		{"unknown session returns 0", "?session_id=nope", 0},
		{"since_id skips earlier", "?since_id=2", 2},
	}
	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodGet, "/api/hub/messages"+tt.query, nil)
			rr := httptest.NewRecorder()
			s.handleHubMessages(rr, req)
			if rr.Code != http.StatusOK {
				t.Fatalf("status = %d, want 200; body=%s", rr.Code, rr.Body.String())
			}
			var resp struct {
				Messages []map[string]any `json:"messages"`
			}
			if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
				t.Fatal(err)
			}
			if len(resp.Messages) != tt.wantCount {
				t.Errorf("count = %d, want %d (body=%s)", len(resp.Messages), tt.wantCount, rr.Body.String())
			}
		})
	}
}

func TestHandleHubMessages_POST_InsertsAsUser(t *testing.T) {
	s, db := hubTestServer(t)

	body := strings.NewReader(`{"content":"@emma what's up","session_id":""}`)
	req := httptest.NewRequest(http.MethodPost, "/api/hub/messages", body)
	rr := httptest.NewRecorder()
	s.handleHubMessages(rr, req)

	if rr.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", rr.Code, rr.Body.String())
	}
	var resp struct {
		ID int64 `json:"id"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	if resp.ID == 0 {
		t.Fatal("returned id is 0")
	}

	msg, ok, err := db.GetMessageByID(resp.ID)
	if err != nil || !ok {
		t.Fatalf("get message: err=%v ok=%v", err, ok)
	}
	if msg.FromAgent != "user" {
		t.Errorf("FromAgent = %q, want %q", msg.FromAgent, "user")
	}
	if msg.Type != "command" {
		t.Errorf("Type = %q, want %q", msg.Type, "command")
	}
	if msg.Content != "@emma what's up" {
		t.Errorf("Content = %q", msg.Content)
	}
}

func TestHandleHubMessages_POST_RejectsEmptyContent(t *testing.T) {
	s, _ := hubTestServer(t)
	req := httptest.NewRequest(http.MethodPost, "/api/hub/messages", bytes.NewReader([]byte(`{"content":""}`)))
	rr := httptest.NewRecorder()
	s.handleHubMessages(rr, req)
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400", rr.Code)
	}
}

func TestHandleHubMessages_RejectsBadMethod(t *testing.T) {
	s, _ := hubTestServer(t)
	req := httptest.NewRequest(http.MethodDelete, "/api/hub/messages", nil)
	rr := httptest.NewRecorder()
	s.handleHubMessages(rr, req)
	if rr.Code != http.StatusMethodNotAllowed {
		t.Fatalf("status = %d, want 405", rr.Code)
	}
}

func TestHandleHubMessages_503WhenNoDB(t *testing.T) {
	s := &Server{}
	req := httptest.NewRequest(http.MethodGet, "/api/hub/messages", nil)
	rr := httptest.NewRecorder()
	s.handleHubMessages(rr, req)
	if rr.Code != http.StatusServiceUnavailable {
		t.Fatalf("status = %d, want 503", rr.Code)
	}
}
