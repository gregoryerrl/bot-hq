package gemma

import (
	"fmt"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func (g *Gemma) OnHubMessageReplay(msg protocol.Message) {
	if msg.FromAgent == agentID {
		return
	}
	d := SentinelMatch(msg)
	if !d.Match {
		return
	}
	// Source-filter — keep replay-path symmetric with OnHubMessage so
	// boot-replay does not write prose-FPs into the dry-run ledger.
	// Registered-agent hub_send is process-self-tool-call (agent-prose),
	// not a crash-report channel; see OnHubMessage for the architectural
	// rationale + empirical anchor (hub.db 14/14 FP query 2026-04-28).
	if g.isFromRegisteredAgent(msg.FromAgent) {
		return
	}
	if d.AlwaysFlag {
		return
	}
	if name, isDryRun := IsDryRunPattern(d.Pattern); isDryRun {
		AppendToDryRunLedger(name, fmt.Sprintf("msg #%d from %s | pattern %s | %s", msg.ID, msg.FromAgent, d.Pattern, summarizeOutput(msg.Content, 200)))
		return
	}
}

// replayThroughSentinel feeds the last replayBacklog hub messages
// through OnHubMessageReplay at boot. Catches active failures from the
// pre-restart window without requiring a separate silence-poll path
// (deferred to post-Phase-F per locked msg 2442).
//
// Uses OnHubMessageReplay (silent variant) instead of OnHubMessage to
// prevent boot-replay from arming hysteresis for live post-boot
// triggers (Phase H slice 3 #4).
//
// Advances lastSentinelMsgID past the replayed window so the
// sentinelPollLoop tick doesn't re-process the same messages. Cross-
// bounce dedup of ledger entries is handled by AppendToDryRunLedger's
// msg-id parse.
func (g *Gemma) replayThroughSentinel() {
	msgs, err := g.db.GetRecentMessages(replayBacklog)
	if err != nil {
		return
	}
	var maxID int64
	for _, m := range msgs {
		g.OnHubMessageReplay(m)
		if m.ID > maxID {
			maxID = m.ID
		}
	}
	g.sentinelMu.Lock()
	if maxID > g.lastSentinelMsgID {
		g.lastSentinelMsgID = maxID
	}
	g.sentinelMu.Unlock()
}
