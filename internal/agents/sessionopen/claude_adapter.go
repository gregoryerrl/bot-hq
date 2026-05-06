package sessionopen

import (
	"fmt"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

// FormatClaude returns the markdown system-prompt prepend for a Claude-class
// agent (Brian, Rain, Clive). Format per design-spike §2.2: section headers,
// section-scoped delimiters, BEGIN/END sentinels for agent parsing.
//
// The output is the final string the SessionStart hook writes to stdout.
func FormatClaude(p *Payload) string {
	var b strings.Builder
	openedAt := time.Now().UTC().Format(time.RFC3339)
	fmt.Fprintf(&b, "<!-- BOT-HQ SESSION-OPEN BEGIN project=%s agent=%s session_open_at=%s -->\n\n", p.Project, p.Agent, openedAt)

	if p.Overview != "" {
		b.WriteString("## Project overview\n\n")
		b.WriteString(strings.TrimSpace(p.Overview))
		b.WriteString("\n\n")
	}

	if p.Bootstrap != nil {
		b.WriteString("## Bootstrap (last session)\n\n")
		fm := p.Bootstrap.Frontmatter
		if fm.LastSessionID != "" {
			fmt.Fprintf(&b, "**Last session:** `%s` ", fm.LastSessionID)
		}
		if !fm.LastSessionCloseAt.IsZero() {
			fmt.Fprintf(&b, "(closed %s)", fm.LastSessionCloseAt.UTC().Format(time.RFC3339))
		}
		if fm.PhaseOrMilestone != "" {
			fmt.Fprintf(&b, "\n**Phase:** %s", fm.PhaseOrMilestone)
		}
		if fm.KeyState != "" {
			fmt.Fprintf(&b, "\n**Key state:** %s", fm.KeyState)
		}
		if fm.WriteTrigger == "stale" {
			b.WriteString("\n\n> WARNING: bootstrap is stale (>24h since last session-close).")
		}
		if fm.WriteTrigger != "" && fm.WriteTrigger != "graceful" {
			fmt.Fprintf(&b, "\n*Write trigger:* %s", fm.WriteTrigger)
		}
		b.WriteString("\n\n")
		if strings.TrimSpace(p.Bootstrap.Body) != "" {
			b.WriteString(strings.TrimSpace(p.Bootstrap.Body))
			b.WriteString("\n\n")
		}
	}

	if len(p.RulesResolved) > 0 {
		b.WriteString("## Resolved rules (general → project → agent)\n\n```yaml\n")
		out, _ := yaml.Marshal(p.RulesResolved)
		b.Write(out)
		b.WriteString("```\n\n")
	}

	if p.Tasks != nil && (len(p.Tasks.Tasks) > 0 || strings.TrimSpace(p.Tasks.Body) != "") {
		b.WriteString("## Active tasks\n\n")
		for _, t := range p.Tasks.Tasks {
			marker := taskMarker(t.Status)
			if t.Owner != "" {
				fmt.Fprintf(&b, "- %s **%s** — %s _(owner: %s)_\n", marker, t.ID, t.Title, t.Owner)
			} else {
				fmt.Fprintf(&b, "- %s **%s** — %s\n", marker, t.ID, t.Title)
			}
		}
		if strings.TrimSpace(p.Tasks.Body) != "" {
			b.WriteString("\n")
			b.WriteString(strings.TrimSpace(p.Tasks.Body))
			b.WriteString("\n")
		}
		b.WriteString("\n")
	}

	if p.Stats.OverHardCap {
		fmt.Fprintf(&b, "> WARNING: session-open payload (%d tokens) exceeds hard cap %d.\n\n", p.Stats.TotalTokens, HardTokenCap)
	}

	b.WriteString("<!-- BOT-HQ SESSION-OPEN END -->\n")
	return b.String()
}

func taskMarker(status string) string {
	switch status {
	case "done":
		return "[x]"
	case "in_progress":
		return "[~]"
	case "blocked":
		return "[!]"
	default:
		return "[ ]"
	}
}
