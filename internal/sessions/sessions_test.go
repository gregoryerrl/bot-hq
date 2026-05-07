package sessions

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func setSessionsDir(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv(sessionsDirEnvVar, dir)
	return dir
}

func TestMakeSessionID(t *testing.T) {
	cases := []struct {
		name    string
		t       time.Time
		project string
		want    string
	}{
		{"basic", time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC), "bcc-ad-manager", "2026-05-05-bcc-ad-manager"},
		{"lowercases project", time.Date(2026, 5, 5, 0, 0, 0, 0, time.UTC), "BotHQ", "2026-05-05-bothq"},
		{"utc-converts", time.Date(2026, 5, 5, 23, 0, 0, 0, time.FixedZone("PST", -8*3600)), "p", "2026-05-06-p"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := MakeSessionID(c.t, c.project); got != c.want {
				t.Errorf("MakeSessionID(%v, %q) = %q; want %q", c.t, c.project, got, c.want)
			}
		})
	}
}

func TestDetectBoundaryFromUserMsg(t *testing.T) {
	cases := []struct {
		name string
		msg  string
		want BoundaryTrigger
	}{
		// T-3 explicit-phrase positive cases (RATIFIED set per OQ-1a)
		{"lets switch to", "let's switch to bcc-ad-manager", TriggerExplicitPhrase},
		{"lets pivot to", "lets pivot to bot-hq", TriggerExplicitPhrase},
		{"new session", "ok new session please", TriggerExplicitPhrase},
		{"clean slate", "let's start with a clean slate", TriggerExplicitPhrase},
		{"new arc", "open new arc for phase O", TriggerExplicitPhrase},
		{"EOD", "EOD: wrapping up", TriggerExplicitPhrase},
		{"wrap up", "let's wrap-up here", TriggerExplicitPhrase},
		{"done for today", "done for today, see you tomorrow", TriggerExplicitPhrase},
		{"signing off", "signing off now", TriggerExplicitPhrase},
		// T-6 rebuild-restart positive cases
		{"rebuild+restart with plus", "rebuild+restart in flight", TriggerRebuildRestart},
		{"rebuild restart with space", "do a rebuild restart please", TriggerRebuildRestart},
		{"restart session", "restart session now", TriggerRebuildRestart},
		// T-6 takes precedence over T-3 when both match
		{"both T-6 wins", "EOD; let's do a rebuild+restart", TriggerRebuildRestart},
		// negative cases
		{"empty", "", TriggerNone},
		{"unrelated text", "the deploy looks good", TriggerNone},
		{"random word match avoided", "newsession", TriggerNone}, // no word boundary
		{"plain restart of foo", "restart of the deploy", TriggerNone},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := DetectBoundaryFromUserMsg(c.msg); got != c.want {
				t.Errorf("DetectBoundaryFromUserMsg(%q) = %q; want %q", c.msg, got, c.want)
			}
		})
	}
}

func TestSessionsDirEnvOverride(t *testing.T) {
	dir := setSessionsDir(t)
	if got := SessionsDir(); got != dir {
		t.Errorf("SessionsDir() = %q; want %q", got, dir)
	}
}

func TestManifestPath(t *testing.T) {
	dir := setSessionsDir(t)
	want := filepath.Join(dir, "2026-05-05-foo", "manifest.md")
	if got := ManifestPath("2026-05-05-foo"); got != want {
		t.Errorf("ManifestPath = %q; want %q", got, want)
	}
}

func TestWriteManifestRequiresID(t *testing.T) {
	setSessionsDir(t)
	err := WriteManifest(Manifest{Project: "x"})
	if err == nil {
		t.Fatalf("expected error for missing ID")
	}
	if !strings.Contains(err.Error(), "ID required") {
		t.Errorf("error = %v; want substring 'ID required'", err)
	}
}

