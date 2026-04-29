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
			strings.Contains(m.Content, "halt + idle in pane") {
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

// TestPlanCapResumeCooldownSuppressesRapidReEmits — Phase J tail fix
// (per hub.db trace msgs 5194-5218 — ~30 RESUME emits between two
// legit halt cycles caused by maxUtil bouncing 95%↔0% across polls).
// Cooldown caps emitPlanCapResume to once per planCapResumeCooldown
// window even if hadHalt-gate fires on multiple polls.
func TestPlanCapResumeCooldownSuppressesRapidReEmits(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	// Sequence: fire→clear (cycle 1) → fire→clear (cycle 2) within
	// cooldown window. Cycle 2's RESUME must be suppressed.
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.96, 0.50, 0.96, 0.50},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	})

	now := time.Now()
	g.checkPlanUsage(now)                                                  // halt fires
	g.checkPlanUsage(now.Add(planUsageBaseInterval + time.Second))         // RESUME emit (cooldown stamps now)
	g.checkPlanUsage(now.Add(2 * (planUsageBaseInterval + time.Second)))   // halt re-fires
	g.checkPlanUsage(now.Add(3 * (planUsageBaseInterval + time.Second)))   // RESUME suppressed (within cooldown)

	msgs, err := db.GetRecentMessages(50)
	if err != nil {
		t.Fatal(err)
	}
	resumes := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && strings.Contains(m.Content, "plan usage reset") {
			resumes++
		}
	}
	// Expected: ONE RESUME per emitPlanCapResume call × 2 recipients (brian+rain) = 2 msgs.
	// If cooldown were broken, we'd see 4 (2 cycles × 2 recipients).
	if resumes != 2 {
		t.Errorf("expected 2 RESUME msgs (1 emit cycle × 2 recipients) post-cooldown; got %d (cooldown broken — re-emit not suppressed)", resumes)
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

// TestPlanCapFireSchedulesWakes locks Phase I W2 hotfix Fix-1: when the
// plan-cap halt fires at 95%+, Emma schedules wakes for brian + rain at
// now + planCapWakeOffset (5h+1min). Belt-and-suspenders backup against
// Anthropic API itself being rate-limited at rollover-time.
func TestPlanCapFireSchedulesWakes(t *testing.T) {
	g, db := newContextCapGemma(t)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.96},
		window:  []string{anthropic.WindowFiveHour},
	})

	now := time.Now()
	g.checkPlanUsage(now)

	wakes, err := db.ListPendingWakes(now.Add(planCapWakeOffset + time.Minute))
	if err != nil {
		t.Fatal(err)
	}
	targets := map[string]int{}
	for _, w := range wakes {
		targets[w.TargetAgent]++
	}
	if targets["brian"] != 1 {
		t.Errorf("expected 1 brian wake, got %d", targets["brian"])
	}
	if targets["rain"] != 1 {
		t.Errorf("expected 1 rain wake, got %d", targets["rain"])
	}
	for _, w := range wakes {
		if !strings.Contains(w.Payload, "plan usage reset") {
			t.Errorf("wake payload must contain resume substring 'plan usage reset'; got %q", w.Payload)
		}
		fireDelta := w.FireAt.Sub(now)
		if fireDelta < planCapWakeOffset-time.Second || fireDelta > planCapWakeOffset+time.Second {
			t.Errorf("wake fire-at offset = %s, want ~%s", fireDelta, planCapWakeOffset)
		}
	}
}

