package panestate

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestComputeActivity locks Phase-E-equivalence parametrically: every case
// passes paneActive == time.Time{} (no pane source wired), and the activity
// tier must match the heartbeat-only Phase E rule. F-core-a's case-i ratchet
// is falsifiable here — any leak from the new pane-tier branches into the
// paneActive=zero path breaks one of these existing assertions.
func TestComputeActivity(t *testing.T) {
	now := time.Now()
	var paneZero time.Time // explicit zero — Phase-E-equivalence path
	cases := []struct {
		name     string
		status   protocol.AgentStatus
		lastSeen time.Time
		want     AgentActivity
	}{
		{"working: 0s recency", protocol.StatusOnline, now, ActivityWorking},
		{"working: 4s recency", protocol.StatusOnline, now.Add(-4 * time.Second), ActivityWorking},
		{"online: 5s recency (boundary)", protocol.StatusOnline, now.Add(-5 * time.Second), ActivityOnline},
		{"online: 30s recency", protocol.StatusOnline, now.Add(-30 * time.Second), ActivityOnline},
		{"stale: 60s recency (boundary)", protocol.StatusOnline, now.Add(-60 * time.Second), ActivityStale},
		{"stale: 1h recency", protocol.StatusOnline, now.Add(-1 * time.Hour), ActivityStale},
		{"offline overrides recency=0s", protocol.StatusOffline, now, ActivityOffline},
		{"offline overrides recency=1h", protocol.StatusOffline, now.Add(-1 * time.Hour), ActivityOffline},
		{"legacy StatusWorking treated by recency only", protocol.StatusWorking, now.Add(-30 * time.Second), ActivityOnline},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := ComputeActivity(tc.status, tc.lastSeen, paneZero, now)
			if got != tc.want {
				t.Errorf("ComputeActivity(%v, recency=%v, paneZero) = %v, want %v", tc.status, now.Sub(tc.lastSeen), got, tc.want)
			}
		})
	}
}

// TestComputeActivityWithPane locks the OR-combination semantics F-core-b
// will activate. Independent-threshold per Push 4: heartbeat and pane tiers
// each compared against their own constants, OR'd at each tier boundary.
func TestComputeActivityWithPane(t *testing.T) {
	now := time.Now()
	cases := []struct {
		name       string
		status     protocol.AgentStatus
		lastSeen   time.Time
		paneActive time.Time
		want       AgentActivity
	}{
		// F1: paneActive zero, heartbeat fresh — Phase-E identity (sanity).
		{"F1: paneZero + fresh heartbeat → working", protocol.StatusOnline, now.Add(-2 * time.Second), time.Time{}, ActivityWorking},

		// F2: heartbeat stale beyond OnlineWindow, pane fresh — pane-tier rescues.
		// This is the msg 2646 false-stale failure mode under fix.
		{"F2: stale heartbeat + fresh pane → working (pane wins)", protocol.StatusOnline, now.Add(-2 * time.Minute), now.Add(-2 * time.Second), ActivityWorking},

		// F3: heartbeat fresh, pane stale — heartbeat-tier preserves cheap-ping behavior.
		{"F3: fresh heartbeat + stale pane → working (heartbeat wins)", protocol.StatusOnline, now.Add(-2 * time.Second), now.Add(-2 * time.Minute), ActivityWorking},

		// F4: status offline always wins regardless of either signal.
		{"F4: offline overrides both fresh signals", protocol.StatusOffline, now.Add(-2 * time.Second), now.Add(-2 * time.Second), ActivityOffline},

		// F5: both signals at exact boundary — strict-inequality semantic preserved
		// from Phase E (recency == Window is NOT in the lower tier).
		{"F5: heartbeat=5s + pane=5s (boundary equality) → online", protocol.StatusOnline, now.Add(-5 * time.Second), now.Add(-5 * time.Second), ActivityOnline},

		// F6: heartbeat past stale threshold but pane fresh — pane-tier rescues
		// from heartbeat-stale, demonstrating independent-threshold OR.
		{"F6: heartbeat past 60s + pane=2s → working (pane-tier OR-fires)", protocol.StatusOnline, now.Add(-61 * time.Second), now.Add(-2 * time.Second), ActivityWorking},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := ComputeActivity(tc.status, tc.lastSeen, tc.paneActive, now)
			if got != tc.want {
				t.Errorf("ComputeActivity(%v, hb=%v, pane=%v) = %v, want %v",
					tc.status, now.Sub(tc.lastSeen), now.Sub(tc.paneActive), got, tc.want)
			}
		})
	}
}

