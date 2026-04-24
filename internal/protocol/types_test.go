package protocol

import "testing"

func TestAgentTypeValid(t *testing.T) {
	valid := []AgentType{AgentCoder, AgentVoice, AgentBrian, AgentDiscord}
	for _, at := range valid {
		if !at.Valid() {
			t.Errorf("expected %s to be valid", at)
		}
	}
	if AgentType("invalid").Valid() {
		t.Error("expected 'invalid' to be invalid")
	}
}

func TestMessageTypeValid(t *testing.T) {
	valid := []MessageType{MsgHandshake, MsgQuestion, MsgResponse, MsgCommand, MsgUpdate, MsgResult, MsgError}
	for _, mt := range valid {
		if !mt.Valid() {
			t.Errorf("expected %s to be valid", mt)
		}
	}
}

func TestSessionModeValid(t *testing.T) {
	valid := []SessionMode{ModeBrainstorm, ModeImplement, ModeChat}
	for _, sm := range valid {
		if !sm.Valid() {
			t.Errorf("expected %s to be valid", sm)
		}
	}
}
