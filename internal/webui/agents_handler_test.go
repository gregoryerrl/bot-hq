package webui

// Tests for /api/agents endpoint (Phase-R-followup-2 (d)).

import (
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestHandleAgents_NoDB(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	status, _ := callRoute(t, s, "GET", "/api/agents")
	if status != http.StatusServiceUnavailable {
		t.Errorf("status = %d, want 503", status)
	}
}

func TestHandleAgents_EmptyList(t *testing.T) {
	s := newTestServerWithDB(t)
	status, body := callRoute(t, s, "GET", "/api/agents")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	if !strings.Contains(body, `"agents"`) {
		t.Errorf("expected agents key in payload; body=%s", body)
	}
}

// TestHandleAgents_PreservesCurrentTaskField is the load-bearing
// presence-test for Phase-R-followup-2 (d): the data-axis half of
// "agents.current_task webui-surface" closure. Locks the contract
// that the JSON payload exposes the current_task field name + value
// for HTTP-side consumers.
func TestHandleAgents_PreservesCurrentTaskField(t *testing.T) {
	s := newTestServerWithDB(t)
	now := time.Now()
	if err := s.db.RegisterAgent(protocol.Agent{
		ID:         "test-brian",
		Name:       "Test Brian",
		Type:       protocol.AgentBrian,
		Status:     protocol.StatusOnline,
		Project:    "bot-hq",
		Registered: now,
		LastSeen:   now,
	}); err != nil {
		t.Fatalf("register: %v", err)
	}
	if err := s.db.SetAgentCurrentTask("test-brian", "smoking carry-forward queue"); err != nil {
		t.Fatalf("set current task: %v", err)
	}
	status, body := callRoute(t, s, "GET", "/api/agents")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	if !strings.Contains(body, `"current_task"`) {
		t.Errorf("JSON payload must expose current_task field name; body=%s", body)
	}
	if !strings.Contains(body, `"smoking carry-forward queue"`) {
		t.Errorf("JSON payload must surface current_task value; body=%s", body)
	}
	if !strings.Contains(body, `"id"`) || !strings.Contains(body, `"test-brian"`) {
		t.Errorf("JSON payload must surface id field; body=%s", body)
	}
}

func TestHandleAgents_RejectsNonGet(t *testing.T) {
	s := newTestServerWithDB(t)
	status, _ := callRoute(t, s, "POST", "/api/agents")
	if status != http.StatusMethodNotAllowed {
		t.Errorf("status = %d, want 405", status)
	}
}
