// Package gemma — custom_rules.go: durable user-custom-rule
// persistence for emma rule-enforcer per Phase-S-followup-2 F2-3.
//
// Background: M6 user-claimed deliverable (msg 16121) — user can
// give emma custom rules via `@emma <directive>` broadcast-mention.
// Directives persist to ~/.bot-hq/emma/custom-rules.md (durable,
// human-readable) and are loaded into the LLM-judgment prompt at
// each enforcement-loop tick. Decoupled ack-immediate vs apply-
// at-next-tick semantics per scope-lock-doc.
//
// Safety caps:
//   - Max 10 active custom rules (FIFO age-out beyond cap)
//   - Per-rule line-cap: 500 chars
//   - Total custom-rules section budget: 5000 chars
//
// Format: each rule on a single line preceded by `- `. File header
// + trailing blank line preserved through edits. Concurrent access
// guarded by package-level mutex (file-IO is RuleEnforcer-only).
package emma

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

const (
	customRulesMaxCount      = 10
	customRulesMaxLineChars  = 500
	customRulesMaxTotalChars = 5000
	customRulesFileHeader    = "# emma custom rules — Phase-S-followup-2 M6\n# Edited via @emma <directive> broadcast-mention OR by hand.\n# Format: one rule per line, leading `- `. Auto-trimmed to caps.\n\n"
)

var customRulesMu sync.Mutex

// CustomRulesPath returns the durable file path for emma's custom
// rules. Honors BOT_HQ_HOME env-var if set (for test-isolation per
// R39); otherwise defaults to ~/.bot-hq/emma/custom-rules.md.
func CustomRulesPath() string {
	if home := os.Getenv("BOT_HQ_HOME"); home != "" {
		return filepath.Join(home, "emma", "custom-rules.md")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ".bot-hq/emma/custom-rules.md"
	}
	return filepath.Join(home, ".bot-hq", "emma", "custom-rules.md")
}

// ReadCustomRules returns the active custom rules in append-order.
// Returns nil + nil error when file does not exist (clean state).
// Caps not enforced on read; AppendCustomRule enforces on write.
func ReadCustomRules() ([]string, error) {
	customRulesMu.Lock()
	defer customRulesMu.Unlock()
	return readRulesUnlocked()
}

func readRulesUnlocked() ([]string, error) {
	data, err := os.ReadFile(CustomRulesPath())
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	rules := []string{}
	for _, line := range strings.Split(string(data), "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}
		// Rule lines start with "- "; strip prefix.
		if strings.HasPrefix(trimmed, "- ") {
			rules = append(rules, strings.TrimSpace(trimmed[2:]))
		} else {
			rules = append(rules, trimmed)
		}
	}
	return rules, nil
}

// AppendCustomRule appends a new rule and enforces caps. Returns
// the persisted rule list + reject reason if rule was dropped due
// to cap-violation. Reject cases:
//   - empty after trim
//   - exceeds 500 chars
//
// Cap behavior: if total count >10 after append, oldest rule
// FIFO-evicted. If total chars >5000 after FIFO, evict more until
// under cap. Always succeeds with at-most-cap rules persisted.
func AppendCustomRule(rule string) (rules []string, rejected string, err error) {
	rule = strings.TrimSpace(rule)
	if rule == "" {
		return nil, "empty rule", nil
	}
	if len(rule) > customRulesMaxLineChars {
		return nil, fmt.Sprintf("rule exceeds %d-char per-line cap (got %d)", customRulesMaxLineChars, len(rule)), nil
	}

	customRulesMu.Lock()
	defer customRulesMu.Unlock()

	existing, err := readRulesUnlocked()
	if err != nil {
		return nil, "", err
	}
	all := append(existing, rule)

	// FIFO cap by count
	if len(all) > customRulesMaxCount {
		all = all[len(all)-customRulesMaxCount:]
	}
	// FIFO cap by total chars
	for totalChars(all) > customRulesMaxTotalChars && len(all) > 1 {
		all = all[1:]
	}

	if err := writeRulesUnlocked(all); err != nil {
		return nil, "", err
	}
	return all, "", nil
}

// ClearCustomRules truncates the rules file (keeps header). Returns
// count of rules removed.
func ClearCustomRules() (removed int, err error) {
	customRulesMu.Lock()
	defer customRulesMu.Unlock()

	existing, err := readRulesUnlocked()
	if err != nil {
		return 0, err
	}
	if err := writeRulesUnlocked(nil); err != nil {
		return 0, err
	}
	return len(existing), nil
}

func writeRulesUnlocked(rules []string) error {
	path := CustomRulesPath()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	var sb strings.Builder
	sb.WriteString(customRulesFileHeader)
	for _, r := range rules {
		fmt.Fprintf(&sb, "- %s\n", r)
	}
	return os.WriteFile(path, []byte(sb.String()), 0o644)
}

func totalChars(rules []string) int {
	n := 0
	for _, r := range rules {
		n += len(r)
	}
	return n
}

// CustomRulesPromptSection formats the active custom rules for
// injection into the LLM-judgment prompt. Returns empty string when
// no rules exist (clean state — keeps prompt short). Used by
// rule_enforcer.go detectInterpretiveViolations at each tick.
func CustomRulesPromptSection() string {
	rules, err := ReadCustomRules()
	if err != nil || len(rules) == 0 {
		return ""
	}
	var sb strings.Builder
	sb.WriteString("\nUSER-CUSTOM RULES (appended via @emma <directive> broadcast-mention; apply same VIOLATION format with rule-id `R-USER-N` where N is 1-indexed in this list):\n")
	for i, r := range rules {
		fmt.Fprintf(&sb, "R-USER-%d: %s\n", i+1, r)
	}
	return sb.String()
}
