package live

import (
	"strings"
	"testing"
)

func TestGeminiModelIsNotDeprecated(t *testing.T) {
	deprecated := []string{
		"gemini-2.0-flash-live-001",
		"gemini-2.0-flash-live",
	}
	for _, d := range deprecated {
		if strings.Contains(geminiModel, d) {
			t.Errorf("geminiModel uses deprecated model %q, should use gemini-3.1-flash-live-preview", d)
		}
	}
	if !strings.Contains(geminiModel, "gemini-3.1-flash-live-preview") {
		t.Errorf("geminiModel = %q, want models/gemini-3.1-flash-live-preview", geminiModel)
	}
}
