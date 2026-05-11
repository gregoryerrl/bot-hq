// Package tmuxsink encapsulates at-least-once tmux pane delivery
// semantics shared by the hub's targeted-dispatch path
// (internal/hub/hub.go dispatchToTmux) and each agent's self-paste path
// (internal/rain/rain.go SendCommand, internal/brian/brian.go SendCommand).
//
// Phase I W2 I-7 Layer-2 FULL extraction. Prior to this package, only the
// hub.dispatchToTmux path had isReady (pane-busy) detection + retry queue;
// rain/brian SendCommand was a naked tmux send-keys + 500ms sleep + Enter
// with no readiness check. Pane-busy at the precise moment of paste caused
// Claude Code's input handler to drop/mangle the keystrokes silently —
// the core dispatch-fail #16 Layer-2 race. By routing all three callers
// through a single Sink, every pane delivery gets the same isReady check
// and the same EnqueueMessage retry semantics.
//
// Architecture:
//   - Sink owns per-target sync.Mutex (serializes concurrent Deliver calls
//     for the same pane — hub-targeted-dispatch and agent-self-paste no
//     longer interleave on send-keys).
//   - Sink does NOT own a goroutine. Drain ticker stays in the hub-side
//     processMessageQueue caller (hub already runs that ticker; rain/brian
//     don't need their own — the hub ticker drains all agent queues
//     including rain's and brian's self-paste queue rows).
//   - Sink takes a narrow Store interface (Enqueue / PendingForAgent /
//     UpdateQueueStatus) instead of *hub.DB directly. Keeps tmuxsink free
//     of any hub import — rain/brian → tmuxsink → tmux package only.
//
// msgID=0 sentinel: rain/brian self-paste calls have no associated hub
// message ID. Pass msgID=0; the queue row gets MessageID=0. Cosmetic
// implication: retry-exhaust alert reads "Message 0 to rain failed
// after 30 attempts" — ugly but correct. Schema extension to NULLable
// or sentinel-aware retry-alert is tracked as a Tier-2-hold ratchet.
package tmuxsink

import (
	"strings"
	"sync"

	"github.com/gregoryerrl/bot-hq/internal/tmux"
)

// QueuedItem is the tmuxsink-internal view of a pending queued message.
// Implementations of Store convert their native row type to this view.
type QueuedItem struct {
	ID            int64
	MessageID     int64
	FormattedText string
	Attempts      int
}

// Store is the narrow persistence interface tmuxsink needs. *hub.DB
// satisfies this implicitly via a thin adapter in package hub. Defined
// here (not in hub) so tmuxsink stays free of hub imports — breaks the
// would-be cycle (rain/brian → tmuxsink → hub.DB → tmuxsink ...).
type Store interface {
	EnqueueMessage(messageID int64, targetAgent, tmuxTarget, formattedText string) error
	PendingForAgent(agentID string) ([]QueuedItem, error)
	UpdateQueueStatus(id int64, status string, attempts int) error
}

// busyMarkerGlyphs are spinner glyphs that appear ONLY during active model
// processing. Claude Code redraws the line in-place during animation, so
// when capture sees `✶` in a frozen frame the agent is mid-stream. After
// the turn completes the line is rewritten to a static `✻ Crunched for Xs`
// summary — different glyph, not flagged. ✻ is intentionally excluded
// because it persists in idle scrollback.
//
// Ported verbatim from internal/hub/hub.go:372 — kept in sync via test
// (TestIsReady_BusyMarkerGlyphs locks the list).
var busyMarkerGlyphs = []string{"✶"}

// busyMarkerLinePrefixes are tool-active state suffixes. Both contain the
// U+2026 ellipsis (`…`, not three dots `...`) which is how Claude Code
// distinguishes them from arbitrary text. Anchored on line-start (after
// trim) to avoid false-busy on quoted log text in agent replies.
var busyMarkerLinePrefixes = []string{"Running…", "Working…"}

