// Package citeanchor implements the Phase T T-1.9 cite-anchor validation
// per phase-t.md v5. HIGHEST-LEVERAGE T-1 sub-task per session evidence:
// catches R31 STAT-CLAIM-CITE drift mechanically (subagent confirmed 34+
// post-graduation drift instances; Rain catch-rate 40% via peer-cross-check).
//
// Per phase-t.md v5 R49 + cite-anchor-validation: extends to scope-lock-doc
// compliance check pre-seal-fire. R49 mechanical pre-seal audit hook (T-2)
// invokes ValidateFile to catch G3-class failures (Phase S 13-miss-failure
// prevention).
//
// Validation classes:
//
//   - msg-id: "msg <N>" or "<N>" in cite-context; verify via hub.DB messages table
//   - file-path: "/abs/path/to/file" or "~/path"; verify via os.Stat
//   - line-anchor: "file.md:NNN" or "file.go:NN-MM"; verify via line-count
//
// API:
//
//   - ValidateFile(path) — read file + extract all cite-anchors + validate each
//   - ValidateString(content) — same on in-memory string
//
// Auto-fire wiring (Write/Edit hook integration) is T-2 mechanical-enforcement
// scope; T-1.9 ships the core validator + CLI invocation surface.
package citeanchor

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// MsgIDChecker is the interface citeanchor uses to verify msg-ID existence.
// Implemented by *hub.DB; mock in tests.
type MsgIDChecker interface {
	MessageExists(ctx context.Context, id int64) (bool, error)
}

// HubMsgChecker wraps *hub.DB to satisfy MsgIDChecker. Implemented separately
// from hub package to avoid import-cycle. T-2 wired to hub.DB.MessageExists.
type HubMsgChecker struct {
	DB *hub.DB
}

// MessageExists delegates to hub.DB.MessageExists (Phase T T-2 wiring per
// hub package public method addition; replaces T-1.9 nil-stub).
func (h *HubMsgChecker) MessageExists(_ context.Context, id int64) (bool, error) {
	if h.DB == nil {
		return false, errors.New("hub.DB is nil")
	}
	return h.DB.MessageExists(id)
}

// CiteAnchor is a single extracted citation reference.
type CiteAnchor struct {
	Class    string // "msg-id" | "file-path" | "line-anchor"
	Raw      string // original substring matched
	Value    string // parsed value (msg-id as string OR file path)
	LineHint int    // optional line-number anchor within file
	LineSpan int    // optional end-line for line-range anchors
	Position int    // byte offset within source content where match starts
	Source   string // source-file path or "<inline>" for ValidateString
}

// ValidationResult aggregates all extraction + validation events.
type ValidationResult struct {
	Anchors  []CiteAnchor
	Valid    int
	Invalid  int
	Skipped  int
	Findings []Finding
}

// Finding is one validation event with status + details.
type Finding struct {
	Anchor   CiteAnchor
	Status   string // "valid" | "invalid" | "skipped" | "warning"
	Message  string
}

// Various regex patterns for cite-anchor extraction. These are intentionally
// conservative — false-positives (over-extracting) are preferable to false-
// negatives (missing real cite-drift opportunities) per R49 pre-seal goal.
var (
	// msg-id: "msg 17094" / "msg-17094" / "msg17094" / "(msg 17094)" / "msg17094)"
	msgIDPattern = regexp.MustCompile(`(?i)\bmsg[\s\-]?(\d{4,6})\b`)

	// file-path: absolute paths starting with / or ~
	// matches /Users/... and ~/.bot-hq/... with optional :line or :line-line suffix
	filePathPattern = regexp.MustCompile(`(/(?:Users|home|tmp|opt|usr|var|etc|private|root)/[A-Za-z0-9_\-./~]+|~/[A-Za-z0-9_\-./~]+)(?::(\d+)(?:-(\d+))?)?`)
)

// Reject markers that look like absolute paths but are URL fragments (false-positives).
var rejectFilePathHints = []string{
	"http://",
	"https://",
}

// ExtractAnchors scans content + returns all cite-anchors found by class.
// Source path used for relative-anchor resolution + Finding context.
func ExtractAnchors(content, source string) []CiteAnchor {
	var anchors []CiteAnchor

	// msg-id extraction
	for _, m := range msgIDPattern.FindAllStringSubmatchIndex(content, -1) {
		raw := content[m[0]:m[1]]
		value := content[m[2]:m[3]]
		anchors = append(anchors, CiteAnchor{
			Class:    "msg-id",
			Raw:      raw,
			Value:    value,
			Position: m[0],
			Source:   source,
		})
	}

	// file-path + line-anchor extraction
	for _, m := range filePathPattern.FindAllStringSubmatchIndex(content, -1) {
		raw := content[m[0]:m[1]]

		// Reject URL fragments
		isReject := false
		for _, hint := range rejectFilePathHints {
			if pos := strings.LastIndex(content[:m[0]], hint); pos != -1 && m[0]-pos < 200 {
				isReject = true
				break
			}
		}
		if isReject {
			continue
		}

		path := content[m[2]:m[3]]
		// Strip trailing sentence-punctuation that the regex over-captured
		path = strings.TrimRight(path, ".,;:)")
		raw = strings.TrimRight(raw, ".,;:)")
		anchor := CiteAnchor{
			Class:    "file-path",
			Raw:      raw,
			Value:    path,
			Position: m[0],
			Source:   source,
		}

		if m[4] != -1 && m[5] != -1 {
			lineStr := content[m[4]:m[5]]
			if line, err := strconv.Atoi(lineStr); err == nil {
				anchor.LineHint = line
				anchor.Class = "line-anchor"
			}
		}
		if m[6] != -1 && m[7] != -1 {
			lineStr := content[m[6]:m[7]]
			if line, err := strconv.Atoi(lineStr); err == nil {
				anchor.LineSpan = line
			}
		}
		anchors = append(anchors, anchor)
	}

	return anchors
}

