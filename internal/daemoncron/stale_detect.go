package daemoncron

// Stale-coder surface — Phase S S-1a-2 (programmatic-move from
// internal/gemma/stale_detect.go flagStaleAgent + checkStaleAgentsAt).
//
// Cadence: 30s tick (mirrors gemma healthLoop pre-migration).
//
// Detection logic — same gates as gemma pre-migration except pane-
// activity tmux check (deferred to follow-up; tmux-specific signal
// requires tmux package import + paneActivity injectable):
//
//  1. Halt-state check — IsHalted() suppresses stale-fires during
//     active halt window (all-hands-idle by convention).
//  2. User HALT-directive check — content-only `[HUB:user] HALT`
//     suppresses until next non-HALT user msg.
//  3. ListAgents scan — only Online/Working agents > staleThreshold
//     LastSeen-aged are candidates.
//  4. flagStaleCoder per-agent gates: intentional-idle short-circuit
//     (CurrentTask non-empty) → recent-msg backstop (HasRecentMessage
//     From) → LastSeen-advance dedupe → rate-cap (1 per 4h window).
//
// Pane-activity gemma-side check is temporarily disabled when daemon-
// cron is online (gemma.checkStaleAgents short-circuits via the
// daemoncronOnline feature-flag). Net effect: pane-active-but-
// LastSeen-stale agents may fire 1 PM per 4h vs pre-migration zero
// PMs. Rate-cap (1/4h per agent) bounds noise; pane-checker
// migration scheduled S-1a-followup once tmux dependency surface
// designed for daemoncron.

import (
	"fmt"
	"log"
	"regexp"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// staleThreshold mirrors gemma's existing const value.
	staleThreshold = 30 * time.Minute

	// staleEmitWindow + staleEmitMaxPerWindow are the rate-cap
	// parameters per Phase Q post-close smoke loosening (1/4h vs
	// pre-3-per-hour).
	staleEmitWindow      = 4 * time.Hour
	staleEmitMaxPerWindow = 1

	// staleTickInterval is the surface's poll cadence (matches gemma
	// healthLoop pre-migration).
	staleTickInterval = 30 * time.Second

	// staleAgentID + staleRecipient — preserved from gemma pre-
	// migration so consumer-side rule-text recognition unchanged.
	staleAgentID    = "emma"
	staleRecipient  = "rain"
)

// userHaltPatternRe — same shape as gemma's userHaltPatternRe but
// scoped to daemoncron package. Matches `[HUB:user] HALT` style
// halt directives (case-insensitive on the HALT token).
var userHaltPatternRe = regexp.MustCompile(`(?i)\bHALT\b`)

// staleState tracks per-agent dedupe + rate-cap state. Package-
// scoped + mu-guarded since stale-coder fires from a single
// goroutine (own ticker).
var (
	staleStateMu     sync.Mutex
	staleFlagTracker map[string]time.Time     // agent_id → last LastSeen seen
	staleEmitTimes   map[string][]time.Time   // agent_id → recent emit timestamps
)

// runStaleCoderSurface is the surfaceFunc for stale-coder. Scans
// active agents; flags those past staleThreshold subject to all
// dedupe/rate-cap gates.
func runStaleCoderSurface(c *Cron) {
	now := c.Now()

	// Halt-state suppression.
	if halted, _ := c.db.IsHalted(); halted {
		return
	}

	// User HALT directive suppression.
	if last, ok, err := c.db.GetLatestMessageFrom("user"); err == nil && ok && userHaltPatternRe.MatchString(last.Content) {
		return
	}

	agents, err := c.db.ListAgents("")
	if err != nil {
		return
	}

	for _, a := range agents {
		if a.ID == staleAgentID {
			continue
		}
		if a.Status != protocol.StatusOnline && a.Status != protocol.StatusWorking {
			continue
		}
		if now.Sub(a.LastSeen) <= staleThreshold {
			continue
		}
		flagStaleCoder(c, a, now)
	}
}

// flagStaleCoder emits the [STALE-CODER] PM gated by intentional-
// idle / recent-msg / LastSeen-advance / rate-cap. Identical gate
// semantics to gemma flagStaleAgent (sans pane-activity check).
func flagStaleCoder(c *Cron, a protocol.Agent, now time.Time) {
	// Intentional-idle filter (Phase-R-followup-1 (f)) — agent
	// declaring active multi-step work-thread via current_task.
	if a.CurrentTask != "" {
		return
	}

	staleStateMu.Lock()
	defer staleStateMu.Unlock()

	if staleFlagTracker == nil {
		staleFlagTracker = make(map[string]time.Time)
	}
	if last, seen := staleFlagTracker[a.ID]; seen && last.Equal(a.LastSeen) {
		return // LastSeen unchanged → same incident → suppress
	}

	// Recent-msg backstop (LastSeen-write-failure-race defense).
	if hasRecent, err := c.db.HasRecentMessageFrom(a.ID, now.Add(-staleThreshold)); err == nil && hasRecent {
		return
	}

	// Time-windowed rate-cap.
	if staleEmitTimes == nil {
		staleEmitTimes = make(map[string][]time.Time)
	}
	cutoff := now.Add(-staleEmitWindow)
	prev := staleEmitTimes[a.ID]
	pruned := prev[:0]
	for _, t := range prev {
		if t.After(cutoff) {
			pruned = append(pruned, t)
		}
	}
	if len(pruned) >= staleEmitMaxPerWindow {
		staleEmitTimes[a.ID] = pruned
		return
	}

	staleFlagTracker[a.ID] = a.LastSeen
	staleEmitTimes[a.ID] = append(pruned, now)
	age := now.Sub(a.LastSeen).Round(time.Second)
	if _, err := c.db.InsertMessage(protocol.Message{
		FromAgent: staleAgentID,
		ToAgent:   staleRecipient,
		Type:      protocol.MsgUpdate,
		Content:   fmt.Sprintf("[STALE-CODER] agent %s last_seen=%s ago, no pane activity since last tick", a.ID, age),
	}); err != nil {
		log.Printf("[daemoncron stale-coder] insert failed: %v", err)
	}
}

// ResetStaleStateForTest clears the package-scoped stale dedupe + rate-
// cap state. Test-only.
func ResetStaleStateForTest() {
	staleStateMu.Lock()
	defer staleStateMu.Unlock()
	staleFlagTracker = nil
	staleEmitTimes = nil
}
