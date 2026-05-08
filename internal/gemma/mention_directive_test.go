// Phase-S-followup-2 F2-2 tests: stripEmmaMention + isClearCommand
// helpers. Full handleMentionDirective integration deferred to F2-3
// when custom-rules.md persistence lands.
package gemma

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
