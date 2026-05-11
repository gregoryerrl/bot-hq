package protocol

import "testing"

func TestFilterForSession(t *testing.T) {
	tests := []struct {
		name          string
		msgSessionID  string
		mySessionID   string
		wantPassFiler bool
	}{
		// out-of-session pollers (clive, discord-relay, daemon)
		{"out-of-session sees session-tagged", "session-A", "", true},
		{"out-of-session sees untagged", "", "", true},

		// in-session pollers
		{"in-session sees own", "session-A", "session-A", true},
		{"in-session sees untagged broadcast", "", "session-A", true},
		{"in-session drops other-session", "session-B", "session-A", false},
		{"in-session drops yet-another", "captain-hook-x", "cl-cleanup-y", false},

		// emma in main-hub (mySessionID="") sees everything
		{"emma main-hub sees session-A", "session-A", "", true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			msg := Message{SessionID: tt.msgSessionID}
			if got := FilterForSession(msg, tt.mySessionID); got != tt.wantPassFiler {
				t.Errorf("FilterForSession(SessionID=%q, mySessionID=%q) = %v, want %v",
					tt.msgSessionID, tt.mySessionID, got, tt.wantPassFiler)
			}
		})
	}
}
