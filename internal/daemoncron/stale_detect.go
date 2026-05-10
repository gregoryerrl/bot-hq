package daemoncron

// Stale-coder surface — Phase S S-1a-2 (programmatic-move from
// internal/gemma/stale_detect.go flagStaleAgent + checkStaleAgentsAt).
//
// Cadence: 30s tick (mirrors gemma healthLoop pre-migration).
//
// Detection logic — Shape γ hybrid:
//
//  1. Halt-state check — IsHalted() suppresses stale-fires during
//     active halt window (all-hands-idle by convention).
//  2. User HALT-directive check — content-only `[HUB:user] HALT`
//     suppresses until next non-HALT user msg.
//  3. ListAgents scan — only Online/Working agents > staleThreshold
//     LastSeen-aged are candidates.
//  4. Pane-activity check (Shape γ): for agents with tmux_target Meta,
//     compare current `#{pane_last_activity}` against last tick's
//     baseline. First tick establishes baseline (no flag); subsequent
//     ticks flag only when pane is silent across ticks. Fall through
//     to Shape α (last_seen alone) for agents without tmux_target.
//  5. flagStaleCoder per-agent gates: intentional-idle short-circuit
//     (CurrentTask non-empty) → recent-msg backstop (HasRecentMessage
//     From) → LastSeen-advance dedupe → rate-cap (1 per 4h window).

import (
	"encoding/json"
	"fmt"
	"log"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
)

const (
	// staleThreshold mirrors gemma's existing const value.
	staleThreshold = 30 * time.Minute

	// staleEmitWindow + staleEmitMaxPerWindow are the rate-cap
	// parameters per Phase Q post-close smoke loosening (1/4h vs
	// pre-3-per-hour).
	staleEmitWindow       = 4 * time.Hour
	staleEmitMaxPerWindow = 1

	// staleTickInterval is the surface's poll cadence (matches gemma
	// healthLoop pre-migration).
	staleTickInterval = 30 * time.Second

	// staleAgentID + staleRecipient — preserved from gemma pre-
	// migration so consumer-side rule-text recognition unchanged.
	staleAgentID   = "emma"
	staleRecipient = "rain"
)

// userHaltPatternRe matches the canonical user-HALT directive shape in
// hub message content. Used by runStaleCoderSurface (Phase I W1b I-6 C1)
// to suppress stale-checks while the trio is intentionally halted by user
// directive — agents are legitimately idle and stale-flag-fires would be
// FPs. The check is content-pattern based rather than halt_state-based
// because user-HALT is currently a content-only signal: no path
// transitions hub.halt_state to active on user-HALT (existing
// SetHaltActive paths fire only on context-cap 95%, not user direction).
// When user emits a non-HALT directive (e.g., "proceed", "go"), the
// halt-state implicitly clears via the latest-user-message check —
// no explicit halt-clear API needed for this surgical fix.
var userHaltPatternRe = regexp.MustCompile(`(?i)\bHALT\b`)

// paneActivityFn returns the tmux #{pane_last_activity} unix timestamp for
// the given pane target. Abstracted as a function field so stale-detection
// tests can inject a deterministic fake without exec'ing real tmux.
type paneActivityFn func(target string) (int64, error)

// defaultPaneActivity shells out to `tmux display -p -t <target>
// '#{pane_last_activity}'`. tmux returns 0 when the target doesn't exist or
// the format is unsupported on the running tmux version — callers treat
// errors as "no usable pane signal" and fall through to the Shape α path.
func defaultPaneActivity(target string) (int64, error) {
	out, err := tmuxpkg.Exec("display", "-p", "-t", target, "#{pane_last_activity}")
	if err != nil {
		return 0, err
	}
	return strconv.ParseInt(strings.TrimSpace(out), 10, 64)
}

// metaTmuxTarget extracts the tmux_target field from an agent's Meta JSON.
// Returns empty string when Meta is empty, unparseable, or omits the key —
// the agent is then routed through the Shape α fallback (last_seen alone).
func metaTmuxTarget(meta string) string {
	if meta == "" {
		return ""
	}
	var m struct {
		TmuxTarget string `json:"tmux_target"`
	}
	if err := json.Unmarshal([]byte(meta), &m); err != nil {
		return ""
	}
	return m.TmuxTarget
}

// staleState tracks per-agent dedupe + rate-cap state. Package-
// scoped + mu-guarded since stale-coder fires from a single
// goroutine (own ticker).
var (
	staleStateMu     sync.Mutex
	staleFlagTracker map[string]time.Time   // agent_id → last LastSeen seen
	staleEmitTimes   map[string][]time.Time // agent_id → recent emit timestamps
	paneBaseline     map[string]int64       // agent_id → last tick's #{pane_last_activity}
	paneActivityImpl paneActivityFn         // injectable for tests
)

// SetPaneActivityForTest overrides the default tmux-shelling pane
// activity checker. Tests inject a deterministic fake; production
// leaves it unset so defaultPaneActivity is used.
func SetPaneActivityForTest(fn paneActivityFn) {
	staleStateMu.Lock()
	defer staleStateMu.Unlock()
	paneActivityImpl = fn
}

// runStaleCoderSurface is the surfaceFunc for stale-coder. Scans
// active agents; flags those past staleThreshold subject to all
// dedupe/rate-cap gates. Mirrors gemma checkStaleAgentsAt semantics.
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

	staleStateMu.Lock()
	checker := paneActivityImpl
	if checker == nil {
		checker = defaultPaneActivity
	}
	if paneBaseline == nil {
		paneBaseline = make(map[string]int64)
	}
	staleStateMu.Unlock()

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

		// Shape γ pane-activity check: for agents with tmux_target Meta,
		// compare current pane activity against last tick's baseline.
		// First observation per agent establishes baseline without
		// flagging — a real "no activity since last tick" signal needs
		// at least one prior tick to compare against.
		target := metaTmuxTarget(a.Meta)
		if target != "" {
			cur, perr := checker(target)
			if perr == nil {
				staleStateMu.Lock()
				prev, seen := paneBaseline[a.ID]
				paneBaseline[a.ID] = cur
				staleStateMu.Unlock()
				if !seen {
					// First observation — establish baseline; defer
					// flagging to the next tick so "since last tick"
					// has meaning.
					continue
				}
				if cur != prev {
					continue // pane advanced → agent alive (probably mid-bash)
				}
				// pane silent across ticks → fall through to flag
			}
			// checker error: degrade to Shape α — flag on last_seen alone.
		}

		flagStaleCoder(c, a, now)
	}
}

// flagStaleCoder emits the [STALE-CODER] PM gated by intentional-
// idle / recent-msg / LastSeen-advance / rate-cap. Identical gate
// semantics to gemma flagStaleAgent (pre-migration).
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
	paneBaseline = nil
	paneActivityImpl = nil
}
