// Package emma supplies the gemma-harness-specific SessionStart adapter
// for Emma (heartbeat-ledger emitter + plan-usage poller). See design-spike
// §2.5 — Emma's harness has tighter context budget (~32K total) and a
// different prompt-prefix mechanism than Claude's SessionStart hook.
//
// Compression strategy:
//   - drop full rules_resolved body (Emma is poll-only-emit-only; only
//     role + heartbeat-cadence lines matter)
//   - drop tasks body (Emma doesn't execute tasks)
//   - keep overview only (cross-restart resume happens via R20 last_state.json
//     + R16 bootstrap from active phase-doc / ratchet ledger / hub backlog —
//     no separate bootstrap.md surface in the post-Phase-V world)
//   - target ~800 tokens total vs Claude's ~2000
package emma

import (
	"fmt"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/agents/sessionopen"
)

// EmmaTokenTarget is the soft target for the Emma-formatted prepend.
const EmmaTokenTarget = 800

// FormatSessionOpen returns the gemma-formatted prompt-prefix string.
// Compresses each section more aggressively than the Claude adapter and
// drops sections Emma doesn't consume.
func FormatSessionOpen(p *sessionopen.Payload) string {
	var b strings.Builder

	openedAt := time.Now().UTC().Format(time.RFC3339)
	fmt.Fprintf(&b, "[bot-hq session-open project=%s agent=%s opened=%s]\n\n", p.Project, p.Agent, openedAt)

	// Overview — first paragraph only (compressed).
	if p.Overview != "" {
		first := firstParagraph(p.Overview, 60)
		fmt.Fprintf(&b, "Project: %s\n", strings.TrimSpace(first))
	}

	// Rules — extract only role + heartbeat-relevant exec subset (not full map).
	if agentMap, ok := p.RulesResolved["agent"].(map[string]any); ok {
		if role, ok := agentMap["role"].(string); ok && role != "" {
			fmt.Fprintf(&b, "Role: %s\n", truncate(role, 100))
		}
		if exec, ok := agentMap["exec"].(map[string]any); ok {
			for _, key := range []string{"hubWrites", "fileWrites"} {
				if v, ok := exec[key].(string); ok && v != "" {
					fmt.Fprintf(&b, "exec.%s: %s\n", key, truncate(v, 80))
				}
			}
		}
	}

	// Tasks — count only (no body).
	if p.Tasks != nil && len(p.Tasks.Tasks) > 0 {
		fmt.Fprintf(&b, "Active tasks: %d\n", len(p.Tasks.Tasks))
	}

	b.WriteString("\n[end session-open]\n")
	return b.String()
}

// firstParagraph returns up to maxWords of the first non-empty paragraph.
func firstParagraph(s string, maxWords int) string {
	for _, para := range strings.Split(s, "\n\n") {
		para = strings.TrimSpace(para)
		// Skip markdown headers — first prose paragraph wins.
		if strings.HasPrefix(para, "#") {
			continue
		}
		if para == "" {
			continue
		}
		words := strings.Fields(para)
		if len(words) > maxWords {
			words = words[:maxWords]
			return strings.Join(words, " ") + "..."
		}
		return strings.Join(words, " ")
	}
	return ""
}

// truncate clips s to at most maxChars (with "..." marker if cut).
func truncate(s string, maxChars int) string {
	if len(s) <= maxChars {
		return s
	}
	if maxChars < 4 {
		return s[:maxChars]
	}
	return s[:maxChars-3] + "..."
}
