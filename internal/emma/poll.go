package emma

// Phase S S-1b emma-Claude polling + message forwarding.
//
// Polls hub.DB for new messages + nudges emma's pane via tmux. Mirrors
// brian/rain pattern but with looser forwarding criteria — emma sees
// (almost) everything because her job is to watch all hub traffic for
// rule-violations. The speech-trigger gating lives in rule-text
// discipline (system prompt) — emma decides at LLM-time whether to
// emit; daemon-side this just forwards.

import (
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
)

// pollLoop ticks pollInterval + reads new hub messages + nudges
// emma's pane. Mirrors brian/rain pattern.
func (e *Emma) pollLoop() {
	ticker := time.NewTicker(pollInterval)
	defer ticker.Stop()
	for {
		select {
		case <-e.stopCh:
			return
		case <-ticker.C:
			e.processNewMessages()
		}
	}
}

// processNewMessages reads all new hub-msgs + nudges emma's pane with
// a single batched line. Filtering via shouldForwardToEmma.
func (e *Emma) processNewMessages() {
	msgs, err := e.db.ReadMessages("", e.lastMsgID, 50)
	if err != nil {
		return
	}
	var pending []string
	for _, msg := range msgs {
		if msg.ID > e.lastMsgID {
			e.lastMsgID = msg.ID
		}
		if shouldForwardToEmma(msg) {
			pending = append(pending, formatNudge(msg))
		}
	}
	if len(pending) == 0 {
		return
	}
	nudge := strings.Join(pending, "\n")
	tmuxpkg.SendKeys(e.tmuxSession, nudge, true)
}

// shouldForwardToEmma decides whether a hub message should be nudged
// into emma's pane. Emma watches all hub traffic for rule-violations;
// forwarding is broader than brian/rain (who filter peer-coord
// chatter). Emma sees: everything except her own messages.
//
// Self-skip prevents feedback loops. Per Phase S S-1b user msg 15734
// "watching you guys" — emma observes peer agents' traffic to catch
// violations.
func shouldForwardToEmma(msg protocol.Message) bool {
	if msg.FromAgent == agentID {
		return false
	}
	return true
}
