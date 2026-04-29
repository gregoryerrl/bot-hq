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

// planUsagePreSnapThreshold is the proactive checkpoint threshold (Phase J
// T2.2-α B1a). Below the halt-fire threshold (0.95) but above the reset
// threshold (0.85) so it fires while agents still have headroom to commit
// in-flight work + write AgentState before halt. Pre-compact-snap fires
// once per planCapPreSnapCooldown window (5min) to prevent spam when
// maxUtil hovers in the [0.90, 0.95) band.
const planUsagePreSnapThreshold = 0.90

// planCapPreSnapCooldown caps emitPreCompactSnap to once per window. Same
// pattern as planCapResumeCooldown — bounded suppress of poll-cycle spam
// when maxUtil hovers near threshold.
const planCapPreSnapCooldown = 5 * time.Minute

// planCapPreSnapFmt is the locked content substring for [PRE-COMPACT-SNAP]
// emit. Format: structured payload telling agents to checkpoint AgentState
// + emit hub_session_close-style SNAP if mid-substantive-work. Recipient
// behavior governed by R22 PRE-COMPACT-SNAP rule.
const planCapPreSnapFmt = "[PRE-COMPACT-SNAP] plan usage at %d%%, approaching halt threshold — checkpoint AgentState (R20) + emit SNAP if mid-substantive-work to preserve resume anchors. headroom remaining before halt-fire."

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
//
// Phase J T1.2 (B3b F2 fold) cleanup: post-Fix-3 wording realignment.
// Prompt-rule (PhaseJv1HaltResumeProtocol H-31) says "idle in pane (do
// NOT close session — stay alive to receive RESUME)"; emitter previously
// said "checkpoint via H-15 + idle for fresh session" (pre-Fix-3 stale).
// Aligned: "halt + idle in pane" preserves the load-bearing trigger
// substrings ("plan usage at" + "halt") while removing the obsolete
// post-halt-action wording. Per docs/plans/2026-04-29-rule-loci-audit.md
// F2. The R16 ratchet test (TestRuleNamespaceRatchet) cross-checks the
// shared substrings between this fmt and the prompt-rule.
const planCapReasonFmt = "plan usage at %d%%%s, halt + idle in pane"

// planCapResumeFmt is the literal substring brian/rain STARTUP prompts
// match against to detect plan-window-rollover resume directives. Phase I
// W2 hotfix (msg 4906): emitted by Emma at the auto-clear path when
// maxUtil drops below planUsageResetThreshold AND a halt was previously
// active. Format: "plan usage reset to <N>%, resume work via R16
// cross-restart-resume protocol bootstrap (a) git status (b)
// ~/.bot-hq/phase/<active-phase>.md (c) ~/.bot-hq/ratchets/active.md
// (d) hub_read backlog since halt-fire". Locked substring: "plan usage
// reset" — agents grep this in their initial-prompt match-rule.
const planCapResumeFmt = "plan usage reset to %d%%, resume work via R16 cross-restart-resume protocol bootstrap (a) git status (b) ~/.bot-hq/phase/<active-phase>.md (c) ~/.bot-hq/ratchets/active.md (d) hub_read backlog since halt-fire"

// planCapResumeCooldown caps emitPlanCapResume to once per window even
// if the hadHalt-gate keeps firing on each poll (observed Phase J pass-3,
// hub.db msgs 5194-5218: ~30 RESUME emits between two legit halt cycles).
// Suspected root cause: maxUtil fluctuation around 95% threshold causes
// firePlanCapHalt to re-create halt-state row each fluctuation poll;
// next <85% poll then emits RESUME. The DB-side hadHalt check is correct
// per-event but doesn't dedup across many rapid event cycles.
//
// 10min window: smaller than the observed ~30min gap between legitimate
// halt-fires (so legit halt→reset transitions still get one RESUME emit
// each), large enough to absorb any rapid-fluctuation noise.
const planCapResumeCooldown = 10 * time.Minute

