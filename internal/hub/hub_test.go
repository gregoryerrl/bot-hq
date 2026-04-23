package hub

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestHubDispatchToTmux(t *testing.T) {
	h := &Hub{}
	cmd := h.FormatTmuxMessage("claude-abc", protocol.Message{
		FromAgent: "live",
		Type:      protocol.MsgResponse,
		Content:   "JWT with refresh tokens",
	})
	if cmd == "" {
		t.Error("expected non-empty tmux command")
	}
}

func TestHubWSClientRegistration(t *testing.T) {
	cfg := DefaultConfig()
	cfg.Hub.DBPath = filepath.Join(t.TempDir(), "test.db")

	h, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Stop()

	ch := h.RegisterWSClient("live")
	if ch == nil {
		t.Fatal("expected non-nil channel")
	}

	// Send a message to the WS client
	go func() {
		h.dispatch(protocol.Message{
			FromAgent: "claude-abc",
			ToAgent:   "live",
			Type:      protocol.MsgResponse,
			Content:   "test message",
		})
	}()

	select {
	case msg := <-ch:
		if msg.Content != "test message" {
			t.Errorf("expected 'test message', got %q", msg.Content)
		}
	case <-time.After(2 * time.Second):
		t.Error("timed out waiting for WS dispatch")
	}

	h.UnregisterWSClient("live")
}

func TestHubBroadcast(t *testing.T) {
	cfg := DefaultConfig()
	cfg.Hub.DBPath = filepath.Join(t.TempDir(), "test.db")

	h, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Stop()

	ch1 := h.RegisterWSClient("client-1")
	ch2 := h.RegisterWSClient("client-2")

	// Broadcast (empty ToAgent)
	go func() {
		h.dispatch(protocol.Message{
			FromAgent: "sender",
			ToAgent:   "",
			Type:      protocol.MsgUpdate,
			Content:   "broadcast msg",
		})
	}()

	for _, ch := range []chan protocol.Message{ch1, ch2} {
		select {
		case msg := <-ch:
			if msg.Content != "broadcast msg" {
				t.Errorf("expected 'broadcast msg', got %q", msg.Content)
			}
		case <-time.After(2 * time.Second):
			t.Error("timed out waiting for broadcast")
		}
	}

	h.UnregisterWSClient("client-1")
	h.UnregisterWSClient("client-2")
}

func TestHubNewAndStop(t *testing.T) {
	cfg := DefaultConfig()
	cfg.Hub.DBPath = filepath.Join(t.TempDir(), "test.db")

	h, err := NewHub(cfg)
	if err != nil {
		t.Fatal(err)
	}

	if h.DB == nil {
		t.Error("expected non-nil DB")
	}

	if err := h.Stop(); err != nil {
		t.Errorf("unexpected error on Stop: %v", err)
	}
}
