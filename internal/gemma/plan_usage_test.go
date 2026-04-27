package gemma

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/anthropic"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// fakePlanUsageFetch records calls and returns scripted (maxUtil, window,
// err) per call. perWindow is unused by Emma's plan_usage logic so the
// fake leaves it nil.
type fakePlanUsageFetch struct {
	calls       int
	maxUtil     []float64
	window      []string
	err         []error
	perWindow   map[string]anthropic.Window
}

func (f *fakePlanUsageFetch) Fetch(_ context.Context) (float64, string, map[string]anthropic.Window, error) {
	idx := f.calls
	f.calls++
	var u float64
	if idx < len(f.maxUtil) {
		u = f.maxUtil[idx]
	}
	var w string
	if idx < len(f.window) {
		w = f.window[idx]
	}
	var e error
	if idx < len(f.err) {
		e = f.err[idx]
	}
	return u, w, f.perWindow, e
}

// recorder captures the most recent HubSnapshot publish.
type hubRecorder struct {
	calls    int
	last     panestate.HubSnapshot
}

func (r *hubRecorder) publish(s panestate.HubSnapshot) {
	r.calls++
	r.last = s
}

func countPlanCapFlags(t *testing.T, db *hub.DB) int {
	t.Helper()
	msgs, err := db.GetRecentMessages(200)
	if err != nil {
		t.Fatal(err)
	}
	n := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && m.Type == protocol.MsgFlag &&
			strings.Contains(m.Content, "[CRITICAL]") &&
			strings.Contains(m.Content, "plan usage at") &&
			strings.Contains(m.Content, "halt + checkpoint via H-15") {
			n++
		}
	}
	return n
}

// TestPlanCapFiresAt95 — first poll at maxUtil=0.96 fires hub_flag,
// upserts plan-cap halt_state, and publishes HubSnapshot{96,five_hour}.
func TestPlanCapFiresAt95(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.96},
		window:  []string{anthropic.WindowFiveHour},
	})

	g.checkPlanUsage(time.Now())

	if got := countPlanCapFlags(t, db); got != 1 {
		t.Fatalf("expected 1 plan-cap flag at 96%%, got %d", got)
	}
	row, ok, err := db.GetHaltCause(hub.HaltCausePlanCap)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("plan-cap halt_state row must be active after fire")
	}
	if !strings.Contains(row.Reason, "plan usage at 96%, halt") {
		t.Errorf("halt reason missing canonical substring; got %q", row.Reason)
	}
	if rec.calls != 1 {
		t.Errorf("hubPublisher must fire exactly once on success; got %d", rec.calls)
	}
	if rec.last.PlanUsagePct != 96 {
		t.Errorf("PlanUsagePct = %d, want 96", rec.last.PlanUsagePct)
	}
	if rec.last.PlanWindow != anthropic.WindowFiveHour {
		t.Errorf("PlanWindow = %q, want %q", rec.last.PlanWindow, anthropic.WindowFiveHour)
	}
}

// TestPlanCapPublishesDualWindowPcts locks the slice-5 hotfix dual-window
// publish: each successful poll surfaces both five_hour + seven_day pcts
// independently of which is max. Missing windows publish -1.
func TestPlanCapPublishesDualWindowPcts(t *testing.T) {
	g, _ := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.91},
		window:  []string{anthropic.WindowFiveHour},
		perWindow: map[string]anthropic.Window{
			anthropic.WindowFiveHour: {Utilization: 0.91},
			anthropic.WindowSevenDay: {Utilization: 0.15},
		},
	})

	g.checkPlanUsage(time.Now())

	if rec.calls != 1 {
		t.Fatalf("expected 1 publish, got %d", rec.calls)
	}
	if rec.last.FiveHourPct != 91 {
		t.Errorf("FiveHourPct = %d, want 91", rec.last.FiveHourPct)
	}
	if rec.last.SevenDayPct != 15 {
		t.Errorf("SevenDayPct = %d, want 15", rec.last.SevenDayPct)
	}
	if rec.last.PlanUsagePct != 91 {
		t.Errorf("PlanUsagePct (max) = %d, want 91", rec.last.PlanUsagePct)
	}
}

// TestPlanCapMissingSevenDayPublishesNeg1 — when the API response omits
// the seven_day window (lower-tier account), the publish carries
// SevenDayPct=-1 so the strip can render `7d:--%` distinguishably from
// `7d:0%`.
func TestPlanCapMissingSevenDayPublishesNeg1(t *testing.T) {
	g, _ := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.50},
		window:  []string{anthropic.WindowFiveHour},
		perWindow: map[string]anthropic.Window{
			anthropic.WindowFiveHour: {Utilization: 0.50},
		},
	})

	g.checkPlanUsage(time.Now())

	if rec.last.FiveHourPct != 50 {
		t.Errorf("FiveHourPct = %d, want 50", rec.last.FiveHourPct)
	}
	if rec.last.SevenDayPct != -1 {
		t.Errorf("SevenDayPct = %d, want -1 (missing)", rec.last.SevenDayPct)
	}
}

