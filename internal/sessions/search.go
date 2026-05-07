package sessions

// Cross-session-search indexed lookup per phase-p.md §P-7 + phase-n.md:295
// "OQ-7 (cross-session-search indexed lookup) — productionize-class".
//
// MVP design (per OQ-P-3 lean: manifests-only v1; full-transcript v2 is
// Phase Q candidate): grep-style substring scan over manifest.md files
// in SessionsDir. Sufficient for typical session-counts (~30 manifests
// at default 30d retention; sub-100ms scan even with multi-KB bodies).
//
// SQLite FTS5 alternative considered + deferred: adds CGO dependency +
// schema/migration complexity for a feature whose data volume stays
// well under filesystem-grep's capabilities. Phase Q can promote to
// FTS5 if profile shows scan-time becomes a bottleneck.

import (
	"bufio"
	"fmt"
	"os"
	"sort"
	"strings"
)

// SearchHit is one match from a cross-session search. SessionID +
// Line + Snippet identify the match site; the operator can then
// `bot-hq session-load <SessionID>` to read the full manifest.
type SearchHit struct {
	SessionID string `json:"session_id"`
	Line      int    `json:"line"`    // 1-indexed line number in manifest.md
	Snippet   string `json:"snippet"` // matched line, truncated to snippetSearchMax chars
}

// snippetSearchMax caps SearchHit.Snippet length so result lists stay
// readable. Operator can fetch full context via session-load.
const snippetSearchMax = 200

// SearchSessions returns hits whose manifest.md contains a case-
// insensitive substring match of `query`. Limit caps the total
// returned hits across all sessions (default 50; hard max 500 to
// bound memory + scan-time on degenerate queries like "the").
//
// Empty query returns empty result + nil error (operator-friendly
// no-op rather than scan-everything class). Search is in-process
// + single-pass; no persistent index. Scales to ~hundreds of
// manifests; if profiling shows scan-time bottleneck at 10K+
// session counts, promote to FTS5 (Phase Q candidate per OQ-P-3).
func SearchSessions(query string, limit int) ([]SearchHit, error) {
	if strings.TrimSpace(query) == "" {
		return nil, nil
	}
	if limit <= 0 {
		limit = 50
	}
	if limit > 500 {
		limit = 500
	}
	ids, err := ListSessionIDs()
	if err != nil {
		return nil, err
	}
	// Reverse-chrono: search most-recent first so early-cap'd results
	// surface fresh sessions over old ones.
	sort.Sort(sort.Reverse(sort.StringSlice(ids)))

	needle := strings.ToLower(query)
	var hits []SearchHit
	for _, id := range ids {
		if len(hits) >= limit {
			break
		}
		path := ManifestPath(id)
		f, err := os.Open(path)
		if err != nil {
			// Missing/unreadable manifest: skip rather than error
			// the whole search (one corrupt session shouldn't block
			// queries against the rest).
			continue
		}
		hits = appendHitsFromFile(hits, f, id, needle, limit)
		_ = f.Close()
	}
	return hits, nil
}

// appendHitsFromFile streams a manifest reading lines + appending
// matches. Stops at limit. Extracted for testability + to keep
// SearchSessions readable.
func appendHitsFromFile(hits []SearchHit, f *os.File, id, needle string, limit int) []SearchHit {
	scanner := bufio.NewScanner(f)
	// Allow up to 1 MiB per line to handle wide manifest tables.
	buf := make([]byte, 0, 64*1024)
	scanner.Buffer(buf, 1024*1024)
	lineNum := 0
	for scanner.Scan() {
		lineNum++
		line := scanner.Text()
		if !strings.Contains(strings.ToLower(line), needle) {
			continue
		}
		hits = append(hits, SearchHit{
			SessionID: id,
			Line:      lineNum,
			Snippet:   truncateSearchSnippet(line),
		})
		if len(hits) >= limit {
			return hits
		}
	}
	return hits
}

// truncateSearchSnippet trims a line to snippetSearchMax chars with
// an ellipsis marker when truncated.
func truncateSearchSnippet(line string) string {
	line = strings.TrimSpace(line)
	if len(line) <= snippetSearchMax {
		return line
	}
	return line[:snippetSearchMax] + "…"
}

// formatSearchHit renders a single hit for stdout consumption by the
// CLI. Format: `<session-id>:<line>: <snippet>` (grep-compatible).
func formatSearchHit(h SearchHit) string {
	return fmt.Sprintf("%s:%d: %s", h.SessionID, h.Line, h.Snippet)
}

// FormatSearchResults renders a result list as plain-text lines
// (one per hit). Caller writes to stdout. Returns "" for empty
// results so caller can detect + emit "no matches" message.
func FormatSearchResults(hits []SearchHit) string {
	if len(hits) == 0 {
		return ""
	}
	var b strings.Builder
	for _, h := range hits {
		b.WriteString(formatSearchHit(h))
		b.WriteByte('\n')
	}
	return b.String()
}

