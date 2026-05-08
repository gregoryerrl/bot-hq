package protocol

import (
	"reflect"
	"testing"
)

func TestMentionsAgent(t *testing.T) {
	tests := []struct {
		name    string
		content string
		agent   string
		want    bool
	}{
		{"brian start-of-string", "@brian please review", "brian", true},
		{"rain after-whitespace", "@brian and @rain BRAIN-2nd", "rain", true},
		{"emma case-insensitive", "@EMMA rule: clear", "emma", true},
		{"emma agent-id-uppercased-arg", "@emma directive", "EMMA", true},
		{"coder hex", "@coder-13ea3a3b done", "coder-13ea3a3b", true},
		{"coder uppercase hex", "@coder-ABCDEF push", "coder-abcdef", true},
		{"no match", "ping rain please", "rain", false},
		{"email-like-not-mention", "user@brian.com", "brian", false},
		{"midword-not-mention", "foo@brian", "brian", false},
		{"trailing punctuation OK", "@brian, please", "brian", true},
		{"empty content", "", "brian", false},
		{"empty agent", "@brian go", "", false},
		{"unknown agent token", "@bobby halp", "bobby", false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := MentionsAgent(tt.content, tt.agent); got != tt.want {
				t.Errorf("MentionsAgent(%q, %q) = %v, want %v", tt.content, tt.agent, got, tt.want)
			}
		})
	}
}

func TestExtractMentions(t *testing.T) {
	tests := []struct {
		name    string
		content string
		want    []string
	}{
		{"single", "@brian please", []string{"brian"}},
		{"multi", "@brian and @rain check @emma", []string{"brian", "rain", "emma"}},
		{"dedup", "@brian @brian @brian", []string{"brian"}},
		{"case-fold-dedup", "@brian @BRIAN @Brian", []string{"brian"}},
		{"coder", "@coder-13ea3a3b done", []string{"coder-13ea3a3b"}},
		{"coder hex case-fold", "@coder-ABCDEF and @coder-abcdef", []string{"coder-abcdef"}},
		{"none", "no mentions here", nil},
		{"email-not-counted", "user@brian.com sent @rain", []string{"rain"}},
		{"empty", "", nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := ExtractMentions(tt.content)
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("ExtractMentions(%q) = %v, want %v", tt.content, got, tt.want)
			}
		})
	}
}

func TestMentionPatternBoundary(t *testing.T) {
	// Word-boundary check: @brian followed by alphanum should not match
	// (prevents matching @brianna against agent "brian").
	if MentionsAgent("@brianna hi", "brian") {
		t.Error("expected @brianna to NOT match brian")
	}
	// But trailing punctuation/whitespace should match.
	if !MentionsAgent("@brian!", "brian") {
		t.Error("expected @brian! to match brian")
	}
	if !MentionsAgent("@brian.", "brian") {
		t.Error("expected @brian. to match brian")
	}
}
