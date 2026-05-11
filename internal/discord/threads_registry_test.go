package discord

import (
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestThreadRegistry_RegisterLookupUnregister(t *testing.T) {
	b := &Bot{hubChannelID: "hub-1"}

	if got := b.threadForSession("s-a"); got != "" {
		t.Errorf("threadForSession before register=%q, want empty", got)
	}
	if got := b.sessionForThread("t-a"); got != "" {
		t.Errorf("sessionForThread before register=%q, want empty", got)
	}

	b.RegisterSessionThread("s-a", "t-a")
	b.RegisterSessionThread("s-b", "t-b")

	if got := b.threadForSession("s-a"); got != "t-a" {
		t.Errorf("threadForSession(s-a)=%q, want t-a", got)
	}
	if got := b.sessionForThread("t-b"); got != "s-b" {
		t.Errorf("sessionForThread(t-b)=%q, want s-b", got)
	}

	b.UnregisterSessionThread("s-a")
	if got := b.threadForSession("s-a"); got != "" {
		t.Errorf("threadForSession(s-a) after unregister=%q, want empty", got)
	}
	if got := b.sessionForThread("t-a"); got != "" {
		t.Errorf("sessionForThread(t-a) after unregister=%q, want empty", got)
	}
	// Other entry untouched.
	if got := b.threadForSession("s-b"); got != "t-b" {
		t.Errorf("threadForSession(s-b) after unrelated unregister=%q, want t-b", got)
	}
}

func TestThreadRegistry_RegisterEmptyIsNoop(t *testing.T) {
	b := &Bot{}
	b.RegisterSessionThread("", "t")
	b.RegisterSessionThread("s", "")
	if b.threadByID != nil || b.sessionByThread != nil {
		t.Errorf("empty register should be no-op; got threadByID=%v sessionByThread=%v", b.threadByID, b.sessionByThread)
	}
}

func TestListensOnChannel_AcceptsRegisteredThreads(t *testing.T) {
	b := &Bot{hubChannelID: "hub-1"}
	if b.listensOnChannel("thread-x") {
		t.Errorf("listensOnChannel(thread-x) before register=true, want false")
	}
	b.RegisterSessionThread("s-x", "thread-x")
	if !b.listensOnChannel("thread-x") {
		t.Errorf("listensOnChannel(thread-x) after register=false, want true")
	}
	if b.listensOnChannel("unknown-channel") {
		t.Errorf("listensOnChannel(unknown-channel)=true, want false")
	}
	b.UnregisterSessionThread("s-x")
	if b.listensOnChannel("thread-x") {
		t.Errorf("listensOnChannel(thread-x) after unregister=true, want false")
	}
}

func TestChannelForMessage_SessionRoutesToThread(t *testing.T) {
	b := &Bot{hubChannelID: "hub-1"}
	b.RegisterSessionThread("s-x", "thread-x")

	got := b.channelForMessage(protocol.Message{SessionID: "s-x"})
	if got != "thread-x" {
		t.Errorf("session-scoped msg dest=%q, want thread-x", got)
	}

	// Unknown session-id falls through to hub channel.
	got = b.channelForMessage(protocol.Message{SessionID: "s-unknown"})
	if got != "hub-1" {
		t.Errorf("unknown-session dest=%q, want hub-1 (fallback)", got)
	}

	// Broadcast (no session-id) goes to hub channel.
	got = b.channelForMessage(protocol.Message{})
	if got != "hub-1" {
		t.Errorf("broadcast dest=%q, want hub-1", got)
	}

	// Flag with session-id still goes to flags channel (flags are global).
	b.flagsChannelID = "flags-1"
	got = b.channelForMessage(protocol.Message{Type: protocol.MsgFlag, SessionID: "s-x"})
	if got != "flags-1" {
		t.Errorf("flag-class with session-id dest=%q, want flags-1 (flags route precedes session)", got)
	}
}
