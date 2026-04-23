package ui

import (
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestSessionsTabSelection(t *testing.T) {
	tab := NewSessionsTab()
	tab.sessions = []protocol.Session{
		{ID: "sess-1", Mode: protocol.ModeBrainstorm, Purpose: "test", Status: protocol.SessionActive},
		{ID: "sess-2", Mode: protocol.ModeImplement, Purpose: "build", Status: protocol.SessionActive},
	}

	if tab.cursor != 0 {
		t.Errorf("initial cursor = %d, want 0", tab.cursor)
	}
}