// TestPlanCapResetClearsHaltAndRearmsHysteresis — fire at 96%, then a
// later poll at 70% deletes the plan-cap halt row AND re-arms the
// shouldFlag hysteresis so a fresh squeeze past 95% fires anew.
func TestPlanCapResetClearsHaltAndRearmsHysteresis(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	fake := &fakePlanUsageFetch{
		maxUtil: []float64{0.96, 0.70, 0.96},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	}
	g.SetPlanUsageFetcher(fake)

	now := time.Now()
	g.checkPlanUsage(now)
	if _, ok, _ := db.GetHaltCause(hub.HaltCausePlanCap); !ok {
		t.Fatal("setup: plan-cap halt must be active after first 96%% fire")
	}

	// Tick 2: 70% — below reset threshold; plan-cap row deleted +
	// hysteresis re-armed.
	g.checkPlanUsage(now.Add(planUsageBaseInterval + time.Second))
	if _, ok, _ := db.GetHaltCause(hub.HaltCausePlanCap); ok {
		t.Errorf("plan-cap halt must clear at maxUtil=0.70")
	}

	// Tick 3: 96% again — fresh fire.
	g.checkPlanUsage(now.Add(2*(planUsageBaseInterval + time.Second)))
	if got := countPlanCapFlags(t, db); got != 2 {
		t.Errorf("expected 2 flags across reset+rearm cycle, got %d", got)
	}
}

// TestPlanCapBetweenThresholdsHoldsState — utilization in [0.85, 0.95)
// neither fires nor clears. The HubSnapshot is updated but halt_state
// remains in whichever state the prior tick left it.
func TestPlanCapBetweenThresholdsHoldsState(t *testing.T) {
	g, db := newContextCapGemma(t)
	g.SetHubPublisher(func(panestate.HubSnapshot) {})
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.90},
		window:  []string{anthropic.WindowSevenDay},
	})

	g.checkPlanUsage(time.Now())

	if got := countPlanCapFlags(t, db); got != 0 {
		t.Errorf("90%% must not fire (between 85 and 95); got %d flags", got)
	}
	if _, ok, _ := db.GetHaltCause(hub.HaltCausePlanCap); ok {
		t.Errorf("plan-cap halt must remain inactive in the 0.85-0.95 squeeze band")
	}
}

// TestPlanCapNearExpirySkipsSilently — fetcher returns ErrTokenExpired;
// no halt set, no flag fired, no HubSnapshot publish (prior value
// preserved by leaving publisher untouched). Backoff stays unset so the
// next 60s tick retries.
func TestPlanCapNearExpirySkipsSilently(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		err: []error{anthropic.ErrTokenExpired},
	})

	g.checkPlanUsage(time.Now())

	if got := countPlanCapFlags(t, db); got != 0 {
		t.Errorf("near-expiry must not fire; got %d flags", got)
	}
	if rec.calls != 0 {
		t.Errorf("near-expiry must not publish a HubSnapshot; got %d calls", rec.calls)
	}
	if !g.planBackoffUntil.IsZero() {
		t.Errorf("near-expiry must not enter backoff; want zero, got %v", g.planBackoffUntil)
	}
}

// TestPlanCapAuthFailPublishesUnknownSnapshot — H-40: on auth-fail / 5xx
// the producer enters 600s backoff but ALSO publishes
// HubSnapshot{-1, five_hour} so the strip surfaces `--%` instead of
// staying blank. Without this, "fresh boot" and "producer errored" render
// identically and the user can't tell the producer is alive but failing.
func TestPlanCapAuthFailPublishesUnknownSnapshot(t *testing.T) {
	g, _ := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		err: []error{errors.New("auth failed: status 401")},
	})

	g.checkPlanUsage(time.Now())

	if rec.calls != 1 {
		t.Fatalf("auth-fail must publish exactly once; got %d", rec.calls)
	}
	if rec.last.PlanUsagePct != -1 {
		t.Errorf("PlanUsagePct = %d, want -1 (unknown)", rec.last.PlanUsagePct)
	}
	if rec.last.PlanWindow != anthropic.WindowFiveHour {
		t.Errorf("PlanWindow = %q, want %q (default tag for --%% render)", rec.last.PlanWindow, anthropic.WindowFiveHour)
	}
	if rec.last.FiveHourPct != -1 || rec.last.SevenDayPct != -1 {
		t.Errorf("auth-fail must publish dual-window -1 sentinels; got 5h=%d 7d=%d", rec.last.FiveHourPct, rec.last.SevenDayPct)
	}
}