// planCapWakeOffset is the duration past lastPlanPoll at which the
// scheduled-wake fires. 5h + 1min: just past Anthropic's plan-window
// rollover so Emma re-polls into the post-rollover usage figure
// (typically near 0%) and emits the immediate resume nudge via the
// auto-clear path. The 1min cushion absorbs polling jitter + clock
// skew. Phase I W2 hotfix.
const planCapWakeOffset = 5*time.Hour + 1*time.Minute

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
	// Phase J T2.2-α (B1a): proactive pre-compact-snap signal in
	// [planUsagePreSnapThreshold, planUsageThreshold). Fires once per
	// planCapPreSnapCooldown window. Cooldown suppresses spam when maxUtil
	// hovers in the band; cooldown stamp shared with resume-emit pattern
	// (mu-protected via planUsageMu).
	if maxUtil >= planUsagePreSnapThreshold {
		g.emitPreCompactSnap(now, pctInt)
		// Fall through — pre-snap is informational; halt + clear logic
		// below still applies on the same poll if state crossed back below
		// reset threshold (rare since we only land here when ≥ 0.90).
	}
	if maxUtil < planUsageResetThreshold {
		// Phase J tail-2 (K-1 RESUME-spam fix): emit gates on the
		// in-memory transition planCapHaltActive true→false (not on
		// hadHalt-from-DB). DB hadHalt-gate alone over-triggers when
		// maxUtil jitters: SetHaltActive runs each ≥95% poll (re-creating
		// halt row), next <85% poll reads hadHalt=true → emits → cycle.
		// In-memory transition tracking debounces this; only emit when
		// THIS instance previously observed an over-threshold poll that
		// transitioned to under-threshold this poll.
		_, hadHaltDB, err := g.db.GetHaltCause(hub.HaltCausePlanCap)
		if err != nil {
			log.Printf("[plan-cap] get halt cause check failed: %v", err)
		}
		g.planUsageMu.Lock()
		prevHaltActive := g.planCapHaltActive
		g.planCapHaltActive = false
		coolingDown := !g.lastPlanCapResumeAt.IsZero() && now.Sub(g.lastPlanCapResumeAt) < planCapResumeCooldown
		shouldEmit := prevHaltActive && !coolingDown
		if shouldEmit {
			g.lastPlanCapResumeAt = now
		}
		g.planUsageMu.Unlock()

		if err := g.db.ClearHalt(hub.HaltCausePlanCap); err != nil {
			log.Printf("[plan-cap] clear halt failed: %v", err)
		}
		// Hysteresis re-arm so a fresh squeeze past 95% can fire again
		// after this organic clear.
		g.flagMu.Lock()
		delete(g.flagHistory, "plan-cap")
		g.flagMu.Unlock()

		_ = hadHaltDB // kept for log-debugging visibility on transition mismatch
		if shouldEmit {
			g.emitPlanCapResume(now, pctInt)
		}
	}
}

// emitPlanCapResume inserts MsgCommand records to brian + rain telling
// each agent to resume work after a plan-window-rollover. Substring
// "plan usage reset" is the locked match against initial-prompt rule
// (mirror of the halt-substring discipline).
//
// Phase I W2 hotfix Fix-2 (msg 4906 user directive — continuability
// when user-AFK).
//
// Phase J post-rebuild fix (2026-04-29): also cancel any pending future
// RESUME wakes for the same targets. The +5h+1min wake_schedule rows
// were belt-and-suspenders backup for the case where Emma's auto-clear
// path missed the rollover (API backoff at the moment). Now that we're
// emitting RESUME via the auto-clear path itself, any scheduled wakes
// are redundant and would just re-spam the agents' panes when fire_at
// hits. Observed root cause: 200+ accumulated wakes fired at 1/min today
// because oscillating maxUtil scheduled one wake per oscillation cycle.
func (g *Gemma) emitPlanCapResume(now time.Time, pct int) {
	content := fmt.Sprintf("[RESUME] %s", fmt.Sprintf(planCapResumeFmt, pct))
	for _, target := range []string{"brian", "rain"} {
		if _, err := g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   target,
			Type:      protocol.MsgCommand,
			Content:   content,
			Created:   now,
		}); err != nil {
			log.Printf("[plan-cap] resume nudge insert failed for %s: %v", target, err)
		}
		// Cancel any pending future RESUME wakes for this target — they're
		// now redundant (we just emitted via the auto-clear path).
		if n, err := g.db.CancelPendingWakesForTargetByPayloadPrefix(target, "[RESUME]"); err != nil {
			log.Printf("[plan-cap] cancel pending RESUME wakes for %s failed: %v", target, err)
		} else if n > 0 {
			log.Printf("[plan-cap] cancelled %d pending RESUME wakes for %s (auto-clear path emitted)", n, target)
		}
	}
}

// emitPreCompactSnap inserts MsgUpdate records to brian + rain telling
// each agent to checkpoint AgentState (R20) + emit hub_session_close-style
// SNAP if mid-substantive-work. Phase J T2.2-α (B1a). Substring
// "[PRE-COMPACT-SNAP]" matches the R22 prompt-rule recognition gate.
//
// Cooldown via planCapPreSnapCooldown (5min) suppresses spam when maxUtil
// hovers in [0.90, 0.95) band. Cooldown stamp is per-Gemma-instance
// (lastPreCompactSnapAt field, mu-protected via planUsageMu).
func (g *Gemma) emitPreCompactSnap(now time.Time, pct int) {
	g.planUsageMu.Lock()
	coolingDown := !g.lastPreCompactSnapAt.IsZero() && now.Sub(g.lastPreCompactSnapAt) < planCapPreSnapCooldown
	if !coolingDown {
		g.lastPreCompactSnapAt = now
	}
	g.planUsageMu.Unlock()
	if coolingDown {
		return
	}
	content := fmt.Sprintf(planCapPreSnapFmt, pct)
	for _, target := range []string{"brian", "rain"} {
		if _, err := g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   target,
			Type:      protocol.MsgUpdate,
			Content:   content,
			Created:   now,
		}); err != nil {
			log.Printf("[plan-cap] pre-compact-snap insert failed for %s: %v", target, err)
		}
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

