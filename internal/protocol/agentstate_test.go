package protocol

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestAgentStateSchema locks the JSON output keys. Accidental field
// removal or rename fails this test; intentional schema changes must
// update both AgentState struct + this test together.
func TestAgentStateSchema(t *testing.T) {
	state := AgentState{
		LastSelfMsgID:          12345,
		LastCommitSHA:          "abcdef0",
		LastPhaseDoc:           "phase-j.md",
		LastRatchetPull:        time.Date(2026, 4, 29, 1, 30, 0, 0, time.UTC),
		LastStateWrite:         time.Date(2026, 4, 29, 2, 15, 42, 0, time.UTC),
		DisciplineAnchorSHA256: "0123456789abcdef",
	}
	data, err := json.Marshal(state)
	if err != nil {
		t.Fatalf("json.Marshal: %v", err)
	}
	wantKeys := []string{
		`"last_self_msg_id"`,
		`"last_commit_sha"`,
		`"last_phase_doc"`,
		`"last_ratchet_pull_at"`,
		`"last_state_write_at"`,
		`"discipline_anchor_sha256"`,
	}
	for _, k := range wantKeys {
		if !strings.Contains(string(data), k) {
			t.Errorf("AgentState JSON missing required key %s\n  output: %s", k, data)
		}
	}
}

// TestAgentStateRoundTrip locks Write+Read symmetry. State written via
// WriteAgentState must Read back equal (modulo LastStateWrite which the
// helper updates to now()).
func TestAgentStateRoundTrip(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)

	want := AgentState{
		LastSelfMsgID:          42,
		LastCommitSHA:          "deadbeef",
		LastPhaseDoc:           "phase-j.md",
		LastRatchetPull:        time.Date(2026, 4, 29, 1, 30, 0, 0, time.UTC),
		DisciplineAnchorSHA256: "abc123def456",
		// LastStateWrite intentionally zero — WriteAgentState sets it to now()
	}

	if err := WriteAgentState("brian", want); err != nil {
		t.Fatalf("WriteAgentState: %v", err)
	}

	got, err := ReadAgentState("brian")
	if err != nil {
		t.Fatalf("ReadAgentState: %v", err)
	}

	if got.LastSelfMsgID != want.LastSelfMsgID {
		t.Errorf("LastSelfMsgID: got %d, want %d", got.LastSelfMsgID, want.LastSelfMsgID)
	}
	if got.LastCommitSHA != want.LastCommitSHA {
		t.Errorf("LastCommitSHA: got %q, want %q", got.LastCommitSHA, want.LastCommitSHA)
	}
	if got.LastPhaseDoc != want.LastPhaseDoc {
		t.Errorf("LastPhaseDoc: got %q, want %q", got.LastPhaseDoc, want.LastPhaseDoc)
	}
	if !got.LastRatchetPull.Equal(want.LastRatchetPull) {
		t.Errorf("LastRatchetPull: got %v, want %v", got.LastRatchetPull, want.LastRatchetPull)
	}
	if got.DisciplineAnchorSHA256 != want.DisciplineAnchorSHA256 {
		t.Errorf("DisciplineAnchorSHA256: got %q, want %q", got.DisciplineAnchorSHA256, want.DisciplineAnchorSHA256)
	}
	if got.LastStateWrite.IsZero() {
		t.Errorf("LastStateWrite must be set by WriteAgentState; got zero")
	}
}

// TestAgentStateMissingFile locks first-ever-call path: missing file
// returns zero-value state without error (valid first-bootstrap signal).
func TestAgentStateMissingFile(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)

	got, err := ReadAgentState("never-written")
	if err != nil {
		t.Fatalf("ReadAgentState should not error on missing file; got %v", err)
	}
	if got.LastSelfMsgID != 0 || got.LastCommitSHA != "" {
		t.Errorf("missing-file should return zero AgentState; got %+v", got)
	}
}

// TestAgentStateAtomicWrite locks the temp-file + rename atomic-write
// behavior — file at the final path is always either absent or
// fully-formed JSON (no half-written content visible to concurrent reader).
func TestAgentStateAtomicWrite(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)

	if err := WriteAgentState("rain", AgentState{LastSelfMsgID: 1}); err != nil {
		t.Fatalf("WriteAgentState: %v", err)
	}
	// Confirm only the final file exists, not the .tmp.
	dir := filepath.Join(tmp, "rain")
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatalf("ReadDir: %v", err)
	}
	for _, e := range entries {
		if strings.HasSuffix(e.Name(), ".tmp") {
			t.Errorf("residual temp file present: %s — atomic-rename failed", e.Name())
		}
	}
}

