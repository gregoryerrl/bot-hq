// Phase-S-followup-2 F2-3 tests: custom_rules.go persistence + caps
// + clear-command. R39-isolation via BOT_HQ_HOME env-var override.
package gemma

import (
	"os"
	"strings"
	"testing"
)

// withTempBotHQ sets BOT_HQ_HOME to a temp dir for R39 test-isolation.
// Returns cleanup-restore func.
func withTempBotHQ(t *testing.T) func() {
	t.Helper()
	dir := t.TempDir()
	prev := os.Getenv("BOT_HQ_HOME")
	os.Setenv("BOT_HQ_HOME", dir)
	return func() {
		if prev == "" {
			os.Unsetenv("BOT_HQ_HOME")
		} else {
			os.Setenv("BOT_HQ_HOME", prev)
		}
	}
}

func TestCustomRulesEmptyState(t *testing.T) {
	defer withTempBotHQ(t)()

	rules, err := ReadCustomRules()
	if err != nil {
		t.Fatalf("ReadCustomRules on empty state: %v", err)
	}
	if len(rules) != 0 {
		t.Errorf("expected 0 rules on empty state, got %d", len(rules))
	}
	if section := CustomRulesPromptSection(); section != "" {
		t.Errorf("expected empty prompt section on empty state, got %q", section)
	}
}

func TestAppendAndReadCustomRules(t *testing.T) {
	defer withTempBotHQ(t)()

	for _, r := range []string{"don't stop until done", "follow phase-s.md", "no shortcuts"} {
		_, rejected, err := AppendCustomRule(r)
		if err != nil {
			t.Fatalf("AppendCustomRule(%q): %v", r, err)
		}
		if rejected != "" {
			t.Errorf("AppendCustomRule(%q) unexpectedly rejected: %s", r, rejected)
		}
	}

	rules, err := ReadCustomRules()
	if err != nil {
		t.Fatalf("ReadCustomRules: %v", err)
	}
	if len(rules) != 3 {
		t.Errorf("expected 3 rules, got %d: %v", len(rules), rules)
	}
	if rules[0] != "don't stop until done" {
		t.Errorf("expected first rule 'don't stop until done', got %q", rules[0])
	}
}

func TestAppendRejectsEmpty(t *testing.T) {
	defer withTempBotHQ(t)()

	_, rejected, _ := AppendCustomRule("   ")
	if rejected == "" {
		t.Error("expected reject reason for empty rule")
	}
}

func TestAppendRejectsOversizeRule(t *testing.T) {
	defer withTempBotHQ(t)()

	long := strings.Repeat("x", customRulesMaxLineChars+1)
	_, rejected, _ := AppendCustomRule(long)
	if rejected == "" {
		t.Errorf("expected reject for oversize rule (%d chars)", len(long))
	}
	if !strings.Contains(rejected, "exceeds") {
		t.Errorf("expected reject reason to mention 'exceeds', got %q", rejected)
	}
}

func TestCapByCount(t *testing.T) {
	defer withTempBotHQ(t)()

	// Append more than max.
	for i := 0; i < customRulesMaxCount+5; i++ {
		_, _, err := AppendCustomRule("rule-" + string(rune('A'+i)))
		if err != nil {
			t.Fatalf("AppendCustomRule iter %d: %v", i, err)
		}
	}

	rules, _ := ReadCustomRules()
	if len(rules) != customRulesMaxCount {
		t.Errorf("expected %d rules after FIFO cap, got %d", customRulesMaxCount, len(rules))
	}
	// Oldest should be evicted (FIFO).
	if rules[0] != "rule-F" {
		t.Errorf("expected oldest rule rule-F (after evicting A-E), got %q", rules[0])
	}
}

func TestCapByTotalChars(t *testing.T) {
	defer withTempBotHQ(t)()

	// Each rule near max line size; under count cap; should hit total cap.
	big := strings.Repeat("x", 480)
	for i := 0; i < customRulesMaxCount; i++ {
		_, _, _ = AppendCustomRule(big + string(rune('a'+i)))
	}

	rules, _ := ReadCustomRules()
	if totalChars(rules) > customRulesMaxTotalChars {
		t.Errorf("expected total chars under cap, got %d (max %d)", totalChars(rules), customRulesMaxTotalChars)
	}
}

func TestClearCustomRules(t *testing.T) {
	defer withTempBotHQ(t)()

	for _, r := range []string{"a", "b", "c"} {
		AppendCustomRule(r)
	}
	removed, err := ClearCustomRules()
	if err != nil {
		t.Fatalf("ClearCustomRules: %v", err)
	}
	if removed != 3 {
		t.Errorf("expected 3 removed, got %d", removed)
	}
	rules, _ := ReadCustomRules()
	if len(rules) != 0 {
		t.Errorf("expected 0 rules after clear, got %d", len(rules))
	}
}

func TestPromptSectionFormat(t *testing.T) {
	defer withTempBotHQ(t)()

	AppendCustomRule("don't stop until done")
	AppendCustomRule("follow phase-s.md")

	section := CustomRulesPromptSection()
	if !strings.Contains(section, "USER-CUSTOM RULES") {
		t.Error("prompt section missing USER-CUSTOM RULES header")
	}
	if !strings.Contains(section, "R-USER-1: don't stop until done") {
		t.Errorf("prompt section missing R-USER-1, got: %s", section)
	}
	if !strings.Contains(section, "R-USER-2: follow phase-s.md") {
		t.Errorf("prompt section missing R-USER-2, got: %s", section)
	}
}

func TestCustomRulesPathHonorsBotHQHome(t *testing.T) {
	defer withTempBotHQ(t)()

	path := CustomRulesPath()
	if !strings.HasSuffix(path, "/emma/custom-rules.md") {
		t.Errorf("expected path to end in /emma/custom-rules.md, got %s", path)
	}
	if !strings.Contains(path, os.Getenv("BOT_HQ_HOME")) {
		t.Errorf("expected path to contain BOT_HQ_HOME, got %s", path)
	}
}