// IsReady determines whether a tmux pane can accept a hub message paste.
// Defaults to ready; returns false ONLY on positive busy-markers (spinner
// glyph ✶ via substring match, or tool-active Running…/Working… via
// line-start match after trim).
//
// Inversion of the prior isAtPrompt predicate. Failure-mode asymmetry
// rationale: false-ready degrades to benign paste-buffering — Claude Code
// queues input to next-turn submit. False-busy degrades to queue-exhaust
// + ledger spam (the H-22-bis incident: persistent INSERT-mode footer
// `-- INSERT -- ⏵⏵ bypass permissions on (shift+tab to cycle)` broke the
// last-line-suffix heuristic, classifying every duo agent as busy and
// exhausting every directed PM at 30/30). Pick the failure mode you can
// survive.
//
// Duo runs with --dangerously-skip-permissions; coders inheriting via
// spawn-contract get the same. Pane-with-active-modal-prompt panes
// (non-bypass coders) will be classified ready and may receive paste
// into the modal — acceptable per spawn-contract baseline.
//
// Scan window: the 7 lines immediately above the most-recent prompt-box
// border (`──...` line). Spinners always render above the input box,
// never inside or below it. Falls back to last 7 lines if no border
// found (older Claude Code UI or transient pane states). Empty capture
// → ready=true (fail-safe: we couldn't read the pane, default to ready).
//
// Glyph-list and line-prefix audits are required on Claude Code UI
// revisions — see ratchet item: migrate to agent-self-reported idle
// (Stop-hook → DB pulse → dispatch reads DB) to obviate the heuristic.
//
// Ported from internal/hub/hub.go:408 (was *Hub method, now pure func).
func IsReady(paneOutput string) bool {
	if paneOutput == "" {
		return true
	}
	lines := strings.Split(strings.TrimRight(paneOutput, "\n"), "\n")

	scanEnd := len(lines)
	for i := len(lines) - 1; i >= 0; i-- {
		if strings.HasPrefix(strings.TrimSpace(lines[i]), "──") {
			scanEnd = i
			break
		}
	}
	scanStart := scanEnd - 7
	if scanStart < 0 {
		scanStart = 0
	}

	for _, line := range lines[scanStart:scanEnd] {
		for _, glyph := range busyMarkerGlyphs {
			if strings.Contains(line, glyph) {
				return false
			}
		}
		// Strip leading whitespace + tool-output box-drawing prefix `⎿`
		// (Claude Code uses ⎿ to nest tool stdout under the ⏺ Bash(...)
		// header line). Anchoring on the post-strip start preserves the
		// false-busy regression-lock against arbitrary text containing
		// "Running…" as a substring.
		trimmed := strings.TrimSpace(strings.TrimLeft(line, " \t⎿"))
		for _, prefix := range busyMarkerLinePrefixes {
			if strings.HasPrefix(trimmed, prefix) {
				return false
			}
		}
	}
	return true
}

// LastLineSummary extracts the trailing pane line for diagnostic logging,
// truncated to 80 bytes to keep JSONL records compact. Whitespace is
// preserved so a true mid-render frame is distinguishable from a clean
// "❯" prompt. Ported from internal/hub/hub.go:331.
func LastLineSummary(paneOutput string) string {
	if paneOutput == "" {
		return ""
	}
	lines := strings.Split(strings.TrimRight(paneOutput, "\n"), "\n")
	last := lines[len(lines)-1]
	if len(last) > 80 {
		last = last[:80]
	}
	return last
}

// Decision is the outcome of a single Deliver call. Callers (hub) use the
// fields to record JSONL diagnostic decisions; rain/brian self-paste paths
// can ignore the details and check Outcome only.
type Decision struct {
	Outcome  string // "sent", "queued", "capture_err", "send_keys_err", "enqueue_err"
	Ready    bool
	LastLine string
	Err      error
}

// captureFn / sendFn are package-level injection points so tests can
// substitute fake tmux without exec'ing real tmux. Production wires the
// real tmux package functions via the package init.
type captureFn func(target string, lines int) (string, error)
type sendFn func(target, keys string, enter bool) error

var (
	defaultCapture captureFn = tmux.CapturePane
	defaultSend    sendFn    = tmux.SendKeys
)

