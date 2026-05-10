package sessions

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestExtractDecisionsFromMessages(t *testing.T) {
	msgs := []protocol.Message{
		{ID: 100, Content: "BRAIN-AGREED on the refactor approach"},
		{ID: 101, Content: "let's pivot the strategy"}, // no keyword match
		{ID: 102, Content: "[CRITICAL] plan-cap halt fired — HALT"},
		{ID: 103, Content: "scope-lock approved per phase-r.md SCOPE-LOCK"},
		{ID: 104, Content: "GREENFLAG to proceed with U-pre cleanup"},
		{ID: 105, Content: "regular work message, no decision keyword"},
	}
	got := ExtractDecisionsFromMessages(msgs)
	if len(got) != 4 {
		t.Fatalf("expected 4 decisions, got %d: %v", len(got), got)
	}

	// Each entry must contain its msg ID + the matched keyword (uppercased)
	wantSubs := map[string][]string{
		"100": {"BRAIN-AGREED"},
		"102": {"HALT"},
		"103": {"SCOPE-LOCK"},
		"104": {"GREENFLAG"},
	}
	for _, entry := range got {
		var msgID string
		if i := strings.Index(entry, ":"); i > 0 {
			msgID = entry[:i]
		}
		expected, ok := wantSubs[msgID]
		if !ok {
			t.Errorf("unexpected entry: %q", entry)
			continue
		}
		for _, sub := range expected {
			if !strings.Contains(entry, sub) {
				t.Errorf("entry %q missing %q", entry, sub)
			}
		}
	}
}

func TestExtractDecisionsFromMessages_TruncatesGloss(t *testing.T) {
	long := strings.Repeat("x", 200)
	msgs := []protocol.Message{
		{ID: 1, Content: "BRAIN-AGREED " + long},
	}
	got := ExtractDecisionsFromMessages(msgs)
	if len(got) != 1 {
		t.Fatalf("expected 1 entry")
	}
	if !strings.HasSuffix(got[0], "...") {
		t.Errorf("expected truncation marker, got %q", got[0])
	}
	// Total entry length stays bounded — caller relies on this for log readability
	if len(got[0]) > 120 {
		t.Errorf("entry too long: %d chars (%q)", len(got[0]), got[0])
	}
}

func TestExtractGitChanges_EmptyRepoPath(t *testing.T) {
	commits, files, err := ExtractGitChanges("", time.Now().Add(-time.Hour), time.Now())
	if err != nil {
		t.Errorf("empty repo path should be no-op, got err: %v", err)
	}
	if commits != nil || files != nil {
		t.Errorf("expected nil/nil for empty repo path, got commits=%v files=%v", commits, files)
	}
}

func TestFinalize_RoundTrip(t *testing.T) {
	dir := t.TempDir()
	t.Setenv(sessionsDirEnvVar, dir)

	// Seed an open manifest
	open := Manifest{
		ID:         "2026-05-10-test",
		Project:    "test",
		StartTS:    time.Date(2026, 5, 10, 9, 0, 0, 0, time.UTC),
		StartMsgID: 100,
		Agents:     []string{"brian"},
	}
	if err := WriteManifest(open); err != nil {
		t.Fatal(err)
	}

	// Finalize with messages + outcome (no repo path → skip git)
	endTime := time.Date(2026, 5, 10, 11, 0, 0, 0, time.UTC)
	out, err := Finalize(open.ID, FinalizeOptions{
		Outcome: "Refactored gemma plan-usage. Rain caught a stat-claim drift; addressed.",
		Status:  "closed",
		Now:     endTime,
		Messages: []protocol.Message{
			{ID: 105, Content: "BRAIN-AGREED on the refactor approach"},
			{ID: 215, Content: "GREENFLAG to ship"},
		},
		LatestMsgID: 287,
	})
	if err != nil {
		t.Fatalf("Finalize: %v", err)
	}

	if out.Status != "closed" {
		t.Errorf("Status = %q, want closed", out.Status)
	}
	if !out.EndTS.Equal(endTime) {
		t.Errorf("EndTS = %v, want %v", out.EndTS, endTime)
	}
	if out.MsgCount != 187 {
		t.Errorf("MsgCount = %d, want 187 (287-100)", out.MsgCount)
	}
	if out.EndMsgID != 287 {
		t.Errorf("EndMsgID = %d, want 287", out.EndMsgID)
	}
	if len(out.Decisions) != 2 {
		t.Errorf("Decisions = %v, want 2 entries", out.Decisions)
	}
	if !strings.Contains(out.Outcome, "Refactored gemma") {
		t.Errorf("Outcome lost: %q", out.Outcome)
	}

	// Round-trip through ReadManifest
	read, err := ReadManifest(open.ID)
	if err != nil {
		t.Fatal(err)
	}
	if read.MsgCount != out.MsgCount || read.Status != out.Status || len(read.Decisions) != len(out.Decisions) {
		t.Errorf("round-trip mismatch: read=%+v vs finalized=%+v", read, out)
	}
}

