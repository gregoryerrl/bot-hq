// Package clschema — archive.go: T-8.9c CL Auto-archive+TTL per
// phase-t.md v5 + user msg 17317 "no defer".
//
// Auto-archive scans CL artifact directories for entries older than the
// per-class TTL and moves them to <root>/archive/<basename>. Default
// policy: sessions auto-prune-after-30d / closed-snapshots retain-forever.
// Caller-side: bot-hq daemon cron OR manual `bot-hq cl archive` invocation.
//
// Layering: stdlib only (errors + fmt + os + path/filepath + time).
// Same-package extension to internal/clschema/.

package clschema

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

// ArchivePolicy describes per-class TTL config.
type ArchivePolicy struct {
	// SubdirTTL maps subdir-name (e.g. "sessions") → max-age duration.
	// Entries older than TTL are archived. duration=0 means never-prune.
	SubdirTTL map[string]time.Duration
	// ArchiveDir is the destination subdir-name under root (default
	// "archive" if empty).
	ArchiveDir string
}

// DefaultPolicy returns the canonical TTL policy:
//   - sessions: 30 days
//   - closed-snapshots: retain forever (duration=0)
func DefaultPolicy() ArchivePolicy {
	return ArchivePolicy{
		SubdirTTL: map[string]time.Duration{
			"sessions": 30 * 24 * time.Hour,
		},
		ArchiveDir: "archive",
	}
}

// ErrPolicyMissing indicates the requested subdir is not in the policy
// (caller treats as no-op or error per use-case).
var ErrPolicyMissing = errors.New("subdir not in archive policy")

// ListArchivable returns paths of entries that would be archived under
// the policy (dry-run; no filesystem mutation). Useful for safe-preview
// before invoking ArchiveOldSessions.
func ListArchivable(root string, policy ArchivePolicy) ([]string, error) {
	if root == "" {
		return nil, errors.New("root is required")
	}
	var out []string
	now := time.Now()
	for subdir, ttl := range policy.SubdirTTL {
		if ttl == 0 {
			continue // retain-forever class
		}
		subPath := filepath.Join(root, subdir)
		entries, err := os.ReadDir(subPath)
		if err != nil {
			if os.IsNotExist(err) {
				continue
			}
			return out, fmt.Errorf("readdir %s: %w", subPath, err)
		}
		for _, e := range entries {
			if e.Name() == policy.ArchiveDir {
				continue // skip archive dir itself
			}
			fullPath := filepath.Join(subPath, e.Name())
			info, err := e.Info()
			if err != nil {
				continue
			}
			if now.Sub(info.ModTime()) > ttl {
				out = append(out, fullPath)
			}
		}
	}
	return out, nil
}

// ArchiveOldSessions moves entries older than TTL from <root>/<subdir>/
// to <root>/<subdir>/<archiveDir>/<basename>. Returns the count of
// archived entries.
func ArchiveOldSessions(root string, policy ArchivePolicy) (int, error) {
	if root == "" {
		return 0, errors.New("root is required")
	}
	archiveDir := policy.ArchiveDir
	if archiveDir == "" {
		archiveDir = "archive"
	}
	count := 0
	now := time.Now()
	for subdir, ttl := range policy.SubdirTTL {
		if ttl == 0 {
			continue
		}
		subPath := filepath.Join(root, subdir)
		entries, err := os.ReadDir(subPath)
		if err != nil {
			if os.IsNotExist(err) {
				continue
			}
			return count, fmt.Errorf("readdir %s: %w", subPath, err)
		}
		archivePath := filepath.Join(subPath, archiveDir)
		if err := os.MkdirAll(archivePath, 0o700); err != nil {
			return count, fmt.Errorf("mkdir archive: %w", err)
		}
		for _, e := range entries {
			if e.Name() == archiveDir {
				continue
			}
			fullPath := filepath.Join(subPath, e.Name())
			info, err := e.Info()
			if err != nil {
				continue
			}
			if now.Sub(info.ModTime()) <= ttl {
				continue
			}
			target := filepath.Join(archivePath, e.Name())
			if err := os.Rename(fullPath, target); err != nil {
				return count, fmt.Errorf("rename %s → %s: %w", fullPath, target, err)
			}
			count++
		}
	}
	return count, nil
}

// ageOf returns the time elapsed since the file's mtime. Helper for
// testing + diagnostics.
func ageOf(path string) (time.Duration, error) {
	info, err := os.Stat(path)
	if err != nil {
		return 0, fmt.Errorf("stat: %w", err)
	}
	return time.Since(info.ModTime()), nil
}
