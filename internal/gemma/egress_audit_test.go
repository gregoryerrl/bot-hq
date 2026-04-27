package gemma

import (
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// fakeEgressPane is a tick-controllable pane fixture for the egress
// auditor. queue() seeds returns in order; checks consume one per call.
type fakeEgressPane struct {
	returns []string
	idx     int
}

func (f *fakeEgressPane) capture(target string, lines int) (string, error) {
	if f.idx >= len(f.returns) {
		return f.returns[len(f.returns)-1], nil
	}
	out := f.returns[f.idx]
	f.idx++
	return out, nil
}

func countEgressGapMsgs(t *testing.T, db *hub.DB) int {
	t.Helper()
	msgs, err := db.GetRecentMessages(200)
	if err != nil {
		t.Fatal(err)
	}
	n := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && strings.Contains(m.Content, "[EGRESS-GAP]") {
			n++
		}
	}
	return n
}

// TestAuditEgressGapBaselineNoFlagFirstTick locks the same first-tick
// no-flag pattern stale-detect uses: the auditor establishes baseline
// on first observation and only flags after a prior tick exists for
// comparison.
func TestAuditEgressGapBaselineNoFlagFirstTick(t *testing.T) {
	g, db := newTestGemma(t)
	target := "test:0.1"
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	}); err != nil {
		t.Fatal(err)
	}

	fake := &fakeEgressPane{returns: []string{"❯ first state"}}
	g.egressPaneCapture = fake.capture

	g.auditEgressGapAt(time.Now())
	if got := countEgressGapMsgs(t, db); got != 0 {
		t.Errorf("first tick must establish baseline without flagging; got %d", got)
	}
}

// TestAuditEgressGapFiresAfterThresholdTicks locks the late-detection
// backstop: after egressGapTickThreshold consecutive ticks of
// pane-advanced AND no hub_send from the agent, an [EGRESS-GAP] alert
// fires.
func TestAuditEgressGapFiresAfterThresholdTicks(t *testing.T) {
	g, db := newTestGemma(t)
	target := "test:0.1"
	db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	})

	// Three distinct pane states across three ticks → pane advancing.
	// No hub_send messages from rain → egress gap.
	fake := &fakeEgressPane{returns: []string{
		"first pane state",
		"second pane state, more text",
		"third pane state, even more output",
	}}
	g.egressPaneCapture = fake.capture

	now := time.Now()
	g.auditEgressGapAt(now)                 // baseline tick — no flag
	g.auditEgressGapAt(now.Add(5 * time.Minute)) // gap tick 1
	g.auditEgressGapAt(now.Add(10 * time.Minute)) // gap tick 2 → flag

	if got := countEgressGapMsgs(t, db); got != 1 {
		t.Errorf("expected 1 [EGRESS-GAP] alert after %d consecutive gap ticks; got %d", egressGapTickThreshold, got)
	}
}

// TestAuditEgressGapResetsOnHubSend locks the negative: if the agent
// emits a hub message between ticks, the consecutive-tick counter
// resets and no flag fires.
func TestAuditEgressGapResetsOnHubSend(t *testing.T) {
	g, db := newTestGemma(t)
	target := "test:0.1"
	db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	})

	fake := &fakeEgressPane{returns: []string{
		"first state",
		"second state",
		"third state",
	}}
	g.egressPaneCapture = fake.capture

	now := time.Now()
	g.auditEgressGapAt(now) // baseline

	// Between ticks: rain emits a hub message. This advances the
	// auditor's MaxMsgID baseline → no gap on next tick.
	db.InsertMessage(protocol.Message{FromAgent: "rain", Type: protocol.MsgUpdate, Content: "[Rain] alive"})
	g.auditEgressGapAt(now.Add(5 * time.Minute)) // pane advanced + hub advanced → gap=0

	g.auditEgressGapAt(now.Add(10 * time.Minute)) // pane advanced, hub silent again → gap=1

	if got := countEgressGapMsgs(t, db); got != 0 {
		t.Errorf("hub-send between ticks should reset gap counter; got %d alerts (want 0 since gap only reached 1)", got)
	}
}

