package clschema

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

// ====== ParseVersionedLastState ======

func TestParseVersionedLastState_extractsVersion(t *testing.T) {
	raw := []byte(`{"agent_id":"brian","last_self_msg_id":1,"saved_at_utc":"x","phase":"x","schema_version":"v2"}`)
	_, version, err := ParseVersionedLastState(raw)
	if err != nil {
		t.Fatalf("ParseVersionedLastState: %v", err)
	}
	if version != "v2" {
		t.Errorf("version = %q, want v2", version)
	}
}

func TestParseVersionedLastState_defaultV0WhenAbsent(t *testing.T) {
	raw := []byte(`{"agent_id":"brian","last_self_msg_id":1,"saved_at_utc":"x","phase":"x"}`)
	_, version, err := ParseVersionedLastState(raw)
	if err != nil {
		t.Fatalf("ParseVersionedLastState: %v", err)
	}
	if version != "v0" {
		t.Errorf("version = %q, want v0 (back-compat default)", version)
	}
}

func TestParseVersionedLastState_invalidPropagatesError(t *testing.T) {
	if _, _, err := ParseVersionedLastState([]byte(`{"agent_id":"x"}`)); err == nil {
		t.Error("expected validation error to propagate")
	}
}

// ====== BumpVersion ======

func TestBumpVersion_createsHistoryFile(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "agent")
	if err := BumpVersion(dir, "v1", "initial"); err != nil {
		t.Fatalf("BumpVersion: %v", err)
	}
	path := filepath.Join(dir, "versions.log.jsonl")
	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("history file not created: %v", err)
	}
	if info.Size() == 0 {
		t.Error("history file empty post-bump")
	}
}

func TestBumpVersion_emptyArgsRejected(t *testing.T) {
	if err := BumpVersion("", "v1", ""); err == nil {
		t.Error("empty agentDir should error")
	}
	if err := BumpVersion(t.TempDir(), "", ""); err == nil {
		t.Error("empty schemaVersion should error")
	}
}

func TestBumpVersion_appendsMultipleEntries(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "agent")
	_ = BumpVersion(dir, "v1", "initial")
	_ = BumpVersion(dir, "v2", "added field X")
	_ = BumpVersion(dir, "v3", "renamed field Y")

	versions, err := ListVersions(dir)
	if err != nil {
		t.Fatalf("ListVersions: %v", err)
	}
	if len(versions) != 3 {
		t.Errorf("len = %d, want 3", len(versions))
	}
	if versions[0].SchemaVersion != "v1" || versions[2].SchemaVersion != "v3" {
		t.Errorf("ordering broke: %+v", versions)
	}
}

// ====== CurrentVersion ======

func TestCurrentVersion_returnsLatest(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "agent")
	_ = BumpVersion(dir, "v1", "")
	_ = BumpVersion(dir, "v5", "newest")
	cur, err := CurrentVersion(dir)
	if err != nil {
		t.Fatalf("CurrentVersion: %v", err)
	}
	if cur.SchemaVersion != "v5" {
		t.Errorf("current = %q, want v5", cur.SchemaVersion)
	}
}

func TestCurrentVersion_noHistoryReturnsErr(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "fresh-agent")
	_, err := CurrentVersion(dir)
	if !errors.Is(err, ErrNoVersionHistory) {
		t.Errorf("err = %v, want errors.Is ErrNoVersionHistory", err)
	}
}

// ====== ListVersions ======

func TestListVersions_emptyDirReturnsNilNoError(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "fresh")
	versions, err := ListVersions(dir)
	if err != nil {
		t.Errorf("ListVersions on missing should not error, got: %v", err)
	}
	if versions != nil {
		t.Errorf("versions = %+v, want nil for missing history", versions)
	}
}

func TestListVersions_skipsBlankLines(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "agent")
	_ = os.MkdirAll(dir, 0o700)
	_ = os.WriteFile(filepath.Join(dir, "versions.log.jsonl"), []byte(`{"schema_version":"v1","bumped_at":"2026-05-10T00:00:00Z"}

{"schema_version":"v2","bumped_at":"2026-05-10T00:01:00Z"}
`), 0o600)
	versions, err := ListVersions(dir)
	if err != nil {
		t.Fatalf("ListVersions: %v", err)
	}
	if len(versions) != 2 {
		t.Errorf("len = %d, want 2 (blank line skipped)", len(versions))
	}
}

func TestListVersions_malformedReturnsErrPlusPartial(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "agent")
	_ = os.MkdirAll(dir, 0o700)
	_ = os.WriteFile(filepath.Join(dir, "versions.log.jsonl"), []byte(`{"schema_version":"v1","bumped_at":"2026-05-10T00:00:00Z"}
{not json
`), 0o600)
	versions, err := ListVersions(dir)
	if err == nil {
		t.Error("expected error for malformed entry")
	}
	if len(versions) != 1 {
		t.Errorf("partial result should have 1 valid entry; got %d", len(versions))
	}
}