func TestFinalize_RequiresOutcome(t *testing.T) {
	t.Setenv(sessionsDirEnvVar, t.TempDir())
	if err := WriteManifest(Manifest{ID: "x", Project: "p", StartTS: time.Now()}); err != nil {
		t.Fatal(err)
	}
	_, err := Finalize("x", FinalizeOptions{Outcome: ""})
	if err == nil {
		t.Error("Finalize must reject empty Outcome")
	}
}

func TestFindActiveForProject(t *testing.T) {
	dir := t.TempDir()
	t.Setenv(sessionsDirEnvVar, dir)
	when := time.Date(2026, 5, 10, 0, 0, 0, 0, time.UTC)

	// Seed: one closed bot-hq session, one active bot-hq session, one active bcc session
	if err := WriteManifest(Manifest{
		ID: "2026-05-09-bot-hq", Project: "bot-hq",
		StartTS: when.Add(-24 * time.Hour),
		EndTS:   when.Add(-12 * time.Hour),
		Status:  "closed",
		Outcome: "yesterday's work; closed normally",
	}); err != nil {
		t.Fatal(err)
	}
	if err := WriteManifest(Manifest{
		ID: "2026-05-10-bot-hq", Project: "bot-hq",
		StartTS: when.Add(time.Hour),
	}); err != nil {
		t.Fatal(err)
	}
	if err := WriteManifest(Manifest{
		ID: "2026-05-10-bcc-ad-manager", Project: "bcc-ad-manager",
		StartTS: when.Add(2 * time.Hour),
	}); err != nil {
		t.Fatal(err)
	}

	got, err := FindActiveForProject("bot-hq")
	if err != nil {
		t.Fatal(err)
	}
	if got != "2026-05-10-bot-hq" {
		t.Errorf("active bot-hq = %q, want 2026-05-10-bot-hq", got)
	}

	gotEmpty, err := FindActiveForProject("nonexistent")
	if err != nil {
		t.Fatal(err)
	}
	if gotEmpty != "" {
		t.Errorf("nonexistent project should have no active session, got %q", gotEmpty)
	}
}

func TestFindActiveForAnyOtherProject(t *testing.T) {
	dir := t.TempDir()
	t.Setenv(sessionsDirEnvVar, dir)
	when := time.Date(2026, 5, 10, 0, 0, 0, 0, time.UTC)

	// bot-hq active, bcc active
	for _, m := range []Manifest{
		{ID: "2026-05-10-bot-hq", Project: "bot-hq", StartTS: when},
		{ID: "2026-05-10-bcc-ad-manager", Project: "bcc-ad-manager", StartTS: when.Add(time.Hour)},
	} {
		if err := WriteManifest(m); err != nil {
			t.Fatal(err)
		}
	}

	// Currently working on bot-hq → should detect bcc as the other active
	otherID, otherProject, err := FindActiveForAnyOtherProject("bot-hq")
	if err != nil {
		t.Fatal(err)
	}
	if otherID != "2026-05-10-bcc-ad-manager" || otherProject != "bcc-ad-manager" {
		t.Errorf("FindActiveForAnyOtherProject(bot-hq) = (%q, %q), want (2026-05-10-bcc-ad-manager, bcc-ad-manager)", otherID, otherProject)
	}

	// Currently working on bcc-ad-manager → should detect bot-hq
	otherID2, _, err := FindActiveForAnyOtherProject("bcc-ad-manager")
	if err != nil {
		t.Fatal(err)
	}
	if otherID2 != "2026-05-10-bot-hq" {
		t.Errorf("FindActiveForAnyOtherProject(bcc) = %q, want 2026-05-10-bot-hq", otherID2)
	}

	// Close all → no other-project active
	for _, id := range []string{"2026-05-10-bot-hq", "2026-05-10-bcc-ad-manager"} {
		m, _ := ReadManifest(id)
		m.EndTS = when.Add(2 * time.Hour)
		m.Status = "closed"
		m.Outcome = "test close"
		if err := WriteManifest(m); err != nil {
			t.Fatal(err)
		}
	}
	idEmpty, _, err := FindActiveForAnyOtherProject("bot-hq")
	if err != nil {
		t.Fatal(err)
	}
	if idEmpty != "" {
		t.Errorf("expected no other-project active after closing all, got %q", idEmpty)
	}

	// Sanity: temp dir really had the manifests we expect
	contents, _ := os.ReadDir(dir)
	if len(contents) < 2 {
		t.Errorf("seed sanity: expected ≥2 session dirs, got %d", len(contents))
	}
	_ = filepath.Join // keep import alive for future test extensions
}
