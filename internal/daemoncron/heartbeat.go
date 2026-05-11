package daemoncron

// Heartbeat-ledger surface — Phase S S-1a-1 (programmatic-move from
// internal/emma/gemma.go:619 runHeartbeatLedger).
//
// Cadence: every 25 hub messages (heartbeatMsgInterval). Tick interval
// 30s mirrors gemma's healthLoop tick — small enough to detect the
// 25-msg threshold within ~1 cadence cycle in busy sessions.
//
// Content: state-anchor pulse (phase-doc / ratchet-ledger paths +
// latest-msg-id) emitted as MsgUpdate to brian + rain. Same defense-
// in-depth role as B1(v) CLAUDE.md Compact Instructions (static) and
// R20 BOOTSTRAP-ON-CONVERSATION-RESUME (reactive) — daemoncron is
// regular-cadence reinforcement.
//
// Dual-emit prevention: gemma.go runHeartbeatLedger checks
// daemoncron.IsRunning via the *Cron handle wired at gemma init;
// when daemoncron-online, gemma's emit-call-site short-circuits
// (interpretation (ii) per Rain msg 15796 PUSH-BACK A).

import (
	"fmt"
	"log"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// heartbeatMsgInterval matches gemma's existing constant; emit
	// fires when latest hub msg-id has advanced by >= this many since
	// the last fire.
	heartbeatMsgInterval = 25

	// heartbeatTickInterval is the surface's poll cadence — checks
	// the threshold once per tick. Mirrors gemma's healthLoop 30s.
	heartbeatTickInterval = 30 * time.Second

	// heartbeatAgentID is the FromAgent identifier used on emits.
	// Z-5h: was "emma" — the pre-Z-5h comment claimed daemon-side emit
	// vs emma-side emit was "invisible to consumers." That violated the
	// hub-messages-must-be-true principle: Emma's model was never
	// invoked for these pings, so signing them as "emma" lied to the
	// user reading the feed. Now "system" — accurate to the cadence-
	// fire surface (daemoncron). Recipients (brian/rain) recognize the
	// heartbeat via the unique "[HEARTBEAT-LEDGER]" content prefix, not
	// via FromAgent.
	heartbeatAgentID = "system"
)

// heartbeatState tracks the last-fired msg-id for cadence dedupe.
// Lives package-scoped + mu-guarded since the heartbeat surface fires
// from a single goroutine (own ticker); state is per-process.
var (
	heartbeatStateMu   sync.Mutex
	heartbeatLastMsgID int64
)

// heartbeatContentTemplate is the canonical content format. Kept in
// a function (not a const) so future-cycle template changes localize
// here. Must match gemma's pre-S-1a format byte-for-byte during
// dual-emit-prevention transition window so round-tripping via either
// emit-source produces identical hub-history.
func heartbeatContentTemplate(latestID int64) string {
	return fmt.Sprintf("[HEARTBEAT-LEDGER] msg-count cadence cycle (every %d msgs). State anchors: phase-doc=~/.bot-hq/projects/bot-hq/phase/<active>.md ratchet-ledger=~/.bot-hq/projects/bot-hq/ratchets/active.md latest-msg-id=%d. R20 AgentState write opportunity.", heartbeatMsgInterval, latestID)
}

// runHeartbeatLedgerSurface is the surfaceFunc for the heartbeat-
// ledger. Reads latest hub msg-id; emits if threshold crossed; no-op
// otherwise. Identical semantics to gemma's runHeartbeatLedger.
func runHeartbeatLedgerSurface(c *Cron) {
	recent, err := c.db.GetRecentMessages(1)
	if err != nil || len(recent) == 0 {
		return
	}
	latestID := recent[0].ID

	heartbeatStateMu.Lock()
	if latestID-heartbeatLastMsgID < heartbeatMsgInterval {
		heartbeatStateMu.Unlock()
		return
	}
	heartbeatLastMsgID = latestID
	heartbeatStateMu.Unlock()

	content := heartbeatContentTemplate(latestID)
	// Z-5h: single broadcast emit (ToAgent="") replaces per-recipient
	// PM emits. Phase S S-4 dropped PM; brian + rain both poll the
	// broadcast channel and recognize the "[HEARTBEAT-LEDGER]" content
	// prefix in their R20-bootstrap path.
	if _, err := c.db.InsertMessage(protocol.Message{
		FromAgent: heartbeatAgentID,
		ToAgent:   "",
		Type:      protocol.MsgUpdate,
		Content:   content,
	}); err != nil {
		log.Printf("[daemoncron heartbeat-ledger] insert failed: %v", err)
	}
}

// ResetHeartbeatStateForTest clears the package-scoped heartbeat
// dedupe state. Test-only: avoids cross-test cadence leak when a
// fixture inserts a high msg-id in one test then a fresh DB in the
// next. Tests must call this in setup.
func ResetHeartbeatStateForTest() {
	heartbeatStateMu.Lock()
	defer heartbeatStateMu.Unlock()
	heartbeatLastMsgID = 0
}
