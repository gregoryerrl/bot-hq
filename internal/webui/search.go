package webui

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// SearchResult is a single hit from the cross-search index. Path is
// canonical-store-relative (matches the format used by /api/files +
// recent-edits). Line is 1-indexed.
type SearchResult struct {
	Path    string `json:"path"`
	Line    int    `json:"line"`
	Snippet string `json:"snippet"`
}

// searchMaxFileSize caps per-file scan to keep walks bounded and avoid
// DoS via very-large files (binary blobs, dumped logs, etc.). Files
// larger than this are skipped silently.
const searchMaxFileSize = 1 << 20 // 1 MiB

// searchSnippetMax bounds emitted snippet length per hit. Long lines
// are truncated with an ellipsis to keep the JSON payload compact.
const searchSnippetMax = 200

// SearchCanonicalStore walks the canonical-store under root and returns
// up to limit matches of query (case-insensitive substring) across all
// canonical-store files (skip-list applied — runtime state never
// surfaces). Per file: read line-by-line, emit one result per matching
// line. Phase O drain per phase-n.md:819 cross-search dashboard.
//
// Empty query → empty result slice (caller's responsibility to enforce
// minimum-length input). Limit clamped by caller to [1, max].
func SearchCanonicalStore(root, query string, limit int) ([]SearchResult, error) {
	if query == "" || limit <= 0 {
		return nil, nil
	}
	needle := strings.ToLower(query)
	var results []SearchResult
	tree, err := walkCanonicalTree(root)
	if err != nil {
		return nil, err
	}
	var paths []string
	collectFilePaths(tree, &paths)
	for _, rel := range paths {
		if len(results) >= limit {
			break
		}
		hits, err := scanFileForQuery(filepath.Join(root, rel), rel, needle, limit-len(results))
		// Append partial hits BEFORE error check — preserves matches
		// collected pre-error (e.g., 50 hits then bufio.ErrTooLong on
		// line 51 wouldn't drop the 50 per Rain msg 14816 catch).
		results = append(results, hits...)
		if err != nil {
			continue // skip remaining lines in this file; keep walking
		}
	}
	return results, nil
}

// collectFilePaths flattens tree nodes to file-only relative paths.
func collectFilePaths(nodes []TreeNode, out *[]string) {
	for _, n := range nodes {
		switch n.Type {
		case "file":
			*out = append(*out, n.Path)
		case "dir":
			collectFilePaths(n.Children, out)
		}
	}
}

// scanFileForQuery reads a file line-by-line and returns up to maxHits
// matches of needle (already lowercased). Files larger than search-
// MaxFileSize are skipped. Each hit emits a SearchResult with the
// matched line as snippet (truncated if longer than searchSnippetMax).
func scanFileForQuery(absPath, relPath, needle string, maxHits int) ([]SearchResult, error) {
	info, err := os.Stat(absPath)
	if err != nil {
		return nil, err
	}
	if info.Size() > searchMaxFileSize {
		return nil, nil
	}
	f, err := os.Open(absPath)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	var hits []SearchResult
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 64*1024), 1024*1024)
	lineNum := 0
	for sc.Scan() {
		lineNum++
		line := sc.Text()
		if strings.Contains(strings.ToLower(line), needle) {
			snippet := line
			if len(snippet) > searchSnippetMax {
				snippet = snippet[:searchSnippetMax] + "…"
			}
			hits = append(hits, SearchResult{
				Path:    relPath,
				Line:    lineNum,
				Snippet: snippet,
			})
			if len(hits) >= maxHits {
				break
			}
		}
	}
	if err := sc.Err(); err != nil {
		return hits, fmt.Errorf("scan %s: %w", relPath, err)
	}
	return hits, nil
}
