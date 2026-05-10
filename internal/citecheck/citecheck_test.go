// Phase T T-12 cycle-3: cite-check tests. R39 TEST-ISOLATION via pure
// MsgLookup mocks; no DB or filesystem.

package citecheck

import (
	"errors"
	"strings"
	"testing"
)

// fakeLookup builds a MsgLookup over a fixed existence set.
func fakeLookup(existing ...int64) MsgLookup {
	set := make(map[int64]struct{}, len(existing))
	for _, id := range existing {
		set[id] = struct{}{}
	}
	return func(id int64) (bool, error) {
		_, ok := set[id]
		return ok, nil
	}
}

func TestInspect_DetectsMissingMsgIDs(t *testing.T) {
	lookup := fakeLookup(17454)
	content := "BRIAN BRAIN-2nd-SEAL on msg 17456 — covers msg 17454 too"

	concerns := Inspect(content, lookup)
	if len(concerns) != 1 {
		t.Fatalf("len(concerns) = %d, want 1; got %#v", len(concerns), concerns)
	}
	if concerns[0].CitedID != 17456 {
		t.Errorf("CitedID = %d, want 17456", concerns[0].CitedID)
	}
	if !strings.Contains(concerns[0].Reason, "not found") {
		t.Errorf("Reason = %q, want substring 'not found'", concerns[0].Reason)
	}
}

func TestInspect_PassesAllExistingCites(t *testing.T) {
	lookup := fakeLookup(17454, 17456, 17460)
	content := "rain msg 17454 + brian msg-17456 + user msg 17460"

	concerns := Inspect(content, lookup)
	if len(concerns) != 0 {
		t.Errorf("expected zero concerns; got %#v", concerns)
	}
}

func TestInspect_RecognizesMultipleCiteShapes(t *testing.T) {
	lookup := fakeLookup() // none exist; we want all 3 shapes to be detected as "not found"
	content := "covers msg 100, msg-200, and id=300"

	concerns := Inspect(content, lookup)
	if len(concerns) != 3 {
		t.Errorf("expected 3 concerns (one per cite shape); got %d: %#v", len(concerns), concerns)
	}
	wantIDs := map[int64]bool{100: true, 200: true, 300: true}
	for _, c := range concerns {
		if !wantIDs[c.CitedID] {
			t.Errorf("unexpected CitedID %d", c.CitedID)
		}
	}
}

func TestInspect_DeduplicatesRepeatedCites(t *testing.T) {
	lookup := fakeLookup() // none exist
	content := "covers msg 17456; per msg 17456; again msg 17456"

	concerns := Inspect(content, lookup)
	if len(concerns) != 1 {
		t.Errorf("expected 1 concern (deduplicated); got %d", len(concerns))
	}
}

func TestInspect_LookupErrorSurfacesAsConcern(t *testing.T) {
	lookup := func(id int64) (bool, error) {
		return false, errors.New("db unreachable")
	}
	content := "see msg 17456"

	concerns := Inspect(content, lookup)
	if len(concerns) != 1 {
		t.Fatalf("expected 1 concern; got %d", len(concerns))
	}
	if !strings.Contains(concerns[0].Reason, "lookup error") {
		t.Errorf("Reason = %q, want 'lookup error' substring", concerns[0].Reason)
	}
}

func TestInspect_DoesNotFalseFireOnNonCiteSubstrings(t *testing.T) {
	lookup := fakeLookup()
	cases := []string{
		"",
		"compact-pipe-content-no-msg-id",
		"`lastMsgID` is camelcase variable",   // word-boundary protects
		"discord-channel-1496436363761946796", // long numeric, no msg/id prefix
		"sha eae04a3 git",                     // hex, not numeric
	}
	for _, content := range cases {
		concerns := Inspect(content, lookup)
		if len(concerns) != 0 {
			t.Errorf("Inspect(%q) unexpectedly fired: %#v", content, concerns)
		}
	}
}

func TestInspect_NilLookupReturnsNil(t *testing.T) {
	concerns := Inspect("msg 17456", nil)
	if concerns != nil {
		t.Errorf("expected nil concerns for nil lookup; got %#v", concerns)
	}
}

func TestFormatNotice_EmptyForCleanContent(t *testing.T) {
	got := FormatNotice(nil)
	if got != "" {
		t.Errorf("FormatNotice(nil) = %q, want empty", got)
	}
}

func TestFormatNotice_JoinsConcerns(t *testing.T) {
	concerns := []Concern{
		{CitedID: 100, Reason: "msg-id not found in hub.db", MatchSpan: "msg 100"},
		{CitedID: 200, Reason: "lookup error: db down", MatchSpan: "msg-200"},
	}
	got := FormatNotice(concerns)
	for _, want := range []string{"cite-check:", "msg 100", "msg-200", "not found", "lookup error"} {
		if !strings.Contains(got, want) {
			t.Errorf("FormatNotice missing %q; got %q", want, got)
		}
	}
}
