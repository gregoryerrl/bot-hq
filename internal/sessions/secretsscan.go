package sessions

// Secrets-scan-on-manifest-author per phase-p.md §P-6 + phase-n.md:294
// "OQ-6 (privacy + secrets-scan-on-manifest-author) — productionize-class".
//
// Scope: regex-based detection of high-confidence secret patterns
// (provider-specific tokens + private-key blocks + .env-style assignments).
// Conservative pattern set — false-positives are worse than false-negatives
// here because hits surface as warnings the operator must triage; a noisy
// scanner produces alert fatigue + masks real findings.
//
// Output policy at WriteManifest boundary:
//   - Default: log findings to stderr + manifest-secrets-log.md, proceed
//   - BOT_HQ_SECRETS_STRICT=1: log + return error (caller decides retry/redact)
//
// The helpers ScanForSecrets + RedactSecrets are pure functions usable
// from anywhere — manifest-author is the load-bearing call site, but a
// future canonical-store-write path could plug them in similarly.

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

// SecretFinding is one matched secret pattern with provenance for
// operator triage. Match is intentionally truncated to avoid leaking
// the full secret into log files (the line number + pattern name
// suffice to locate + investigate).
type SecretFinding struct {
	Pattern string // human-friendly pattern name
	Line    int    // 1-indexed line number in scanned text
	Snippet string // truncated/redacted match preview (≤32 chars)
}

// secretPattern bundles a name + compiled regex. Patterns ordered
// most-specific first so multi-match lines pick the strongest signal.
type secretPattern struct {
	Name string
	Re   *regexp.Regexp
}

// secretPatterns is the conservative pattern set. Each entry is
// high-confidence (provider-prefix or distinctive structure); generic
// "long string" / "high entropy" detectors deliberately omitted to
// keep false-positive rate low.
var secretPatterns = []secretPattern{
	{Name: "anthropic-api-key", Re: regexp.MustCompile(`sk-ant-[A-Za-z0-9_-]{20,}`)},
	{Name: "openai-api-key", Re: regexp.MustCompile(`sk-[A-Za-z0-9]{40,}`)},
	{Name: "github-personal-token", Re: regexp.MustCompile(`gh[pousr]_[A-Za-z0-9_]{36,}`)},
	{Name: "aws-access-key-id", Re: regexp.MustCompile(`AKIA[0-9A-Z]{16}`)},
	{Name: "aws-secret-access-key", Re: regexp.MustCompile(`(?i)aws[_-]?secret[_-]?(access[_-]?)?key\s*[:=]\s*['"]?[A-Za-z0-9/+=]{40}['"]?`)},
	{Name: "private-key-block", Re: regexp.MustCompile(`-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----`)},
	{Name: "slack-token", Re: regexp.MustCompile(`xox[bpoas]-[A-Za-z0-9-]{10,}`)},
	{Name: "generic-password-assignment", Re: regexp.MustCompile(`(?i)\b(password|passwd|pwd)\s*[:=]\s*['"][^'"\s]{8,}['"]`)},
	{Name: "generic-token-assignment", Re: regexp.MustCompile(`(?i)\b(token|api[_-]?key|secret[_-]?key|access[_-]?key)\s*[:=]\s*['"][A-Za-z0-9_/+=-]{16,}['"]`)},
}

// scanRedactPlaceholder is the substitution used by RedactSecrets to
// replace matched secrets while preserving line/offset structure for
// post-redaction diffing.
const scanRedactPlaceholder = "[REDACTED-SECRET]"

// snippetMaxLen is the max length of SecretFinding.Snippet — long
// secrets are truncated so log files don't accidentally re-leak them.
const snippetMaxLen = 32

// ScanForSecrets returns a list of findings (one per match) in the
// given text. Empty slice when clean. Patterns are checked in
// most-specific-first order; the same line can produce multiple
// findings if multiple distinct patterns hit (rare in practice).
func ScanForSecrets(text string) []SecretFinding {
	if text == "" {
		return nil
	}
	var findings []SecretFinding
	lines := strings.Split(text, "\n")
	for i, line := range lines {
		for _, pat := range secretPatterns {
			matches := pat.Re.FindAllString(line, -1)
			for _, m := range matches {
				findings = append(findings, SecretFinding{
					Pattern: pat.Name,
					Line:    i + 1,
					Snippet: redactSnippet(m),
				})
			}
		}
	}
	return findings
}

// RedactSecrets returns the input with all matched secret patterns
// replaced by scanRedactPlaceholder. Useful when caller wants a
// safe-to-log version of a manifest body. Returns the redacted text
// + the list of findings discovered (so caller can still report
// what was redacted).
func RedactSecrets(text string) (string, []SecretFinding) {
	findings := ScanForSecrets(text)
	out := text
	for _, pat := range secretPatterns {
		out = pat.Re.ReplaceAllString(out, scanRedactPlaceholder)
	}
	return out, findings
}

// redactSnippet truncates a matched secret to snippetMaxLen chars
// with a "…" marker — keeps log files / SecretFinding.Snippet from
// echoing the full secret value while still letting the operator
// recognize the leak class.
func redactSnippet(s string) string {
	if len(s) <= snippetMaxLen {
		return s
	}
	// Show prefix only; suffix is the high-entropy tail, omit.
	return s[:snippetMaxLen] + "…"
}

// secretsLogPath returns the file where manifest-author secret
// findings are appended. Override via BOT_HQ_SECRETS_LOG_PATH for
// R39 TEST-ISOLATION. Default is <SessionsDir>/../manifest-secrets-log.md
// so the log lives alongside the canonical-store but outside the
// per-session dirs (one shared log; operator triage at file boundary).
func secretsLogPath() string {
	if p := os.Getenv("BOT_HQ_SECRETS_LOG_PATH"); p != "" {
		return p
	}
	dir := filepath.Dir(SessionsDir())
	return filepath.Join(dir, "manifest-secrets-log.md")
}

// LogSecretFindings appends findings to the secrets-log file with a
// timestamp + manifest-id header. Best-effort; errors do NOT propagate
// (manifest write must not fail because of log-write failure).
//
// strictMode (BOT_HQ_SECRETS_STRICT=1) is the caller's signal to
// return an error from the calling write-path; this helper just
// records to the log regardless.
func LogSecretFindings(manifestID string, findings []SecretFinding) {
	if len(findings) == 0 {
		return
	}
	logPath := secretsLogPath()
	if err := os.MkdirAll(filepath.Dir(logPath), 0o755); err != nil {
		fmt.Fprintf(os.Stderr, "secretsscan: mkdir log dir failed: %v\n", err)
		return
	}
	f, err := os.OpenFile(logPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		fmt.Fprintf(os.Stderr, "secretsscan: open log failed: %v\n", err)
		return
	}
	defer f.Close()
	now := time.Now().UTC().Format(time.RFC3339)
	fmt.Fprintf(f, "## %s — %s (%d finding(s))\n", now, manifestID, len(findings))
	for _, fnd := range findings {
		fmt.Fprintf(f, "- L%d %s: `%s`\n", fnd.Line, fnd.Pattern, fnd.Snippet)
	}
	fmt.Fprintln(f)
}

// secretsStrictMode reports whether BOT_HQ_SECRETS_STRICT=1 is set —
// in that case manifest-author returns an error on findings rather
// than the default log-and-proceed.
func secretsStrictMode() bool {
	return strings.TrimSpace(os.Getenv("BOT_HQ_SECRETS_STRICT")) == "1"
}
