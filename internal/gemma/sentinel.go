package gemma

import (
	"regexp"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// preFilterPatterns is the sole volume gate for hub-reactive Emma. A hub
// message whose content does NOT match any of these regexes is silently
// dropped — default-ignore is the system default.
//
// Pattern set is intentionally narrow: high-signal failure language that
// almost never appears in healthy chatter. Adding patterns here directly
// raises Emma's processing cost (each match runs each regex), so curate
// rather than expand.
var preFilterPatterns = []*regexp.Regexp{
	regexp.MustCompile(`(?i)\bpanic(:|\()`),
	regexp.MustCompile(`(?i)\bfatal\b`),
	regexp.MustCompile(`(?i)\bdeadlock!`),
	regexp.MustCompile(`(?i)rate[\s\-]?limit`),
	regexp.MustCompile(`(?i)\bOOM\b|out[\s\-]of[\s\-]memory`),
	regexp.MustCompile(`(?i)process\s+exit(ed)?`),
	regexp.MustCompile(`(?i)schema\s+constraint\s+(violation|failed|error)`),
	regexp.MustCompile(`(?i)stack\s+overflow`),
	regexp.MustCompile(`(?i)segmentation\s+fault|SIGSEGV`),
}

// alwaysFlagPatterns is a strict subset of preFilterPatterns. A match
// here promotes the decision from observation to flag (Type=MsgFlag,
// rate-cap + hysteresis still apply via Gemma.shouldFlag).
var alwaysFlagPatterns = []*regexp.Regexp{
	regexp.MustCompile(`(?i)\bpanic(:|\()`),
	regexp.MustCompile(`(?i)\bdeadlock!`),
	regexp.MustCompile(`(?i)rate[\s\-]?limit`),
	regexp.MustCompile(`(?i)process\s+exit(ed)?`),
	regexp.MustCompile(`(?i)schema\s+constraint\s+(violation|failed|error)`),
	regexp.MustCompile(`(?i)segmentation\s+fault|SIGSEGV`),
}

// SentinelDecision is the outcome of running a hub message through the
// sentinel classifiers.
type SentinelDecision struct {
	Match      bool   // pre-filter matched; non-match = drop silently
	AlwaysFlag bool   // matched the always-flag list (strict subset of Match)
	Pattern    string // pattern source string that matched first (for the flag body)
}

// SentinelMatch classifies a hub message against the pre-filter and
// always-flag list. Pure function — no IO, no goroutines, safe to call
// from any goroutine including the OnMessage callback.
func SentinelMatch(msg protocol.Message) SentinelDecision {
	text := msg.Content
	var d SentinelDecision
	for _, p := range preFilterPatterns {
		if p.MatchString(text) {
			d.Match = true
			d.Pattern = p.String()
			break
		}
	}
	if !d.Match {
		return d
	}
	for _, p := range alwaysFlagPatterns {
		if p.MatchString(text) {
			d.AlwaysFlag = true
			break
		}
	}
	return d
}