// TestPlanCapResetEmitsResumeNudge locks Phase I W2 hotfix Fix-2: when
// the plan-cap halt clears (maxUtil drops below 0.85 from a prior halt),
// Emma emits MsgCommand records to brian + rain with content containing
// "plan usage reset" so agents recognize the resume directive and
// re-bootstrap via R16.
func TestPlanCapResetEmitsResumeNudge(t *testing.T) {
	g, db := newContextCapGemma(t)
	fake := &fakePlanUsageFetch{
		maxUtil: []float64{0.96, 0.70},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	}
	g.SetPlanUsageFetcher(fake)

	now := time.Now()
	g.checkPlanUsage(now)
	// Advance past base interval so the next poll fires.
	g.checkPlanUsage(now.Add(planUsageBaseInterval + time.Second))

	msgs, err := db.GetRecentMessages(50)
	if err != nil {
		t.Fatal(err)
	}
	gotBrian, gotRain := 0, 0
	for _, m := range msgs {
		if m.FromAgent != agentID || m.Type != protocol.MsgCommand {
			continue
		}
		if !strings.Contains(m.Content, "plan usage reset") {
			continue
		}
		switch m.ToAgent {
		case "brian":
			gotBrian++
		case "rain":
			gotRain++
		}
	}
	if gotBrian != 1 {
		t.Errorf("expected 1 resume MsgCommand to brian, got %d", gotBrian)
	}
	if gotRain != 1 {
		t.Errorf("expected 1 resume MsgCommand to rain, got %d", gotRain)
	}
}

// TestPlanCapResetWithoutPriorHaltDoesNotEmitResumeNudge locks the
// idempotency guard: at sub-threshold usage with no prior halt active,
// no resume nudges fire. Otherwise every steady-state poll would re-emit.
func TestPlanCapResetWithoutPriorHaltDoesNotEmitResumeNudge(t *testing.T) {
	g, db := newContextCapGemma(t)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.50},
		window:  []string{anthropic.WindowFiveHour},
	})

	g.checkPlanUsage(time.Now())

	msgs, err := db.GetRecentMessages(50)
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range msgs {
		if m.FromAgent == agentID && m.Type == protocol.MsgCommand && strings.Contains(m.Content, "plan usage reset") {
			t.Errorf("steady-state sub-threshold poll must not emit resume nudge; got: %+v", m)
		}
	}
}

// TestPlanCapPayloadMirrorSymmetry asserts the runtime emit format strings
// (planCapResumeFmt + planCapReasonFmt) contain the shared-substring set
// declared by the protocol package's PayloadMirrorSubstrings(ruleID) helper.
// Phase J T1.1 (B1(iv)) — first-instance runtime-half of the Option B
// shared-substring-set discipline (B3d rule-namespace-ratchet payload-mirror
// attribute). Companion to TestRuleNamespaceRatchet (registry_test.go) which
// asserts the const-text half.
//
// On wording drift: if planCapResumeFmt or planCapReasonFmt diverge from the
// shared substring set, this test fires. Either (a) update the runtime fmt
// to restore the substrings, OR (b) update the substring set with explicit
// review — the registry-side test will then fire if the corresponding const
// drifts from the new set, surfacing both halves.
func TestPlanCapPayloadMirrorSymmetry(t *testing.T) {
	cases := []struct {
		ruleID  string
		runtime string
	}{
		{"R16", planCapResumeFmt},
		{"RESUME-FROM-HALT", planCapResumeFmt},
		{"H-31-HALT", planCapReasonFmt},
	}
	for _, c := range cases {
		t.Run(c.ruleID, func(t *testing.T) {
			subs := protocol.PayloadMirrorSubstrings(c.ruleID)
			if len(subs) == 0 {
				t.Errorf("rule %q: PayloadMirrorSubstrings empty — schema gap", c.ruleID)
				return
			}
			for _, s := range subs {
				if !strings.Contains(c.runtime, s) {
					t.Errorf("rule %q: runtime fmt missing payload-mirror substring %q (runtime=%q)", c.ruleID, s, c.runtime)
				}
			}
		})
	}
}

// TestPreCompactSnapEmitAtThreshold (Phase J T2.2-α B1a) — emits
// [PRE-COMPACT-SNAP] to brian + rain when maxUtil hits the proactive
// pre-snap threshold (0.90+) but is below halt (0.95).
func TestPreCompactSnapEmitAtThreshold(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.92},
		window:  []string{anthropic.WindowFiveHour},
	})

	g.checkPlanUsage(time.Now())

	msgs, err := db.GetRecentMessages(20)
	if err != nil {
		t.Fatal(err)
	}
	preSnaps := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && strings.Contains(m.Content, "[PRE-COMPACT-SNAP]") {
			preSnaps++
		}
	}
	// Expected: 1 emit cycle × 2 recipients = 2 msgs.
	if preSnaps != 2 {
		t.Errorf("expected 2 [PRE-COMPACT-SNAP] msgs (brian + rain), got %d", preSnaps)
	}

	// halt_state must NOT be active (we're in pre-snap band, below halt threshold).
	if _, ok, _ := db.GetHaltCause(hub.HaltCausePlanCap); ok {
		t.Errorf("plan-cap halt must NOT fire in pre-snap band 0.90-0.95")
	}
}

