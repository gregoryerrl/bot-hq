package ui

import (
	"fmt"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// makeSnap builds a panestate.AgentSnapshot for tests.
func makeSnap(id string, t protocol.AgentType, a panestate.AgentActivity) panestate.AgentSnapshot {
	return panestate.AgentSnapshot{
		ID:       id,
		Name:     id,
		Type:     t,
		Activity: a,
	}
}

// makeStaleGenSnap is makeSnap + StaleGen=true. Phase G v1 #20 helper.
func makeStaleGenSnap(id string, t protocol.AgentType, a panestate.AgentActivity) panestate.AgentSnapshot {
	s := makeSnap(id, t, a)
	s.StaleGen = true
	return s
}

func TestRenderStripShowsAllExceptOffline(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("a-working", protocol.AgentBrian, panestate.ActivityWorking),
		makeSnap("b-online", protocol.AgentQA, panestate.ActivityOnline),
		makeSnap("c-stale", protocol.AgentCoder, panestate.ActivityStale),
		makeSnap("d-offline", protocol.AgentCoder, panestate.ActivityOffline),
	}
	out := renderStrip(snap, 0)
	if !strings.Contains(out, "a-working") {
		t.Errorf("strip should contain a-working, got: %q", out)
	}
	if !strings.Contains(out, "b-online") {
		t.Errorf("strip should contain b-online, got: %q", out)
	}
	if !strings.Contains(out, "c-stale") {
		t.Errorf("strip should contain c-stale (visible after filter relax), got: %q", out)
	}
	if strings.Contains(out, "d-offline") {
		t.Errorf("strip should hide d-offline, got: %q", out)
	}
}

func TestRenderStripOrdersByTier(t *testing.T) {
	// Mixed input order; expected: tier 1 (Brian/QA/Voice) → tier 2 (Discord/Gemma) → tier 3 (Coder).
	snap := []panestate.AgentSnapshot{
		makeSnap("worker", protocol.AgentCoder, panestate.ActivityWorking),
		makeSnap("svc", protocol.AgentDiscord, panestate.ActivityWorking),
		makeSnap("peer", protocol.AgentBrian, panestate.ActivityWorking),
	}
	out := renderStrip(snap, 0)
	peerPos := strings.Index(out, "peer")
	svcPos := strings.Index(out, "svc")
	workerPos := strings.Index(out, "worker")
	if peerPos < 0 || svcPos < 0 || workerPos < 0 {
		t.Fatalf("missing IDs in output: %q (peer=%d svc=%d worker=%d)", out, peerPos, svcPos, workerPos)
	}
	if !(peerPos < svcPos && svcPos < workerPos) {
		t.Errorf("tier order broken: peer=%d svc=%d worker=%d in %q", peerPos, svcPos, workerPos, out)
	}
}

func TestRenderStripOrdersByNameWithinTier(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		{ID: "z-brian", Name: "z-brian", Type: protocol.AgentBrian, Activity: panestate.ActivityWorking},
		{ID: "a-rain", Name: "a-rain", Type: protocol.AgentQA, Activity: panestate.ActivityWorking},
	}
	out := renderStrip(snap, 0)
	rainPos := strings.Index(out, "a-rain")
	brianPos := strings.Index(out, "z-brian")
	if rainPos < 0 || brianPos < 0 {
		t.Fatalf("missing IDs: %q", out)
	}
	if !(rainPos < brianPos) {
		t.Errorf("expected name-sort within tier1: a-rain before z-brian, got %q", out)
	}
}

func TestRenderStripCapsAt8(t *testing.T) {
	snap := make([]panestate.AgentSnapshot, 0, 12)
	for i := 0; i < 12; i++ {
		snap = append(snap, makeSnap(fmt.Sprintf("agent%02d", i), protocol.AgentCoder, panestate.ActivityWorking))
	}
	out := renderStrip(snap, 0)
	// Exactly 8 distinct IDs visible (agent00 .. agent07).
	for i := 0; i < 8; i++ {
		want := fmt.Sprintf("agent%02d", i)
		if !strings.Contains(out, want) {
			t.Errorf("missing %s in capped output: %q", want, out)
		}
	}
	// Surplus 4 should NOT be visible by ID.
	for i := 8; i < 12; i++ {
		want := fmt.Sprintf("agent%02d", i)
		if strings.Contains(out, want) {
			t.Errorf("surplus %s should be collapsed, got: %q", want, out)
		}
	}
	// "+4" suffix present.
	if !strings.Contains(out, "+4") {
		t.Errorf("expected '+4' suffix for 4 surplus agents, got: %q", out)
	}
}

