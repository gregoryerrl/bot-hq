package tmux

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

func TestHasTmux(t *testing.T) {
	// Just verify it doesn't panic
	_ = HasTmux()
}

func TestNewAndKillSession(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping tmux test in short mode")
	}
	if !HasTmux() {
		t.Skip("tmux not available")
	}
	name := fmt.Sprintf("bot-hq-test-%d", time.Now().UnixNano())
	if err := NewSession(name, "/tmp"); err != nil {
		t.Fatal(err)
	}
	defer KillSession(name)

	sessions, err := ListSessions()
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, s := range sessions {
		if s.Name == name {
			found = true
			break
		}
	}
	if !found {
		t.Error("session not found in list")
	}
}

func TestSendKeysAndCapture(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping tmux test in short mode")
	}
	if !HasTmux() {
		t.Skip("tmux not available")
	}
	name := fmt.Sprintf("bot-hq-test-%d", time.Now().UnixNano())
	if err := NewSession(name, "/tmp"); err != nil {
		t.Fatal(err)
	}
	defer KillSession(name)

	if err := SendKeys(name, "echo hello-test", true); err != nil {
		t.Fatal(err)
	}

	time.Sleep(500 * time.Millisecond)

	output, err := CapturePane(name, 50)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(output, "hello-test") {
		t.Errorf("expected output to contain 'hello-test', got: %s", output)
	}
}
