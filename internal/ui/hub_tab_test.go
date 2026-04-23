package ui

import (
	"testing"
)

func TestParseCommand(t *testing.T) {
	tests := []struct {
		input   string
		target  string
		content string
	}{
		{"@brain hello", "brain", "hello"},
		{"@claude-abc stop", "claude-abc", "stop"},
		{"@live check status", "live", "check status"},
		{"hello world", "", "hello world"},
		{"spawn bcc-ad-manager", "", "spawn bcc-ad-manager"},
		{"@brain", "brain", ""},
	}

	for _, tt := range tests {
		target, content := parseCommand(tt.input)
		if target != tt.target {
			t.Errorf("parseCommand(%q) target = %q, want %q", tt.input, target, tt.target)
		}
		if content != tt.content {
			t.Errorf("parseCommand(%q) content = %q, want %q", tt.input, content, tt.content)
		}
	}
}