func TestManagerRefresh(t *testing.T) {
	fake := &fakeSource{
		agents: []protocol.Agent{
			{ID: "a1", Name: "Agent 1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now()},
			{ID: "a2", Name: "Agent 2", Type: protocol.AgentCoder, Status: protocol.StatusOffline, LastSeen: time.Now().Add(-1 * time.Hour)},
		},
	}
	m := NewManager(fake, noPaneCapture)
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	snap := m.Snapshot()
	if len(snap) != 2 {
		t.Fatalf("len(snapshot) = %d, want 2", len(snap))
	}
	if snap[0].Activity != ActivityWorking {
		t.Errorf("snap[0].Activity = %v, want ActivityWorking", snap[0].Activity)
	}
	if snap[1].Activity != ActivityOffline {
		t.Errorf("snap[1].Activity = %v, want ActivityOffline", snap[1].Activity)
	}
}

func TestManagerSnapshotFreshness(t *testing.T) {
	// After Refresh runs against new DB state, Snapshot reflects the new state.
	// Locks against tabs holding stale []AgentSnapshot copies instead of *Manager references.
	fake := &fakeSource{
		agents: []protocol.Agent{
			{ID: "a1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now().Add(-1 * time.Hour)},
		},
	}
	m := NewManager(fake, noPaneCapture)
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].Activity; got != ActivityStale {
		t.Errorf("first snapshot got %v, want ActivityStale", got)
	}

	fake.agents[0].LastSeen = time.Now()
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].Activity; got != ActivityWorking {
		t.Errorf("second snapshot got %v, want ActivityWorking after Refresh", got)
	}
}

// TestSnapshotMarksStaleGen locks Phase G v1 #20: when an agent's
// rebuild_gen does not match the source's CurrentRebuildGen(), the
// snapshot's StaleGen flag is set. Zero-gen rows (legacy / pre-feature)
// are never flagged stale.
func TestSnapshotMarksStaleGen(t *testing.T) {
	now := time.Now()
	fake := &fakeSource{
		rebuildGen: 5,
		agents: []protocol.Agent{
			{ID: "current", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now, RebuildGen: 5},
			{ID: "prior", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now, RebuildGen: 4},
			{ID: "legacy", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: now, RebuildGen: 0},
		},
	}
	m := NewManager(fake, noPaneCapture)
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	snap := m.Snapshot()

	byID := map[string]AgentSnapshot{}
	for _, s := range snap {
		byID[s.ID] = s
	}
	if byID["current"].StaleGen {
		t.Errorf("current-gen agent should not be flagged stale")
	}
	if !byID["prior"].StaleGen {
		t.Errorf("prior-gen agent should be flagged stale")
	}
	if byID["legacy"].StaleGen {
		t.Errorf("legacy (gen=0) agent should not be flagged stale")
	}
}

// TestSnapshotStaleGenDisabledWhenSourceGenZero locks the back-compat
// invariant: if the source returns CurrentRebuildGen()==0 (pre-feature
// hub), no agent is ever flagged stale regardless of their RebuildGen.
func TestSnapshotStaleGenDisabledWhenSourceGenZero(t *testing.T) {
	fake := &fakeSource{
		rebuildGen: 0, // disabled
		agents: []protocol.Agent{
			{ID: "anything", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now(), RebuildGen: 99},
		},
	}
	m := NewManager(fake, noPaneCapture)
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if m.Snapshot()[0].StaleGen {
		t.Errorf("StaleGen should never be true when source returns gen=0")
	}
}

