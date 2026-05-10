// Phase W lookback: render a session manifest as agent-friendly
// retrospective markdown.
//
// Audience: the trio (brian/rain/emma). User said they won't typically
// invoke lookback themselves; the trio reads + reasons over the output.
// Optimized for that — leads with the outcome narrative + structured
// fields so the agent can scan in O(seconds) and pick up the patterns
// retro requires.

package sessions

import (
	"fmt"
	"os"
	"strings"
	"time"
)

// Lookback returns a markdown blob rendering the session manifest as a
// retrospective view. The manifest must already exist; missing-id
// returns os-class error (caller can check os.IsNotExist).
//
// Format optimized for agent consumption: outcome first, then
// structured fields, then raw body. Section headers are stable across
// renders so the agent can pattern-match without surprises.
func Lookback(id string) (string, error) {
	m, err := ReadManifest(id)
	if err != nil {
		return "", err
	}
	return RenderLookback(m), nil
}

// RenderLookback formats a Manifest as the lookback markdown blob.
// Pure function over Manifest — no IO. Useful for in-memory tests
// and for future callers that already hold a Manifest (e.g., the EOD
// aggregator can use this per-session).
func RenderLookback(m Manifest) string {
	var b strings.Builder
	fmt.Fprintf(&b, "# Session retrospective: %s\n\n", m.ID)

	// Top-level metadata block
	if m.Project != "" {
		fmt.Fprintf(&b, "**Project:** %s\n", m.Project)
	}
	status := m.Status
	if status == "" {
		if m.EndTS.IsZero() {
			status = "active"
		} else {
			status = "closed"
		}
	}
	fmt.Fprintf(&b, "**Status:** %s\n", status)

	if !m.StartTS.IsZero() {
		startStr := m.StartTS.UTC().Format(time.RFC3339)
		if !m.EndTS.IsZero() {
			endStr := m.EndTS.UTC().Format(time.RFC3339)
			dur := m.EndTS.Sub(m.StartTS).Round(time.Minute)
			fmt.Fprintf(&b, "**Time:** %s → %s (%s)\n", startStr, endStr, dur)
		} else {
			fmt.Fprintf(&b, "**Time:** %s → (still active)\n", startStr)
		}
	}

	if m.StartMsgID > 0 || m.EndMsgID > 0 {
		fmt.Fprintf(&b, "**Hub msg-id range:** %d → %d", m.StartMsgID, m.EndMsgID)
		if m.MsgCount > 0 {
			fmt.Fprintf(&b, " (%d messages)", m.MsgCount)
		}
		b.WriteString("\n")
	}

	if len(m.Agents) > 0 {
		fmt.Fprintf(&b, "**Agents:** %s\n", strings.Join(m.Agents, ", "))
	}

	if m.Phase != "" || m.ActiveWorkstream != "" {
		fmt.Fprintf(&b, "**Phase / workstream:** %s / %s\n",
			fallback(m.Phase, "—"), fallback(m.ActiveWorkstream, "—"))
	}

	b.WriteString("\n")

	// Outcome narrative — leads because it's the highest-signal field
	// for retrospective. Agent reads this first to understand WHAT
	// happened; structured fields back the narrative with mechanical
	// evidence.
	if m.Outcome != "" {
		b.WriteString("## Outcome\n\n")
		b.WriteString(m.Outcome)
		if !strings.HasSuffix(m.Outcome, "\n") {
			b.WriteString("\n")
		}
		b.WriteString("\n")
	}

	// Structured fields — secondary to outcome but enable cross-session
	// queries when agent does multi-session retro.
	hasStructured := len(m.Decisions) > 0 || len(m.CommitsLanded) > 0 || len(m.FilesTouched) > 0
	if hasStructured {
		b.WriteString("## Structured fields\n\n")

		if len(m.Decisions) > 0 {
			fmt.Fprintf(&b, "### Decisions (%d)\n\n", len(m.Decisions))
			for _, d := range m.Decisions {
				fmt.Fprintf(&b, "- %s\n", d)
			}
			b.WriteString("\n")
		}

		if len(m.CommitsLanded) > 0 {
			fmt.Fprintf(&b, "### Commits landed (%d)\n\n", len(m.CommitsLanded))
			for _, sha := range m.CommitsLanded {
				fmt.Fprintf(&b, "- `%s`\n", shortSHA(sha))
			}
			b.WriteString("\n")
		}

		if len(m.FilesTouched) > 0 {
			fmt.Fprintf(&b, "### Files touched (%d)\n\n", len(m.FilesTouched))
			for _, f := range m.FilesTouched {
				fmt.Fprintf(&b, "- `%s`\n", f)
			}
			b.WriteString("\n")
		}
	}

	// Body free-form content — last because it's open-ended; agent
	// reads after the outcome+structured-fields scan to pick up
	// purpose / mode / checkpoint notes.
	if m.Body != "" {
		b.WriteString("## Manifest body\n\n")
		b.WriteString(m.Body)
		if !strings.HasSuffix(m.Body, "\n") {
			b.WriteString("\n")
		}
	}

	// Cite-anchor footer
	fmt.Fprintf(&b, "\n_Source: %s_\n", ManifestPath(m.ID))

	return b.String()
}

func fallback(s, otherwise string) string {
	if s == "" {
		return otherwise
	}
	return s
}

// shortSHA returns the first 12 chars of a git SHA when the input
// looks like one (≥40 hex chars), otherwise returns it verbatim.
// Cleans up Commits-landed display without losing readability.
func shortSHA(sha string) string {
	if len(sha) >= 40 && allHex(sha) {
		return sha[:12]
	}
	return sha
}

func allHex(s string) bool {
	for _, r := range s {
		switch {
		case r >= '0' && r <= '9':
		case r >= 'a' && r <= 'f':
		case r >= 'A' && r <= 'F':
		default:
			return false
		}
	}
	return true
}

// LookbackErrIsMissing reports whether err from Lookback indicates
// the session manifest doesn't exist. Wraps os.IsNotExist for callers
// that want to give friendlier "session-id not found" messages.
func LookbackErrIsMissing(err error) bool {
	return os.IsNotExist(err)
}