// Validate validates one anchor against the appropriate backend.
//   - msg-id: check via msgChecker (nil → skip with warning)
//   - file-path: check via os.Stat
//   - line-anchor: file exists + line-number within file bounds
func Validate(ctx context.Context, anchor CiteAnchor, msgChecker MsgIDChecker) Finding {
	switch anchor.Class {
	case "msg-id":
		if msgChecker == nil {
			return Finding{Anchor: anchor, Status: "skipped", Message: "msg-id check skipped: no msgChecker provided (pass HubMsgChecker for live validation)"}
		}
		id, err := strconv.ParseInt(anchor.Value, 10, 64)
		if err != nil {
			return Finding{Anchor: anchor, Status: "invalid", Message: fmt.Sprintf("malformed msg-id: %v", err)}
		}
		exists, err := msgChecker.MessageExists(ctx, id)
		if err != nil {
			return Finding{Anchor: anchor, Status: "warning", Message: fmt.Sprintf("msg-id lookup error: %v", err)}
		}
		if !exists {
			return Finding{Anchor: anchor, Status: "invalid", Message: fmt.Sprintf("msg-id %d does not exist in hub.db", id)}
		}
		return Finding{Anchor: anchor, Status: "valid", Message: fmt.Sprintf("msg-id %d exists", id)}

	case "file-path":
		path := expandPath(anchor.Value)
		if _, err := os.Stat(path); err != nil {
			if os.IsNotExist(err) {
				return Finding{Anchor: anchor, Status: "invalid", Message: fmt.Sprintf("file does not exist: %s", path)}
			}
			return Finding{Anchor: anchor, Status: "warning", Message: fmt.Sprintf("stat error: %v", err)}
		}
		return Finding{Anchor: anchor, Status: "valid", Message: fmt.Sprintf("file exists: %s", path)}

	case "line-anchor":
		path := expandPath(anchor.Value)
		info, err := os.Stat(path)
		if err != nil {
			if os.IsNotExist(err) {
				return Finding{Anchor: anchor, Status: "invalid", Message: fmt.Sprintf("file does not exist: %s", path)}
			}
			return Finding{Anchor: anchor, Status: "warning", Message: fmt.Sprintf("stat: %v", err)}
		}
		if info.IsDir() {
			return Finding{Anchor: anchor, Status: "invalid", Message: fmt.Sprintf("expected file, got directory: %s", path)}
		}
		// Verify line-number within file bounds (reads file; cap at 10MB)
		const maxFileSize = 10 * 1024 * 1024
		if info.Size() > maxFileSize {
			return Finding{Anchor: anchor, Status: "warning", Message: fmt.Sprintf("file too large for line-anchor check (%d bytes; cap %d)", info.Size(), maxFileSize)}
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return Finding{Anchor: anchor, Status: "warning", Message: fmt.Sprintf("read: %v", err)}
		}
		lineCount := strings.Count(string(data), "\n") + 1
		if anchor.LineHint > lineCount {
			return Finding{Anchor: anchor, Status: "invalid", Message: fmt.Sprintf("line-anchor %d exceeds file length (%d lines)", anchor.LineHint, lineCount)}
		}
		if anchor.LineSpan > 0 && anchor.LineSpan > lineCount {
			return Finding{Anchor: anchor, Status: "invalid", Message: fmt.Sprintf("line-span end %d exceeds file length (%d lines)", anchor.LineSpan, lineCount)}
		}
		return Finding{Anchor: anchor, Status: "valid", Message: fmt.Sprintf("line-anchor %s:%d valid (file has %d lines)", path, anchor.LineHint, lineCount)}

	default:
		return Finding{Anchor: anchor, Status: "skipped", Message: fmt.Sprintf("unknown anchor class: %s", anchor.Class)}
	}
}

// ValidateString runs ExtractAnchors + Validate on each anchor.
func ValidateString(ctx context.Context, content, source string, msgChecker MsgIDChecker) ValidationResult {
	anchors := ExtractAnchors(content, source)
	res := ValidationResult{Anchors: anchors}
	for _, a := range anchors {
		f := Validate(ctx, a, msgChecker)
		res.Findings = append(res.Findings, f)
		switch f.Status {
		case "valid":
			res.Valid++
		case "invalid":
			res.Invalid++
		case "skipped":
			res.Skipped++
		}
	}
	return res
}

// ValidateFile reads + validates a file. Convenience wrapper around ValidateString.
func ValidateFile(ctx context.Context, path string, msgChecker MsgIDChecker) (ValidationResult, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return ValidationResult{}, fmt.Errorf("read %s: %w", path, err)
	}
	return ValidateString(ctx, string(data), path, msgChecker), nil
}

// expandPath resolves ~ to $HOME for path-class anchors.
func expandPath(path string) string {
	if strings.HasPrefix(path, "~/") {
		home, err := os.UserHomeDir()
		if err == nil {
			return filepath.Join(home, path[2:])
		}
	}
	return path
}