func TestRenderStripEmpty(t *testing.T) {
	out := renderStrip(nil, 0)
	if strings.Contains(out, "●") || strings.Contains(out, "◐") {
		t.Errorf("empty input should produce no dot chars, got: %q", out)
	}
	if strings.Contains(out, "+") {
		t.Errorf("empty input should not produce '+N' suffix, got: %q", out)
	}
}

func TestRenderStripStaleVisibleOfflineHidden(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("stale-agent", protocol.AgentBrian, panestate.ActivityStale),
		makeSnap("offline-agent", protocol.AgentCoder, panestate.ActivityOffline),
	}
	out := renderStrip(snap, 0)
	if !strings.Contains(out, "stale-agent") {
		t.Errorf("stale agent should be visible (filter relaxed), got: %q", out)
	}
	if strings.Contains(out, "offline-agent") {
		t.Errorf("offline agent should remain hidden, got: %q", out)
	}
	// Stale glyph (○) should appear; Offline glyph (·) should not.
	if !strings.Contains(out, "○") {
		t.Errorf("expected Stale glyph ○ in output, got: %q", out)
	}
	if strings.Contains(out, "·") {
		t.Errorf("Offline glyph · should not appear in output, got: %q", out)
	}
}

func TestRenderStripAllOfflineEmpty(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("a", protocol.AgentBrian, panestate.ActivityOffline),
		makeSnap("b", protocol.AgentCoder, panestate.ActivityOffline),
	}
	out := renderStrip(snap, 0)
	if strings.Contains(out, "a") || strings.Contains(out, "b") {
		t.Errorf("all-offline input should produce no IDs, got: %q", out)
	}
}

func TestRenderStripFourTierVisibility(t *testing.T) {
	// Spec lock: Working, Online, Stale visible; Offline hidden.
	snap := []panestate.AgentSnapshot{
		makeSnap("w", protocol.AgentBrian, panestate.ActivityWorking),
		makeSnap("o", protocol.AgentQA, panestate.ActivityOnline),
		makeSnap("s", protocol.AgentVoice, panestate.ActivityStale),
		makeSnap("x", protocol.AgentCoder, panestate.ActivityOffline),
	}
	out := renderStrip(snap, 0)
	for _, want := range []string{"w", "o", "s"} {
		if !strings.Contains(out, want) {
			t.Errorf("expected %q visible, got: %q", want, out)
		}
	}
	// Glyph count is format-stable: an offline-'x' leak forces count=4 regardless
	// of separator/glyph-mapping changes. Substring "x" check would false-pass if
	// 'x' became a substring of an unrelated render token.
	glyphCount := strings.Count(out, "●") + strings.Count(out, "◐") + strings.Count(out, "○")
	if glyphCount != 3 {
		t.Errorf("expected 3 glyphs (one per visible agent), got %d in: %q", glyphCount, out)
	}
	// All three visible glyphs should be present.
	for _, glyph := range []string{"●", "◐", "○"} {
		if !strings.Contains(out, glyph) {
			t.Errorf("expected glyph %q in output, got: %q", glyph, out)
		}
	}
}

func TestAgentTypeTier(t *testing.T) {
	cases := []struct {
		t    protocol.AgentType
		want int
	}{
		{protocol.AgentBrian, 1},
		{protocol.AgentQA, 1},
		{protocol.AgentVoice, 1},
		{protocol.AgentDiscord, 2},
		{protocol.AgentGemma, 2},
		{protocol.AgentCoder, 3},
		{protocol.AgentType("unknown"), 4},
	}
	for _, tc := range cases {
		if got := agentTypeTier(tc.t); got != tc.want {
			t.Errorf("agentTypeTier(%q) = %d, want %d", tc.t, got, tc.want)
		}
	}
}