// TestPreCompactSnapCooldown (Phase J T2.2-α) — locks the cooldown gate.
// Two polls in succession, both at pre-snap threshold, must yield ONE
// emit cycle (2 msgs) — second poll suppressed by cooldown.
func TestPreCompactSnapCooldown(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.92, 0.91},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	})

	now := time.Now()
	g.checkPlanUsage(now)
	g.checkPlanUsage(now.Add(planUsageBaseInterval + time.Second))

	msgs, err := db.GetRecentMessages(20)
	if err != nil {
		t.Fatal(err)
	}
	preSnaps := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && strings.Contains(m.Content, "[PRE-COMPACT-SNAP]") {
			preSnaps++
		}
	}
	// Expected: 1 emit cycle × 2 recipients = 2 msgs (NOT 4 — cooldown caps).
	if preSnaps != 2 {
		t.Errorf("expected 2 [PRE-COMPACT-SNAP] msgs (cooldown caps to 1 emit cycle); got %d (cooldown broken)", preSnaps)
	}
}

// TestPreCompactSnapPayloadMirrorSymmetry — runtime fmt vs registry
// substring set lock for R22 (mirror of TestPlanCapPayloadMirrorSymmetry).
func TestPreCompactSnapPayloadMirrorSymmetry(t *testing.T) {
	subs := protocol.PayloadMirrorSubstrings("R22")
	if len(subs) == 0 {
		t.Fatal("R22 PayloadMirrorSubstrings empty — schema gap")
	}
	for _, s := range subs {
		if !strings.Contains(planCapPreSnapFmt, s) {
			t.Errorf("R22: planCapPreSnapFmt missing payload-mirror substring %q", s)
		}
	}
}

// TestPlanCapResumeNoEmitOnPreExistingHaltWithoutTransition (Phase J
// tail-2 K-1) — locks the in-memory transition gate. Halt row exists
// in DB (e.g., persisted from prior session), but THIS Gemma instance
// never observed an over-threshold poll → planCapHaltActive stays
// false → no RESUME emit even though hadHalt-from-DB is true.
func TestPlanCapResumeNoEmitOnPreExistingHaltWithoutTransition(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	// Pre-seed halt_state row (simulating persisted state from prior session
	// or from a fluctuation-fired halt that was already cleared in-memory).
	if err := db.SetHaltActive(hub.HaltCausePlanCap, "preseeded", "test"); err != nil {
		t.Fatal(err)
	}
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.50, 0.50, 0.50, 0.50, 0.50},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	})

	now := time.Now()
	for i := 0; i < 5; i++ {
		g.checkPlanUsage(now.Add(time.Duration(i) * (planUsageBaseInterval + time.Second)))
	}

	msgs, err := db.GetRecentMessages(50)
	if err != nil {
		t.Fatal(err)
	}
	resumes := 0
	for _, m := range msgs {
		if m.FromAgent == agentID && strings.Contains(m.Content, "plan usage reset") {
			resumes++
		}
	}
	if resumes != 0 {
		t.Errorf("transition gate broken: pre-existing halt without in-memory transition produced %d RESUME emits (want 0)", resumes)
	}
}

// TestFirePlanCapHaltIdempotentOnRepeatedOverThresholdPolls (Phase J
// tail-2 K-1) — locks idempotency: 5 consecutive ≥95% polls produce
// EXACTLY ONE flag-emit + ONE wake-schedule-pair (not 5 of each).
func TestFirePlanCapHaltIdempotentOnRepeatedOverThresholdPolls(t *testing.T) {
	g, db := newContextCapGemma(t)
	rec := &hubRecorder{}
	g.SetHubPublisher(rec.publish)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.96, 0.97, 0.98, 0.99, 1.00},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	})

	now := time.Now()
	for i := 0; i < 5; i++ {
		g.checkPlanUsage(now.Add(time.Duration(i) * (planUsageBaseInterval + time.Second)))
	}

	// Expect exactly 1 plan-cap flag (idempotent on repeated transitions
	// staying within the over-threshold band).
	if got := countPlanCapFlags(t, db); got != 1 {
		t.Errorf("repeated over-threshold polls must emit exactly 1 flag (false→true transition); got %d", got)
	}
}

