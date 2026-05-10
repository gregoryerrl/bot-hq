package sessions

import (
	"strings"
	"testing"
	"time"
)

func TestSummarizeDate_Empty(t *testing.T) {
	t.Setenv(sessionsDirEnvVar, t.TempDir())
	when := time.Date(2026, 5, 10, 0, 0, 0, 0, time.UTC)
	md, err := SummarizeDate(when)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(md, "No sessions found") {
		t.Errorf("empty summary missing stub: %q", md)
	}
}

func TestSummarizeDate_AggregatesAcrossProjects(t *testing.T) {
	t.Setenv(sessionsDirEnvVar, t.TempDir())
	when := time.Date(2026, 5, 10, 0, 0, 0, 0, time.UTC)

	// Two bot-hq sessions (one closed, one active) + one BCC session (closed)
	for _, m := range []Manifest{
		{
			ID:            "2026-05-10-bot-hq",
			Project:       "bot-hq",
			StartTS:       when.Add(time.Hour),
			EndTS:         when.Add(3 * time.Hour),
			Status:        "closed",
			Agents:        []string{"brian", "rain"},
			Outcome:       "Shipped W-1 + W-2 + W-3.",
			CommitsLanded: []string{"abc1", "def2", "ghi3"},
			FilesTouched:  []string{"a.go", "b.go"},
			Decisions:     []string{"100: GREENFLAG | ship"},
			MsgCount:      45,
		},
		{
			ID:      "2026-05-10-bot-hq-2",
			Project: "bot-hq",
			StartTS: when.Add(4 * time.Hour),
			Agents:  []string{"brian"},
		},
		{
			ID:            "2026-05-10-bcc-ad-manager",
			Project:       "bcc-ad-manager",
			StartTS:       when.Add(2 * time.Hour),
			EndTS:         when.Add(5 * time.Hour),
			Status:        "closed-eod",
			Agents:        []string{"brian"},
			Outcome:       "Fixed staging deploy.",
			CommitsLanded: []string{"xyz9"},
			FilesTouched:  []string{"deploy.sh"},
		},
	} {
		if err := WriteManifest(m); err != nil {
			t.Fatal(err)
		}
	}

	md, err := SummarizeDate(when)
	if err != nil {
		t.Fatal(err)
	}

	wantSubs := []string{
		"# Session summary: 2026-05-10",
		"**Sessions:** 3 (across 2 projects, 2 closed / 1 still active)",
		"**Total commits landed:** 4", // 3 + 0 + 1
		"**Total files touched (sum, may double-count across sessions):** 3", // 2 + 0 + 1
		"## bcc-ad-manager",
		"## bot-hq",
		"### 2026-05-10-bot-hq — closed",
		"### 2026-05-10-bot-hq-2 — active",
		"### 2026-05-10-bcc-ad-manager — closed-eod",
		"Shipped W-1",
		"Fixed staging deploy",
		"_(active — no outcome yet)_",
	}
	for _, sub := range wantSubs {
		if !strings.Contains(md, sub) {
			t.Errorf("summary missing %q\nFull output:\n%s", sub, md)
		}
	}
}

func TestMigrateStaleActive(t *testing.T) {
	t.Setenv(sessionsDirEnvVar, t.TempDir())
	cutoff := time.Date(2026, 5, 10, 0, 0, 0, 0, time.UTC)

	// Stale active (older than cutoff)
	stale := Manifest{
		ID:      "2026-05-08-bot-hq",
		Project: "bot-hq",
		StartTS: cutoff.Add(-48 * time.Hour),
		Agents:  []string{"brian"},
	}
	// Today's active (must NOT be migrated)
	today := Manifest{
		ID:      "2026-05-10-bot-hq",
		Project: "bot-hq",
		StartTS: cutoff.Add(time.Hour),
		Agents:  []string{"brian"},
	}
	// Already closed (must NOT be migrated)
	closed := Manifest{
		ID:      "2026-05-09-bcc-ad-manager",
		Project: "bcc-ad-manager",
		StartTS: cutoff.Add(-24 * time.Hour),
		EndTS:   cutoff.Add(-12 * time.Hour),
		Status:  "closed",
		Outcome: "Pre-existing closure",
	}
	for _, m := range []Manifest{stale, today, closed} {
		if err := WriteManifest(m); err != nil {
			t.Fatal(err)
		}
	}

	migrated, err := MigrateStaleActive(cutoff)
	if err != nil {
		t.Fatal(err)
	}

	if len(migrated) != 1 || migrated[0] != stale.ID {
		t.Errorf("expected to migrate only %q, got %v", stale.ID, migrated)
	}

	// Verify the migrated session got the right status + synthetic outcome
	got, err := ReadManifest(stale.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got.Status != "closed-auto-migrated" {
		t.Errorf("Status = %q, want closed-auto-migrated", got.Status)
	}
	if !strings.Contains(got.Outcome, "Auto-migrated by Phase W") {
		t.Errorf("synthetic outcome missing: %q", got.Outcome)
	}
	if got.EndTS.IsZero() {
		t.Errorf("EndTS not set after migration")
	}

	// Today's still active
	todayRead, _ := ReadManifest(today.ID)
	if !todayRead.EndTS.IsZero() {
		t.Errorf("today's active should NOT be migrated; EndTS = %v", todayRead.EndTS)
	}

	// Already-closed unchanged
	closedRead, _ := ReadManifest(closed.ID)
	if closedRead.Status != "closed" {
		t.Errorf("pre-closed status changed: %q (expected closed)", closedRead.Status)
	}

	// Idempotency: re-run returns nothing
	again, err := MigrateStaleActive(cutoff)
	if err != nil {
		t.Fatal(err)
	}
	if len(again) != 0 {
		t.Errorf("re-run migrated additional %v (should be idempotent)", again)
	}
}
