// Phase T T-11 cycle-3: API-key paste-detection for hub_send content.
//
// Pre-DB-write defensive screen: when an agent or user invokes hub_send
// with content containing what looks like an API-key (sk-xxx pattern),
// the daemon halts the send + returns an actionable error so the secret
// never lands in hub.db / Discord forwards.
//
// Recurrence evidence (Phase T cycle-3 rotation cycle): hub-paste of
// DEEPSEEK_API_KEY at msgs 17421 + 17460 — the second occurred AFTER
// an explicit "do NOT paste into hub" warning at msg 17454 step-2.
// Warning-text-as-prevention-mechanism is empirically insufficient;
// mechanical check at the daemon entry point is the load-bearing
// terminator class (mirrors R36 OUTBOUND-DISCIPLINE-MECHANICAL +
// R33 PRE-EXECUTE-GATE-FILE-READ enforcement-conversion precedents).
//
// Pattern coverage (per Rain msg 17486 push-back C inclusivity):
//
//	sk-[a-zA-Z0-9_-]{20,}    — DeepSeek + OpenAI + Anthropic + generic
//
// Specific provider-format-validation (length + prefix) is reserved
// for the vault-write step; this detector is provider-agnostic.

package pastedetect

import (
	"fmt"
	"regexp"
)

// apiKeyPattern matches the canonical `sk-` API-key prefix used by
// DeepSeek / OpenAI / Anthropic / etc. The 20-char minimum suffix
// avoids matching short tokens like `sk-test` in test fixtures while
// still catching all production keys.
var apiKeyPattern = regexp.MustCompile(`\bsk-[a-zA-Z0-9_-]{20,}\b`)

// Detection describes a paste-detection match. Match is empty when no
// API-key pattern was found in the content.
type Detection struct {
	Match      string // the matched substring (truncated for safety)
	Suggestion string // remediation hint for the caller
}

// Found returns true when the detection identified an API-key pattern.
func (d Detection) Found() bool {
	return d.Match != ""
}

// Inspect scans content for API-key patterns and returns a Detection.
// Returns an empty Detection (Found()==false) when the content is clean.
func Inspect(content string) Detection {
	loc := apiKeyPattern.FindStringIndex(content)
	if loc == nil {
		return Detection{}
	}
	matched := content[loc[0]:loc[1]]
	return Detection{
		Match:      truncate(matched, 8),
		Suggestion: defaultSuggestion(),
	}
}

// truncate returns a head-only excerpt of s with `…` suffix when s is
// longer than max. Used to keep the matched secret out of error logs.
func truncate(s string, max int) string {
	if len(s) <= max {
		return s + "…"
	}
	return s[:max] + "…"
}

// defaultSuggestion returns the standard remediation hint for an
// API-key paste-detection block.
func defaultSuggestion() string {
	return "API-key pattern detected in hub_send content. Secrets must NOT pass through the hub channel — they would be persisted in hub.db + forwarded to Discord channels. Write the secret directly to the FileVault path (e.g., `~/.bot-hq/agents/<agent>/.env` with chmod 0600) via filesystem editor (vi/nano/Write). The daemon's vault watcher will detect the file change automatically; no hub round-trip needed."
}

// FormatBlockReason returns a complete error message suitable for
// returning to the MCP caller when a paste is detected.
func FormatBlockReason(d Detection) string {
	return fmt.Sprintf("hub_send BLOCKED — %s (matched prefix: %s)", d.Suggestion, d.Match)
}
