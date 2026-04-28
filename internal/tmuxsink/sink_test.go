package tmuxsink

import (
	"errors"
	"strings"
	"sync"
	"testing"
)

// fakeStore implements Store with in-memory queue for testing.
type fakeStore struct {
	mu       sync.Mutex
	queue    []QueuedItem
	nextID   int64
	enqErr   error
	pendErr  error
	statErr  error
	enqCalls int
}

func (f *fakeStore) EnqueueMessage(messageID int64, targetAgent, tmuxTarget, formattedText string) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.enqErr != nil {
		return f.enqErr
	}
	f.enqCalls++
	f.nextID++
	f.queue = append(f.queue, QueuedItem{
		ID:            f.nextID,
		MessageID:     messageID,
		FormattedText: formattedText,
		Attempts:      0,
	})
	return nil
}

func (f *fakeStore) PendingForAgent(agentID string) ([]QueuedItem, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.pendErr != nil {
		return nil, f.pendErr
	}
	out := make([]QueuedItem, len(f.queue))
	copy(out, f.queue)
	return out, nil
}

func (f *fakeStore) UpdateQueueStatus(id int64, status string, attempts int) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.statErr != nil {
		return f.statErr
	}
	if status == "delivered" {
		filtered := f.queue[:0]
		for _, qm := range f.queue {
			if qm.ID != id {
				filtered = append(filtered, qm)
			}
		}
		f.queue = filtered
	}
	return nil
}

// fakePane records capture calls + lets test control the response.
type fakePane struct {
	mu           sync.Mutex
	captureRet   []string // queue of capture-pane output strings
	captureErr   error
	sendCalls    []string // text passed to send
	sendErr      error
	captureCalls int
}

func (p *fakePane) capture(target string, lines int) (string, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.captureCalls++
	if p.captureErr != nil {
		return "", p.captureErr
	}
	if len(p.captureRet) == 0 {
		return "", nil // ready by default (empty pane = ready per IsReady contract)
	}
	out := p.captureRet[0]
	p.captureRet = p.captureRet[1:]
	return out, nil
}

func (p *fakePane) send(target, keys string, enter bool) error {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.sendErr != nil {
		return p.sendErr
	}
	p.sendCalls = append(p.sendCalls, keys)
	return nil
}

// newTestSink builds a Sink with fake store + fake pane wired in.
func newTestSink() (*Sink, *fakeStore, *fakePane) {
	store := &fakeStore{}
	pane := &fakePane{}
	s := New(store, "test-agent", "test:0.0")
	s.capture = pane.capture
	s.send = pane.send
	return s, store, pane
}

// === IsReady tests (ported from hub_test.go isReady cases) ===

func TestIsReady_EmptyPaneIsReady(t *testing.T) {
	if !IsReady("") {
		t.Error("empty pane must default to ready (fail-safe)")
	}
}

func TestIsReady_BusySpinnerGlyph(t *testing.T) {
	pane := "some text\n  ✶ Crunching\n──── prompt-box ────\n❯ "
	if IsReady(pane) {
		t.Error("✶ glyph in scan window must read as busy")
	}
}

func TestIsReady_BusyRunningPrefix(t *testing.T) {
	pane := "Running… 5s\n──── ────\n❯ "
	if IsReady(pane) {
		t.Error("Running… line-prefix must read as busy")
	}
}

func TestIsReady_BusyWorkingPrefix(t *testing.T) {
	pane := "Working… 12s\n──── ────\n❯ "
	if IsReady(pane) {
		t.Error("Working… line-prefix must read as busy")
	}
}

func TestIsReady_StaleCrunchedSummaryNotBusy(t *testing.T) {
	// ✻ persists in idle scrollback; must NOT classify as busy.
	pane := "✻ Crunched for 12s\n──── ────\n❯ "
	if !IsReady(pane) {
		t.Error("✻ scrollback summary must NOT classify as busy")
	}
}

