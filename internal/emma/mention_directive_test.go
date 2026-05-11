// Phase-S-followup-2 F2-2 tests: stripEmmaMention + isClearCommand
// helpers. Full handleMentionDirective integration deferred to F2-3
// when custom-rules.md persistence lands.
package emma

import "testing"

func TestStripEmmaMention(t *testing.T) {
	tests := []struct {
		name string
		in   string
		want string
	}{
		{"prefix space", "@emma do the thing", "do the thing"},
		{"prefix colon", "@emma: rule clear", "rule clear"},
		{"prefix comma", "@emma, please review", "please review"},
		{"case-insensitive", "@EMMA hello", "hello"},
		{"mixed case", "@Emma directive", "directive"},
		{"with leading whitespace", "   @emma directive", "directive"},
		{"only @emma", "@emma", ""},
		{"only @emma with whitespace", "@emma   ", ""},
		{"multiline directive", "@emma do not let them stop\nuntil halt", "do not let them stop\nuntil halt"},
		{"no @emma", "no mention here", "no mention here"},
		{"@emma not at start", "hello @emma rule clear", "rule clear"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := stripEmmaMention(tt.in); got != tt.want {
				t.Errorf("stripEmmaMention(%q) = %q, want %q", tt.in, got, tt.want)
			}
		})
	}
}

func TestIsClearCommand(t *testing.T) {
	tests := []struct {
		name      string
		directive string
		want      bool
	}{
		{"canonical rule: clear", "rule: clear", true},
		{"no space", "rule:clear", true},
		{"clear rules variant", "clear rules", true},
		{"clear custom rules variant", "clear custom rules", true},
		{"case-insensitive RULE: CLEAR", "RULE: CLEAR", true},
		{"with whitespace", "  rule: clear  ", true},
		{"non-clear directive", "don't let them stop", false},
		{"empty", "", false},
		{"clear-substring not match", "rule: clear all caches", false},
		{"rule but not clear", "rule: ignore X", false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := isClearCommand(tt.directive); got != tt.want {
				t.Errorf("isClearCommand(%q) = %v, want %v", tt.directive, got, tt.want)
			}
		})
	}
}

// TestHasRulePrefix exercises Phase T T-1.5 R53 EFFICIENCY-DRIVEN-DESIGN
// parser fix per phase-t.md v5: explicit `rule:` literal-prefix discriminator
// gates custom-rule promotion, eliminating cascade-bug overhead.
func TestHasRulePrefix(t *testing.T) {
	tests := []struct {
		name      string
		directive string
		want      bool
	}{
		// Cascade-fix: rule-class operations require `rule:` prefix
		{"canonical rule prefix", "rule: do X", true},
		{"no space after colon", "rule:do X", true},
		{"case-insensitive RULE", "RULE: do X", true},
		{"whitespace tolerant", "  rule: do X  ", true},
		{"clear command via prefix", "rule: clear", true},

		// Backward-compat for legacy clear-command forms
		{"legacy clear rules", "clear rules", true},
		{"legacy clear custom rules", "clear custom rules", true},

		// Non-rule conversation-class (DROPPED post-T-1.5 fix)
		{"plain conversation", "hello there", false},
		{"acknowledgement", "thanks", false},
		{"hub_send-style emit", "emma|status|active:5", false},
		{"agent peer-coord", "Brian-1st-pass on msg 17146", false},
		{"chat-class directive without prefix", "do not let them stop", false},
		{"empty", "", false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := hasRulePrefix(tt.directive); got != tt.want {
				t.Errorf("hasRulePrefix(%q) = %v, want %v", tt.directive, got, tt.want)
			}
		})
	}
}

func TestStripRulePrefix(t *testing.T) {
	tests := []struct {
		name      string
		directive string
		want      string
	}{
		{"canonical", "rule: do X", "do X"},
		{"no space", "rule:do X", "do X"},
		{"upper case", "RULE: do X", "do X"},
		{"with whitespace", "  rule:   do X  ", "do X"},
		{"empty after strip", "rule:", ""},
		{"empty after strip with space", "rule:  ", ""},
		{"no prefix unchanged", "no prefix here", "no prefix here"},
		{"only whitespace", "   ", ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := stripRulePrefix(tt.directive); got != tt.want {
				t.Errorf("stripRulePrefix(%q) = %q, want %q", tt.directive, got, tt.want)
			}
		})
	}
}
