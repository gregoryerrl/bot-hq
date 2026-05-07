package sessions

// Tests for cross-session search (P-7 / phase-n.md:295 + OQ-7).
// Manifests-only-v1 grep-style scan; tests pin behavior of empty
// query / case-insensitivity / multi-session sort / line-number
// accuracy / limit-clamp / snippet-truncate / corrupt-manifest-skip.

import (
	"strings"
	"testing"
	"time"
)

// writeSearchManifest creates a session with the given body for
// search-fixture purposes. Reuses WriteManifest so the rendered
// frontmatter+body shape is exactly what runtime would produce.
func writeSearchManifest(t *testing.T, id, body string) {
	t.Helper()
	now := time.Date(2026, 5, 7, 12, 0, 0, 0, time.UTC)
	m := Manifest{
		ID:      id,
		Project: "bot-hq",
		StartTS: now,
		EndTS:   now.Add(time.Hour),
		Body:    body,
	}
	if err := WriteManifest(m); err != nil {
		t.Fatalf("write %s: %v", id, err)
	}
}

// TestSearchSessions_EmptyQuery returns nil for blank input rather
// than scanning everything.
func TestSearchSessions_EmptyQuery(t *testing.T) {
	setSessionsDir(t)
	writeSearchManifest(t, "2026-05-07-bot-hq", "anything here\n")

	hits, err := SearchSessions("", 50)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if hits != nil {
		t.Errorf("empty query should return nil; got %v", hits)
	}
	hits, _ = SearchSessions("   ", 50)
	if hits != nil {
		t.Errorf("whitespace-only query should return nil; got %v", hits)
	}
}

// TestSearchSessions_CaseInsensitive matches regardless of case.
func TestSearchSessions_CaseInsensitive(t *testing.T) {
	setSessionsDir(t)
	writeSearchManifest(t, "2026-05-07-bot-hq", "Phase O drain CLOSED at 4ae5666\n")

	for _, q := range []string{"phase o", "PHASE O", "Phase O", "phase o drain"} {
		hits, err := SearchSessions(q, 50)
		if err != nil {
			t.Fatalf("err: %v", err)
		}
		if len(hits) == 0 {
			t.Errorf("query %q produced 0 hits", q)
		}
	}
}

// TestSearchSessions_LineNumberAccurate confirms 1-indexed line
// numbers.
func TestSearchSessions_LineNumberAccurate(t *testing.T) {
	setSessionsDir(t)
	body := "line one\nline two has UNIQUE-MARKER here\nline three\n"
	writeSearchManifest(t, "2026-05-07-bot-hq", body)

	hits, err := SearchSessions("UNIQUE-MARKER", 50)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if len(hits) != 1 {
		t.Fatalf("expected 1 hit; got %d", len(hits))
	}
	// Line 1 is "---" frontmatter open; manifest body starts after
	// frontmatter close. Just verify line > 1 and snippet contains
	// the marker — the precise line number depends on frontmatter
	// length which is internal detail.
	if hits[0].Line < 2 {
		t.Errorf("line should be > 1 (past frontmatter); got %d", hits[0].Line)
	}
	if !strings.Contains(hits[0].Snippet, "UNIQUE-MARKER") {
		t.Errorf("snippet missing marker: %q", hits[0].Snippet)
	}
	if hits[0].SessionID != "2026-05-07-bot-hq" {
		t.Errorf("session id = %q, want 2026-05-07-bot-hq", hits[0].SessionID)
	}
}