// TestAuditEgressGapPaneStableNoFlag locks the negative: a pane that
// hasn't changed (idle agent at a clean prompt) does NOT count as
// advancing, so no gap accumulates.
func TestAuditEgressGapPaneStableNoFlag(t *testing.T) {
	g, db := newTestGemma(t)
	target := "test:0.1"
	db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	})

	fake := &fakeEgressPane{returns: []string{"❯", "❯", "❯", "❯"}}
	g.egressPaneCapture = fake.capture

	now := time.Now()
	g.auditEgressGapAt(now)
	g.auditEgressGapAt(now.Add(5 * time.Minute))
	g.auditEgressGapAt(now.Add(10 * time.Minute))
	g.auditEgressGapAt(now.Add(15 * time.Minute))

	if got := countEgressGapMsgs(t, db); got != 0 {
		t.Errorf("stable pane (no advance) must not flag; got %d alerts", got)
	}
}

// TestAuditEgressGapHysteresisKeyIncludesHash locks the per-(agent,
// pane-hash) dedup contract Rain caveated in 4071 Q4: a re-fire after
// the first flag requires a NEW pane state, not just persistence of
// the gap on the same state.
func TestAuditEgressGapHysteresisKeyIncludesHash(t *testing.T) {
	g, db := newTestGemma(t)
	target := "test:0.1"
	db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	})

	// Pane advances through 3 states then stalls on state C.
	fake := &fakeEgressPane{returns: []string{
		"state A",
		"state B",
		"state C",
		"state C", // identical → no advance, gap counter resets
		"state C", // still identical
	}}
	g.egressPaneCapture = fake.capture

	now := time.Now()
	g.auditEgressGapAt(now)                       // baseline
	g.auditEgressGapAt(now.Add(5 * time.Minute))  // gap=1
	g.auditEgressGapAt(now.Add(10 * time.Minute)) // gap=2 → flag at state C
	g.auditEgressGapAt(now.Add(15 * time.Minute)) // pane stable (still C) → gap resets to 0
	g.auditEgressGapAt(now.Add(20 * time.Minute)) // still stable → gap stays 0

	if got := countEgressGapMsgs(t, db); got != 1 {
		t.Errorf("flag should fire exactly once at threshold; persistent same-hash state must not re-fire. got %d", got)
	}
}

// TestPaneContentHashStable locks that the hash function is
// deterministic — same input → same hex string.
func TestPaneContentHashStable(t *testing.T) {
	a := paneContentHash("hello world\n❯")
	b := paneContentHash("hello world\n❯")
	if a != b {
		t.Errorf("paneContentHash not deterministic: %q vs %q", a, b)
	}
	c := paneContentHash("hello world\n❯ different")
	if a == c {
		t.Errorf("different inputs must hash differently")
	}
}

// TestLastNonEmptyLine locks the alert-snippet helper: returns the
// last non-blank line, trims trailing whitespace, ignores trailing
// blank lines.
func TestLastNonEmptyLine(t *testing.T) {
	cases := map[string]string{
		"hello":              "hello",
		"hello\n":            "hello",
		"hello\nworld":       "world",
		"hello\nworld\n":     "world",
		"hello\nworld\n\n\n": "world",
		"":                   "",
		"\n\n":               "",
		"a\nb\n   \n":        "b",
	}
	for in, want := range cases {
		if got := lastNonEmptyLine(in); got != want {
			t.Errorf("lastNonEmptyLine(%q) = %q, want %q", in, got, want)
		}
	}
}

// TestAuditEgressGapHaltSuppression locks H-31 parity: while the
// halt-state is active, the auditor must no-op. Agents finishing
// their final tool call before posting close-SNAP would otherwise
// false-flag during the halt window.
func TestAuditEgressGapHaltSuppression(t *testing.T) {
	g, db := newTestGemma(t)
	target := "test:0.1"
	db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	})
	if err := db.SetHaltActive("trio", "test halt", "rain"); err != nil {
		t.Fatal(err)
	}

	fake := &fakeEgressPane{returns: []string{"a", "b", "c"}}
	g.egressPaneCapture = fake.capture

	now := time.Now()
	g.auditEgressGapAt(now)
	g.auditEgressGapAt(now.Add(5 * time.Minute))
	g.auditEgressGapAt(now.Add(10 * time.Minute))

	if got := countEgressGapMsgs(t, db); got != 0 {
		t.Errorf("halt-suppression violated: got %d alerts during active halt", got)
	}
}
