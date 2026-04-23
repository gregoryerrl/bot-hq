package discord

import (
	"testing"
)

func TestNewBotRequiresToken(t *testing.T) {
	_, err := NewBot("", "channel-id", nil)
	if err == nil {
		t.Error("expected error for empty token")
	}
}

func TestNewBotRequiresChannel(t *testing.T) {
	_, err := NewBot("token", "", nil)
	if err == nil {
		t.Error("expected error for empty channel")
	}
}

func TestNewBotRequiresHub(t *testing.T) {
	_, err := NewBot("token", "channel-id", nil)
	if err == nil {
		t.Error("expected error for nil hub")
	}
}

func TestSplitMessageShort(t *testing.T) {
	chunks := splitMessage("hello world", 2000)
	if len(chunks) != 1 {
		t.Errorf("expected 1 chunk, got %d", len(chunks))
	}
	if chunks[0] != "hello world" {
		t.Errorf("expected 'hello world', got %q", chunks[0])
	}
}

func TestSplitMessageLong(t *testing.T) {
	// Create a message longer than 100 chars and split at limit=100
	msg := ""
	for i := 0; i < 250; i++ {
		msg += "a"
	}
	chunks := splitMessage(msg, 100)
	if len(chunks) < 2 {
		t.Errorf("expected at least 2 chunks, got %d", len(chunks))
	}
	for _, c := range chunks {
		if len(c) > 100 {
			t.Errorf("chunk exceeds limit: len=%d", len(c))
		}
	}
	// Verify all content is preserved
	total := 0
	for _, c := range chunks {
		total += len(c)
	}
	if total != 250 {
		t.Errorf("expected total length 250, got %d", total)
	}
}

func TestSplitMessageNewlineBreak(t *testing.T) {
	// Build a message with a newline in the right spot for splitting
	msg := ""
	for i := 0; i < 70; i++ {
		msg += "a"
	}
	msg += "\n"
	for i := 0; i < 50; i++ {
		msg += "b"
	}
	// Total = 121 chars, limit 100 — should split at the newline (pos 70)
	chunks := splitMessage(msg, 100)
	if len(chunks) != 2 {
		t.Errorf("expected 2 chunks, got %d", len(chunks))
	}
}
