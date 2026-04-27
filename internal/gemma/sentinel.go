package gemma

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// ledgerMsgIDRe extracts a `msg #<id>` token from a ledger line so
// AppendToDryRunLedger can dedup cross-bounce re-fires (e.g., Emma
// boot-replay processing the same 50-msg window already ledgered in
// a prior boot). Lines without this token are treated as legacy
// free-form observations and are not deduped.
var ledgerMsgIDRe = regexp.MustCompile(`msg #(\d+)\b`)

// queueFailPattern is the Phase H slice 2 H-22 regex shape — pattern-
// match on `[queue] failed after K attempts` log lines. Pure pattern
// (no interpretation, no spec-comparison) so gemma4:e4b safe.
//
// Tracked as a const so tests can reference it and so dispatchSentinelHit
// can map matched patterns to the dry-run ledger without re-compiling.
const queueFailPattern = `(?i)\[queue\]\s+failed\s+after\s+\d+\s+attempts?`

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
	regexp.MustCompile(queueFailPattern), // H-22 — in dry-run period; see dryRunPatterns
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

// dryRunPatterns names patterns currently in the H-22/H-23 tuning-gate
// dry-run period. Key = pattern source string (matches
// SentinelDecision.Pattern). Value = sentinel name (used as ledger
// filename prefix).
//
// A match against a dry-run pattern triggers a ledger write to
// ~/.bot-hq/sentinels/<name>-dryrun.log instead of a hub_send to Rain.
// Promotion to live (and removal from this map, plus addition to
// alwaysFlagPatterns or kept in observation tier) happens after Rain
// reviews the dry-run output and confirms ≤5% false-positive rate per
// the universal Emma tuning-gate discipline (master design).
var dryRunPatterns = map[string]string{
	queueFailPattern: "queuefail",
}

// IsDryRunPattern reports whether the given pattern source string is
// currently in the dry-run period. Caller (dispatchSentinelHit) uses
// this to route observations to the ledger instead of Rain.
func IsDryRunPattern(pattern string) (name string, isDryRun bool) {
	name, ok := dryRunPatterns[pattern]
	return name, ok
}

// sentinelsDir returns ~/.bot-hq/sentinels, honoring BOT_HQ_HOME for tests.
// Mirrors the pattern used by internal/projects.projectsDir.
func sentinelsDir() (string, error) {
	if h := os.Getenv("BOT_HQ_HOME"); h != "" {
		return filepath.Join(h, "sentinels"), nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("locate home dir: %w", err)
	}
	return filepath.Join(home, ".bot-hq", "sentinels"), nil
}

// AppendToDryRunLedger writes an observation line to the dry-run ledger
// for the named sentinel. Best-effort: errors are swallowed because we
// don't want a missing-dir or permission issue to crash sentinel
// processing. Format is timestamped one-line-per-observation so Rain
// can grep-review false-positive rate during tuning gate.
//
// Cross-bounce dedup: if `line` embeds a `msg #<id>` token (the
// dispatchSentinelHit format), an entry already on file for the same
// msg-id is a no-op. Idempotent across Emma boot-replay re-fires that
// would otherwise re-append the same hit each bounce. Lines without
// the token (legacy free-form callers, tests) bypass dedup.
func AppendToDryRunLedger(name string, line string) {
	dir, err := sentinelsDir()
	if err != nil {
		return
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return
	}
	path := filepath.Join(dir, name+"-dryrun.log")

	if m := ledgerMsgIDRe.FindStringSubmatch(line); m != nil {
		if existing, err := os.ReadFile(path); err == nil {
			// Anchor the dedup needle to the canonical-format position:
			// `<RFC3339> | msg #<id> ...`. The leading " | " separator
			// disambiguates against a free-form ledger line that mentions
			// the same msg-id in its body (e.g., "see msg #42 for context").
			needle := fmt.Sprintf(" | msg #%s ", m[1])
			if strings.Contains(string(existing), needle) {
				return
			}
		}
	}

	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return
	}
	defer f.Close()
	fmt.Fprintf(f, "%s | %s\n", time.Now().UTC().Format(time.RFC3339), line)
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
