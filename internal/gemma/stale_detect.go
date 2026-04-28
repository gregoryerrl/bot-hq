package gemma

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
)

// staleThreshold is the last_seen-age cutoff after which Emma considers an
// agent's MCP heartbeat stale and falls back to the tmux pane backup signal
// (Shape γ hybrid).
//
// Bumped from 5min → 10min in Phase I W1b (I-6 fix). Driver: compact-pipe
// peer-coordination mode produces 4-6min thinking-windows between hub_send
// tool calls (BRAIN-cycle reasoning chains, peer-message reading + drafting
// in compact format). 5min was tuned for prose-cadence; compact-mode
// routinely exceeded it, generating FP stale-fires while agents were
// actively reasoning. 10min comfortably accommodates compact-mode thinking
// while still flagging genuinely-silent agents within the next tick window
// (real-incident detection latency now ~10min vs prior ~5min — acceptable
// trade for the FP-rate reduction).
const staleThreshold = 10 * time.Minute

// userHaltPatternRe matches the canonical user-HALT directive shape in
// hub message content. Used by checkStaleAgentsAt (Phase I W1b I-6 C1) to
// suppress stale-checks while the trio is intentionally halted by user
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

// checkStaleAgents scans live agents (online + working) for stale heartbeats
// using the Shape γ hybrid: implicit MCP last_seen as primary, tmux pane
// activity as backup for agents with tmux_target Meta. Called from Emma's
// healthLoop tick — no new ticker required.
//
// Two-signal AND: only flag when last_seen has aged past staleThreshold AND
// the pane has produced no new output since the previous tick's baseline.
// First observation per agent establishes baseline without flagging — a
// real "no activity since last tick" signal needs at least one prior tick
// to compare against.
//
// Agents without tmux_target Meta (future webhook/voice agents) fall back
// to Shape α: flag on last_seen alone, since no pane backup is available.
func (g *Gemma) checkStaleAgents() {
	g.checkStaleAgentsAt(time.Now())
}

// checkStaleAgentsAt is the testable variant: callers inject a virtual now
// so tests can advance simulated time past staleThreshold without sleeping
// or backdating database rows.
func (g *Gemma) checkStaleAgentsAt(now time.Time) {
	// Phase H slice 4 C6 (H-31): halt-state suppresses ALL H-3a stale-fires
	// during an active halt. Halt = all-hands-idle by convention; agents
	// finishing their final tool call before posting close-SNAP would
	// otherwise read as stale on this tick. The early-return drops the
	// tmux_target qualifier per BRAIN P1 fold rationale: selective
	// suppression leaks alerts on non-pane agents (Emma, voice/discord)
	// during the halt window.
	if halted, _ := g.db.IsHalted(); halted {
		return
	}

	// Phase I W1b I-6 C1: user-HALT-directive suppresses stale-checks. The
	// halt_state path above only catches Emma-initiated halts (context-cap
	// 95%); user-typed `[HUB:user] HALT` is a content-only signal that
	// doesn't transition halt_state. Surgical fix: check the latest user
	// message; if it's a HALT directive, suppress until user emits a
	// non-HALT directive (latest-message check naturally clears once user
	// proceeds). Errors fail-open — if the query fails, fall through to
	// normal stale-check logic rather than silently disabling detection.
	if last, ok, err := g.db.GetLatestMessageFrom("user"); err == nil && ok && userHaltPatternRe.MatchString(last.Content) {
		return
	}
	agents, err := g.db.ListAgents("")
	if err != nil {
		return
	}
	checker := g.paneActivity
	if checker == nil {
		checker = defaultPaneActivity
	}

	g.staleMu.Lock()
	defer g.staleMu.Unlock()
	if g.paneBaseline == nil {
		g.paneBaseline = make(map[string]int64)
	}

	for _, a := range agents {
		if a.ID == agentID {
			continue
		}
		if a.Status != protocol.StatusOnline && a.Status != protocol.StatusWorking {
			continue
		}
		if now.Sub(a.LastSeen) <= staleThreshold {
			continue
		}

		target := metaTmuxTarget(a.Meta)
		if target != "" {
			cur, perr := checker(target)
			if perr == nil {
				prev, seen := g.paneBaseline[a.ID]
				g.paneBaseline[a.ID] = cur
				if !seen {
					// First observation — establish baseline; defer flagging
					// to the next tick so "since last tick" has meaning.
					continue
				}
				if cur != prev {
					continue // pane advanced → agent alive (probably mid-bash)
				}
				// pane silent across ticks → fall through to flag
			}
			// checker error: degrade to Shape α — flag on last_seen alone.
		}
		g.flagStaleAgent(a, now)
	}
}

// flagStaleAgent emits the [STALE-CODER] PM to Rain, gated by the per-agent
// LastSeen advance-check (lean (b)): re-firing is suppressed when LastSeen
// has not advanced since the most recent stale-coder flag for this agent.
// "Stale" is sticky once observed — the agent is either dead (won't recover)
// or intentional-idle (covered upstream by halt-state). A new flag fires
// only when LastSeen advances past the tracked value, which captures the
// meaningful event: "agent came back, then went stale again."
//
// Phase I W1b I-6 (B): hub.db backstop check. Before flagging, query for
// any message from the agent in the last staleThreshold window. If the
// agent has hub_send activity in the window, suppress the flag — defends
// against the LastSeen-write-failure-race class where UpdateAgentLastSeen
// silently failed (writes are best-effort per mcp/tools.go withLastSeen).
// Errors fail-open — query failures fall through to flag, since silent
// suppression on DB error is worse than an FP.
//
// Caller (checkStaleAgentsAt) holds g.staleMu, so direct map access is safe.
func (g *Gemma) flagStaleAgent(a protocol.Agent, now time.Time) {
	if g.staleFlagTracker == nil {
		g.staleFlagTracker = make(map[string]time.Time)
	}
	if last, seen := g.staleFlagTracker[a.ID]; seen && last.Equal(a.LastSeen) {
		return // LastSeen unchanged → same incident → suppress
	}
	if hasRecent, err := g.db.HasRecentMessageFrom(a.ID, now.Add(-staleThreshold)); err == nil && hasRecent {
		// Recent hub_send proves agent active despite stale LastSeen —
		// LastSeen-write-failure-race likely. Suppress flag.
		return
	}
	g.staleFlagTracker[a.ID] = a.LastSeen
	age := now.Sub(a.LastSeen).Round(time.Second)
	g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   fmt.Sprintf("[STALE-CODER] agent %s last_seen=%s ago, no pane activity since last tick", a.ID, age),
	})
}
