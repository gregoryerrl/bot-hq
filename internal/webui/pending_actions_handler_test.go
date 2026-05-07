package webui

// Tests for pending-actions HTTP endpoints + auto-create policy
// (P-9 / phase-n.md:818).

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)


// TestShouldQueueAsPending verifies the auto-create policy filters.
func TestShouldQueueAsPending(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want bool
	}{
		{"hr-broadcast-from-rain", protocol.Message{FromAgent: "rain", ToAgent: "", Content: "[HR] @user — review needed"}, true},
		{"hr-targeted-user", protocol.Message{FromAgent: "brian", ToAgent: "user", Content: "[HR] decision required"}, true},
		{"hr-pm-other-agent", protocol.Message{FromAgent: "rain", ToAgent: "brian", Content: "[HR] peer-coord"}, false},
		{"non-hr-broadcast", protocol.Message{FromAgent: "rain", ToAgent: "", Content: "no marker"}, false},
		{"from-user-skip", protocol.Message{FromAgent: "user", ToAgent: "", Content: "[HR] my own"}, false},
		{"from-empty-skip", protocol.Message{FromAgent: "", ToAgent: "", Content: "[HR] anon"}, false},
		{"hr-with-leading-space", protocol.Message{FromAgent: "rain", ToAgent: "", Content: "  [HR] indented"}, true},
		// Phase-R-followup-2 (a): MsgFlag-typed messages queue
		// regardless of [HR] prefix — FLAG-class is implicitly elevated.
		{"flag-without-hr-broadcast", protocol.Message{FromAgent: "rain", ToAgent: "", Type: protocol.MsgFlag, Content: "agent stuck on push-failure"}, true},
		{"flag-without-hr-targeted-user", protocol.Message{FromAgent: "brian", ToAgent: "user", Type: protocol.MsgFlag, Content: "auth failure"}, true},
		{"flag-pm-other-agent-still-skipped", protocol.Message{FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgFlag, Content: "peer-coord flag"}, false},
		{"flag-from-user-still-skipped", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgFlag, Content: "user flag"}, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := shouldQueueAsPending(tc.msg)
			if got != tc.want {
				t.Errorf("shouldQueueAsPending(%v) = %v, want %v", tc.msg.Content, got, tc.want)
			}
		})
	}
}

// TestPendingSummaryFor strips [HR] prefix + trims + newline-replaces.
func TestPendingSummaryFor(t *testing.T) {
	cases := []struct {
		in   string
		want string
	}{
		{"[HR] simple", "simple"},
		{"  [HR] indented", "indented"},
		{"[HR] line one\nline two", "line one · line two"},
		{"[HR]\n\nbody only", "body only"},
	}
	for _, tc := range cases {
		got := pendingSummaryFor(protocol.Message{Content: tc.in})
		if got != tc.want {
			t.Errorf("pendingSummaryFor(%q) = %q, want %q", tc.in, got, tc.want)
		}
	}
}

// TestKindForMessage maps protocol message types to queue kinds.
func TestKindForMessage(t *testing.T) {
	if got := kindForMessage(protocol.Message{Type: protocol.MsgFlag}); got != "flag" {
		t.Errorf("flag kind = %q", got)
	}
	if got := kindForMessage(protocol.Message{Type: protocol.MsgError}); got != "error" {
		t.Errorf("error kind = %q", got)
	}
	if got := kindForMessage(protocol.Message{Type: protocol.MsgUpdate}); got != "hr-broadcast" {
		t.Errorf("update kind = %q (default hr-broadcast)", got)
	}
}

// TestHandlePendingActions_NoDB returns 503 gracefully.
func TestHandlePendingActions_NoDB(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	status, _ := callRoute(t, s, "GET", "/api/pending-actions")
	if status != http.StatusServiceUnavailable {
		t.Errorf("status = %d, want 503", status)
	}
}

// TestHandlePendingActions_EmptyList returns actions=[] (not null).
func TestHandlePendingActions_EmptyList(t *testing.T) {
	s := newTestServerWithDB(t)
	status, body := callRoute(t, s, "GET", "/api/pending-actions")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	if !strings.Contains(body, `"actions": []`) && !strings.Contains(body, `"actions":[]`) {
		t.Errorf("expected empty array; body=%s", body)
	}
}

// TestHandlePendingActions_PreservesAgentIDInJSON locks the Phase-R-
// followup R2 webui display-strip contract: frontend hides agent_id in
// pending-action rendering for authorless-display, but JSON payload
// MUST retain agent_id for forensic-trail / direct-API queries. This
// test guards against accidental backend-side stripping that would
// break audit queries.
func TestHandlePendingActions_PreservesAgentIDInJSON(t *testing.T) {
	s := newTestServerWithDB(t)
	_, _ = s.db.InsertPendingAction("rain", "hr-broadcast", "test summary", 0)
	status, body := callRoute(t, s, "GET", "/api/pending-actions")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	if !strings.Contains(body, `"agent_id"`) {
		t.Errorf("JSON payload must retain agent_id field for forensic-trail (R2 backend-side preservation contract); body=%s", body)
	}
	if !strings.Contains(body, `"rain"`) {
		t.Errorf("JSON payload must surface the actual agent_id value; body=%s", body)
	}
}

// TestHandlePendingActions_CountQueryParam returns count-only.
func TestHandlePendingActions_CountQueryParam(t *testing.T) {
	s := newTestServerWithDB(t)
	_, _ = s.db.InsertPendingAction("rain", "hr", "x", 0)
	_, _ = s.db.InsertPendingAction("brian", "hr", "y", 0)
	status, body := callRoute(t, s, "GET", "/api/pending-actions?count=1")
	if status != http.StatusOK {
		t.Fatalf("status = %d", status)
	}
	var resp map[string]any
	if err := json.Unmarshal([]byte(body), &resp); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if resp["count"].(float64) != 2 {
		t.Errorf("count = %v, want 2", resp["count"])
	}
}

// TestHandlePendingActionAck end-to-end POST + status transition.
func TestHandlePendingActionAck(t *testing.T) {
	s := newTestServerWithDB(t)
	id, _ := s.db.InsertPendingAction("rain", "hr", "review", 0)
	target := "/api/pending-actions/" + itoa(id) + "/ack"
	req := httptest.NewRequest("POST", target, nil)
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		body, _ := io.ReadAll(w.Result().Body)
		t.Fatalf("status = %d, body=%s", w.Code, body)
	}
	body, _ := io.ReadAll(w.Result().Body)
	var resp map[string]any
	_ = json.Unmarshal(body, &resp)
	if resp["ack"] != true {
		t.Errorf("ack response %v, want true", resp["ack"])
	}
	// Second ack should report false (already ack'd).
	w2 := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w2, httptest.NewRequest("POST", target, nil))
	body2, _ := io.ReadAll(w2.Result().Body)
	var resp2 map[string]any
	_ = json.Unmarshal(body2, &resp2)
	if resp2["ack"] != false {
		t.Errorf("second ack should report false; got %v", resp2["ack"])
	}
}

// TestHandlePendingActionAck_InvalidID returns 400.
func TestHandlePendingActionAck_InvalidID(t *testing.T) {
	s := newTestServerWithDB(t)
	req := httptest.NewRequest("POST", "/api/pending-actions/abc/ack", nil)
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusBadRequest {
		t.Errorf("status = %d, want 400", w.Code)
	}
}

// itoa is a tiny int64-to-string helper local to tests.
func itoa(n int64) string {
	if n == 0 {
		return "0"
	}
	var s string
	for n > 0 {
		s = string(rune('0'+n%10)) + s
		n /= 10
	}
	return s
}
