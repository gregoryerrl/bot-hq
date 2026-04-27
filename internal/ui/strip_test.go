package ui

import (
	"fmt"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/anthropic"
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

// emptyHub is the zero-snapshot HubSnapshot used by tests that don't
// exercise the right-aligned plan segment. PlanUsagePct=-1 with empty
// PlanWindow means "omit segment". FiveHourPct/SevenDayPct=-1 mirror
// the producer's fresh-boot publish for the dual-window strip.
var emptyHub = panestate.HubSnapshot{PlanUsagePct: -1, FiveHourPct: -1, SevenDayPct: -1}

func TestRenderStripShowsAllExceptOffline(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("a-working", protocol.AgentBrian, panestate.ActivityWorking),
		makeSnap("b-online", protocol.AgentQA, panestate.ActivityOnline),
		makeSnap("c-stale", protocol.AgentCoder, panestate.ActivityStale),
		makeSnap("d-offline", protocol.AgentCoder, panestate.ActivityOffline),
	}
	out := renderStrip(snap, emptyHub, 0)
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
	snap := []panestate.AgentSnapshot{
		makeSnap("worker", protocol.AgentCoder, panestate.ActivityWorking),
		makeSnap("svc", protocol.AgentDiscord, panestate.ActivityWorking),
		makeSnap("peer", protocol.AgentBrian, panestate.ActivityWorking),
	}
	out := renderStrip(snap, emptyHub, 0)
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
	out := renderStrip(snap, emptyHub, 0)
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
	out := renderStrip(snap, emptyHub, 0)
	for i := 0; i < 8; i++ {
		want := fmt.Sprintf("agent%02d", i)
		if !strings.Contains(out, want) {
			t.Errorf("missing %s in capped output: %q", want, out)
		}
	}
	for i := 8; i < 12; i++ {
		want := fmt.Sprintf("agent%02d", i)
		if strings.Contains(out, want) {
			t.Errorf("surplus %s should be collapsed, got: %q", want, out)
		}
	}
	if !strings.Contains(out, "+4") {
		t.Errorf("expected '+4' suffix for 4 surplus agents, got: %q", out)
	}
}

func TestRenderStripEmpty(t *testing.T) {
	out := renderStrip(nil, emptyHub, 0)
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
	out := renderStrip(snap, emptyHub, 0)
	if !strings.Contains(out, "stale-agent") {
		t.Errorf("stale agent should be visible (filter relaxed), got: %q", out)
	}
	if strings.Contains(out, "offline-agent") {
		t.Errorf("offline agent should remain hidden, got: %q", out)
	}
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
	out := renderStrip(snap, emptyHub, 0)
	if strings.Contains(out, "a") || strings.Contains(out, "b") {
		t.Errorf("all-offline input should produce no IDs, got: %q", out)
	}
}

func TestRenderStripFourTierVisibility(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("w", protocol.AgentBrian, panestate.ActivityWorking),
		makeSnap("o", protocol.AgentQA, panestate.ActivityOnline),
		makeSnap("s", protocol.AgentVoice, panestate.ActivityStale),
		makeSnap("x", protocol.AgentCoder, panestate.ActivityOffline),
	}
	out := renderStrip(snap, emptyHub, 0)
	for _, want := range []string{"w", "o", "s"} {
		if !strings.Contains(out, want) {
			t.Errorf("expected %q visible, got: %q", want, out)
		}
	}
	glyphCount := strings.Count(out, "●") + strings.Count(out, "◐") + strings.Count(out, "○")
	if glyphCount != 3 {
		t.Errorf("expected 3 glyphs (one per visible agent), got %d in: %q", glyphCount, out)
	}
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

// TestRenderStripOmitsStaleGen locks Phase G v1 #20 strip behavior.
func TestRenderStripOmitsStaleGen(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("current-gen", protocol.AgentBrian, panestate.ActivityWorking),
		makeStaleGenSnap("prior-gen", protocol.AgentQA, panestate.ActivityOnline),
	}
	out := renderStrip(snap, emptyHub, 0)
	if !strings.Contains(out, "current-gen") {
		t.Errorf("strip should contain current-gen agent, got: %q", out)
	}
	if strings.Contains(out, "prior-gen") {
		t.Errorf("strip should omit stale-gen agent, got: %q", out)
	}
}