func TestManagerSnapshotIsCopy(t *testing.T) {
	// Snapshot must return a copy; mutating it must not affect Manager state.
	fake := &fakeSource{
		agents: []protocol.Agent{
			{ID: "a1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now()},
		},
	}
	m := NewManager(fake, noPaneCapture)
	m.Refresh()
	s1 := m.Snapshot()
	s1[0].ID = "mutated"
	s2 := m.Snapshot()
	if s2[0].ID != "a1" {
		t.Errorf("Manager state was mutated by external snapshot mutation: got %q", s2[0].ID)
	}
}

func TestThresholds(t *testing.T) {
	// Lock the spec'd window values so tuning changes are intentional.
	// F-core-a Push 4 lock: heartbeat- and pane-tier constants are independent.
	// Initial pane values match heartbeat for Phase-E-equivalence at F-core-a;
	// tunable independently once F-core-b activates pane sourcing.
	if HeartbeatWorkingWindow != 5*time.Second {
		t.Errorf("HeartbeatWorkingWindow = %v, want 5s", HeartbeatWorkingWindow)
	}
	if HeartbeatOnlineWindow != 60*time.Second {
		t.Errorf("HeartbeatOnlineWindow = %v, want 60s", HeartbeatOnlineWindow)
	}
	if PaneWorkingWindow != 5*time.Second {
		t.Errorf("PaneWorkingWindow = %v, want 5s (initial = heartbeat per Phase-E-equivalence)", PaneWorkingWindow)
	}
	if PaneOnlineWindow != 60*time.Second {
		t.Errorf("PaneOnlineWindow = %v, want 60s (initial = heartbeat per Phase-E-equivalence)", PaneOnlineWindow)
	}
	if StaleAgentWindow != 7*24*time.Hour {
		t.Errorf("StaleAgentWindow = %v, want 7d", StaleAgentWindow)
	}
}

type fakeSource struct {
	agents     []protocol.Agent
	rebuildGen int64
}

func (f *fakeSource) ListAgents(string) ([]protocol.Agent, error) {
	return f.agents, nil
}

func (f *fakeSource) CurrentRebuildGen() int64 {
	return f.rebuildGen
}

// noPaneCapture is the test default for tests that don't exercise pane logic.
// Returns empty content + nil error so first-tick seed always lands; agents
// without tmux_target Meta short-circuit before this is called.
func noPaneCapture(target string, lines int) (string, error) {
	return "", nil
}

// scriptedCapture returns a capturePane fake that yields outputs in order.
// Each invocation pops the next entry; a nil error means success, else error.
// Tests that step through deterministic content sequences across Refresh ticks
// use this to lock pane-cache transitions.
type scriptedCapture struct {
	outputs []string
	errs    []error
	calls   int
}

func (s *scriptedCapture) fn(target string, lines int) (string, error) {
	if s.calls >= len(s.outputs) {
		s.calls++
		return "", nil
	}
	out := s.outputs[s.calls]
	var err error
	if s.calls < len(s.errs) {
		err = s.errs[s.calls]
	}
	s.calls++
	return out, err
}

// agentWithTmux constructs a protocol.Agent with the tmux_target Meta field
// set — exercises the extractTmuxTarget JSON parse path.
func agentWithTmux(id, target string) protocol.Agent {
	return protocol.Agent{
		ID:       id,
		Name:     id,
		Type:     protocol.AgentBrian,
		Status:   protocol.StatusOnline,
		LastSeen: time.Now(),
		Meta:     `{"tmux_target":"` + target + `"}`,
	}
}

// TestRefreshPaneActivity_FirstTickSeeds locks E4: first observed capture for
// an agent populates the cache but leaves LastPaneActivity == zero (no prior
// frame to compare against — promoting on first sight inflates working-tier
// on startup).
func TestRefreshPaneActivity_FirstTickSeeds(t *testing.T) {
	fake := &fakeSource{agents: []protocol.Agent{agentWithTmux("a1", "session:0.0")}}
	cap := &scriptedCapture{outputs: []string{"abc"}}
	m := NewManager(fake, cap.fn)

	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if cap.calls != 1 {
		t.Fatalf("capturePane calls = %d, want 1", cap.calls)
	}
	snap := m.Snapshot()
	if !snap[0].LastPaneActivity.IsZero() {
		t.Errorf("first-tick LastPaneActivity = %v, want zero (seed-only)", snap[0].LastPaneActivity)
	}
	cached, ok := m.paneCache["a1"]
	if !ok {
		t.Fatal("paneCache missing entry for a1 after first-tick seed")
	}
	if cached.lastHash == 0 {
		t.Error("paneCache[a1].lastHash = 0, want non-zero (FNV of \"abc\")")
	}
	if !cached.lastActivity.IsZero() {
		t.Errorf("paneCache[a1].lastActivity = %v, want zero (seed-only)", cached.lastActivity)
	}
}

// TestRefreshPaneActivity_HashChangePromotes locks E5: hash differs between
// ticks → paneActive = now, cache updated. This is the primary "pane signal
// detected activity" path that F-core-b activates.
func TestRefreshPaneActivity_HashChangePromotes(t *testing.T) {
	fake := &fakeSource{agents: []protocol.Agent{agentWithTmux("a1", "session:0.0")}}
	cap := &scriptedCapture{outputs: []string{"abc", "xyz"}}
	m := NewManager(fake, cap.fn)

	// Tick 1: seed.
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	// Tick 2: hash changes → promote.
	before := time.Now()
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	after := time.Now()

	snap := m.Snapshot()
	if snap[0].LastPaneActivity.IsZero() {
		t.Fatal("LastPaneActivity = zero after hash-change tick, want non-zero")
	}
	if snap[0].LastPaneActivity.Before(before) || snap[0].LastPaneActivity.After(after) {
		t.Errorf("LastPaneActivity = %v, want within [%v, %v]", snap[0].LastPaneActivity, before, after)
	}
}

// TestRefreshPaneActivity_HashStableCarriesForward locks E3: hash matches
// across ticks → carry forward cached lastActivity. After a hash-change tick
// stamps a non-zero value, subsequent stable ticks must preserve it (until
// PaneOnlineWindow eventually demotes via threshold).
func TestRefreshPaneActivity_HashStableCarriesForward(t *testing.T) {
	fake := &fakeSource{agents: []protocol.Agent{agentWithTmux("a1", "session:0.0")}}
	cap := &scriptedCapture{outputs: []string{"abc", "xyz", "xyz"}}
	m := NewManager(fake, cap.fn)

	// Tick 1: seed.
	m.Refresh()
	// Tick 2: hash changes → promote, capture timestamp.
	m.Refresh()
	promotedAt := m.Snapshot()[0].LastPaneActivity
	if promotedAt.IsZero() {
		t.Fatal("setup: tick 2 should have produced non-zero LastPaneActivity")
	}
	// Tick 3: hash stable → carry forward.
	m.Refresh()
	carried := m.Snapshot()[0].LastPaneActivity
	if !carried.Equal(promotedAt) {
		t.Errorf("tick 3 LastPaneActivity = %v, want %v (carry-forward)", carried, promotedAt)
	}
}

// TestRefreshPaneActivity_NoTmuxTargetSkips locks E1: agents with no
// tmux_target Meta never hit capturePane and remain on the heartbeat-only
// path (LastPaneActivity == zero).
func TestRefreshPaneActivity_NoTmuxTargetSkips(t *testing.T) {
	fake := &fakeSource{agents: []protocol.Agent{
		{ID: "a1", Name: "a1", Type: protocol.AgentBrian, Status: protocol.StatusOnline, LastSeen: time.Now()},
	}}
	cap := &scriptedCapture{outputs: []string{"abc"}}
	m := NewManager(fake, cap.fn)

	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if cap.calls != 0 {
		t.Errorf("capturePane called %d times for tmux-targetless agent, want 0", cap.calls)
	}
	if !m.Snapshot()[0].LastPaneActivity.IsZero() {
		t.Errorf("LastPaneActivity = %v, want zero (no tmux_target → skip)", m.Snapshot()[0].LastPaneActivity)
	}
	if _, hadCache := m.paneCache["a1"]; hadCache {
		t.Error("paneCache should not contain entries for tmux-targetless agents")
	}
}

// TestRefreshPaneActivity_CaptureErrorCarriesForward locks E2: capturePane
// error carries forward cached lastActivity, NOT zero. Three-step shape per
// Brian-pattern-3.5 — without step 2 producing a non-zero cached value, a
// step-3 zero would also pass and the carry-forward semantic would be
// vacuously asserted.
func TestRefreshPaneActivity_CaptureErrorCarriesForward(t *testing.T) {
	fake := &fakeSource{agents: []protocol.Agent{agentWithTmux("a1", "session:0.0")}}
	captureErr := errString("tmux capture-pane: target gone")
	cap := &scriptedCapture{
		outputs: []string{"abc", "xyz", ""},
		errs:    []error{nil, nil, captureErr},
	}
	m := NewManager(fake, cap.fn)

	// Step 1: seed.
	m.Refresh()
	// Step 2: hash change → non-zero promotion timestamp (precondition for
	// observable carry-forward in step 3).
	m.Refresh()
	step2 := m.Snapshot()[0].LastPaneActivity
	if step2.IsZero() {
		t.Fatal("setup: step 2 LastPaneActivity must be non-zero for step 3 to be observable")
	}
	// Step 3: capturePane error → carry forward step 2's value, NOT zero.
	m.Refresh()
	step3 := m.Snapshot()[0].LastPaneActivity
	if !step3.Equal(step2) {
		t.Errorf("step 3 LastPaneActivity = %v, want %v (carry-forward from step 2)", step3, step2)
	}
}

// errString is a minimal error implementation for capturePane fake errors.
type errString string

func (e errString) Error() string { return string(e) }

// TestParseUsagePctKnownFormat locks H-30 parser against the published Claude
// Code 2.1.119 indicator format `${D}% until auto-compact`. UsagePct inverts
// remaining-pct to high-is-bad: D=5 → UsagePct=95.
//
// Format source: `@anthropic-ai/claude-code/cli.js` template literal
// `${D}% until auto-compact[· ${MESSAGE}]` (published-source discovery, not
// hypothesis). The high-usage fixture mirrors a CC pane near auto-compact.
func TestParseUsagePctKnownFormat(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want int
	}{
		{"bare 5% remaining → 95% used", "  5% until auto-compact", 95},
		{"with message suffix", "20% until auto-compact · approaching cap", 80},
		{"zero remaining → 100 used", "0% until auto-compact", 100},
		{"max remaining → 0 used", "100% until auto-compact", 0},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := parseUsagePct(tc.in); got != tc.want {
				t.Errorf("parseUsagePct(%q) = %d, want %d", tc.in, got, tc.want)
			}
		})
	}

	// Fixture-driven regression — committed pane capture re-parses to the
	// same UsagePct. If CC changes the indicator format and the fixture is
	// refreshed, this asserts the parser stays in sync.
	data, err := os.ReadFile(filepath.Join("testdata", "cc_pane_high_usage.txt"))
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}
	if got := parseUsagePct(string(data)); got != 95 {
		t.Errorf("fixture parseUsagePct = %d, want 95 (5%% remaining)", got)
	}
}