// TestComputeDisciplineAnchorSHA256_Present locks the SHA-256 hex digest
// computation for a present discipline-anchors.md file. Phase K K-12.
func TestComputeDisciplineAnchorSHA256_Present(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)

	dir := filepath.Join(tmp, "brian")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	content := "# discipline anchors\n\n## class-split\nHANDS=brian / EYES=rain\n"
	if err := os.WriteFile(filepath.Join(dir, "discipline-anchors.md"), []byte(content), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	got, err := ComputeDisciplineAnchorSHA256("brian")
	if err != nil {
		t.Fatalf("ComputeDisciplineAnchorSHA256: %v", err)
	}
	expectSum := sha256.Sum256([]byte(content))
	want := hex.EncodeToString(expectSum[:])
	if got != want {
		t.Errorf("SHA mismatch: got %q, want %q", got, want)
	}
}

// TestComputeDisciplineAnchorSHA256_Absent locks the absent-file path:
// returns ("", nil) without error so VerifyDisciplineAnchor can decide
// drift semantics. Phase K K-12.
func TestComputeDisciplineAnchorSHA256_Absent(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)

	got, err := ComputeDisciplineAnchorSHA256("never-written")
	if err != nil {
		t.Fatalf("absent file should not error; got %v", err)
	}
	if got != "" {
		t.Errorf("absent file should return empty SHA; got %q", got)
	}
}

// TestVerifyDisciplineAnchor_Match locks the normal bootstrap path:
// stored SHA == current SHA → matches=true. Phase K K-12.
func TestVerifyDisciplineAnchor_Match(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)

	dir := filepath.Join(tmp, "brian")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	content := "# anchors v1\n"
	if err := os.WriteFile(filepath.Join(dir, "discipline-anchors.md"), []byte(content), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	expectSum := sha256.Sum256([]byte(content))
	storedSHA := hex.EncodeToString(expectSum[:])

	matches, current, err := VerifyDisciplineAnchor("brian", AgentState{DisciplineAnchorSHA256: storedSHA})
	if err != nil {
		t.Fatalf("Verify: %v", err)
	}
	if !matches {
		t.Errorf("expected match; got mismatch (current=%q stored=%q)", current, storedSHA)
	}
	if current != storedSHA {
		t.Errorf("expected currentSHA=%q, got %q", storedSHA, current)
	}
}

// TestVerifyDisciplineAnchor_Mismatch locks the drift-detection path:
// stored SHA != current SHA → matches=false (anchor file edited between
// last checkpoint and this bootstrap; signals possible autocompact-drift
// or post-edit re-read needed). Phase K K-12.
func TestVerifyDisciplineAnchor_Mismatch(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)

	dir := filepath.Join(tmp, "brian")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "discipline-anchors.md"), []byte("new content v2\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	staleSHA := "0000000000000000000000000000000000000000000000000000000000000000"
	matches, current, err := VerifyDisciplineAnchor("brian", AgentState{DisciplineAnchorSHA256: staleSHA})
	if err != nil {
		t.Fatalf("Verify: %v", err)
	}
	if matches {
		t.Errorf("expected mismatch (stored=stale, current=fresh); got match")
	}
	if current == "" {
		t.Errorf("expected non-empty currentSHA on mismatch (file present); got empty")
	}
	if current == staleSHA {
		t.Errorf("currentSHA should differ from staleSHA; got equal %q", current)
	}
}

// TestVerifyDisciplineAnchor_FirstWrite locks the first-bootstrap
// semantics: stored SHA empty → matches=true (no drift detectable yet;
// caller persists currentSHA so subsequent boots can detect). Phase K K-12.
func TestVerifyDisciplineAnchor_FirstWrite(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)

	dir := filepath.Join(tmp, "brian")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "discipline-anchors.md"), []byte("anchor content\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	// Stored is zero-value AgentState (DisciplineAnchorSHA256 == "").
	matches, current, err := VerifyDisciplineAnchor("brian", AgentState{})
	if err != nil {
		t.Fatalf("Verify: %v", err)
	}
	if !matches {
		t.Errorf("expected match on first-write (stored empty); got mismatch")
	}
	if current == "" {
		t.Errorf("expected currentSHA returned on first-write so caller can persist it; got empty")
	}
}

// TestVerifyDisciplineAnchor_StoredNonEmpty_FileAbsent locks the
// drift-on-deletion path: stored SHA non-empty + file absent →
// matches=false (anchor file was present at last checkpoint but is
// now gone; possibly accidental delete OR home-dir misconfigured).
// Rain refinement-1 (msg 6411). Phase K K-12.
func TestVerifyDisciplineAnchor_StoredNonEmpty_FileAbsent(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)
	// Note: create the per-agent dir but NOT the discipline-anchors.md file
	dir := filepath.Join(tmp, "brian")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	matches, current, err := VerifyDisciplineAnchor("brian",
		AgentState{DisciplineAnchorSHA256: "deadbeef"},
	)
	if err != nil {
		t.Fatalf("Verify: %v", err)
	}
	if matches {
		t.Errorf("expected mismatch (file absent + stored non-empty = drift signal); got match")
	}
	if current != "" {
		t.Errorf("expected empty currentSHA on file-absent; got %q", current)
	}
}
