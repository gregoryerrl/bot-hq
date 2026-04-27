package gemma

import (
	"context"
	"errors"
	"fmt"
	"log"
	"runtime"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/anthropic"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// planUsageThreshold is the maxUtil value at or above which Emma fires the
// plan-cap halt-flag. 0.95 leaves a small runway before the account hits
// the absolute window cap so the trio can finish the current tool call
// and post a close-SNAP via H-15 before user-driven rebuild-for-autonomy.
const planUsageThreshold = 0.95

// planUsageResetThreshold is the maxUtil value below which Emma clears
// the plan-cap halt. The 0.95/0.85 gap mirrors the context-cap hysteresis
// — prevents flapping when utilization hovers in the squeeze band; the
// halt only re-arms after a clean drop below 0.85.
const planUsageResetThreshold = 0.85

// planUsageBaseInterval is the steady-state poll cadence (60s). Tests
// override via SetPlanUsageNow to step the gate deterministically.
const planUsageBaseInterval = 60 * time.Second

// planUsageBackoffInterval is the cadence Emma falls back to after a 5xx
// or auth-fail response. 10× base — long enough that a sustained outage
// doesn't burn through quota, short enough that recovery surfaces inside
// a typical work session.
const planUsageBackoffInterval = 600 * time.Second

// planUsageFetchTimeout is the per-call cap. Slightly above the
// anthropic-side fetchTimeout so context cancellation surfaces as the
// inner timeout rather than a hard cancel race.
const planUsageFetchTimeout = 6 * time.Second

// planCapReasonFmt is the literal substring brian/rain STARTUP prompts
// match against ("plan usage at <N>%, halt"). Any reformat MUST be
// mirrored in the prompt convention or the halt-all-work contract breaks
// silently. Slice-5 C1 (H-33).
const planCapReasonFmt = "plan usage at %d%%%s, halt + checkpoint via H-15 + idle for fresh session"

// PlanUsageFetcher abstracts the anthropic.UsageClient for testability.
// Production wires a real client; tests inject a deterministic fake.
type PlanUsageFetcher interface {
	Fetch(ctx context.Context) (maxUtil float64, maxWindow string, perWindow map[string]anthropic.Window, err error)
}

// SetPlanUsageFetcher overrides the producer used by checkPlanUsage. Used
// by tests to inject a deterministic fake; production calls
// initPlanUsageDefault on Start.
func (g *Gemma) SetPlanUsageFetcher(f PlanUsageFetcher) {
	g.planUsageMu.Lock()
	defer g.planUsageMu.Unlock()
	g.planUsageFetch = f
}

// SetHubPublisher wires the panestate.Manager.SetHubSnapshot sink so
// successful plan-usage polls update the right-aligned strip segment.
// cmd/bot-hq/main.go calls this after the TUI's Manager exists; tests
// inject a recorder.
func (g *Gemma) SetHubPublisher(fn func(panestate.HubSnapshot)) {
	g.planUsageMu.Lock()
	defer g.planUsageMu.Unlock()
	g.hubPublisher = fn
}

// initPlanUsageDefault wires the production PlanUsageFetcher when none has
// been injected. macOS-only — non-darwin hosts publish PlanUsagePct=-1
// once and skip polling forever (no subprocess spawn, no API call).
func (g *Gemma) initPlanUsageDefault() {
	g.planUsageMu.Lock()
	defer g.planUsageMu.Unlock()
	if g.planUsageFetch != nil {
		return
	}
	if runtime.GOOS != "darwin" {
		// Publish unknown once so the strip renders --% on first paint
		// instead of stale-zero, then leave fetch nil — checkPlanUsage
		// short-circuits when fetch is nil.
		if g.hubPublisher != nil {
			g.hubPublisher(panestate.HubSnapshot{PlanUsagePct: -1, FiveHourPct: -1, SevenDayPct: -1})
		}
		return
	}
	g.planUsageFetch = anthropic.NewUsageClient("", &anthropic.KeychainCredentialSource{})
}

// checkPlanUsage is the slice-5 C1 plan-cap entry point. Mirrors
// context_cap.go shape closely: walks utilization, fires halt+flag when
// crossing the squeeze threshold, clears the halt when usage decays. The
// 60s base cadence + 600s backoff are enforced inside this function via
// lastPlanPoll/planBackoffUntil; callers (sentinelPollLoop tick) invoke
// every 5s.
func (g *Gemma) checkPlanUsage(now time.Time) {
	g.planUsageMu.Lock()
	fetch := g.planUsageFetch
	publisher := g.hubPublisher
	last := g.lastPlanPoll
	backoffUntil := g.planBackoffUntil
	g.planUsageMu.Unlock()

	if fetch == nil {
		return
	}
	// Cadence gate: 60s base, or whatever lastPlanPoll + interval permits.
	// Backoff window overrides: while inside, the 600s gate dominates.
	gate := planUsageBaseInterval
	if !backoffUntil.IsZero() && now.Before(backoffUntil) {
		return
	}
	if !last.IsZero() && now.Sub(last) < gate {
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), planUsageFetchTimeout)
	defer cancel()
	maxUtil, maxWindow, perWindow, err := fetch.Fetch(ctx)
	g.planUsageMu.Lock()
	g.lastPlanPoll = now
	g.planUsageMu.Unlock()

	if err != nil {
		g.handlePlanUsageError(now, err)
		return
	}

	// Successful fetch — clear backoff, publish HubSnapshot.
	g.planUsageMu.Lock()
	g.planBackoffUntil = time.Time{}
	g.planUsageMu.Unlock()
	pctInt := utilToPct(maxUtil)
	if publisher != nil {
		publisher(panestate.HubSnapshot{
			PlanUsagePct: pctInt,
			PlanWindow:   maxWindow,
			FiveHourPct:  windowPct(perWindow, anthropic.WindowFiveHour),
			SevenDayPct:  windowPct(perWindow, anthropic.WindowSevenDay),
		})
	}

	// Halt + clear logic. Threshold crossing fires hub_flag; a clean drop
	// below the reset threshold deletes the plan-cap row regardless of
	// any prior fire (organic clear via window-rollover or quota decay).
	if maxUtil >= planUsageThreshold {
		g.firePlanCapHalt(now, pctInt, maxWindow)
		return
	}
	if maxUtil < planUsageResetThreshold {
		if err := g.db.ClearHalt(hub.HaltCausePlanCap); err != nil {
			log.Printf("[plan-cap] clear halt failed: %v", err)
		}
		// Hysteresis re-arm so a fresh squeeze past 95% can fire again
		// after this organic clear.
		g.flagMu.Lock()
		delete(g.flagHistory, "plan-cap")
		g.flagMu.Unlock()
	}
}

