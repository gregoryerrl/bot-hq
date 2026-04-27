package gemma

import (
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// fakePaneActivity is a deterministic checker for stale-detection tests.
// Each call returns the next value queued for that target; when the queue
// runs out the last value sticks (sustained-silence semantics).
type fakePaneActivity struct {
	values map[string][]int64
	calls  map[string]int
}

func newFakePaneActivity() *fakePaneActivity {
	return &fakePaneActivity{values: map[string][]int64{}, calls: map[string]int{}}
}

func (f *fakePaneActivity) queue(target string, vs ...int64) {
	f.values[target] = append(f.values[target], vs...)
}

func (f *fakePaneActivity) check(target string) (int64, error) {
	vs := f.values[target]
	idx := f.calls[target]
	f.calls[target] = idx + 1
	if idx >= len(vs) {
		if len(vs) == 0 {
			return 0, nil
		}
		return vs[len(vs)-1], nil
	}
	return vs[idx], nil
}

func newTestGemma(t *testing.T) (*Gemma, *hub.DB) {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return New(db, hub.GemmaConfig{}), db
}

func countStaleCoderMsgs(t *testing.T, db *hub.DB) int {
	t.Helper()
	msgs, err := db.GetRecentMessages(200)
	if err != nil {
		t.Fatal(err)
	}
	n := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && m.ToAgent == "rain" && strings.Contains(m.Content, "[STALE-CODER]") {
			n++
		}
	}
	return n
}

// TestStaleDetectionFiresOnSilentMcpAndPane locks the Shape γ stale path:
// when last_seen is past the threshold AND the tmux pane has produced no
// new output since the previous tick's baseline, Emma PMs Rain.
func TestStaleDetectionFiresOnSilentMcpAndPane(t *testing.T) {
	g, db := newTestGemma(t)

	target := "test:0.1"
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "coder-stuck",
		Name:   "Stuck",
		Type:   protocol.AgentCoder,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	}); err != nil {
		t.Fatal(err)
	}

	fake := newFakePaneActivity()
	fake.queue(target, 1000, 1000) // identical across both ticks → silent
	g.paneActivity = fake.check

	// Tick 1: establish baseline (no flag yet — no prior observation).
	virtualNow := time.Now().Add(6 * time.Minute)
	g.checkStaleAgentsAt(virtualNow)
	if got := countStaleCoderMsgs(t, db); got != 0 {
		t.Fatalf("first tick must establish baseline without flagging; got %d stale msgs", got)
	}

	// Tick 2: same pane activity timestamp → genuinely silent → flag.
	g.checkStaleAgentsAt(virtualNow.Add(30 * time.Second))
	if got := countStaleCoderMsgs(t, db); got != 1 {
		t.Errorf("expected 1 stale-coder PM after second silent tick, got %d", got)
	}
}

// TestStaleDetectionSuppressedByPaneActivity locks the false-positive guard:
// last_seen stale BUT tmux pane produced new output between ticks → NO flag,
// because the pane backup signal indicates the agent is alive (probably mid-
// long-running Bash with timeout up to 600s).
func TestStaleDetectionSuppressedByPaneActivity(t *testing.T) {
	g, db := newTestGemma(t)

	target := "test:0.2"
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "coder-busy",
		Name:   "Busy",
		Type:   protocol.AgentCoder,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	}); err != nil {
		t.Fatal(err)
	}

	fake := newFakePaneActivity()
	fake.queue(target, 1000, 1042) // pane advancing — bash producing output
	g.paneActivity = fake.check

	virtualNow := time.Now().Add(6 * time.Minute)
	g.checkStaleAgentsAt(virtualNow)
	g.checkStaleAgentsAt(virtualNow.Add(30 * time.Second))

	if got := countStaleCoderMsgs(t, db); got != 0 {
		t.Errorf("pane-activity-alive agent must not be flagged stale; got %d PMs", got)
	}
}

// TestStaleDetectionFallbackForNoTmuxTarget locks the Shape α fallback path:
// agents without tmux_target Meta (future webhook/voice agents) get flagged
// purely on last_seen staleness — no pane backup signal available.
func TestStaleDetectionFallbackForNoTmuxTarget(t *testing.T) {
	g, db := newTestGemma(t)

	if err := db.RegisterAgent(protocol.Agent{
		ID:     "voice-1",
		Name:   "Voice",
		Type:   protocol.AgentVoice,
		Status: protocol.StatusOnline,
		Meta:   "", // no tmux_target → Shape α fallback
	}); err != nil {
		t.Fatal(err)
	}

	// Pane checker should never be called for no-tmux-target agent; install
	// one that errors to catch any accidental tmux query.
	called := false
	g.paneActivity = func(target string) (int64, error) {
		called = true
		return 0, nil
	}

	virtualNow := time.Now().Add(6 * time.Minute)
	g.checkStaleAgentsAt(virtualNow)

	if called {
		t.Errorf("paneActivity must not be called for agent without tmux_target")
	}
	if got := countStaleCoderMsgs(t, db); got != 1 {
		t.Errorf("Shape α fallback must flag on first tick; got %d PMs", got)
	}
}