func TestWriteManifestMinimal(t *testing.T) {
	setSessionsDir(t)
	m := Manifest{
		ID:      "2026-05-05-bot-hq",
		Project: "bot-hq",
		StartTS: time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC),
	}
	if err := WriteManifest(m); err != nil {
		t.Fatalf("WriteManifest: %v", err)
	}
	body, err := os.ReadFile(ManifestPath(m.ID))
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	got := string(body)
	wantSubstrings := []string{
		"---\n",
		"id: 2026-05-05-bot-hq\n",
		"project: bot-hq\n",
		"start_ts: 2026-05-05T12:00:00Z\n",
		"---\n\n",
	}
	for _, s := range wantSubstrings {
		if !strings.Contains(got, s) {
			t.Errorf("manifest missing substring %q; got:\n%s", s, got)
		}
	}
}

func TestWriteManifestFull(t *testing.T) {
	setSessionsDir(t)
	m := Manifest{
		ID:              "2026-05-05-bcc-ad-manager",
		Project:         "bcc-ad-manager",
		StartTS:         time.Date(2026, 5, 5, 9, 0, 0, 0, time.UTC),
		EndTS:           time.Date(2026, 5, 5, 17, 30, 0, 0, time.UTC),
		StartMsgID:      7900,
		EndMsgID:        7980,
		Agents:          []string{"brian", "rain", "emma"},
		PivotInMsgID:    7905,
		PivotOutMsgID:   7975,
		ParentSessionID: "2026-05-05-bot-hq",
		Body:            "## Deliverables\n- PR #368\n- PR #369\n",
	}
	if err := WriteManifest(m); err != nil {
		t.Fatalf("WriteManifest: %v", err)
	}
	body, _ := os.ReadFile(ManifestPath(m.ID))
	got := string(body)
	wantSubstrings := []string{
		"end_ts: 2026-05-05T17:30:00Z\n",
		"start_msg_id: 7900\n",
		"end_msg_id: 7980\n",
		"agents:\n  - brian\n  - rain\n  - emma\n",
		"pivot_in_msg_id: 7905\n",
		"pivot_out_msg_id: 7975\n",
		"parent_session_id: 2026-05-05-bot-hq\n",
		"## Deliverables\n- PR #368\n- PR #369\n",
	}
	for _, s := range wantSubstrings {
		if !strings.Contains(got, s) {
			t.Errorf("manifest missing substring %q; got:\n%s", s, got)
		}
	}
}

func TestWriteManifestIdempotent(t *testing.T) {
	setSessionsDir(t)
	m := Manifest{ID: "2026-05-05-foo", Project: "foo", StartTS: time.Date(2026, 5, 5, 0, 0, 0, 0, time.UTC)}
	if err := WriteManifest(m); err != nil {
		t.Fatalf("first write: %v", err)
	}
	first, _ := os.ReadFile(ManifestPath(m.ID))
	if err := WriteManifest(m); err != nil {
		t.Fatalf("second write: %v", err)
	}
	second, _ := os.ReadFile(ManifestPath(m.ID))
	if string(first) != string(second) {
		t.Errorf("idempotency broken: outputs differ across two writes\nfirst:\n%s\nsecond:\n%s", first, second)
	}
}

func TestWriteManifestUpdate(t *testing.T) {
	setSessionsDir(t)
	m := Manifest{ID: "2026-05-05-foo", Project: "foo", StartTS: time.Date(2026, 5, 5, 0, 0, 0, 0, time.UTC)}
	if err := WriteManifest(m); err != nil {
		t.Fatal(err)
	}
	m.EndTS = time.Date(2026, 5, 5, 18, 0, 0, 0, time.UTC)
	m.Body = "## Final\nshipped.\n"
	if err := WriteManifest(m); err != nil {
		t.Fatal(err)
	}
	body, _ := os.ReadFile(ManifestPath(m.ID))
	got := string(body)
	if !strings.Contains(got, "end_ts: 2026-05-05T18:00:00Z\n") {
		t.Errorf("missing updated end_ts in: %s", got)
	}
	if !strings.Contains(got, "## Final") {
		t.Errorf("missing updated body in: %s", got)
	}
}