// TestPlanCapBackoffCadenceOn5xx — fetcher returns a generic 5xx-shaped
// error; checkPlanUsage records lastPlanPoll AND sets planBackoffUntil
// to now+600s. Subsequent calls inside that window short-circuit without
// hitting the fetcher again.
func TestPlanCapBackoffCadenceOn5xx(t *testing.T) {
	g, _ := newContextCapGemma(t)
	g.SetHubPublisher(func(panestate.HubSnapshot) {})
	fake := &fakePlanUsageFetch{
		err: []error{errors.New("server error: status 502"), errors.New("never reached")},
	}
	g.SetPlanUsageFetcher(fake)

	now := time.Now()
	g.checkPlanUsage(now)
	if fake.calls != 1 {
		t.Fatalf("setup: first tick must fetch once; got %d", fake.calls)
	}
	if g.planBackoffUntil.IsZero() {
		t.Errorf("5xx must enter backoff; planBackoffUntil = zero")
	}
	if got, want := g.planBackoffUntil.Sub(now), planUsageBackoffInterval; got != want {
		t.Errorf("backoff window = %v, want %v", got, want)
	}

	// Second tick well inside the backoff window — must not fetch.
	g.checkPlanUsage(now.Add(2 * time.Minute))
	if fake.calls != 1 {
		t.Errorf("inside backoff window: fetch called %d times, want 1", fake.calls)
	}

	// Third tick past the backoff window — fetches again (and would error
	// again per the script, but the call count is what we lock).
	g.checkPlanUsage(now.Add(planUsageBackoffInterval + time.Second))
	if fake.calls != 2 {
		t.Errorf("past backoff window: fetch called %d times, want 2", fake.calls)
	}
}

// TestPlanCapTickGate — checkPlanUsage called twice within the 60s base
// cadence only fetches once. Locks the cadence gate against accidental
// re-issue when the surrounding sentinelPollLoop ticks at 5s.
func TestPlanCapTickGate(t *testing.T) {
	g, _ := newContextCapGemma(t)
	g.SetHubPublisher(func(panestate.HubSnapshot) {})
	fake := &fakePlanUsageFetch{
		maxUtil: []float64{0.50, 0.50},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	}
	g.SetPlanUsageFetcher(fake)

	now := time.Now()
	g.checkPlanUsage(now)
	g.checkPlanUsage(now.Add(30 * time.Second))
	g.checkPlanUsage(now.Add(45 * time.Second))
	if fake.calls != 1 {
		t.Errorf("inside 60s gate: fetch called %d times, want 1", fake.calls)
	}
	g.checkPlanUsage(now.Add(planUsageBaseInterval + time.Second))
	if fake.calls != 2 {
		t.Errorf("after 60s gate: fetch called %d times, want 2", fake.calls)
	}
}

// TestPlanCapSurvivesContextCapHalt — set context-cap halt manually,
// then run a successful plan-usage tick at 70% (below reset threshold).
// The plan-cap clear must not touch the context-cap row; halts coexist
// independently.
func TestPlanCapDoesNotClearContextCap(t *testing.T) {
	g, db := newContextCapGemma(t)
	g.SetHubPublisher(func(panestate.HubSnapshot) {})
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.70},
		window:  []string{anthropic.WindowFiveHour},
	})

	if err := db.SetHaltActive(hub.HaltCauseContextCap, "context test", "emma"); err != nil {
		t.Fatal(err)
	}

	g.checkPlanUsage(time.Now())

	if _, ok, _ := db.GetHaltCause(hub.HaltCauseContextCap); !ok {
		t.Errorf("context-cap halt must survive plan-cap clear; was deleted")
	}
}

// TestPlanCapWindowTagSurfaces — fire at 96%% on a non-five_hour window
// and verify the reason text carries the matching tag.
func TestPlanCapWindowTagSurfaces(t *testing.T) {
	g, db := newContextCapGemma(t)
	g.SetHubPublisher(func(panestate.HubSnapshot) {})
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.96},
		window:  []string{anthropic.WindowSevenDayOpus},
	})

	g.checkPlanUsage(time.Now())

	row, ok, err := db.GetHaltCause(hub.HaltCausePlanCap)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("plan-cap halt must be active")
	}
	if !strings.Contains(row.Reason, "(opus)") {
		t.Errorf("seven_day_opus window must tag reason text with (opus); got %q", row.Reason)
	}
}

// TestPlanCapNilFetcherIsNoop — when no fetcher is wired (e.g.
// initPlanUsageDefault skipped on non-darwin), checkPlanUsage is a
// no-op. Locks the safe-default contract.
func TestPlanCapNilFetcherIsNoop(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)

	g.checkPlanUsage(time.Now())

	if got := countPlanCapFlags(t, db); got != 0 {
		t.Errorf("nil fetcher must be a no-op; got %d flags", got)
	}
	if rec.calls != 0 {
		t.Errorf("nil fetcher must not publish; got %d calls", rec.calls)
	}
}