func TestIsReady_BusyMarkerInQuotedText_NotBusy(t *testing.T) {
	// Busy markers below the prompt-box border must NOT classify as busy.
	// Scan window is the 7 lines ABOVE the border.
	pane := "above-border-line\n──── ────\nuser said \"Running…\""
	if !IsReady(pane) {
		t.Error("Running… below border (in quoted user text) must NOT classify as busy")
	}
}

func TestIsReady_RunningWithBoxDrawingPrefix(t *testing.T) {
	// ⎿ tool-output box-drawing prefix must be stripped before line-prefix check.
	pane := "  ⎿ Running… 5s\n──── ────\n❯ "
	if IsReady(pane) {
		t.Error("Running… after ⎿ strip must classify as busy")
	}
}

// TestIsReady_RealWorldFixtures locks IsReady against real-world Claude
// Code pane captures (idle trio, active synth ✶ spinner, active bash
// Running…, quoted Running… inside agent reply).
//
// Ported from internal/hub/hub_test.go TestIsReady (which tested the prior
// *Hub.isReady method, removed in Phase I W2 I-7 Layer-2 refactor when
// the pure tmuxsink.IsReady function replaced it). H-22-bis regression
// locks preserved: INSERT-mode footer + ✻ summary glyph must NOT
// false-busy; spinner ✶ in scan window MUST classify busy.
func TestIsReady_RealWorldFixtures(t *testing.T) {
	// Realistic capture of an idle trio agent pane: prompt-box bordered by
	// `──` lines, INSERT-mode footer below. The `✻ Crunched` line is the
	// most-recent turn's static summary glyph — must not flag busy.
	idleTrioPane := strings.Join([]string{
		"⏺ Brian's drive. Watching.",
		"",
		"✻ Crunched for 3s",
		"",
		"────────────────────────",
		"❯ ",
		"────────────────────────",
		"  -- INSERT -- ⏵⏵ bypass permissions on (shift+tab to cycle)",
		"",
	}, "\n")

	// Active synthesizing — `✶` glyph in the live spinner frame.
	activeSynthPane := strings.Join([]string{
		"⏺ Working through the change set.",
		"",
		"✶ Synthesizing… (49s · ↓ 2.9k tokens)",
		"",
		"────────────────────────",
		"❯ ",
		"────────────────────────",
		"  -- INSERT -- ⏵⏵ bypass permissions on (shift+tab to cycle)",
	}, "\n")

	// Active bash tool — `Running…` indented under the `⏺ Bash(...)` line.
	activeBashPane := strings.Join([]string{
		"⏺ Bash(cd ~/Projects/bot-hq && go test ./...)",
		"  ⎿  Running…",
		"",
		"────────────────────────",
		"❯ ",
		"────────────────────────",
		"  -- INSERT -- ⏵⏵ bypass permissions on (shift+tab to cycle)",
	}, "\n")

	// Regression-lock for line-prefix discipline: agent reply text contains
	// "Running…" as substring (e.g. quoting a log line). Must NOT false-busy
	// because the substring is not at line-start (after trim).
	quotedRunningPane := strings.Join([]string{
		"⏺ Investigating the failure log:",
		`  "[queue] Message 4123 to rain — Running… retry pending"`,
		"  Diagnosis complete.",
		"",
		"────────────────────────",
		"❯ ",
		"────────────────────────",
	}, "\n")

	tests := []struct {
		name   string
		output string
		want   bool
	}{
		// Pre-inversion test cases — kept for backwards-compat baseline.
		{"shell prompt $", "some output\n$ ", true},
		{"zsh prompt ❯", "some output\n❯", true},
		{"generic prompt >", "some output\n>", true},
		// Inversion philosophy: heuristic-miss → fail-safe-to-ready. Both of
		// the following were `false` under the old isAtPrompt (last-line check
		// failed because content didn't end with a known prompt suffix). Under
		// IsReady, no busy-marker present → ready=true. The shift IS the fix.
		{"trailing newline only", "some output\n", true},
		{"unknown content no busy markers", "Processing your request\n  working on it", true},
		{"prompt after output", "some output\n$ ", true},

		// H-22-bis regression-lock + Rain's real-world pane fixtures.
		{"idle trio pane with INSERT footer + ✻ summary glyph", idleTrioPane, true},
		{"active synthesizing — ✶ glyph above prompt box", activeSynthPane, false},
		{"active bash — Running… line-prefix above prompt box", activeBashPane, false},
		{"quoted Running… inside agent reply (substring not prefix)", quotedRunningPane, true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := IsReady(tt.output)
			if got != tt.want {
				t.Errorf("IsReady(%q) = %v, want %v", tt.output, got, tt.want)
			}
		})
	}
}