// TestParseUsagePctUnknownFallback locks fail-soft: any input that doesn't
// match the indicator regex returns -1. Display silently omits the segment
// rather than rendering a misleading 0% / 100%. This is the FIRST-class path
// per design: format-mismatch must never break runtime.
func TestParseUsagePctUnknownFallback(t *testing.T) {
	cases := []struct {
		name string
		in   string
	}{
		{"empty string", ""},
		{"no indicator at all", "⏺ Bash(go test ./...)\n  ⎿ ok"},
		{"missing percent suffix", "5 until auto-compact"},
		{"malformed: word instead of digits", "five% until auto-compact"},
		{"truncated: missing 'auto-compact'", "5% until auto"},
		{"different feature mentioned", "Context: 50% complete"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := parseUsagePct(tc.in); got != -1 {
				t.Errorf("parseUsagePct(%q) = %d, want -1 (unknown)", tc.in, got)
			}
		})
	}

	// Fixture-driven — the no-indicator pane capture must parse to -1.
	data, err := os.ReadFile(filepath.Join("testdata", "cc_pane_no_indicator.txt"))
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}
	if got := parseUsagePct(string(data)); got != -1 {
		t.Errorf("no-indicator fixture parseUsagePct = %d, want -1", got)
	}
}

// TestRefreshPaneActivity_UsagePctPropagates locks the wiring path: a fresh
// capture containing the indicator surfaces UsagePct on the snapshot, AND a
// subsequent capture-error tick carries the prior value forward instead of
// regressing to -1. Three-step shape — prior-zero step would falsely pass a
// trivial implementation.
func TestRefreshPaneActivity_UsagePctPropagates(t *testing.T) {
	fake := &fakeSource{agents: []protocol.Agent{agentWithTmux("a1", "session:0.0")}}
	captureErr := errString("tmux gone")
	cap := &scriptedCapture{
		outputs: []string{"5% until auto-compact", "", ""},
		errs:    []error{nil, nil, captureErr},
	}
	m := NewManager(fake, cap.fn)

	// Tick 1: seed with high-usage indicator → UsagePct=95.
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].UsagePct; got != 95 {
		t.Errorf("tick 1 UsagePct = %d, want 95 (5%% remaining)", got)
	}
	// Tick 2: empty pane (no indicator) → UsagePct=-1 (parse-unknown).
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].UsagePct; got != -1 {
		t.Errorf("tick 2 UsagePct = %d, want -1 (no indicator)", got)
	}
	// Tick 3: capture errors → carry forward prior cached value (-1).
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].UsagePct; got != -1 {
		t.Errorf("tick 3 UsagePct = %d, want -1 (carry-forward)", got)
	}
}