// Sink encapsulates at-least-once tmux pane delivery for a single agent's
// pane. Callers construct one Sink per (agentID, tmuxTarget) and call
// Deliver to send formatted text. The Sink owns the per-target Mutex so
// concurrent Deliver calls (hub-targeted-dispatch + agent-self-paste)
// serialize on the same pane.
type Sink struct {
	store   Store
	agentID string
	target  string
	mu      sync.Mutex

	// Test injection points. Nil means "use package defaults".
	capture captureFn
	send    sendFn
}

// New constructs a Sink for the given agent and tmux pane target.
func New(store Store, agentID, target string) *Sink {
	return &Sink{store: store, agentID: agentID, target: target}
}

// captureFunc / sendFunc resolve injection or fall back to package defaults.
func (s *Sink) captureFunc() captureFn {
	if s.capture != nil {
		return s.capture
	}
	return defaultCapture
}
func (s *Sink) sendFunc() sendFn {
	if s.send != nil {
		return s.send
	}
	return defaultSend
}

// Deliver attempts to send formattedText to the tmux pane. If the pane
// is ready (IsReady), text is sent immediately and the queue is drained
// for this agent. If the pane is busy, the message is enqueued via
// Store.EnqueueMessage and will be retried on the next Drain pass.
//
// msgID=0 is the sentinel for self-paste calls (rain/brian SendCommand)
// where no hub message ID exists. Queue row gets MessageID=0.
//
// Returns a Decision describing the outcome — Outcome ∈ {sent, queued,
// capture_err, send_keys_err, enqueue_err}. Callers may use this to
// record JSONL diagnostic logs (hub does); rain/brian ignore the details
// and check Err only.
func (s *Sink) Deliver(msgID int64, formattedText string) Decision {
	s.mu.Lock()
	defer s.mu.Unlock()

	output, err := s.captureFunc()(s.target, 5)
	if err != nil {
		return Decision{Outcome: "capture_err", Err: err}
	}

	ready := IsReady(output)
	lastLine := LastLineSummary(output)

	if ready {
		if sendErr := s.sendFunc()(s.target, formattedText, true); sendErr != nil {
			return Decision{Outcome: "send_keys_err", Ready: true, LastLine: lastLine, Err: sendErr}
		}
		// Drain previously queued messages for this agent. Held under same
		// mu — drainLocked must NOT re-lock.
		s.drainLocked()
		return Decision{Outcome: "sent", Ready: true, LastLine: lastLine}
	}

	// Pane busy → queue for retry. Delivery is at-least-once: if we crash
	// between SendKeys and UpdateQueueStatus, the message stays "pending"
	// and may be re-sent.
	if enqErr := s.store.EnqueueMessage(msgID, s.agentID, s.target, formattedText); enqErr != nil {
		return Decision{Outcome: "enqueue_err", Ready: false, LastLine: lastLine, Err: enqErr}
	}
	return Decision{Outcome: "queued", Ready: false, LastLine: lastLine}
}

// Drain attempts to deliver any pending queued messages for this agent.
// Re-checks pane readiness before each send. Stops on first busy pane or
// send error. Returns the count of successfully delivered messages.
//
// Locks the per-target Mutex internally — must NOT be called from a
// context that already holds s.mu (use drainLocked for that).
func (s *Sink) Drain() (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.drainLocked()
}

// drainLocked is the implementation of Drain assuming s.mu is already
// held by the caller. Used by Deliver's "ready+sent" path to immediately
// drain any prior queued messages without releasing-then-reacquiring the
// mutex.
func (s *Sink) drainLocked() (int, error) {
	pending, err := s.store.PendingForAgent(s.agentID)
	if err != nil {
		return 0, err
	}
	delivered := 0
	for _, qm := range pending {
		output, err := s.captureFunc()(s.target, 5)
		if err != nil {
			return delivered, err
		}
		if !IsReady(output) {
			return delivered, nil
		}
		if err := s.sendFunc()(s.target, qm.FormattedText, true); err != nil {
			return delivered, err
		}
		// SendKeys already sleeps 500ms for bracketed paste — no extra
		// delay needed.
		if err := s.store.UpdateQueueStatus(qm.ID, "delivered", qm.Attempts+1); err != nil {
			return delivered, err
		}
		delivered++
	}
	return delivered, nil
}