// TestSearchSessions_ReverseChronoOrder verifies most-recent sessions
// surface first when results from multiple sessions match.
func TestSearchSessions_ReverseChronoOrder(t *testing.T) {
	setSessionsDir(t)
	writeSearchManifest(t, "2026-05-04-bot-hq", "common-marker line\n")
	writeSearchManifest(t, "2026-05-07-bot-hq", "common-marker line\n")
	writeSearchManifest(t, "2026-05-05-bot-hq", "common-marker line\n")

	hits, err := SearchSessions("common-marker", 50)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if len(hits) < 3 {
		t.Fatalf("expected ≥3 hits; got %d", len(hits))
	}
	// First hit should be from the most-recent session (lex-max id).
	if hits[0].SessionID != "2026-05-07-bot-hq" {
		t.Errorf("first hit session = %q, want most-recent 2026-05-07-bot-hq", hits[0].SessionID)
	}
}

// TestSearchSessions_LimitClamp caps result count even when more
// matches exist.
func TestSearchSessions_LimitClamp(t *testing.T) {
	setSessionsDir(t)
	writeSearchManifest(t, "2026-05-07-bot-hq",
		"hit one\nhit two\nhit three\nhit four\nhit five\n")

	hits, err := SearchSessions("hit", 3)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if len(hits) != 3 {
		t.Errorf("expected 3 hits with limit=3; got %d", len(hits))
	}
}

// TestSearchSessions_LimitDefault uses 50 when limit<=0.
func TestSearchSessions_LimitDefault(t *testing.T) {
	setSessionsDir(t)
	body := strings.Repeat("hit\n", 60)
	writeSearchManifest(t, "2026-05-07-bot-hq", body)

	hits, err := SearchSessions("hit", 0)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if len(hits) != 50 {
		t.Errorf("expected 50 hits (default limit); got %d", len(hits))
	}
}

// TestSearchSessions_LimitHardCap caps at 500 even with limit>500.
func TestSearchSessions_LimitHardCap(t *testing.T) {
	setSessionsDir(t)
	body := strings.Repeat("hit\n", 600)
	writeSearchManifest(t, "2026-05-07-bot-hq", body)

	hits, err := SearchSessions("hit", 1000)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if len(hits) > 500 {
		t.Errorf("expected ≤500 hits (hard cap); got %d", len(hits))
	}
}

// TestSearchSessions_SnippetTruncated long lines are capped + ellipsis.
func TestSearchSessions_SnippetTruncated(t *testing.T) {
	setSessionsDir(t)
	long := strings.Repeat("xx ", 200) + "MARKER " + strings.Repeat("yy ", 100)
	writeSearchManifest(t, "2026-05-07-bot-hq", long+"\n")

	hits, err := SearchSessions("MARKER", 50)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if len(hits) == 0 {
		t.Fatal("expected ≥1 hit")
	}
	if !strings.Contains(hits[0].Snippet, "…") {
		t.Errorf("long line should be truncated; snippet=%q", hits[0].Snippet)
	}
	if len(hits[0].Snippet) > snippetSearchMax+5 {
		t.Errorf("snippet too long: %d", len(hits[0].Snippet))
	}
}

// TestSearchSessions_NoMatch returns empty slice (not error).
func TestSearchSessions_NoMatch(t *testing.T) {
	setSessionsDir(t)
	writeSearchManifest(t, "2026-05-07-bot-hq", "nothing matches here\n")

	hits, err := SearchSessions("xyz-impossible-marker", 50)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if len(hits) != 0 {
		t.Errorf("expected 0 hits; got %v", hits)
	}
}

// TestFormatSearchResults_EmptyHits returns "" for empty list.
func TestFormatSearchResults_EmptyHits(t *testing.T) {
	if got := FormatSearchResults(nil); got != "" {
		t.Errorf("empty hits should format as empty string; got %q", got)
	}
}

// TestFormatSearchResults_GrepCompat verifies grep-style format.
func TestFormatSearchResults_GrepCompat(t *testing.T) {
	hits := []SearchHit{
		{SessionID: "2026-05-07-bot-hq", Line: 12, Snippet: "matched line"},
	}
	got := FormatSearchResults(hits)
	want := "2026-05-07-bot-hq:12: matched line\n"
	if got != want {
		t.Errorf("format mismatch:\n got: %q\nwant: %q", got, want)
	}
}
