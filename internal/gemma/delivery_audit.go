package gemma

import (
	"fmt"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// deliveryGapAge is the wall-clock age at which a still-pending queued
// message becomes "user-perceptible lag" and warrants a [DELIVERY-GAP]
// alert. Anchored to user-noticing, not to retry-mechanism MaxAttempts —
// if retry behavior is later tuned (3s interval, 30 max), the alert
// threshold should not drift, so we measure created-age rather than
// attempt count.
//
// 60s is the slice-5 v1 lean per Rain's BRAIN-second on the Q1
// signal-choice push-back: K-attempt-based was an indirect proxy for
// what users actually feel as lag.
const deliveryGapAge = 60 * time.Second

// auditDeliveryGap scans the message_queue for pending messages older
// than deliveryGapAge and emits a [DELIVERY-GAP] broadcast for each
// newly-aged entry. Predictive of retry-exhaust (item 6 catches the
// terminal case at attempt 30 ≈ 90s; this catches mid-stall at 60s).
//
// Hysteresis: a message_queue row's queue-id is the dedup key. Once
// flagged, the same row is suppressed forever — a stalled message
// either drains (status flips to delivered, no longer pending) or
// exhausts (item 6's bridge fires its own terminal alert). Periodic
// re-fire on the same pending row would just spam.
//
// Folded into Emma's existing healthLoop tick (30s cadence) per
// Rain-greenlit scheduler-split — no new ticker plumbing.
func (g *Gemma) auditDeliveryGap() {
	g.auditDeliveryGapAt(time.Now())
}

// auditDeliveryGapAt is the testable variant. Tests inject a virtual
// `now` so they can establish queue rows with a deterministic age
// without sleeping or backdating SQL.
func (g *Gemma) auditDeliveryGapAt(now time.Time) {
	pending, err := g.db.GetPendingMessages()
	if err != nil {
		return
	}
	cutoff := now.Add(-deliveryGapAge)

	g.deliveryFlagMu.Lock()
	defer g.deliveryFlagMu.Unlock()
	if g.deliveryFlagTracker == nil {
		g.deliveryFlagTracker = make(map[int64]struct{})
	}

	for _, qm := range pending {
		if !qm.Created.Before(cutoff) {
			continue
		}
		if _, seen := g.deliveryFlagTracker[qm.ID]; seen {
			continue
		}
		g.deliveryFlagTracker[qm.ID] = struct{}{}
		age := now.Sub(qm.Created).Round(time.Second)
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			Type:      protocol.MsgUpdate,
			Content:   fmt.Sprintf("[DELIVERY-GAP] msg %d to %s pending for %s (queue-id %d, %d attempts)", qm.MessageID, qm.TargetAgent, age, qm.ID, qm.Attempts),
		})
	}

	// Prune flag-tracker entries whose underlying queue rows are no
	// longer pending. Without this, the tracker grows unbounded as
	// messages drain or exhaust. Cheap: O(tracker_size) per tick.
	pendingIDs := make(map[int64]struct{}, len(pending))
	for _, qm := range pending {
		pendingIDs[qm.ID] = struct{}{}
	}
	for id := range g.deliveryFlagTracker {
		if _, ok := pendingIDs[id]; !ok {
			delete(g.deliveryFlagTracker, id)
		}
	}
}