// seedPlanCapHaltActiveFromDB seeds the in-memory planCapHaltActive bool
// from hub.db halt_state at Gemma startup. Closes the asymmetry between
// clear-path (~line 237 already cross-checks db.GetHaltCause via
// hadHaltDB) and fire-path (~line 418 reads planCapHaltActive direct,
// treating in-mem zero-value false as fresh-halt even when the DB row
// indicates continuous halt across the restart boundary).
//
// Without seeding: process restart while halt was active leaves in-mem
// at false. First post-restart over-threshold poll fires firePlanCapHalt,
// sees wasActive=false → re-emits hub_flag + reschedules wake. The
// 9ac82a7 Fix-A (HasPendingWakeForTarget) dedups the wake-schedule
// reinsertion, but a fresh hub_flag still goes out. With seeding:
// in-mem mirrors DB so the transition gate correctly sees wasActive=true
// and the fire-path stays idempotent across restart, matching the
// already-correct behavior of the clear-path.
//
// Phase J tail-4 (K-1-bis-deeper Axis A; user msg 5928 sequence-locked
// 2026-04-29; Rain msg 5909 BRAIN-2nd-acked + Rain msg 5933 reminder
// pre-commit). cite_anchor: plan_usage.go:237/418 asymmetry + commit
// 9ac82a7 + ratchet K-1-bis-resolved.
func (g *Gemma) seedPlanCapHaltActiveFromDB() {
	_, active, err := g.db.GetHaltCause(hub.HaltCausePlanCap)
	if err != nil {
		log.Printf("[plan-cap] seed planCapHaltActive: GetHaltCause failed: %v", err)
		return
	}
	if active {
		g.planUsageMu.Lock()
		g.planCapHaltActive = true
		g.planUsageMu.Unlock()
		log.Printf("[plan-cap] seeded planCapHaltActive=true from DB halt_state on startup (post-restart asymmetry close)")
	}
}

// firePlanCapHalt fires hub_flag and sets the halt_state row. Wraps
// shouldFlag for hysteresis + rate cap so a stuck-near-threshold account
// doesn't burn through Emma's flag budget. The halt_state row is upserted
// idempotently — repeated fires update set_at/reason without thrashing.
func (g *Gemma) firePlanCapHalt(now time.Time, pct int, window string) {
	tag := windowDisplayTag(window)
	reason := fmt.Sprintf(planCapReasonFmt, pct, tag)

	// Phase J tail-2 (K-1 RESUME-spam fix): in-memory transition gate.
	// MsgFlag emit + wake-schedule insertion only fire on false→true
	// transition (NEW halt). Repeated ≥95% polls update DB row but skip
	// repeated user-visible flag + wake-schedule spam. ClearHalt path
	// reads + flips this state on true→false transition.
	g.planUsageMu.Lock()
	wasActive := g.planCapHaltActive
	g.planCapHaltActive = true
	g.planUsageMu.Unlock()

	// shouldFlag still applies as belt-and-suspenders rate-cap on the
	// MsgFlag user-visible emit path.
	if !wasActive && g.shouldFlag("plan-cap", now) {
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

	// SetHaltActive always runs — IsHalted consumers (e.g., checkStaleAgents
	// suppression-during-halt) need accurate DB state regardless of in-memory
	// transition tracking.
	if err := g.db.SetHaltActive(hub.HaltCausePlanCap, reason, agentID); err != nil {
		log.Printf("[plan-cap] set halt active failed: %v", err)
	}

	// Phase I W2 hotfix Fix-1 + Phase J tail-2 K-1: schedule wakes ONLY on
	// false→true transition (NEW halt). Eliminates wake_schedule spam
	// observed accumulating 100s of pending rows under maxUtil-jitter.
	// The wake's payload IS the resume content; dispatchWakes() inserts
	// it as MsgCommand to target_agent at fire-time, matching the
	// emitPlanCapResume substring "plan usage reset" so agents recognize
	// it identically to the auto-clear path.
	//
	// Phase J post-rebuild fix (2026-04-29): the transition-gate above is
	// not sufficient when planCapHaltActive flips false→true→false→true
	// across maxUtil oscillation cycles (each oscillation looks like a
	// "new halt" to the gate and schedules another wake). Add a per-target
	// HasPendingWakeForTarget check as defense-in-depth: if a pending
	// RESUME wake already exists for the agent, skip scheduling another.
	// The first wake's payload is identical to the second's, so deduping
	// at schedule-time is correct.
	if !wasActive {
		wakeAt := now.Add(planCapWakeOffset)
		wakePayload := fmt.Sprintf("[RESUME] %s", fmt.Sprintf(planCapResumeFmt, 0))
		for _, target := range []string{"brian", "rain"} {
			if pending, err := g.db.HasPendingWakeForTarget(target); err == nil && pending {
				// Already a wake queued — skip duplicate.
				continue
			}
			if _, err := g.db.InsertWakeSchedule(target, agentID, wakePayload, wakeAt); err != nil {
				log.Printf("[plan-cap] schedule wake insert failed for %s: %v", target, err)
			}
		}
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