// TestRenderStripOmitsStaleGen locks Phase G v1 #20 strip behavior:
// stale-gen agents (registered pre-rebuild) are omitted from the strip
// even though their activity is non-Offline. They remain visible in the
// agents tab with a (stale-gen) suffix for manual pruning.
func TestRenderStripOmitsStaleGen(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("current-gen", protocol.AgentBrian, panestate.ActivityWorking),
		makeStaleGenSnap("prior-gen", protocol.AgentQA, panestate.ActivityOnline),
	}
	out := renderStrip(snap, 0)
	if !strings.Contains(out, "current-gen") {
		t.Errorf("strip should contain current-gen agent, got: %q", out)
	}
	if strings.Contains(out, "prior-gen") {
		t.Errorf("strip should omit stale-gen agent, got: %q", out)
	}
}

// makeUsageSnap is makeSnap + UsagePct setter for H-30 strip tests.
func makeUsageSnap(id string, t protocol.AgentType, a panestate.AgentActivity, pct int) panestate.AgentSnapshot {
	s := makeSnap(id, t, a)
	s.UsagePct = pct
	return s
}

// TestStripRendersUsageSegmentWhenDataPresent locks H-30: when at least one
// visible agent has UsagePct ≥ usageSegmentMinPct (80), the strip appends a
// right-aligned `<id> NN%` segment. The agent with the *maximum* UsagePct is
// the one named — this is the worst-case squeeze, the one users need to see
// first.
func TestStripRendersUsageSegmentWhenDataPresent(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeUsageSnap("low", protocol.AgentBrian, panestate.ActivityWorking, 30),
		makeUsageSnap("hot", protocol.AgentCoder, panestate.ActivityWorking, 87),
	}
	out := renderStrip(snap, 80)
	if !strings.Contains(out, "hot") {
		t.Errorf("expected hottest agent id 'hot' in usage segment, got: %q", out)
	}
	if !strings.Contains(out, "87%") {
		t.Errorf("expected '87%%' in usage segment, got: %q", out)
	}
	// Right-aligned: the segment must come after the dot row, separated by
	// at least usageSegmentMinSpacer spaces. Locating "hot 87%" past the
	// dot-row body proves the spacer was inserted.
	idx := strings.Index(out, "hot 87%")
	if idx < 0 {
		t.Fatalf("expected literal 'hot 87%%' substring (segment), got: %q", out)
	}
	// The dot row contains "low" (the non-hot agent) — verify the segment
	// trails it. lipgloss color escapes around 'low' are fine; we only need
	// 'low' to appear before 'hot 87%'.
	lowIdx := strings.Index(out, "low")
	if lowIdx < 0 || lowIdx >= idx {
		t.Errorf("expected dot-row 'low' to precede segment 'hot 87%%', got: %q", out)
	}
}

// TestStripOmitsUsageSegmentWhenAllUnknown locks H-30 fail-soft: when every
// visible agent has UsagePct=-1 (no pane / parse-unknown) OR when all known
// values are below the threshold, the segment is omitted entirely. No false
// 0% display, no clutter at low usage. Two-stage: (1) all unknown, (2) all
// known but sub-threshold.
func TestStripOmitsUsageSegmentWhenAllUnknown(t *testing.T) {
	// Stage 1: all UsagePct = -1.
	allUnknown := []panestate.AgentSnapshot{
		makeUsageSnap("emma", protocol.AgentBrian, panestate.ActivityWorking, -1),
		makeUsageSnap("disc", protocol.AgentDiscord, panestate.ActivityOnline, -1),
	}
	out := renderStrip(allUnknown, 80)
	if strings.Contains(out, "%") {
		t.Errorf("all-unknown input should produce no '%%' in output, got: %q", out)
	}
	// Stage 2: all known but sub-threshold.
	subThreshold := []panestate.AgentSnapshot{
		makeUsageSnap("a", protocol.AgentBrian, panestate.ActivityWorking, 30),
		makeUsageSnap("b", protocol.AgentCoder, panestate.ActivityWorking, 79),
	}
	out2 := renderStrip(subThreshold, 80)
	if strings.Contains(out2, "%") {
		t.Errorf("sub-threshold input should produce no '%%' in output, got: %q", out2)
	}
}
