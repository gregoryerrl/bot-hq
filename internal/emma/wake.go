package emma

import (
	"log"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func (g *Emma) wakeDispatchLoop() {
	ticker := time.NewTicker(wakeDispatchInterval)
	defer ticker.Stop()
	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			g.dispatchWakes()
		}
	}
}

// dispatchWakes fires every pending wake whose fire_at has elapsed. Each row
// transitions to a terminal state (fired on hub_send success, failed on
// hub_send error) — drop-on-fail per arch lean 4 (no retry in v1; failed
// rows surface via the Emma log path). The terminal-flip uses
// MarkWakeFired/MarkWakeFailed which are pending-only UPDATEs, so a concurrent
// hub_cancel_wake racing the dispatch tick will leave the row in whichever
// state won the race without state-machine corruption.
//
// Phase H slice 3 C7 (#6 H-23 periodic invoker): wakes with target_agent
// prefixed `_internal:` route to in-process handlers via dispatchInternalWake
// instead of writing a hub_send. Each handler is responsible for re-arming
// itself.
func (g *Emma) dispatchWakes() {
	wakes, err := g.db.ListPendingWakes(time.Now())
	if err != nil {
		log.Printf("[wake] list pending: %v", err)
		return
	}
	for _, w := range wakes {
		if strings.HasPrefix(w.TargetAgent, internalTargetPrefix) {
			g.dispatchInternalWake(w)
			if _, err := g.db.MarkWakeFired(w.ID); err != nil {
				log.Printf("[wake] mark-fired errored (id=%d): %v", w.ID, err)
			}
			continue
		}
		msg := protocol.Message{
			FromAgent: agentID,
			ToAgent:   w.TargetAgent,
			Type:      protocol.MsgCommand,
			Content:   w.Payload,
			Created:   time.Now(),
		}
		if _, err := g.db.InsertMessage(msg); err != nil {
			log.Printf("[wake] dispatch failed (id=%d target=%s): %v", w.ID, w.TargetAgent, err)
			if _, mErr := g.db.MarkWakeFailed(w.ID); mErr != nil {
				log.Printf("[wake] mark-failed errored (id=%d): %v", w.ID, mErr)
			}
			continue
		}
		if _, err := g.db.MarkWakeFired(w.ID); err != nil {
			log.Printf("[wake] mark-fired errored (id=%d): %v", w.ID, err)
		}
	}
}

// internalTargetPrefix marks wake_schedule rows whose target is an in-
// process handler instead of a registered agent. Routed via
// dispatchInternalWake.
const internalTargetPrefix = "_internal:"

// internalDocDriftTarget is the target_agent value for the periodic H-23
// doc-drift sentinel scan. Each fire scans bot-hq's docs/arcs and emits
// observations via slice 2's EmitDocDriftObservations, then re-arms.
const internalDocDriftTarget = "_internal:docdrift"

// docDriftInterval is the cadence between H-23 doc-drift scans. 30 minutes
// is well below the typical arc-pointer-drift turnaround (a closed-arc
// pointer that should have been refined would otherwise sit stale until
// next manual catch).
const docDriftInterval = 30 * time.Minute

// dispatchInternalWake routes _internal:* wakes to in-process handlers.
// Each handler is responsible for re-arming itself; this dispatcher is
// pure routing. Unknown internal targets are logged and dropped (the row
// is still marked fired by the caller so it doesn't pile up as pending).
//
// Phase H slice 3 C7 (#6 H-23 periodic invoker).
func (g *Emma) dispatchInternalWake(w hub.WakeSchedule) {
	switch w.TargetAgent {
	case internalDocDriftTarget:
		g.runDocDriftScanAndReArm()
	default:
		log.Printf("[wake] unknown internal target: %s (id=%d)", w.TargetAgent, w.ID)
	}
}

// runDocDriftScanAndReArm scans bot-hq's docs/arcs for arc-pointer drift
// and emits observations to the slice 2 docdrift ledger via the existing
// EmitDocDriftObservations path. Re-arms the next wake unconditionally so
// the loop continues even if the scan itself errored.
//
// Best-effort: if cwd is not a bot-hq checkout (no docs/arcs/), the scan
// returns 0 observations and the re-arm still fires — the next tick is
// equally a no-op until the binary lands somewhere with arcs to scan.
func (g *Emma) runDocDriftScanAndReArm() {
	obs, err := ScanArcsForDocDrift("docs/arcs", ".")
	if err != nil {
		log.Printf("[wake] docdrift scan: %v", err)
	} else {
		EmitDocDriftObservations(obs)
	}
	next := time.Now().Add(docDriftInterval)
	if _, err := g.db.InsertWakeSchedule(internalDocDriftTarget, agentID, "", next); err != nil {
		log.Printf("[wake] docdrift re-arm: %v", err)
	}
}

// bootstrapInternalDocDrift schedules the first _internal:docdrift wake at
// Emma start if no pending one exists. Subsequent wakes are scheduled by
// runDocDriftScanAndReArm. Idempotent across rebuilds — skips if a
// pending wake from a prior boot is still in the table.
func (g *Emma) bootstrapInternalDocDrift() {
	pending, err := g.db.HasPendingWakeForTarget(internalDocDriftTarget)
	if err != nil {
		log.Printf("[wake] docdrift bootstrap pending check: %v", err)
		return
	}
	if pending {
		return
	}
	next := time.Now().Add(docDriftInterval)
	if _, err := g.db.InsertWakeSchedule(internalDocDriftTarget, agentID, "", next); err != nil {
		log.Printf("[wake] docdrift bootstrap: %v", err)
	}
}
