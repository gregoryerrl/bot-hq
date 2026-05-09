// Package clschema — versioning.go: T-8.9b CL versioning + history per
// phase-t.md v5 + user msg 17317 "no defer".
//
// Extends T-8.9a last_state.json validator with schema_version field +
// per-agent append-only versions.log.jsonl history. Markdown-class CL
// artifacts (phase-doc / ratchet-ledger) are different schema-class
// (frontmatter not JSON) — deferred to T-8.9b-followup OR Phase V.
//
// Layering: stdlib only (no internal deps; verified via go list -deps).

package clschema

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

// Version is one entry in versions.log.jsonl. Append-only history.
type Version struct {
	SchemaVersion string    `json:"schema_version"`
	BumpedAt      time.Time `json:"bumped_at"`
	DeltaSummary  string    `json:"delta_summary,omitempty"`
}

// ErrNoVersionHistory indicates the agent's versions.log.jsonl does
// not exist OR is empty.
var ErrNoVersionHistory = errors.New("no version history")

// ParseVersionedLastState reads + validates last_state.json AND extracts
// the schema_version. Defaults to "v0" if the field is absent (back-
// compat for pre-T-8.9b last_state.json files).
func ParseVersionedLastState(raw []byte) (*LastState, string, error) {
	ls, err := ParseLastState(raw)
	if err != nil {
		return nil, "", err
	}
	var doc map[string]interface{}
	_ = json.Unmarshal(raw, &doc)
	version := "v0"
	if v, ok := doc["schema_version"].(string); ok && v != "" {
		version = v
	}
	return ls, version, nil
}

// BumpVersion appends a Version entry to the agent's versions.log.jsonl.
// Creates the file + parent dir if missing (0700/0600 perms).
func BumpVersion(agentDir, schemaVersion, deltaSummary string) error {
	if agentDir == "" {
		return errors.New("agentDir is required")
	}
	if schemaVersion == "" {
		return errors.New("schemaVersion is required")
	}
	if err := os.MkdirAll(agentDir, 0o700); err != nil {
		return fmt.Errorf("mkdir agent dir: %w", err)
	}
	v := Version{
		SchemaVersion: schemaVersion,
		BumpedAt:      time.Now().UTC(),
		DeltaSummary:  deltaSummary,
	}
	line, err := json.Marshal(v)
	if err != nil {
		return fmt.Errorf("marshal version: %w", err)
	}
	path := filepath.Join(agentDir, "versions.log.jsonl")
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return fmt.Errorf("open versions.log: %w", err)
	}
	defer f.Close()
	if _, err := f.Write(append(line, '\n')); err != nil {
		return fmt.Errorf("append version: %w", err)
	}
	return nil
}

// CurrentVersion returns the most-recent Version entry. Returns
// ErrNoVersionHistory when the file is missing or empty.
func CurrentVersion(agentDir string) (*Version, error) {
	versions, err := ListVersions(agentDir)
	if err != nil {
		return nil, err
	}
	if len(versions) == 0 {
		return nil, fmt.Errorf("%w: %s", ErrNoVersionHistory, agentDir)
	}
	return &versions[len(versions)-1], nil
}

// ListVersions returns the full append-history. Empty slice + nil err
// when file is missing (treated as zero-history; differs from
// CurrentVersion which returns ErrNoVersionHistory).
func ListVersions(agentDir string) ([]Version, error) {
	if agentDir == "" {
		return nil, errors.New("agentDir is required")
	}
	path := filepath.Join(agentDir, "versions.log.jsonl")
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil // no history yet — caller treats as zero
		}
		return nil, fmt.Errorf("open versions.log: %w", err)
	}
	defer f.Close()
	var out []Version
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 4096), 1024*1024)
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(line) == 0 {
			continue
		}
		var v Version
		if err := json.Unmarshal(line, &v); err != nil {
			return out, fmt.Errorf("malformed history entry: %w", err)
		}
		out = append(out, v)
	}
	if err := scanner.Err(); err != nil {
		return out, fmt.Errorf("scan: %w", err)
	}
	return out, nil
}
