// Package snap implements the SNAP footer protocol used by bot-hq orchestrator
// agents. SNAP is a 4-field block appended to substantive replies, compressing
// multi-artifact state into grep-able lines so a reader can pick up cold
// without parsing the body. The canonical writer convention lives in user
// memory (feedback_snap_footer.md); the protocol contract lives here and in
// docs/phase-g/slice-2/arc.md.
//
// v1 escape mechanism: paren-depth. Commas inside parentheses are part of the
// surrounding item, not a list separator. No quoting, no backslash. If format
// strain emerges (commas in non-paren contexts, paren-as-literal), v2+ adds
// explicit quoting — do not backdoor it into v1.
package snap

import "strings"

// SNAP is the typed form of the 4-field footer.
//
// List fields (Branches, Agents) are stored as opaque strings. v1 does not
// normalize sub-shape (branch@sha(state), id(state)); query workload has not
// justified the cost. Defer normalization to a later slice if needed.
type SNAP struct {
	Branches []string `json:"branches"`
	Agents   []string `json:"agents"`
	Pending  string   `json:"pending"`
	Next     string   `json:"next"`
}

const (
	prefixBranches = "Branches: "
	prefixAgents   = "Agents:   "
	prefixPending  = "Pending:  "
	prefixNext     = "Next:     "
)

// Format serializes a SNAP into canonical wire form. The output round-trips
// through Parse: Parse(Format(s)) equals s for any well-formed s.
//
// Format does NOT emit a trailing newline. Callers appending the SNAP block
// to a message body must supply their own separator (typically "\n\n" before
// the block) and any trailing newline. This keeps the canonical form a
// self-contained substring rather than a line-terminated record.
func (s SNAP) Format() string {
	var b strings.Builder
	b.WriteString("SNAP:\n")
	b.WriteString(prefixBranches)
	b.WriteString(strings.Join(s.Branches, ", "))
	b.WriteByte('\n')
	b.WriteString(prefixAgents)
	b.WriteString(strings.Join(s.Agents, ", "))
	b.WriteByte('\n')
	b.WriteString(prefixPending)
	b.WriteString(s.Pending)
	b.WriteByte('\n')
	b.WriteString(prefixNext)
	b.WriteString(s.Next)
	return b.String()
}
