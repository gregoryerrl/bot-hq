package sessions

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestFullLifecycle exercises the end-to-end session-cluster flow per
// N-1 (a) §6 implementation sketch:
//
//  1. boundary detect → MakeSessionID
//  2. WriteManifest (minimal-create at session-open per Q-III hybrid)
//  3. WriteIndex (rolling list updated)
//  4. WriteManifest (finalize at session-close — populated EndTS + Body)
//  5. WriteIndex (rebuild after close)
//  6. ReadManifest roundtrip
//  7. LoadManifestContent (raw read for hub_session_load)
//  8. MostRecentForProject (auto-load semantics)
//
// Phase N v2 #7 N-1(c) full-flow integration test per scope-lock
// §Acceptance #7.
func TestFullLifecycle(t *testing.T) {
	dir := setSessionsDir(t)

	// Step 1: boundary detect from user message
	userMsg := "let's pivot to bot-hq Phase N v2 work"
	trigger := DetectBoundaryFromUserMsg(userMsg)
	if trigger != TriggerExplicitPhrase {
		t.Fatalf("step 1: expected TriggerExplicitPhrase; got %q", trigger)
	}
	id := MakeSessionID(time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC), "bot-hq")
	if id != "2026-05-05-bot-hq" {
		t.Fatalf("step 1: unexpected session-id: %q", id)
	}

	// Step 2: minimal-create manifest at session-open
	open := Manifest{
		ID:         id,
		Project:    "bot-hq",
		StartTS:    time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC),
		StartMsgID: 7995,
		Agents:     []string{"brian", "rain", "emma"},
	}
	if err := WriteManifest(open); err != nil {
		t.Fatalf("step 2: WriteManifest open: %v", err)
	}

	// Step 3: index updated at session-open (rebuild)
	if err := WriteIndex(); err != nil {
		t.Fatalf("step 3: WriteIndex post-open: %v", err)
	}
	indexBody, err := os.ReadFile(IndexPath())
	if err != nil {
		t.Fatalf("step 3: read index: %v", err)
	}
	if !strings.Contains(string(indexBody), "## bot-hq") || !strings.Contains(string(indexBody), id) {
		t.Errorf("step 3: index missing project section or session-id; got:\n%s", indexBody)
	}
	if !strings.Contains(string(indexBody), "active") {
		t.Errorf("step 3: open session should show 'active' marker for end_ts; got:\n%s", indexBody)
	}

	// Step 4: finalize manifest at session-close (populated EndTS + Body)
	closed := open
	closed.EndTS = time.Date(2026, 5, 5, 22, 0, 0, 0, time.UTC)
	closed.EndMsgID = 8200
	closed.Body = "## Deliverables\n- 8 commits Phase N v2\n\n## Empirical observations\n- R37 Q1-trajectory empirical-WIN\n- R36-sub HANDSHAKE-ACK-BLIND-SPOT empirical-WIN\n"
	if err := WriteManifest(closed); err != nil {
		t.Fatalf("step 4: WriteManifest close: %v", err)
	}

	// Step 5: index rebuilt after close (no longer 'active')
	if err := WriteIndex(); err != nil {
		t.Fatalf("step 5: WriteIndex post-close: %v", err)
	}
	indexBody, _ = os.ReadFile(IndexPath())
	if !strings.Contains(string(indexBody), "2026-05-05T22:00:00Z") {
		t.Errorf("step 5: index missing end_ts after close; got:\n%s", indexBody)
	}

	// Step 6: ReadManifest roundtrip — frontmatter + body parsed correctly
	got, err := ReadManifest(id)
	if err != nil {
		t.Fatalf("step 6: ReadManifest: %v", err)
	}
	if got.ID != closed.ID || got.Project != closed.Project {
		t.Errorf("step 6: id/project mismatch: %+v vs %+v", got, closed)
	}
	if !got.StartTS.Equal(closed.StartTS) || !got.EndTS.Equal(closed.EndTS) {
		t.Errorf("step 6: timestamps mismatch")
	}
	if got.StartMsgID != closed.StartMsgID || got.EndMsgID != closed.EndMsgID {
		t.Errorf("step 6: msg-id range mismatch")
	}
	if len(got.Agents) != 3 {
		t.Errorf("step 6: agents count: got %d want 3", len(got.Agents))
	}
	if !strings.Contains(got.Body, "## Deliverables") {
		t.Errorf("step 6: body missing Deliverables section; got: %q", got.Body)
	}

	// Step 7: LoadManifestContent — raw read for hub_session_load
	content, err := LoadManifestContent(id)
	if err != nil {
		t.Fatalf("step 7: LoadManifestContent: %v", err)
	}
	if !strings.Contains(content, "id: "+id) || !strings.Contains(content, "## Deliverables") {
		t.Errorf("step 7: raw content missing id or body; got:\n%s", content)
	}

	// Step 8: MostRecentForProject — auto-load semantics
	recent, err := MostRecentForProject("bot-hq")
	if err != nil {
		t.Fatalf("step 8: MostRecentForProject: %v", err)
	}
	if recent != id {
		t.Errorf("step 8: most-recent for bot-hq: got %q want %q", recent, id)
	}

	// Step 9: verify dir layout matches N-1 (a) §4 storage shape
	expectedManifestPath := filepath.Join(dir, id, "manifest.md")
	if _, err := os.Stat(expectedManifestPath); err != nil {
		t.Errorf("step 9: manifest.md not at expected path %q: %v", expectedManifestPath, err)
	}
	expectedIndexPath := filepath.Join(dir, "index.md")
	if _, err := os.Stat(expectedIndexPath); err != nil {
		t.Errorf("step 9: index.md not at expected path %q: %v", expectedIndexPath, err)
	}
}

