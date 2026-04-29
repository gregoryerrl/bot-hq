package toolgate

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// peerGreenflagFooterRegex matches the canonical commit-message footer
// that R26 requires on HANDS-class commits: `peer-greenflag-msg-id: <N>`.
// Allows trailing whitespace and case-insensitive prefix; msg-id must be
// a positive integer.
var peerGreenflagFooterRegex = regexp.MustCompile(`(?im)^peer-greenflag-msg-id:\s*(\d+)\s*$`)

// greenflagPatterns enumerates the substring patterns that count as a
// peer-greenflag in the cited msg's content. Substring-match here is
// safe because we only match against the SPECIFIC peer message the
// committer cited via footer (not random conversational text).
var greenflagPatterns = []string{
	"BRAIN-AGREED",
	"GREENFLAG",
}

// defaultGreenflagWindow is the default age limit for the cited peer
// greenflag msg. Override via R12_GREENFLAG_WINDOW_MIN env var.
const defaultGreenflagWindow = 60 * time.Minute

// R12Verdict reports the outcome of a R12-pre-commit verification.
type R12Verdict struct {
	// Allow is true if the commit may proceed.
	Allow bool

	// Reason is a human-readable explanation (filled on Allow=false; may
	// also fill on Allow=true with "skipped: <form>" for soft-allow paths).
	Reason string

	// SkippedForm is non-empty if the verification was skipped due to
	// commit-message-form coverage limit (no-flag editor form / amend /
	// hub.db absent / etc.). Used for hub-side logging.
	SkippedForm string
}

// VerifyCommit runs the R12 pre-commit verification for a `git commit`
// invocation. Decision tree:
//
//  1. BRIAN_R12_OVERRIDE=1 env var → Allow=true with SkippedForm="override"
//  2. Extract commit message from `-m` or `-F file` form. No-flag (editor)
//     OR `--amend` → Allow=true with SkippedForm="form-not-covered"
//     (deferred to K-13-ext per Rain msg 6438)
//  3. Find `peer-greenflag-msg-id: <N>` footer in message. Absent →
//     Allow=false with reason
//  4. Open hub.db (BOT_HQ_HOME or ~/.bot-hq/hub.db). Absent → Allow=true
//     with SkippedForm="hubdb-absent" (fail-soft per Rain msg 6438)
//  5. Query msg <N> from hub.db:
//     - Not found → Allow=false
//     - From == committer (self-greenflag) → Allow=false
//     - Older than window (default 60min, override via env) → Allow=false
//     - Content lacks any greenflagPatterns substring → Allow=false
//  6. All checks pass → Allow=true
//
// agentID is the committer (BOT_HQ_AGENT_ID env var); used to detect
// self-greenflag attempts (must be FROM peer, not from self).
//
// Phase K K-13.
func VerifyCommit(command, agentID string) R12Verdict {
	// (1) Override
	if os.Getenv("BRIAN_R12_OVERRIDE") == "1" {
		return R12Verdict{Allow: true, SkippedForm: "override"}
	}

	// (2) Extract commit message
	commitMsg, form, ok := extractCommitMessage(command)
	if !ok {
		return R12Verdict{Allow: true, SkippedForm: form}
	}

	// (3) Find peer-greenflag-msg-id footer
	matches := peerGreenflagFooterRegex.FindStringSubmatch(commitMsg)
	if matches == nil {
		return R12Verdict{
			Allow:  false,
			Reason: "commit message missing required footer `peer-greenflag-msg-id: <N>` per R26",
		}
	}
	msgID, err := strconv.ParseInt(matches[1], 10, 64)
	if err != nil || msgID <= 0 {
		return R12Verdict{
			Allow:  false,
			Reason: fmt.Sprintf("peer-greenflag-msg-id %q is not a positive integer", matches[1]),
		}
	}

	// (4) Open hub.db (fail-soft if absent)
	dbPath := hubDBPath()
	if _, err := os.Stat(dbPath); err != nil {
		if os.IsNotExist(err) {
			return R12Verdict{Allow: true, SkippedForm: "hubdb-absent"}
		}
		return R12Verdict{Allow: true, SkippedForm: "hubdb-stat-error"}
	}
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		return R12Verdict{Allow: true, SkippedForm: "hubdb-open-error"}
	}
	defer db.Close()

	// (5) Query msg <N>
	msg, found, err := db.GetMessageByID(msgID)
	if err != nil {
		return R12Verdict{Allow: true, SkippedForm: "hubdb-query-error"}
	}
	if !found {
		return R12Verdict{
			Allow:  false,
			Reason: fmt.Sprintf("cited peer-greenflag-msg-id %d not found in hub.db", msgID),
		}
	}
	if msg.FromAgent == agentID {
		return R12Verdict{
			Allow:  false,
			Reason: fmt.Sprintf("cited msg %d is from self (%s); peer-greenflag must be from the OTHER agent", msgID, agentID),
		}
	}
	window := greenflagWindow()
	if time.Since(msg.Created) > window {
		return R12Verdict{
			Allow:  false,
			Reason: fmt.Sprintf("cited msg %d is older than greenflag window (%s); peer must re-greenflag for current commit cycle", msgID, window),
		}
	}
	if !hasGreenflagContent(msg.Content) {
		return R12Verdict{
			Allow:  false,
			Reason: fmt.Sprintf("cited msg %d content lacks greenflag pattern (BRAIN-AGREED / GREENFLAG); peer's msg must be a deliberate greenflag", msgID),
		}
	}

	// (6) All checks pass
	return R12Verdict{Allow: true}
}

