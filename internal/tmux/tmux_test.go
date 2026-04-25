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

// promptByteAnchor must match the literal U+276F + space sequence and
// nothing else. Pure unit, no tmux dependency. Locks the empirical hexdump
// finding (Rain's investigation msg 2317) into the source.
func TestPromptByteAnchor_MatchesLiteral(t *testing.T) {
	want := "\xe2\x9d\xaf\xc2\xa0" // ❯ + NBSP (U+00A0)
	if promptByteAnchor != want {
		t.Errorf("promptByteAnchor mismatch: bytes % x, want % x", promptByteAnchor, want)
	}
	cases := []struct {
		name string
		in   string
		want bool
	}{
		{"exact match (❯+NBSP)", "\xe2\x9d\xaf\xc2\xa0", true},
		{"trailing context (❯+NBSP+text)", "$ \xe2\x9d\xaf\xc2\xa0ready", true},
		{"leading context", "[mode]\n\xe2\x9d\xaf\xc2\xa0", true},
		{"no anchor", "$ no prompt here", false},
		{"❯ + REGULAR space (must NOT match)", "\xe2\x9d\xaf\x20", false}, // distinguishes Claude pane from chat text where humans type `❯ ` literally
		{"❯ alone (no trailing byte)", "\xe2\x9d\xaf", false},             // strict: NBSP required
		{"different bullet (›)", "›\xc2\xa0", false},                      // U+203A, not U+276F
		{"different bullet (>)", "> ", false},                              // ASCII gt
		{"empty", "", false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := strings.Contains(tc.in, promptByteAnchor); got != tc.want {
				t.Errorf("contains(%q, anchor) = %v, want %v", tc.in, got, tc.want)
			}
		})
	}
}

// Helper: spin a fresh tmux session for a test, return name + cleanup.
func newTestSession(t *testing.T) string {
	t.Helper()
	if testing.Short() {
		t.Skip("skipping tmux test in short mode")
	}
	if !HasTmux() {
		t.Skip("tmux not available")
	}
	name := fmt.Sprintf("bot-hq-wfp-%d", time.Now().UnixNano())
	if err := NewSession(name, "/tmp"); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { KillSession(name) })
	return name
}

func TestCheckPromptOnce_FindsAnchorWhenPresent(t *testing.T) {
	name := newTestSession(t)
	if err := SendKeys(name, "printf '\\xe2\\x9d\\xaf\\xc2\\xa0X'", true); err != nil {
		t.Fatal(err)
	}
	time.Sleep(300 * time.Millisecond) // let printf land in pane
	at, out, err := checkPromptOnce(name)
	if err != nil {
		t.Fatalf("checkPromptOnce error: %v", err)
	}
	if !at {
		t.Errorf("expected atPrompt=true, got false. capture:\n%s", out)
	}
}

func TestCheckPromptOnce_NoAnchorWhenAbsent(t *testing.T) {
	name := newTestSession(t)
	if err := SendKeys(name, "echo no-anchor-here", true); err != nil {
		t.Fatal(err)
	}
	time.Sleep(300 * time.Millisecond)
	at, _, err := checkPromptOnce(name)
	if err != nil {
		t.Fatalf("checkPromptOnce error: %v", err)
	}
	if at {
		t.Error("expected atPrompt=false, got true on pane without anchor")
	}
}

func TestWaitForPrompt_InstantDetection(t *testing.T) {
	name := newTestSession(t)
	if err := SendKeys(name, "printf '\\xe2\\x9d\\xaf\\xc2\\xa0X'", true); err != nil {
		t.Fatal(err)
	}
	time.Sleep(300 * time.Millisecond)
	start := time.Now()
	at, _, err := WaitForPrompt(name, 5*time.Second)
	elapsed := time.Since(start)
	if err != nil {
		t.Fatalf("WaitForPrompt error: %v", err)
	}
	if !at {
		t.Error("expected atPrompt=true on already-prompting pane")
	}
	if elapsed > 500*time.Millisecond {
		t.Errorf("instant detection too slow: %v (expected <500ms)", elapsed)
	}
}

func TestWaitForPrompt_DelayedDetection(t *testing.T) {
	name := newTestSession(t)
	// Print the anchor after ~600ms — must be detected during polling, not
	// returned immediately. WaitForPrompt should resolve in roughly the
	// delay window (within one poll interval of 200ms slack).
	if err := SendKeys(name, "(sleep 0.6; printf '\\xe2\\x9d\\xaf\\xc2\\xa0X')&", true); err != nil {
		t.Fatal(err)
	}
	start := time.Now()
	at, _, err := WaitForPrompt(name, 5*time.Second)
	elapsed := time.Since(start)
	if err != nil {
		t.Fatalf("WaitForPrompt error: %v", err)
	}
	if !at {
		t.Error("expected atPrompt=true after delayed prompt")
	}
	if elapsed < 500*time.Millisecond {
		t.Errorf("returned too early: %v (anchor only printed at ~600ms)", elapsed)
	}
	if elapsed > 2*time.Second {
		t.Errorf("returned too late: %v (expected ~600-900ms with 200ms poll slack)", elapsed)
	}
}

func TestWaitForPrompt_TimeoutWhenNoPrompt(t *testing.T) {
	name := newTestSession(t)
	if err := SendKeys(name, "echo no-prompt-here-ever", true); err != nil {
		t.Fatal(err)
	}
	start := time.Now()
	at, out, err := WaitForPrompt(name, 800*time.Millisecond)
	elapsed := time.Since(start)
	if err != nil {
		t.Fatalf("WaitForPrompt should not error on timeout, got: %v", err)
	}
	if at {
		t.Errorf("expected atPrompt=false on timeout, got true. capture:\n%s", out)
	}
	if elapsed < 700*time.Millisecond {
		t.Errorf("returned before timeout deadline: %v (expected ≥800ms)", elapsed)
	}
	if elapsed > 1500*time.Millisecond {
		t.Errorf("returned too long after deadline: %v (expected ~800-1100ms)", elapsed)
	}
	if out == "" {
		t.Error("expected non-empty lastCapture on timeout for diagnostics")
	}
}

func TestWaitForPrompt_ZeroTimeoutSingleShot(t *testing.T) {
	name := newTestSession(t)
	if err := SendKeys(name, "echo bare", true); err != nil {
		t.Fatal(err)
	}
	time.Sleep(200 * time.Millisecond)
	start := time.Now()
	at, _, err := WaitForPrompt(name, 0)
	elapsed := time.Since(start)
	if err != nil {
		t.Fatalf("WaitForPrompt error: %v", err)
	}
	if at {
		t.Error("expected atPrompt=false on bare-shell pane")
	}
	if elapsed > 300*time.Millisecond {
		t.Errorf("single-shot check too slow: %v (expected <300ms — must not poll)", elapsed)
	}
}

func TestWaitForPrompt_CaptureErrorOnUnknownTarget(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping tmux test in short mode")
	}
	if !HasTmux() {
		t.Skip("tmux not available")
	}
	bogus := fmt.Sprintf("bot-hq-no-such-session-%d", time.Now().UnixNano())
	at, out, err := WaitForPrompt(bogus, 0)
	if err == nil {
		t.Error("expected error on unknown tmux target, got nil")
	}
	if at {
		t.Error("expected atPrompt=false on capture error")
	}
	if out != "" {
		t.Errorf("expected empty output on capture error, got %q", out)
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
