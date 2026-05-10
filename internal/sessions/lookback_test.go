package sessions

import (
	"strings"
	"testing"
	"time"
)

func TestRenderLookback_FullManifest(t *testing.T) {
	m := Manifest{
		ID:            "2026-05-10-test-2",
		Project:       "test",
		StartTS:       time.Date(2026, 5, 10, 9, 0, 0, 0, time.UTC),
		EndTS:         time.Date(2026, 5, 10, 11, 30, 0, 0, time.UTC),
		StartMsgID:    100,
		EndMsgID:      287,
		MsgCount:      187,
		Agents:        []string{"brian", "rain"},
		Status:        "closed-pivoted-out",
		Phase:         "phase-w",
		Outcome:       "Refactored gemma plan-usage. Rain caught a stat-claim drift; addressed via cite-from-actual.",
		CommitsLanded: []string{"abc123def4567890ffffeeeeddddccccbbbbaaaa", "def456abc1234567ffffeeeeddddccccbbbbaaaa"},
		FilesTouched:  []string{"internal/foo/foo.go", "cmd/bar/main.go"},
		Decisions:     []string{"105: BRAIN-AGREED | refactor approach", "215: GREENFLAG | ship the work"},
		Body:          "Free-form notes here.",
	}

	got := RenderLookback(m)

	wantSubs := []string{
		"# Session retrospective: 2026-05-10-test-2",
		"**Project:** test",
		"**Status:** closed-pivoted-out",
		"**Time:** 2026-05-10T09:00:00Z → 2026-05-10T11:30:00Z (2h30m0s)",
		"**Hub msg-id range:** 100 → 287 (187 messages)",
		"**Agents:** brian, rain",
		"## Outcome",
		"Refactored gemma plan-usage",
		"## Structured fields",
		"### Decisions (2)",
		"105: BRAIN-AGREED",
		"### Commits landed (2)",
		"`abc123def456`", // shortSHA truncates 40-char hex to 12
		"### Files touched (2)",
		"internal/foo/foo.go",
		"## Manifest body",
		"Free-form notes here",
		"_Source:",
	}
	for _, sub := range wantSubs {
		if !strings.Contains(got, sub) {
			t.Errorf("RenderLookback missing %q\nFull output:\n%s", sub, got)
		}
	}
}

func TestRenderLookback_ActiveSession(t *testing.T) {
	// Status defaults to "active" when EndTS is zero
	m := Manifest{
		ID:      "active-session",
		Project: "test",
		StartTS: time.Date(2026, 5, 10, 9, 0, 0, 0, time.UTC),
	}
	got := RenderLookback(m)
	if !strings.Contains(got, "**Status:** active") {
		t.Errorf("default-status fallback broken: %q", got)
	}
	if !strings.Contains(got, "still active") {
		t.Errorf("active-time-line missing: %q", got)
	}
}

func TestRenderLookback_MinimalManifest(t *testing.T) {
	// Empty / new manifest still renders something readable
	m := Manifest{
		ID: "minimal",
	}
	got := RenderLookback(m)
	if !strings.Contains(got, "# Session retrospective: minimal") {
		t.Errorf("minimal render lost id: %q", got)
	}
	// No outcome / no structured fields → those sections omitted
	if strings.Contains(got, "## Outcome") {
		t.Errorf("Outcome section should be omitted for empty Outcome")
	}
	if strings.Contains(got, "## Structured fields") {
		t.Errorf("Structured fields section should be omitted when none present")
	}
}

func TestShortSHA(t *testing.T) {
	cases := []struct{ in, want string }{
		{"abc123def4567890ffffeeeeddddccccbbbbaaaa", "abc123def456"}, // 40 hex → truncate
		{"abc123", "abc123"}, // too short → verbatim
		{"non-hex-content", "non-hex-content"},
	}
	for _, c := range cases {
		if got := shortSHA(c.in); got != c.want {
			t.Errorf("shortSHA(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}