// === LastLineSummary tests ===

func TestLastLineSummary_Empty(t *testing.T) {
	if LastLineSummary("") != "" {
		t.Error("empty pane → empty last-line")
	}
}

func TestLastLineSummary_Truncates80(t *testing.T) {
	long := strings.Repeat("a", 100)
	got := LastLineSummary(long)
	if len(got) != 80 {
		t.Errorf("expected 80-byte truncation, got %d", len(got))
	}
}

func TestLastLineSummary_PreservesWhitespace(t *testing.T) {
	got := LastLineSummary("line1\n  ❯  ")
	if got != "  ❯  " {
		t.Errorf("expected whitespace preserved, got %q", got)
	}
}

// === Sink.Deliver tests ===

func TestSink_Deliver_ReadyPaneSends(t *testing.T) {
	s, store, pane := newTestSink()
	pane.captureRet = []string{""} // empty = ready

	dec := s.Deliver(42, "hello")

	if dec.Outcome != "sent" {
		t.Errorf("expected outcome=sent, got %s (err=%v)", dec.Outcome, dec.Err)
	}
	if !dec.Ready {
		t.Error("expected Ready=true")
	}
	if len(pane.sendCalls) != 1 || pane.sendCalls[0] != "hello" {
		t.Errorf("expected send called with 'hello', got %v", pane.sendCalls)
	}
	if store.enqCalls != 0 {
		t.Errorf("expected no enqueue, got %d", store.enqCalls)
	}
}

func TestSink_Deliver_BusyPaneEnqueues(t *testing.T) {
	s, store, pane := newTestSink()
	pane.captureRet = []string{"Working… 30s\n──── ────\n❯ "} // busy

	dec := s.Deliver(99, "queued msg")

	if dec.Outcome != "queued" {
		t.Errorf("expected outcome=queued, got %s (err=%v)", dec.Outcome, dec.Err)
	}
	if dec.Ready {
		t.Error("expected Ready=false")
	}
	if len(pane.sendCalls) != 0 {
		t.Errorf("busy pane must NOT send, got %v", pane.sendCalls)
	}
	if store.enqCalls != 1 {
		t.Errorf("expected 1 enqueue, got %d", store.enqCalls)
	}
	if len(store.queue) != 1 || store.queue[0].MessageID != 99 {
		t.Errorf("queue should contain msg 99, got %+v", store.queue)
	}
}

func TestSink_Deliver_DrainsQueueOnReady(t *testing.T) {
	s, store, pane := newTestSink()
	// Pre-seed queue with 2 pending messages.
	store.EnqueueMessage(101, "test-agent", "test:0.0", "queued-1")
	store.EnqueueMessage(102, "test-agent", "test:0.0", "queued-2")
	// Capture returns ready 4 times: once for Deliver, then once per drain iter (2) + 1 final check
	pane.captureRet = []string{"", "", "", ""}

	dec := s.Deliver(200, "fresh")

	if dec.Outcome != "sent" {
		t.Errorf("expected sent, got %s", dec.Outcome)
	}
	// Should have sent: fresh + queued-1 + queued-2 = 3 sends total.
	if len(pane.sendCalls) != 3 {
		t.Errorf("expected 3 sends (fresh + 2 drained), got %d: %v", len(pane.sendCalls), pane.sendCalls)
	}
	if pane.sendCalls[0] != "fresh" {
		t.Errorf("first send should be the new msg, got %q", pane.sendCalls[0])
	}
	if len(store.queue) != 0 {
		t.Errorf("queue should be drained, still has %d", len(store.queue))
	}
}