// handlePlanUsageError applies the documented backoff policy. Auth-fail
// (401/403) and 5xx both produce the same 600s backoff. ErrTokenExpired
// is the documented near-expiry skip — log once, no backoff (caller will
// retry next 60s gate; if still expired, the log stays quiet thanks to
// the once-log guard). ErrUnsupportedPlatform is logged once and never
// retried (initPlanUsageDefault already filtered it).
func (g *Gemma) handlePlanUsageError(now time.Time, err error) {
	if errors.Is(err, anthropic.ErrTokenExpired) {
		log.Printf("[plan-cap] keychain credential near-expiry; skipping poll until refreshed")
		return
	}
	if errors.Is(err, anthropic.ErrUnsupportedPlatform) {
		g.planUsageMu.Lock()
		warned := g.planUsageWarnedOS
		g.planUsageWarnedOS = true
		publisher := g.hubPublisher
		g.planUsageMu.Unlock()
		if !warned {
			log.Printf("[plan-cap] %v — plan-usage producer disabled", err)
		}
		if publisher != nil {
			publisher(panestate.HubSnapshot{PlanUsagePct: -1, FiveHourPct: -1, SevenDayPct: -1})
		}
		return
	}
	log.Printf("[plan-cap] fetch failed: %v — backing off %s", err, planUsageBackoffInterval)
	g.planUsageMu.Lock()
	g.planBackoffUntil = now.Add(planUsageBackoffInterval)
	publisher := g.hubPublisher
	g.planUsageMu.Unlock()
	// H-40: surface the producer-errored state on the strip via `--%` so
	// it's visually distinguishable from "fresh boot, no observation yet."
	// PlanWindow=five_hour is the conventional default tag (renders bare per
	// planWindowTag); if a future producer-side error preserves prior window,
	// substitute it here (slice-5 H-40b candidate).
	if publisher != nil {
		publisher(panestate.HubSnapshot{
			PlanUsagePct: -1,
			PlanWindow:   anthropic.WindowFiveHour,
			FiveHourPct:  -1,
			SevenDayPct:  -1,
		})
	}
}

// windowPct returns the integer 0-100 percent for a named window in the
// Fetch response, or -1 if absent. Slice-5 hotfix dual-window: lets the
// strip render `5h:NN% 7d:NN%` with `--%` for windows the API didn't
// include (e.g. accounts that haven't yet observed seven_day traffic).
func windowPct(perWindow map[string]anthropic.Window, name string) int {
	w, ok := perWindow[name]
	if !ok {
		return -1
	}
	return utilToPct(w.Utilization)
}

// utilToPct rounds a 0-1 utilization float to 0-100, clamping the upper
// bound. Centralized so the max + per-window fields share rounding.
func utilToPct(u float64) int {
	pctInt := int(u*100 + 0.5)
	if pctInt > 100 {
		pctInt = 100
	}
	return pctInt
}

// firePlanCapHalt fires hub_flag and sets the halt_state row. Wraps
// shouldFlag for hysteresis + rate cap so a stuck-near-threshold account
// doesn't burn through Emma's flag budget. The halt_state row is upserted
// idempotently — repeated fires update set_at/reason without thrashing.
func (g *Gemma) firePlanCapHalt(now time.Time, pct int, window string) {
	tag := windowDisplayTag(window)
	reason := fmt.Sprintf(planCapReasonFmt, pct, tag)

	if g.shouldFlag("plan-cap", now) {
		content := fmt.Sprintf("[CRITICAL] %s", reason)
		if _, err := g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   "user",
			Type:      protocol.MsgFlag,
			Content:   content,
			Created:   now,
		}); err != nil {
			log.Printf("[plan-cap] flag insert failed: %v", err)
		}
	}
	if err := g.db.SetHaltActive(hub.HaltCausePlanCap, reason, agentID); err != nil {
		log.Printf("[plan-cap] set halt active failed: %v", err)
	}
}

// windowDisplayTag maps an oauth_usage window name to the suffix tag the
// strip + reason text both use. Empty for five_hour (the default,
// most-frequently-binding window — display stays compact). Other windows
// surface as " (weekly)" / " (opus)" / " (extra)" so the fire reason text
// makes the binding limit obvious in the hub log.
func windowDisplayTag(window string) string {
	switch window {
	case anthropic.WindowFiveHour, "":
		return ""
	case anthropic.WindowSevenDay:
		return " (weekly)"
	case anthropic.WindowSevenDayOpus:
		return " (opus)"
	case anthropic.WindowSevenDaySonnet:
		return " (extra)"
	}
	return " (" + window + ")"
}
