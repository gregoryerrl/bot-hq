package protocol

import (
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
		LastSelfMsgID:   12345,
		LastCommitSHA:   "abcdef0",
		LastPhaseDoc:    "phase-j.md",
		LastRatchetPull: time.Date(2026, 4, 29, 1, 30, 0, 0, time.UTC),
		LastStateWrite:  time.Date(2026, 4, 29, 2, 15, 42, 0, time.UTC),
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
		LastSelfMsgID:   42,
		LastCommitSHA:   "deadbeef",
		LastPhaseDoc:    "phase-j.md",
		LastRatchetPull: time.Date(2026, 4, 29, 1, 30, 0, 0, time.UTC),
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