// TestRefreshPaneActivity_H6_LauncherShapedMeta locks the H6 fix path:
// launcher-registered agents (internal/brian/brian.go, internal/rain/rain.go)
// carry tmux_target in Meta JSON with the format "bot-hq-<role>-<unix>".
// panestate must read this, drive the pane-tier observer, and produce
// continuous-promotion ActivityWorking under continuously-changing pane content.
//
// Regression preventer (consumer side). The producer-side companion test —
// hub_register handler preserving Meta on Claude STARTUP re-register — lives
// in internal/mcp/server_test.go. Rain-pattern-8 + Brian-pattern-3.12: audit
// both ends of the contract.
func TestRefreshPaneActivity_H6_LauncherShapedMeta(t *testing.T) {
	target := "bot-hq-brian-1777154445"
	fake := &fakeSource{agents: []protocol.Agent{agentWithTmux("brian", target)}}
	cap := &scriptedCapture{outputs: []string{"tick0", "tick1", "tick2", "tick3", "tick4"}}
	m := NewManager(fake, cap.fn)

	// Tick 1: seed. agentWithTmux uses fresh LastSeen, so heartbeat tier holds
	// Working regardless of pane state.
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}

	// Ticks 2-5: each hash differs → pane-tier stamps LastPaneActivity, and the
	// fresh heartbeat keeps the row at Working (continuous-promotion locked).
	for i := 2; i <= 5; i++ {
		if err := m.Refresh(); err != nil {
			t.Fatalf("tick %d: %v", i, err)
		}
		snap := m.Snapshot()
		if snap[0].LastPaneActivity.IsZero() {
			t.Fatalf("tick %d: LastPaneActivity = zero, want non-zero (continuous-promotion)", i)
		}
		if snap[0].Activity != ActivityWorking {
			t.Errorf("tick %d: Activity = %v, want ActivityWorking", i, snap[0].Activity)
		}
	}

	if cap.calls != 5 {
		t.Errorf("capturePane calls = %d, want 5 (no skip-no-target on launcher-shaped Meta)", cap.calls)
	}
}