func TestReadManifestRoundtrip(t *testing.T) {
	setSessionsDir(t)
	original := Manifest{
		ID:              "2026-05-05-roundtrip",
		Project:         "test",
		StartTS:         time.Date(2026, 5, 5, 10, 0, 0, 0, time.UTC),
		EndTS:           time.Date(2026, 5, 5, 18, 0, 0, 0, time.UTC),
		StartMsgID:      100,
		EndMsgID:        200,
		Agents:          []string{"brian", "rain"},
		PivotInMsgID:    105,
		ParentSessionID: "parent-id",
		Body:            "## Section\nbody content here.\n",
	}
	if err := WriteManifest(original); err != nil {
		t.Fatal(err)
	}
	got, err := ReadManifest(original.ID)
	if err != nil {
		t.Fatalf("ReadManifest: %v", err)
	}
	if got.ID != original.ID {
		t.Errorf("ID: got %q want %q", got.ID, original.ID)
	}
	if got.Project != original.Project {
		t.Errorf("Project: got %q want %q", got.Project, original.Project)
	}
	if !got.StartTS.Equal(original.StartTS) {
		t.Errorf("StartTS: got %v want %v", got.StartTS, original.StartTS)
	}
	if !got.EndTS.Equal(original.EndTS) {
		t.Errorf("EndTS: got %v want %v", got.EndTS, original.EndTS)
	}
	if got.StartMsgID != original.StartMsgID || got.EndMsgID != original.EndMsgID {
		t.Errorf("MsgID range: got (%d,%d) want (%d,%d)", got.StartMsgID, got.EndMsgID, original.StartMsgID, original.EndMsgID)
	}
	if len(got.Agents) != 2 || got.Agents[0] != "brian" || got.Agents[1] != "rain" {
		t.Errorf("Agents: got %v want %v", got.Agents, original.Agents)
	}
	if got.PivotInMsgID != original.PivotInMsgID {
		t.Errorf("PivotInMsgID: got %d want %d", got.PivotInMsgID, original.PivotInMsgID)
	}
	if got.ParentSessionID != original.ParentSessionID {
		t.Errorf("ParentSessionID: got %q want %q", got.ParentSessionID, original.ParentSessionID)
	}
	if !strings.Contains(got.Body, "## Section") {
		t.Errorf("Body: got %q want substring '## Section'", got.Body)
	}
}

func TestReadManifestMissingFile(t *testing.T) {
	setSessionsDir(t)
	_, err := ReadManifest("nonexistent")
	if err == nil {
		t.Fatalf("expected error for missing manifest")
	}
	if !os.IsNotExist(err) {
		t.Errorf("expected os.IsNotExist error; got: %v", err)
	}
}