// TestStaleDetectionHysteresis locks the no-spam invariant: same stale agent
// across multiple ticks emits exactly ONE PM. Phase H slice 4 C7 replaced
// the 30min shouldFlag hysteresis with the lean-(b) advance-check: re-fire
// is suppressed until LastSeen advances. A frozen agent's LastSeen never
// advances, so 4 ticks still collapse to 1 PM — same observable result,
// tighter semantic (no spurious re-fires every 30min on persistent dead
// agents). Per Rain C3.5 finding (msg 3801).
func TestStaleDetectionHysteresis(t *testing.T) {
	g, db := newTestGemma(t)

	target := "test:0.3"
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "coder-frozen",
		Name:   "Frozen",
		Type:   protocol.AgentCoder,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	}); err != nil {
		t.Fatal(err)
	}

	fake := newFakePaneActivity()
	fake.queue(target, 500, 500, 500, 500) // sustained silence
	g.paneActivity = fake.check

	base := time.Now().Add(6 * time.Minute)
	// Tick 1 establishes baseline; ticks 2-4 each see silent pane.
	for i := 0; i < 4; i++ {
		g.checkStaleAgentsAt(base.Add(time.Duration(i) * 30 * time.Second))
	}

	if got := countStaleCoderMsgs(t, db); got != 1 {
		t.Errorf("hysteresis must collapse repeated stale ticks to 1 PM; got %d", got)
	}
}

// TestStaleFlagFiresWhenLastSeenAdvances locks the lean-(b) re-arm semantic:
// once a stale-coder flag has fired for an agent, the advance-check tracks
// the LastSeen value at flag time. When LastSeen later advances (agent
// recovered) and then the agent goes stale again with a new LastSeen
// further in the past relative to a later virtual-now, a second flag must
// fire. This is the meaningful event the lean-(b) framing was designed to
// surface — agent came back, then went stale again — that the prior 30min
// shouldFlag window could miss or delay arbitrarily depending on rate-cap
// interactions (Rain C3.5 msg 3801).
func TestStaleFlagFiresWhenLastSeenAdvances(t *testing.T) {
	g, db := newTestGemma(t)

	target := "test:0.4"
	if err := db.RegisterAgent(protocol.Agent{
		ID:     "coder-recovered",
		Name:   "Recovered",
		Type:   protocol.AgentCoder,
		Status: protocol.StatusOnline,
		Meta:   `{"tmux_target":"` + target + `"}`,
	}); err != nil {
		t.Fatal(err)
	}

	fake := newFakePaneActivity()
	// First incident: sustained silence (500, 500). Recovery + second incident:
	// pane advances briefly (700) then goes silent again (700) — simulates
	// "agent did some work, then froze a second time."
	fake.queue(target, 500, 500, 700, 700)
	g.paneActivity = fake.check

	// First incident: virtual-now is past register-time + staleThreshold.
	// Tick 1 establishes baseline; tick 2 sees cur==prev → flag fires.
	firstNow := time.Now().Add(6 * time.Minute)
	g.checkStaleAgentsAt(firstNow)
	g.checkStaleAgentsAt(firstNow.Add(30 * time.Second))

	if got := countStaleCoderMsgs(t, db); got != 1 {
		t.Fatalf("first incident: expected 1 flag, got %d", got)
	}

	// Agent recovers — bump LastSeen via the production path. This advances
	// the tracked value past what flagStaleAgent stored at first-flag time,
	// re-arming the advance-check. Sleep guarantees millisecond separation
	// from RegisterAgent's now() so LastSeen strictly advances even on fast
	// hardware (UnixMilli resolution + tight scheduling can otherwise yield
	// equal timestamps; the advance-check relies on strict inequality).
	time.Sleep(2 * time.Millisecond)
	if err := db.UpdateAgentLastSeen("coder-recovered"); err != nil {
		t.Fatal(err)
	}
	recovered, err := db.GetAgent("coder-recovered")
	if err != nil {
		t.Fatal(err)
	}

	// Second incident: virtual-now advances past the new LastSeen +
	// staleThreshold. Tick 3 sees pane advance 500→700 (agent's brief
	// recovery activity) → continue (alive). Tick 4 sees cur==prev=700
	// (silent again) → fall through to flagStaleAgent. Tracker has
	// LastSeen=T_register from first flag; current LastSeen=T_recover →
	// not equal → second flag fires.
	secondNow := recovered.LastSeen.Add(6 * time.Minute)
	g.checkStaleAgentsAt(secondNow)
	g.checkStaleAgentsAt(secondNow.Add(30 * time.Second))

	if got := countStaleCoderMsgs(t, db); got != 2 {
		t.Errorf("second incident after LastSeen advance: expected 2 total flags, got %d", got)
	}
}