// extractCommitMessage extracts the intended commit message from a
// `git commit` invocation. Returns (message, "", true) on success.
// Returns ("", form, false) for unsupported forms (no-flag editor;
// --amend; etc.) so caller can soft-allow with logging per Rain msg
// 6438 form-coverage spec.
//
// Supported forms (MVP):
//   - `git commit -m "msg"` / `git commit -m 'msg'` / `git commit -m msg`
//   - `git commit -F file` / `git commit --file=file`
//
// Unsupported (returns false with form name for logging):
//   - `git commit` (no flag — opens editor; hook fires pre-editor-open)
//   - `git commit --amend` (rewrites previous; semantics differ)
func extractCommitMessage(command string) (msg, form string, ok bool) {
	tokens := tokenize(command)
	if len(tokens) < 2 || tokens[0] != "git" || tokens[1] != "commit" {
		return "", "non-commit", false
	}

	// Check for --amend flag → unsupported MVP form
	for _, t := range tokens[2:] {
		if t == "--amend" {
			return "", "amend", false
		}
	}

	// Look for -m / -F flag
	for i := 2; i < len(tokens); i++ {
		t := tokens[i]
		if t == "-m" || t == "--message" {
			if i+1 >= len(tokens) {
				return "", "incomplete-m-flag", false
			}
			return stripQuotes(tokens[i+1]), "", true
		}
		if t == "-F" || t == "--file" {
			if i+1 >= len(tokens) {
				return "", "incomplete-f-flag", false
			}
			path := stripQuotes(tokens[i+1])
			data, err := os.ReadFile(path)
			if err != nil {
				return "", "f-file-unreadable", false
			}
			return string(data), "", true
		}
		if strings.HasPrefix(t, "-m=") {
			return stripQuotes(strings.TrimPrefix(t, "-m=")), "", true
		}
		if strings.HasPrefix(t, "--message=") {
			return stripQuotes(strings.TrimPrefix(t, "--message=")), "", true
		}
		if strings.HasPrefix(t, "-F=") {
			path := stripQuotes(strings.TrimPrefix(t, "-F="))
			data, err := os.ReadFile(path)
			if err != nil {
				return "", "f-file-unreadable", false
			}
			return string(data), "", true
		}
		if strings.HasPrefix(t, "--file=") {
			path := stripQuotes(strings.TrimPrefix(t, "--file="))
			data, err := os.ReadFile(path)
			if err != nil {
				return "", "f-file-unreadable", false
			}
			return string(data), "", true
		}
	}

	// No -m / -F → editor form
	return "", "editor-form", false
}

// stripQuotes removes surrounding single or double quotes from a token
// (tokenize preserves them as part of atomic spans).
func stripQuotes(s string) string {
	if len(s) >= 2 {
		if (s[0] == '\'' && s[len(s)-1] == '\'') || (s[0] == '"' && s[len(s)-1] == '"') {
			return s[1 : len(s)-1]
		}
	}
	return s
}

// hasGreenflagContent reports whether the given content contains any
// greenflag-class substring. Substring-match is safe here because the
// content is the SPECIFIC peer message the committer cited (not random
// conversational text).
func hasGreenflagContent(content string) bool {
	for _, p := range greenflagPatterns {
		if strings.Contains(content, p) {
			return true
		}
	}
	return false
}

// hubDBPath returns the hub.db path honoring BOT_HQ_HOME for tests.
// Defaults to ~/.bot-hq/hub.db.
func hubDBPath() string {
	if home := os.Getenv("BOT_HQ_HOME"); home != "" {
		return filepath.Join(home, "hub.db")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".bot-hq", "hub.db")
}

// greenflagWindow returns the configured age limit for cited peer
// greenflag messages. Honors R12_GREENFLAG_WINDOW_MIN env var (minutes);
// defaults to 60 minutes.
func greenflagWindow() time.Duration {
	if v := os.Getenv("R12_GREENFLAG_WINDOW_MIN"); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			return time.Duration(n) * time.Minute
		}
	}
	return defaultGreenflagWindow
}
