package gemma

import (
	"fmt"
	"log"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
)

// contextCapThreshold is the per-agent UsagePct value at or above which Emma
// fires the context-cap halt-flag. 95% leaves a small runway for the trio to
// finish the current tool call, post a close-SNAP via H-15, and idle before
// auto-compact would otherwise trigger.
const contextCapThreshold = 95

// contextCapResetThreshold is the value below which the per-agent hysteresis
// is allowed to re-arm. The 95/85 gap prevents flapping when a session
// hovers in the squeeze band; once usage drops below 85% the next squeeze
// past 95% may fire a fresh halt.
const contextCapResetThreshold = 85

// haltReasonPrefix is the literal substring that brian/rain STARTUP prompts
// match against (`^agent .* at \d+%, halt`). Any reformat of this string MUST
// be mirrored in the prompt convention or the halt-all-work contract breaks
// silently — agents would receive the FLAG but not recognize it as the halt
// trigger.
const haltReasonPrefix = "agent %s at %d%%, halt + checkpoint via H-15 + idle for fresh session"

// paneSnapshotFn returns the current per-agent panestate snapshot. Abstracted
// as a function field so context-cap tests inject a fixed slice without
// constructing a real panestate.Manager.
type paneSnapshotFn func() []panestate.AgentSnapshot

// initContextCapDefault wires the default paneSnapFn to a panestate.Manager
// backed by the hub DB + real tmux.CapturePane. Lazy-init: tests override
// before Start() runs and skip this path entirely.
func (g *Gemma) initContextCapDefault() {
	if g.paneSnapFn != nil {
		return
	}
	mgr := panestate.NewManager(g.db, tmuxpkg.CapturePane)
	g.paneSnapFn = func() []panestate.AgentSnapshot {
		if err := mgr.Refresh(); err != nil {
			return nil
		}
		return mgr.Snapshot()
	}
}

// checkContextCap is the per-tick H-31 entry point. Walks the panestate
// snapshot for non-emma agents whose UsagePct is at or above
// contextCapThreshold; fires hub_flag(severity=critical) AND sets halt_state
// active when hysteresis allows AND the halt is not already active.
//
// Halt-suppression check first (belt-and-suspenders alongside hysteresis):
// the C6 hysteresis is keyed per-agent on `context-cap:<id>`, but a quiet
// halt would still let a NEW agent (different id, same squeeze) double-fire
// before the existing halt is cleared. The IsHaltActive gate collapses any
// in-flight squeeze into the single in-progress halt window.
func (g *Gemma) checkContextCap(now time.Time) {
	if g.paneSnapFn == nil {
		return
	}
	active, _, err := g.db.IsHaltActive()
	if err != nil {
		log.Printf("[context-cap] halt status check failed: %v", err)
		return
	}
	if active {
		return
	}

	snap := g.paneSnapFn()
	for _, s := range snap {
		if s.ID == agentID {
			continue
		}
		// Hysteresis reset: drop below contextCapResetThreshold re-arms the
		// per-agent flag history so a fresh squeeze past 95% can fire again
		// after a successful fresh-session restart.
		if s.UsagePct >= 0 && s.UsagePct < contextCapResetThreshold {
			g.resetContextCapHysteresis(s.ID)
			continue
		}
		if s.UsagePct < contextCapThreshold {
			continue
		}
		key := "context-cap:" + s.ID
		if !g.shouldFlag(key, now) {
			continue
		}
		reason := fmt.Sprintf(haltReasonPrefix, s.ID, s.UsagePct)
		content := fmt.Sprintf("[CRITICAL] %s", reason)
		if _, err := g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   "user",
			Type:      protocol.MsgFlag,
			Content:   content,
			Created:   now,
		}); err != nil {
			log.Printf("[context-cap] flag insert failed for %s: %v", s.ID, err)
			continue
		}
		if err := g.db.SetHaltActive(agentID, reason); err != nil {
			log.Printf("[context-cap] set halt active failed: %v", err)
		}
	}
}

// resetContextCapHysteresis clears the per-agent flag-history entry so the
// next squeeze past 95% is allowed to fire even if it falls inside the
// 30-min hysteresis window of the previous fire.
func (g *Gemma) resetContextCapHysteresis(agentID string) {
	g.flagMu.Lock()
	defer g.flagMu.Unlock()
	delete(g.flagHistory, "context-cap:"+agentID)
}
