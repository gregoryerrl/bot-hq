package daemoncron

import (
	"errors"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestBuildOnlineContent_Format(t *testing.T) {
	got := BuildOnlineContent("gemma4:e4b")
	if got != "Emma online. Model: gemma4:e4b" {
		t.Errorf("unexpected online content; got %q", got)
	}
}

func TestBuildOllamaRestartSuccessContent_Format(t *testing.T) {
	got := BuildOllamaRestartSuccessContent()
	if got != "Ollama restarted successfully." {
		t.Errorf("unexpected restart-success content; got %q", got)
	}
}

func TestBuildOllamaRestartFailContent_Format(t *testing.T) {
	err := errors.New("permission denied")
	got := BuildOllamaRestartFailContent(err)
	if !strings.HasPrefix(got, "Ollama restart failed:") {
		t.Errorf("missing prefix; got %q", got)
	}
	if !strings.Contains(got, "permission denied") {
		t.Errorf("error string not embedded; got %q", got)
	}
}

func TestBuildOllamaHealthCheckFailContent_Format(t *testing.T) {
	got := BuildOllamaHealthCheckFailContent()
	if !strings.Contains(got, "health check failed") {
		t.Errorf("expected 'health check failed' substring; got %q", got)
	}
}

func TestBuildRosterPruneContent_Format(t *testing.T) {
	got := BuildRosterPruneContent(2, []string{"agent-a", "agent-b"})
	if !strings.HasPrefix(got, "[ROSTER-PRUNE] Removed 2 stale-offline") {
		t.Errorf("missing [ROSTER-PRUNE] prefix + count; got %q", got)
	}
	if !strings.Contains(got, "agent-a, agent-b") {
		t.Errorf("agent ids not embedded; got %q", got)
	}
}

func TestEmitOnline_FiresMsgUpdate(t *testing.T) {
	db := setupTestDB(t)
	EmitOnline(db, "test-model")
	msgs, _ := db.GetRecentMessages(10)
	count := 0
	for _, m := range msgs {
		if m.FromAgent == lifecycleAgentID && strings.Contains(m.Content, "Emma online") {
			count++
			if m.Type != protocol.MsgUpdate {
				t.Errorf("online emit should be MsgUpdate; got %s", m.Type)
			}
		}
	}
	if count != 1 {
		t.Errorf("expected 1 online emit; got %d", count)
	}
}

func TestEmitOllamaHealthCheckFail_FiresMsgError(t *testing.T) {
	db := setupTestDB(t)
	EmitOllamaHealthCheckFail(db)
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if m.FromAgent == lifecycleAgentID && strings.Contains(m.Content, "health check failed") {
			if m.Type != protocol.MsgError {
				t.Errorf("health-fail emit should be MsgError; got %s", m.Type)
			}
			return
		}
	}
	t.Error("expected ollama-health-fail emit not found")
}

func TestEmitOllamaRestartSuccess_FiresMsgUpdate(t *testing.T) {
	db := setupTestDB(t)
	EmitOllamaRestartSuccess(db)
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if m.FromAgent == lifecycleAgentID && strings.Contains(m.Content, "restarted successfully") {
			if m.Type != protocol.MsgUpdate {
				t.Errorf("restart-success emit should be MsgUpdate; got %s", m.Type)
			}
			return
		}
	}
	t.Error("expected ollama-restart-success emit not found")
}

func TestEmitOllamaRestartFail_FiresMsgError(t *testing.T) {
	db := setupTestDB(t)
	EmitOllamaRestartFail(db, errors.New("boom"))
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if m.FromAgent == lifecycleAgentID && strings.Contains(m.Content, "Ollama restart failed: boom") {
			if m.Type != protocol.MsgError {
				t.Errorf("restart-fail emit should be MsgError; got %s", m.Type)
			}
			return
		}
	}
	t.Error("expected ollama-restart-fail emit not found")
}

func TestEmitRosterPrune_NoOpEmptyList(t *testing.T) {
	db := setupTestDB(t)
	EmitRosterPrune(db, nil)
	EmitRosterPrune(db, []string{})
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if strings.HasPrefix(m.Content, "[ROSTER-PRUNE]") {
			t.Errorf("empty id list should not emit; got %q", m.Content)
		}
	}
}

func TestEmitRosterPrune_FiresWithIDs(t *testing.T) {
	db := setupTestDB(t)
	EmitRosterPrune(db, []string{"x", "y", "z"})
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if strings.HasPrefix(m.Content, "[ROSTER-PRUNE] Removed 3") {
			if m.ToAgent != "rain" {
				t.Errorf("roster-prune should target rain; got %q", m.ToAgent)
			}
			return
		}
	}
	t.Error("expected roster-prune emit not found")
}
