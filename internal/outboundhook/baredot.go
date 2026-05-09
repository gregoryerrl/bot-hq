// Package outboundhook — baredot.go: R50 MECHANICAL-BLOCK-ON-BARE-DOT
// per phase-t.md v5 T-2.5.
//
// Extends the existing R36 OUTBOUND-DISCIPLINE-MECHANICAL Stop-hook with
// bare-"." pattern detection + minimal-content patterns ("Standing.",
// "Idle.", ".") that are commonly emitted as silent-commitment ack-loops
// when an agent's last hub_send was earlier in the turn but the agent
// emits residual pane-text without re-wrapping in hub_send.
//
// Empirical justification per phase-t.md v5:
//   "HEARTBEAT-LOOP-ANTIPATTERN structurally produced by bilateral
//    architecture; Phase Q msg 15329/15331 bilateral 'Idle.' pane text
//    without hub_send during halt-handoff" (discipline-log.md:493+1078)
//
// Multiple this-session R36-self-trip instances on minimal-pane-text
// without hub_send wrap.
//
// R-rule-graduation success-criteria: R50 graduates if heartbeat-loop
// antipattern frequency reduces to <5% of turns post-mechanical-block
// deployment.

package outboundhook

import "strings"

// bareDotPatterns is the set of normalized minimal-content emit-strings
// that R50 treats as bare-dot class. Match is case-insensitive on the
// trimmed assistant-text snip.
var bareDotPatterns = []string{
	".",
	"..",
	"...",
	"standing.",
	"standing",
	"idle.",
	"idle",
	"acknowledged.",
	"acknowledged",
	"ack.",
	"ack",
	"ok.",
	"ok",
	"done.",
	"done",
	"continuing.",
	"continuing",
	"loop fully closed.",
	"loop closed.",
	"holding.",
	"holding",
	"waiting.",
	"waiting",
}

// isBareDotPattern reports whether assistant-text is a bare-dot class
// pattern per R50. Case-insensitive match on trimmed text against
// bareDotPatterns. Multi-line patterns rejected (only single-line/short
// minimal-content qualifies).
func isBareDotPattern(text string) bool {
	t := strings.ToLower(strings.TrimSpace(text))
	if t == "" {
		return false
	}
	if strings.Contains(t, "\n") {
		// Multi-line content is not bare-dot class
		return false
	}
	if len(t) > 50 {
		// Substantive content is not bare-dot class (R36 keyword/length
		// check covers the opposite end)
		return false
	}
	for _, p := range bareDotPatterns {
		if t == p {
			return true
		}
	}
	return false
}