// TestStripRendersDualWindowSegment locks the slice-5 hotfix dual-window
// shape: both 5h and 7d render side-by-side regardless of which is max.
// Color tier follows PlanUsagePct (max-of-both) so the worst-case window
// drives urgency signaling.
func TestStripRendersDualWindowSegment(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("brian", protocol.AgentBrian, panestate.ActivityWorking),
	}
	hub := panestate.HubSnapshot{
		PlanUsagePct: 91,
		PlanWindow:   anthropic.WindowFiveHour,
		FiveHourPct:  91,
		SevenDayPct:  15,
	}
	out := renderStrip(snap, hub, 80)
	if !strings.Contains(out, "5h:91%") {
		t.Errorf("expected '5h:91%%' in output, got: %q", out)
	}
	if !strings.Contains(out, "7d:15%") {
		t.Errorf("expected '7d:15%%' in output, got: %q", out)
	}
}

// TestStripRendersDualWindowMissingSeven locks the missing-window path:
// when the API didn't include seven_day (lower-tier accounts), the strip
// renders `5h:NN% 7d:--%` so the user knows the second window is
// unobserved rather than truly 0%.
func TestStripRendersDualWindowMissingSeven(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("brian", protocol.AgentBrian, panestate.ActivityWorking),
	}
	hub := panestate.HubSnapshot{
		PlanUsagePct: 50,
		PlanWindow:   anthropic.WindowFiveHour,
		FiveHourPct:  50,
		SevenDayPct:  -1,
	}
	out := renderStrip(snap, hub, 80)
	if !strings.Contains(out, "5h:50%") {
		t.Errorf("expected '5h:50%%' in output, got: %q", out)
	}
	if !strings.Contains(out, "7d:--%") {
		t.Errorf("expected '7d:--%%' for missing window, got: %q", out)
	}
}

// TestStripRendersDualWindowMissingFive locks the symmetric case where
// only seven_day is present — keeps the format stable regardless of
// which window is the unobserved one.
func TestStripRendersDualWindowMissingFive(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("brian", protocol.AgentBrian, panestate.ActivityWorking),
	}
	hub := panestate.HubSnapshot{
		PlanUsagePct: 30,
		PlanWindow:   anthropic.WindowSevenDay,
		FiveHourPct:  -1,
		SevenDayPct:  30,
	}
	out := renderStrip(snap, hub, 80)
	if !strings.Contains(out, "5h:--%") {
		t.Errorf("expected '5h:--%%' for missing window, got: %q", out)
	}
	if !strings.Contains(out, "7d:30%") {
		t.Errorf("expected '7d:30%%' in output, got: %q", out)
	}
}

// TestStripOmitsPlanSegmentWhenUnknownAndNoWindow locks the fresh-boot
// behavior: PlanUsagePct=-1 + empty PlanWindow → segment omitted entirely.
func TestStripOmitsPlanSegmentWhenUnknownAndNoWindow(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("brian", protocol.AgentBrian, panestate.ActivityWorking),
	}
	out := renderStrip(snap, panestate.HubSnapshot{PlanUsagePct: -1}, 80)
	if strings.Contains(out, "%") {
		t.Errorf("unknown-without-window must omit segment; got: %q", out)
	}
	if strings.Contains(out, "--") {
		t.Errorf("unknown-without-window must not render --%%; got: %q", out)
	}
}

// TestStripRendersDashesWhenUnknownButWindowKnown locks the
// transient-error path: producer published PlanUsagePct=-1 alongside a
// last-known window.
func TestStripRendersDashesWhenUnknownButWindowKnown(t *testing.T) {
	snap := []panestate.AgentSnapshot{
		makeSnap("brian", protocol.AgentBrian, panestate.ActivityWorking),
	}
	hub := panestate.HubSnapshot{PlanUsagePct: -1, PlanWindow: anthropic.WindowFiveHour}
	out := renderStrip(snap, hub, 80)
	if !strings.Contains(out, "--%") {
		t.Errorf("unknown-with-known-window must render --%%; got: %q", out)
	}
}

// TestStripPlanSegmentColorTiers locks the green/yellow/red boundaries.
// Inspects the returned style directly (lipgloss render output depends on
// terminal capabilities, which differ between CI and local runs); the
// style's Bold flag is the stable contract.
func TestStripPlanSegmentColorTiers(t *testing.T) {
	cases := []struct {
		pct  int
		bold bool // red/yellow tiers render bold; green does not
	}{
		{50, false}, // green
		{69, false}, // green (just below yellow)
		{70, true},  // yellow boundary
		{89, true},  // yellow
		{90, true},  // red boundary
		{100, true}, // red
	}
	for _, tc := range cases {
		t.Run(fmt.Sprintf("pct=%d", tc.pct), func(t *testing.T) {
			style := planUsageTierStyle(tc.pct)
			if got := style.GetBold(); got != tc.bold {
				t.Errorf("pct=%d bold=%v, want bold=%v", tc.pct, got, tc.bold)
			}
		})
	}
}