func TestSink_Deliver_CaptureErrReturnsError(t *testing.T) {
	s, _, pane := newTestSink()
	pane.captureErr = errors.New("tmux died")

	dec := s.Deliver(0, "anything")

	if dec.Outcome != "capture_err" {
		t.Errorf("expected capture_err, got %s", dec.Outcome)
	}
	if dec.Err == nil {
		t.Error("expected error to be propagated")
	}
}

func TestSink_Deliver_SendErrReturnsError(t *testing.T) {
	s, _, pane := newTestSink()
	pane.captureRet = []string{""}
	pane.sendErr = errors.New("send-keys failed")

	dec := s.Deliver(7, "x")

	if dec.Outcome != "send_keys_err" {
		t.Errorf("expected send_keys_err, got %s", dec.Outcome)
	}
	if dec.Err == nil {
		t.Error("expected error to be propagated")
	}
}

func TestSink_Deliver_EnqueueErrReturnsError(t *testing.T) {
	s, store, pane := newTestSink()
	pane.captureRet = []string{"Running… 5s\n──── ────\n❯ "} // busy
	store.enqErr = errors.New("db locked")

	dec := s.Deliver(11, "x")

	if dec.Outcome != "enqueue_err" {
		t.Errorf("expected enqueue_err, got %s", dec.Outcome)
	}
	if dec.Err == nil {
		t.Error("expected error to be propagated")
	}
}

func TestSink_Deliver_SelfPasteMsgIDZero(t *testing.T) {
	// Rain/Brian self-paste use msgID=0 sentinel. Verify queue row stores it.
	s, store, pane := newTestSink()
	pane.captureRet = []string{"Working… 5s\n──── ────\n❯ "} // busy

	dec := s.Deliver(0, "self-paste")

	if dec.Outcome != "queued" {
		t.Errorf("expected queued, got %s", dec.Outcome)
	}
	if len(store.queue) != 1 || store.queue[0].MessageID != 0 {
		t.Errorf("self-paste queue row must have MessageID=0, got %+v", store.queue)
	}
}

func TestSink_Deliver_PerTargetMutexSerializes(t *testing.T) {
	// Concurrent Deliver calls on the same Sink must not interleave.
	// Use a fakePane that records call ordering.
	s, _, pane := newTestSink()
	pane.captureRet = []string{"", "", "", "", "", "", "", "", "", ""} // 10 ready captures

	const N = 5
	var wg sync.WaitGroup
	for i := 0; i < N; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			s.Deliver(int64(i), "msg")
		}(i)
	}
	wg.Wait()

	if len(pane.sendCalls) != N {
		t.Errorf("expected %d sends total (concurrent serialized), got %d", N, len(pane.sendCalls))
	}
}

// === Drain tests ===

func TestSink_Drain_DeliversReady(t *testing.T) {
	s, store, pane := newTestSink()
	store.EnqueueMessage(1, "test-agent", "test:0.0", "a")
	store.EnqueueMessage(2, "test-agent", "test:0.0", "b")
	pane.captureRet = []string{"", "", ""}

	n, err := s.Drain()

	if err != nil {
		t.Fatalf("unexpected drain error: %v", err)
	}
	if n != 2 {
		t.Errorf("expected 2 delivered, got %d", n)
	}
	if len(pane.sendCalls) != 2 {
		t.Errorf("expected 2 send calls, got %d", len(pane.sendCalls))
	}
}

func TestSink_Drain_StopsOnBusy(t *testing.T) {
	s, store, pane := newTestSink()
	store.EnqueueMessage(1, "test-agent", "test:0.0", "first")
	store.EnqueueMessage(2, "test-agent", "test:0.0", "second")
	// First capture: ready. Second capture: busy. Drain should stop after #1.
	pane.captureRet = []string{"", "Running… 5s\n──── ────\n❯ "}

	n, err := s.Drain()

	if err != nil {
		t.Fatalf("unexpected drain error: %v", err)
	}
	if n != 1 {
		t.Errorf("expected 1 delivered before busy, got %d", n)
	}
	if len(store.queue) != 1 {
		t.Errorf("expected 1 row remaining (the second), got %d", len(store.queue))
	}
}
