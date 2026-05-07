package emma

// Phase S S-1b emma-Claude formatNudge — hub→pane formatter.
//
// Mirrors brian/rain formatNudge contract but with emma-specific
// nudge-shape: emma observes traffic for rule-violations so render
// includes FromAgent + ToAgent + Type for full audit-context.
// Speech-trigger gating happens at LLM-side (rule-text discipline);
// formatNudge is purely render-layer.

import (
	"fmt"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// formatNudge builds the compact tag emma's session reads.
// Format:
//
//	[HUB:<sender>] content                — broadcast (ToAgent="")
//	[HUB:<sender>→<target>] content       — directed (ToAgent set)
//	[HUB:FLAG:<sender>] content           — MsgFlag class (broadcast)
//	[HUB:FLAG:<sender>→<target>] content  — MsgFlag class (directed)
//
// Note: emma-Claude renders FROM-AGENT verbatim — unlike brian/rain
// formatNudge which strips sender on [HR]/FLAG per Phase R R2
// authorless-display. Emma's job is to OBSERVE who said what for
// rule-violation detection, so attribution is load-bearing.
func formatNudge(msg protocol.Message) string {
	from := msg.FromAgent
	if from == "" {
		from = "unknown"
	}
	prefix := "HUB"
	if msg.Type == protocol.MsgFlag {
		prefix = "HUB:FLAG"
	}
	var senderTag string
	if msg.ToAgent != "" {
		senderTag = fmt.Sprintf("%s:%s→%s", prefix, from, msg.ToAgent)
	} else {
		senderTag = fmt.Sprintf("%s:%s", prefix, from)
	}
	content := strings.TrimSpace(msg.Content)
	return fmt.Sprintf("[%s] %s", senderTag, content)
}