// TestMultiProjectIndex exercises the multi-project + multi-day index
// rendering. Verifies the project sections + DESC sort within section
// per Q-V auto-load-most-recent semantics.
func TestMultiProjectIndex(t *testing.T) {
	setSessionsDir(t)
	manifests := []Manifest{
		{ID: "2026-05-05-bot-hq", Project: "bot-hq", StartTS: time.Date(2026, 5, 5, 9, 0, 0, 0, time.UTC), Agents: []string{"brian", "rain"}},
		{ID: "2026-05-04-bot-hq", Project: "bot-hq", StartTS: time.Date(2026, 5, 4, 9, 0, 0, 0, time.UTC), EndTS: time.Date(2026, 5, 4, 18, 0, 0, 0, time.UTC), Agents: []string{"brian", "rain"}},
		{ID: "2026-05-05-bcc-ad-manager", Project: "bcc-ad-manager", StartTS: time.Date(2026, 5, 5, 10, 0, 0, 0, time.UTC), EndTS: time.Date(2026, 5, 5, 17, 30, 0, 0, time.UTC), Agents: []string{"brian", "rain"}},
		{ID: "2026-05-03-988-utah-gov", Project: "988-utah-gov", StartTS: time.Date(2026, 5, 3, 14, 0, 0, 0, time.UTC), EndTS: time.Date(2026, 5, 3, 16, 0, 0, 0, time.UTC), Agents: []string{"brian"}},
	}
	for _, m := range manifests {
		if err := WriteManifest(m); err != nil {
			t.Fatal(err)
		}
	}
	if err := WriteIndex(); err != nil {
		t.Fatal(err)
	}
	body, _ := os.ReadFile(IndexPath())
	got := string(body)

	// All 3 projects present
	for _, project := range []string{"988-utah-gov", "bcc-ad-manager", "bot-hq"} {
		if !strings.Contains(got, "## "+project) {
			t.Errorf("missing project section: %s", project)
		}
	}

	// All 4 sessions present
	for _, m := range manifests {
		if !strings.Contains(got, "- "+m.ID) {
			t.Errorf("missing session entry: %s", m.ID)
		}
	}

	// Active session marker for the open bot-hq one (no EndTS)
	if !strings.Contains(got, "active") {
		t.Errorf("expected 'active' marker for open session")
	}

	// MostRecentForProject confirms cross-checks with index DESC ordering
	recent, _ := MostRecentForProject("bot-hq")
	if recent != "2026-05-05-bot-hq" {
		t.Errorf("MostRecentForProject(bot-hq): got %q want 2026-05-05-bot-hq", recent)
	}
}

// TestBoundaryAndAuthorTogether — verifies common path: detect boundary
// from a user msg, derive session-id, write minimal manifest. Mimics
// what an agent-side adapter would do at session-open turn.
func TestBoundaryAndAuthorTogether(t *testing.T) {
	setSessionsDir(t)

	// Boundary trigger from user msg
	msg := "EOD; signing off"
	trigger := DetectBoundaryFromUserMsg(msg)
	if trigger != TriggerExplicitPhrase {
		t.Fatalf("expected explicit-phrase trigger; got %q", trigger)
	}

	// Compose + write minimal manifest
	now := time.Date(2026, 5, 5, 22, 0, 0, 0, time.UTC)
	id := MakeSessionID(now, "bot-hq")
	if err := WriteManifest(Manifest{
		ID:         id,
		Project:    "bot-hq",
		StartTS:    now,
		StartMsgID: 8000,
		Agents:     []string{"brian"},
	}); err != nil {
		t.Fatalf("WriteManifest: %v", err)
	}

	// Verify index discovers it
	if err := WriteIndex(); err != nil {
		t.Fatalf("WriteIndex: %v", err)
	}
	body, _ := os.ReadFile(IndexPath())
	if !strings.Contains(string(body), id) {
		t.Errorf("index missing session-id %q", id)
	}
}
