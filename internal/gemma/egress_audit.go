package gemma

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
)

// egressGapTickThreshold is how many consecutive monitorLoop ticks the
// pane-advanced-no-hub-msg condition must persist before emitting an
// EGRESS-GAP alert. At monitorLoop's 5min cadence, N=2 = 10min wall.
// Late-detection backstop role per Rain's BRAIN-second: item 3 Stop hook
// fires sub-second on turn-end; this catches the residual cases where
// the hook missed (env var absent, install drift, hook script bug).
//
// Long legitimate thinking turns (>10min silent rendering) can
// false-positive at N=2. If observed in v1 deploy, raise to N=3
// (15min). Tunable post-deploy without schema changes.
const egressGapTickThreshold = 2

// egressPaneTailLines is the pane-tail capture depth used for content
// hashing + alert-snippet excerpting. 30 lines covers the typical
// claude-code prompt-and-response visible region without paying for
// scrollback noise.
const egressPaneTailLines = 30

// egressPaneCaptureFn abstracts tmuxpkg.CapturePane so tests can inject
// a deterministic fake pane state without exec'ing real tmux.
type egressPaneCaptureFn func(target string, lines int) (string, error)

func defaultEgressPaneCapture(target string, lines int) (string, error) {
	return tmuxpkg.CapturePane(target, lines)
}

// auditEgressGap scans live agents (excluding Emma herself) for the
// pane→hub egress class: pane content has advanced since the last
// tick's baseline AND no new hub message from that agent has appeared
// since the baseline. The condition must persist for
// egressGapTickThreshold consecutive ticks before flagging — a single
// tick with pane-advance-no-msg is normal mid-thought, not a discipline
// gap.
//
// Folded into Emma's existing monitorLoop tick (5min cadence) per
// Rain-greenlit scheduler-split. Co-located with stale-detect by
// design intent.
func (g *Gemma) auditEgressGap() {
	g.auditEgressGapAt(time.Now())
}

// auditEgressGapAt is the testable variant.
func (g *Gemma) auditEgressGapAt(now time.Time) {
	if halted, _ := g.db.IsHalted(); halted {
		// Per H-31 halt-suppression precedent: do not flag during halt.
		return
	}
	agents, err := g.db.ListAgents("")
	if err != nil {
		return
	}
	capture := g.egressPaneCapture
	if capture == nil {
		capture = defaultEgressPaneCapture
	}

	g.egressMu.Lock()
	defer g.egressMu.Unlock()
	if g.egressBaseline == nil {
		g.egressBaseline = make(map[string]egressBaselineEntry)
	}
	if g.egressFlagTracker == nil {
		g.egressFlagTracker = make(map[string]struct{})
	}

	for _, a := range agents {
		if a.ID == agentID {
			continue
		}
		if a.Status != protocol.StatusOnline && a.Status != protocol.StatusWorking {
			continue
		}
		target := metaTmuxTarget(a.Meta)
		if target == "" {
			continue // no pane to inspect — skip Shape α-style fallback
		}
		paneOut, err := capture(target, egressPaneTailLines)
		if err != nil {
			continue
		}
		hash := paneContentHash(paneOut)

		// Did the agent emit any hub messages since baseline?
		prev, hasPrev := g.egressBaseline[a.ID]
		var sinceID int64
		if hasPrev {
			sinceID = prev.MaxMsgID
		}
		newMsgs, err := g.db.GetMessagesFromAgent(a.ID, sinceID, 10)
		if err != nil {
			continue
		}
		hubAdvanced := len(newMsgs) > 0
		var maxMsgID int64
		if hasPrev {
			maxMsgID = prev.MaxMsgID
		}
		for _, m := range newMsgs {
			if m.ID > maxMsgID {
				maxMsgID = m.ID
			}
		}

		paneAdvanced := hasPrev && hash != prev.Hash

		if !hasPrev {
			// First observation — establish baseline; defer flagging
			// until we have a prior tick to compare against.
			g.egressBaseline[a.ID] = egressBaselineEntry{Hash: hash, MaxMsgID: maxMsgID, GapTicks: 0}
			continue
		}

		gapTicks := prev.GapTicks
		if paneAdvanced && !hubAdvanced {
			gapTicks++
		} else {
			gapTicks = 0
		}

		// Hysteresis key includes pane hash so a NEW pane state re-arms
		// the alert rather than perma-suppressing on a stuck-shape
		// agent. Per Rain's caveat on Q4.
		flagKey := fmt.Sprintf("%s:%s", a.ID, hash)

		if gapTicks >= egressGapTickThreshold {
			if _, alreadyFlagged := g.egressFlagTracker[flagKey]; !alreadyFlagged {
				g.egressFlagTracker[flagKey] = struct{}{}
				snippet := lastNonEmptyLine(paneOut)
				if len(snippet) > 120 {
					snippet = snippet[:120] + "…"
				}
				g.db.InsertMessage(protocol.Message{
					FromAgent: agentID,
					Type:      protocol.MsgUpdate,
					Content:   fmt.Sprintf("[EGRESS-GAP] agent %s pane advanced over %d ticks but no hub_send. Last line: %q", a.ID, gapTicks, snippet),
				})
			}
		}

		g.egressBaseline[a.ID] = egressBaselineEntry{Hash: hash, MaxMsgID: maxMsgID, GapTicks: gapTicks}
	}
}

// egressBaselineEntry is the per-agent tick-to-tick state for the
// egress-gap auditor.
type egressBaselineEntry struct {
	Hash     string
	MaxMsgID int64
	GapTicks int
}

// paneContentHash returns a stable short identifier for the given pane
// output. Used as the state-tracker comparison key — equality means the
// pane has not advanced since the previous tick. Hash, not raw content,
// to keep the in-memory tracker bounded.
func paneContentHash(s string) string {
	sum := sha256.Sum256([]byte(s))
	return hex.EncodeToString(sum[:8])
}

// lastNonEmptyLine returns the last non-blank line of a pane capture,
// trimmed of trailing whitespace. Used for the EGRESS-GAP alert snippet
// so the user sees what the agent emitted that triggered the gap.
func lastNonEmptyLine(s string) string {
	lines := strings.Split(strings.TrimRight(s, "\n"), "\n")
	for i := len(lines) - 1; i >= 0; i-- {
		t := strings.TrimRight(lines[i], " \t")
		if t != "" {
			return t
		}
	}
	return ""
}