func TestReadManifestMalformedNoOpenMarker(t *testing.T) {
	dir := setSessionsDir(t)
	id := "malformed"
	manifestDir := filepath.Join(dir, id)
	if err := os.MkdirAll(manifestDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(manifestDir, "manifest.md"), []byte("no frontmatter here"), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := ReadManifest(id)
	if err == nil || !strings.Contains(err.Error(), "frontmatter open marker") {
		t.Errorf("expected frontmatter-open-marker error; got: %v", err)
	}
}

func TestReadManifestMalformedNoCloseMarker(t *testing.T) {
	dir := setSessionsDir(t)
	id := "malformed2"
	manifestDir := filepath.Join(dir, id)
	if err := os.MkdirAll(manifestDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(manifestDir, "manifest.md"), []byte("---\nid: x\nno close marker"), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := ReadManifest(id)
	if err == nil || !strings.Contains(err.Error(), "frontmatter close marker") {
		t.Errorf("expected frontmatter-close-marker error; got: %v", err)
	}
}

func TestLoadManifestContent(t *testing.T) {
	setSessionsDir(t)
	m := Manifest{
		ID:      "2026-05-05-loadtest",
		Project: "test",
		StartTS: time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC),
		Body:    "## Body\nhello\n",
	}
	if err := WriteManifest(m); err != nil {
		t.Fatal(err)
	}
	content, err := LoadManifestContent(m.ID)
	if err != nil {
		t.Fatalf("LoadManifestContent: %v", err)
	}
	if !strings.Contains(content, "id: 2026-05-05-loadtest") {
		t.Errorf("missing id in content; got: %s", content)
	}
	if !strings.Contains(content, "## Body") {
		t.Errorf("missing body in content; got: %s", content)
	}
}

func TestLoadManifestContentMissing(t *testing.T) {
	setSessionsDir(t)
	_, err := LoadManifestContent("nonexistent")
	if err == nil {
		t.Fatalf("expected error for missing manifest")
	}
	if !os.IsNotExist(err) {
		t.Errorf("expected os.IsNotExist; got: %v", err)
	}
}

func TestListSessionIDsEmpty(t *testing.T) {
	setSessionsDir(t)
	ids, err := ListSessionIDs()
	if err != nil {
		t.Fatalf("ListSessionIDs on empty dir: %v", err)
	}
	if len(ids) != 0 {
		t.Errorf("expected empty list; got: %v", ids)
	}
}

func TestListSessionIDsMissingDir(t *testing.T) {
	t.Setenv(sessionsDirEnvVar, filepath.Join(t.TempDir(), "does-not-exist"))
	ids, err := ListSessionIDs()
	if err != nil {
		t.Fatalf("ListSessionIDs on missing dir should not error; got: %v", err)
	}
	if len(ids) != 0 {
		t.Errorf("expected empty list; got: %v", ids)
	}
}

func TestListSessionIDsSkipsFiles(t *testing.T) {
	dir := setSessionsDir(t)
	if err := os.MkdirAll(filepath.Join(dir, "2026-05-05-foo"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(dir, "2026-05-04-bar"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "index.md"), []byte("placeholder"), 0o644); err != nil {
		t.Fatal(err)
	}
	ids, err := ListSessionIDs()
	if err != nil {
		t.Fatal(err)
	}
	if len(ids) != 2 {
		t.Errorf("expected 2 session-id dirs; got %d: %v", len(ids), ids)
	}
	for _, id := range ids {
		if id == "index.md" {
			t.Errorf("index.md (file) should be skipped; got it in: %v", ids)
		}
	}
}

func TestMostRecentForProject(t *testing.T) {
	dir := setSessionsDir(t)
	mkdir := func(name string) {
		if err := os.MkdirAll(filepath.Join(dir, name), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	mkdir("2026-05-03-bot-hq")
	mkdir("2026-05-04-bot-hq")
	mkdir("2026-05-05-bot-hq")
	mkdir("2026-05-04-bcc-ad-manager")
	mkdir("2026-05-05-bcc-ad-manager")

	id, err := MostRecentForProject("bot-hq")
	if err != nil {
		t.Fatal(err)
	}
	if id != "2026-05-05-bot-hq" {
		t.Errorf("MostRecentForProject(bot-hq) = %q; want 2026-05-05-bot-hq", id)
	}

	id, err = MostRecentForProject("bcc-ad-manager")
	if err != nil {
		t.Fatal(err)
	}
	if id != "2026-05-05-bcc-ad-manager" {
		t.Errorf("MostRecentForProject(bcc-ad-manager) = %q; want 2026-05-05-bcc-ad-manager", id)
	}
}

func TestMostRecentForProjectCaseInsensitive(t *testing.T) {
	dir := setSessionsDir(t)
	if err := os.MkdirAll(filepath.Join(dir, "2026-05-05-bot-hq"), 0o755); err != nil {
		t.Fatal(err)
	}
	id, err := MostRecentForProject("BOT-HQ")
	if err != nil {
		t.Fatal(err)
	}
	if id != "2026-05-05-bot-hq" {
		t.Errorf("MostRecentForProject(BOT-HQ) = %q; want 2026-05-05-bot-hq", id)
	}
}

func TestMostRecentForProjectNoMatch(t *testing.T) {
	dir := setSessionsDir(t)
	if err := os.MkdirAll(filepath.Join(dir, "2026-05-05-foo"), 0o755); err != nil {
		t.Fatal(err)
	}
	id, err := MostRecentForProject("bar")
	if err != nil {
		t.Fatalf("expected nil error on no-match; got: %v", err)
	}
	if id != "" {
		t.Errorf("expected empty string on no-match; got: %q", id)
	}
}

func TestIndexPath(t *testing.T) {
	dir := setSessionsDir(t)
	want := filepath.Join(dir, "index.md")
	if got := IndexPath(); got != want {
		t.Errorf("IndexPath = %q; want %q", got, want)
	}
}

func TestWriteIndexEmpty(t *testing.T) {
	setSessionsDir(t)
	if err := WriteIndex(); err != nil {
		t.Fatalf("WriteIndex on empty: %v", err)
	}
	body, err := os.ReadFile(IndexPath())
	if err != nil {
		t.Fatalf("read index: %v", err)
	}
	got := string(body)
	if !strings.Contains(got, "# Bot-HQ Sessions Index") {
		t.Errorf("missing header in: %s", got)
	}
}

func TestWriteIndexGroupsByProjectSortedDesc(t *testing.T) {
	setSessionsDir(t)
	manifests := []Manifest{
		{ID: "2026-05-03-bot-hq", Project: "bot-hq", StartTS: time.Date(2026, 5, 3, 9, 0, 0, 0, time.UTC), EndTS: time.Date(2026, 5, 3, 18, 0, 0, 0, time.UTC), Agents: []string{"brian", "rain"}},
		{ID: "2026-05-05-bot-hq", Project: "bot-hq", StartTS: time.Date(2026, 5, 5, 9, 0, 0, 0, time.UTC), Agents: []string{"brian", "rain", "emma"}},
		{ID: "2026-05-04-bot-hq", Project: "bot-hq", StartTS: time.Date(2026, 5, 4, 9, 0, 0, 0, time.UTC), EndTS: time.Date(2026, 5, 4, 20, 0, 0, 0, time.UTC), Agents: []string{"brian", "rain"}},
		{ID: "2026-05-05-bcc-ad-manager", Project: "bcc-ad-manager", StartTS: time.Date(2026, 5, 5, 10, 0, 0, 0, time.UTC), EndTS: time.Date(2026, 5, 5, 17, 30, 0, 0, time.UTC), Agents: []string{"brian", "rain"}},
	}
	for _, m := range manifests {
		if err := WriteManifest(m); err != nil {
			t.Fatal(err)
		}
	}
	if err := WriteIndex(); err != nil {
		t.Fatalf("WriteIndex: %v", err)
	}
	body, _ := os.ReadFile(IndexPath())
	got := string(body)

	// Project sections (alphabetical: bcc-ad-manager before bot-hq)
	bccIdx := strings.Index(got, "## bcc-ad-manager")
	botIdx := strings.Index(got, "## bot-hq")
	if bccIdx < 0 || botIdx < 0 || bccIdx >= botIdx {
		t.Errorf("project sections out of order or missing; bccIdx=%d botIdx=%d; got:\n%s", bccIdx, botIdx, got)
	}

	// Within bot-hq section, most-recent (2026-05-05) comes before older
	id5 := strings.Index(got, "- 2026-05-05-bot-hq")
	id4 := strings.Index(got, "- 2026-05-05-bot-hq")
	id3 := strings.Index(got, "- 2026-05-03-bot-hq")
	id4_actual := strings.Index(got, "- 2026-05-04-bot-hq")
	if id5 < 0 || id4_actual < 0 || id3 < 0 {
		t.Fatalf("missing session-id rows: id5=%d id4=%d id3=%d in:\n%s", id5, id4_actual, id3, got)
	}
	if !(id5 < id4_actual && id4_actual < id3) {
		t.Errorf("session-ids not sorted DESC: id5=%d id4=%d id3=%d (want id5 < id4 < id3)", id5, id4_actual, id3)
	}
	_ = id4
}

func TestWriteIndexEntryFormat(t *testing.T) {
	setSessionsDir(t)
	m := Manifest{
		ID:      "2026-05-05-foo",
		Project: "foo",
		StartTS: time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC),
		EndTS:   time.Date(2026, 5, 5, 18, 30, 0, 0, time.UTC),
		Agents:  []string{"brian", "rain"},
	}
	if err := WriteManifest(m); err != nil {
		t.Fatal(err)
	}
	if err := WriteIndex(); err != nil {
		t.Fatal(err)
	}
	body, _ := os.ReadFile(IndexPath())
	want := "- 2026-05-05-foo | 2026-05-05T12:00:00Z | 2026-05-05T18:30:00Z | brian,rain | foo\n"
	if !strings.Contains(string(body), want) {
		t.Errorf("entry format mismatch; want substring %q; got:\n%s", want, body)
	}
}

func TestWriteIndexActiveSession(t *testing.T) {
	setSessionsDir(t)
	m := Manifest{
		ID:      "2026-05-05-active",
		Project: "active",
		StartTS: time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC),
		Agents:  []string{"brian"},
	}
	if err := WriteManifest(m); err != nil {
		t.Fatal(err)
	}
	if err := WriteIndex(); err != nil {
		t.Fatal(err)
	}
	body, _ := os.ReadFile(IndexPath())
	if !strings.Contains(string(body), "| active |") || !strings.Contains(string(body), "active") {
		t.Errorf("expected 'active' marker for missing end_ts; got: %s", body)
	}
}

func TestWriteIndexIdempotent(t *testing.T) {
	setSessionsDir(t)
	m := Manifest{ID: "2026-05-05-foo", Project: "foo", StartTS: time.Date(2026, 5, 5, 0, 0, 0, 0, time.UTC), Agents: []string{"brian"}}
	if err := WriteManifest(m); err != nil {
		t.Fatal(err)
	}
	if err := WriteIndex(); err != nil {
		t.Fatal(err)
	}
	first, _ := os.ReadFile(IndexPath())
	if err := WriteIndex(); err != nil {
		t.Fatal(err)
	}
	second, _ := os.ReadFile(IndexPath())
	if string(first) != string(second) {
		t.Errorf("WriteIndex not idempotent")
	}
}

func TestExplicitPhraseRegexCaseInsensitive(t *testing.T) {
	cases := []string{
		"LET'S SWITCH TO project",
		"New Session please",
		"Eod",
		"WRAP-UP time",
	}
	for _, c := range cases {
		t.Run(c, func(t *testing.T) {
			if !explicitPhraseRegex.MatchString(c) {
				t.Errorf("expected match for case-insensitive: %q", c)
			}
		})
	}
}

// Phase R R5 (d-2) tests for Manifest checkpoint fields + WriteCheckpoint helper.

func TestReadManifestBackwardsCompat_PreR5(t *testing.T) {
	setSessionsDir(t)
	// Pre-R5 manifest content without any checkpoint fields.
	id := "2026-05-08-precompat"
	dir := filepath.Join(SessionsDir(), id)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	preR5 := "---\nid: " + id + "\nproject: test\n---\n\nbody content\n"
	if err := os.WriteFile(filepath.Join(dir, "manifest.md"), []byte(preR5), 0o644); err != nil {
		t.Fatal(err)
	}
	got, err := ReadManifest(id)
	if err != nil {
		t.Fatalf("ReadManifest pre-R5: %v", err)
	}
	if got.ID != id || got.Project != "test" {
		t.Errorf("base fields lost: %+v", got)
	}
	// New fields must be zero-valued (Refine-B backwards-compat).
	if got.ActiveWorkstream != "" || got.LastCommitSHA != "" || got.Phase != "" || got.Posture != "" {
		t.Errorf("new fields should be zero-value, got: ActiveWorkstream=%q LastCommitSHA=%q Phase=%q Posture=%q",
			got.ActiveWorkstream, got.LastCommitSHA, got.Phase, got.Posture)
	}
	if !got.CheckpointTS.IsZero() {
		t.Errorf("CheckpointTS should be zero-value, got %v", got.CheckpointTS)
	}
}

func TestWriteManifestCheckpointFieldsRoundtrip(t *testing.T) {
	setSessionsDir(t)
	original := Manifest{
		ID:               "2026-05-08-checkpoint",
		Project:          "bot-hq",
		StartTS:          time.Date(2026, 5, 8, 0, 0, 0, 0, time.UTC),
		ActiveWorkstream: "Phase R R5 sessions",
		LastCommitSHA:    "26e1117",
		Phase:            "Phase-R-OPEN",
		Posture:          "HANDS",
		CheckpointTS:     time.Date(2026, 5, 8, 1, 23, 45, 0, time.UTC),
	}
	if err := WriteManifest(original); err != nil {
		t.Fatal(err)
	}
	got, err := ReadManifest(original.ID)
	if err != nil {
		t.Fatalf("ReadManifest: %v", err)
	}
	if got.ActiveWorkstream != original.ActiveWorkstream {
		t.Errorf("ActiveWorkstream: got %q want %q", got.ActiveWorkstream, original.ActiveWorkstream)
	}
	if got.LastCommitSHA != original.LastCommitSHA {
		t.Errorf("LastCommitSHA: got %q want %q", got.LastCommitSHA, original.LastCommitSHA)
	}
	if got.Phase != original.Phase {
		t.Errorf("Phase: got %q want %q", got.Phase, original.Phase)
	}
	if got.Posture != original.Posture {
		t.Errorf("Posture: got %q want %q", got.Posture, original.Posture)
	}
	if !got.CheckpointTS.Equal(original.CheckpointTS) {
		t.Errorf("CheckpointTS: got %v want %v", got.CheckpointTS, original.CheckpointTS)
	}
}

func TestWriteCheckpoint_MergesNonEmpty(t *testing.T) {
	setSessionsDir(t)
	id := "2026-05-08-merge"
	if err := WriteManifest(Manifest{ID: id, Project: "test", ActiveWorkstream: "initial"}); err != nil {
		t.Fatal(err)
	}
	// Update only LastCommitSHA + Phase; ActiveWorkstream should preserve.
	if err := WriteCheckpoint(id, CheckpointFields{LastCommitSHA: "abc1234", Phase: "Phase-R"}); err != nil {
		t.Fatal(err)
	}
	got, err := ReadManifest(id)
	if err != nil {
		t.Fatal(err)
	}
	if got.ActiveWorkstream != "initial" {
		t.Errorf("ActiveWorkstream should preserve, got %q", got.ActiveWorkstream)
	}
	if got.LastCommitSHA != "abc1234" {
		t.Errorf("LastCommitSHA: got %q want abc1234", got.LastCommitSHA)
	}
	if got.Phase != "Phase-R" {
		t.Errorf("Phase: got %q want Phase-R", got.Phase)
	}
	if got.CheckpointTS.IsZero() {
		t.Error("CheckpointTS should be refreshed by WriteCheckpoint")
	}
}

func TestWriteCheckpoint_BodyAppend(t *testing.T) {
	setSessionsDir(t)
	id := "2026-05-08-append"
	if err := WriteManifest(Manifest{ID: id, Project: "test", Body: "initial body\n"}); err != nil {
		t.Fatal(err)
	}
	if err := WriteCheckpoint(id, CheckpointFields{BodyAppend: "appended chunk"}); err != nil {
		t.Fatal(err)
	}
	got, err := ReadManifest(id)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(got.Body, "initial body") {
		t.Errorf("original body lost: %q", got.Body)
	}
	if !strings.Contains(got.Body, "appended chunk") {
		t.Errorf("appended chunk missing: %q", got.Body)
	}
	if !strings.Contains(got.Body, "## Checkpoint") {
		t.Errorf("checkpoint header missing: %q", got.Body)
	}
}

func TestWriteCheckpoint_NotExistError(t *testing.T) {
	setSessionsDir(t)
	err := WriteCheckpoint("nonexistent-session", CheckpointFields{Phase: "X"})
	if err == nil {
		t.Fatal("expected error for non-existent session")
	}
	if !os.IsNotExist(err) {
		t.Errorf("expected os.IsNotExist err, got %v", err)
	}
}
