package brain

import (
	"strings"
	"testing"
)

func TestNudgeContainsMessageContent(t *testing.T) {
	content := "fix the login bug"
	nudge := formatNudge("user", content)

	if !strings.Contains(nudge, content) {
		t.Errorf("nudge should contain message content %q, got: %s", content, nudge)
	}
	if !strings.Contains(nudge, "user") {
		t.Errorf("nudge should contain sender 'user', got: %s", nudge)
	}
}

func TestFormatNudgeIsNotEmpty(t *testing.T) {
	nudge := formatNudge("user", "hello")
	if nudge == "" {
		t.Error("formatNudge should return non-empty string")
	}
	if !strings.Contains(nudge, "hello") {
		t.Error("formatNudge should include content")
	}
	if !strings.Contains(nudge, "hub_send") {
		t.Error("formatNudge should instruct Claude to use hub_send")
	}
}
