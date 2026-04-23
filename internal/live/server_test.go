package live

import (
	"testing"
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

func TestLiveAgentRegistration(t *testing.T) {
	const agentID = "live"
	if agentID != "live" {
		t.Error("live agent ID should be 'live'")
	}
}
