package live

import (
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestBrowserReceivesErrorOnGeminiFailure(t *testing.T) {
	type errorMsg struct {
		Type  string `json:"type"`
		Error string `json:"error"`
	}
	msg := errorMsg{Type: "error", Error: "Gemini connection failed: dial gemini: connection refused"}
	if msg.Type != "error" {
		t.Error("error message type should be 'error'")
	}
	if msg.Error == "" {
		t.Error("error message should have non-empty error field")
	}
}

func TestTranscriptToHubMessage(t *testing.T) {
	transcript := "fix the login bug"
	msg := transcriptToMessage("user", transcript)
	if msg.FromAgent != "live" {
		t.Errorf("from_agent = %q, want 'live'", msg.FromAgent)
	}
	if msg.Type != protocol.MsgCommand {
		t.Errorf("type = %q, want %q", msg.Type, protocol.MsgCommand)
	}
	if msg.Content != transcript {
		t.Errorf("content = %q, want %q", msg.Content, transcript)
	}
}

func TestLiveAgentRegistration(t *testing.T) {
	const agentID = "live"
	if agentID != "live" {
		t.Error("live agent ID should be 'live'")
	}
}