// TestPlanCapWakeScheduleDedupesAcrossOscillation locks the Phase J post-
// rebuild fix (2026-04-29) for the wake_schedule accumulation observed in
// production: oscillating maxUtil (0.96 → 0.50 → 0.96 → 0.50) caused
// firePlanCapHalt to schedule a fresh +5h+1min RESUME wake on every
// oscillation cycle (each looked like a new false→true transition because
// the < 0.85 branch cleared planCapHaltActive). 30min of oscillation
// accumulated 200+ pending rows that all fired at fire_at as RESUME spam.
//
// Fix: HasPendingWakeForTarget gate inside firePlanCapHalt's wake-schedule
// block — if a pending RESUME wake already exists for the agent, skip
// scheduling another. Defense-in-depth alongside the existing transition
// gate.
func TestPlanCapWakeScheduleDedupesAcrossOscillation(t *testing.T) {
	g, db := newContextCapGemma(t)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.96, 0.50, 0.96, 0.50, 0.96, 0.50},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	})
	now := time.Now()
	for i := 0; i < 6; i++ {
		g.checkPlanUsage(now.Add(time.Duration(i) * (planUsageBaseInterval + time.Second)))
	}

	wakes, err := db.ListPendingWakes(now.Add(planCapWakeOffset + time.Hour))
	if err != nil {
		t.Fatal(err)
	}
	pendingByTarget := map[string]int{}
	for _, w := range wakes {
		if !strings.Contains(w.Payload, "plan usage reset") {
			continue
		}
		pendingByTarget[w.TargetAgent]++
	}
	// Across 3 halt-fires, at most ONE pending RESUME wake per target
	// should remain. Earlier accumulation bug would produce 3 per target.
	if pendingByTarget["brian"] > 1 {
		t.Errorf("oscillation-with-cancel-on-clear should accumulate at most 1 pending RESUME wake per target; got brian=%d", pendingByTarget["brian"])
	}
	if pendingByTarget["rain"] > 1 {
		t.Errorf("oscillation-with-cancel-on-clear should accumulate at most 1 pending RESUME wake per target; got rain=%d", pendingByTarget["rain"])
	}
}

// TestEmitPlanCapResumeCancelsPendingWakes locks the Phase J post-rebuild
// fix (2026-04-29): when Emma emits RESUME via the auto-clear path, any
// pending future +5h+1min RESUME wakes for the same targets must be
// cancelled. Otherwise the scheduled wakes fire later as redundant
// re-spam (observed: 504 fired wakes today during the failure window).
func TestEmitPlanCapResumeCancelsPendingWakes(t *testing.T) {
	g, db := newContextCapGemma(t)
	g.SetPlanUsageFetcher(&fakePlanUsageFetch{
		maxUtil: []float64{0.96, 0.50},
		window:  []string{anthropic.WindowFiveHour, anthropic.WindowFiveHour},
	})

	now := time.Now()
	g.checkPlanUsage(now)
	// After halt fire: 2 wakes scheduled (brian + rain).
	wakes, _ := db.ListPendingWakes(now.Add(planCapWakeOffset + time.Hour))
	if len(wakes) < 2 {
		t.Fatalf("expected >=2 pending wakes after halt-fire, got %d", len(wakes))
	}

	g.checkPlanUsage(now.Add(planUsageBaseInterval + time.Second)) // <0.85: emitPlanCapResume runs
	wakes, _ = db.ListPendingWakes(now.Add(planCapWakeOffset + time.Hour))
	pending := 0
	for _, w := range wakes {
		if strings.Contains(w.Payload, "plan usage reset") {
			pending++
		}
	}
	if pending != 0 {
		t.Errorf("emitPlanCapResume must cancel pending RESUME wakes; %d still pending", pending)
	}
}