// TestRefreshPaneActivity_H6_PaneRescuesStaleHeartbeat locks F-core-b case-ii
// for launcher-registered agents: heartbeat past OnlineWindow + pane scrolls →
// pane-tier rescues to ActivityWorking. This is the runtime failure shape that
// the H6 investigation revealed (saltegge bash runs #1-#3) — once tmux_target
// is populated through the launcher fix, the existing F-core-b OR-combination
// must produce the rescue.
func TestRefreshPaneActivity_H6_PaneRescuesStaleHeartbeat(t *testing.T) {
	target := "bot-hq-brian-1777154445"
	a := agentWithTmux("brian", target)
	a.LastSeen = time.Now().Add(-2 * time.Minute) // past HeartbeatOnlineWindow
	fake := &fakeSource{agents: []protocol.Agent{a}}
	cap := &scriptedCapture{outputs: []string{"tick0", "tick1"}}
	m := NewManager(fake, cap.fn)

	// Tick 1: seed. Heartbeat stale, pane has no prior frame to compare → row
	// must be ActivityStale (heartbeat-only path with stale recency).
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].Activity; got != ActivityStale {
		t.Errorf("tick 1 Activity = %v, want ActivityStale (heartbeat past OnlineWindow, pane seed-only)", got)
	}

	// Tick 2: hash differs → pane-tier stamps now, OR-combines with stale
	// heartbeat → row promotes to Working. This is the case-ii ratchet.
	if err := m.Refresh(); err != nil {
		t.Fatal(err)
	}
	if got := m.Snapshot()[0].Activity; got != ActivityWorking {
		t.Errorf("tick 2 Activity = %v, want ActivityWorking (pane-tier rescue from stale heartbeat)", got)
	}
}
