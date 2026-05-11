package protocol

import "testing"

func TestIsElevated(t *testing.T) {
	cases := []struct {
		name string
		msg  Message
		want bool
	}{
		{"plain update", Message{Type: MsgUpdate, Content: "hello"}, false},
		{"response", Message{Type: MsgResponse, Content: "yes"}, false},
		{"result", Message{Type: MsgResult, Content: "done"}, false},
		{"command", Message{Type: MsgCommand, Content: "/run"}, false},
		{"error", Message{Type: MsgError, Content: "boom"}, false},
		{"flag", Message{Type: MsgFlag, Content: "attention"}, true},
		{"HR prefix space", Message{Type: MsgUpdate, Content: "[HR] cross-session"}, true},
		{"HR prefix newline", Message{Type: MsgUpdate, Content: "[HR]\nmulti-line"}, true},
		{"HR not at start", Message{Type: MsgUpdate, Content: "see [HR] later"}, false},
		{"empty", Message{}, false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := IsElevated(c.msg); got != c.want {
				t.Errorf("IsElevated(%+v) = %v, want %v", c.msg, got, c.want)
			}
		})
	}
}

func TestPassesMainHubView(t *testing.T) {
	cases := []struct {
		name string
		msg  Message
		want bool
	}{
		{"main hub broadcast", Message{SessionID: "", Type: MsgUpdate, Content: "hi"}, true},
		{"main hub system event", Message{SessionID: "", Type: MsgUpdate, Content: "[HEARTBEAT-LEDGER]"}, true},
		{"session non-elevated", Message{SessionID: "s-x", Type: MsgUpdate, Content: "brian working"}, false},
		{"session response", Message{SessionID: "s-x", Type: MsgResponse, Content: "ack"}, false},
		{"session flag", Message{SessionID: "s-x", Type: MsgFlag, Content: "halt"}, true},
		{"session HR", Message{SessionID: "s-x", Type: MsgUpdate, Content: "[HR] elevated"}, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := PassesMainHubView(c.msg); got != c.want {
				t.Errorf("PassesMainHubView(%+v) = %v, want %v", c.msg, got, c.want)
			}
		})
	}
}
