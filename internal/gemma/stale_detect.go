package gemma

import (
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
)

// staleThreshold is the last_seen-age cutoff after which Emma considers an
// agent's MCP heartbeat stale and falls back to the tmux pane backup signal
// (Shape γ hybrid). 5min comfortably exceeds the longest legitimate Bash
// timeout (600s) used by coders without alarming on idle quiet periods, and
// matches the design-doc lock for slice 3 C4.
const staleThreshold = 5 * time.Minute

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
	if active, _, _ := g.db.IsHaltActive(); active {
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

// flagStaleAgent emits the [STALE-CODER] PM to Rain, gated by shouldFlag's
// existing 30min hysteresis on key "stale-coder:<id>" so the same agent
// across multiple ticks produces a single PM until the window elapses.
func (g *Gemma) flagStaleAgent(a protocol.Agent, now time.Time) {
	key := "stale-coder:" + a.ID
	if !g.shouldFlag(key, now) {
		return
	}
	age := now.Sub(a.LastSeen).Round(time.Second)
	g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   fmt.Sprintf("[STALE-CODER] agent %s last_seen=%s ago, no pane activity since last tick", a.ID, age),
	})
}
