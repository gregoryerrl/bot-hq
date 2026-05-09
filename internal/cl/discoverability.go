// Package cl — discoverability.go: T-1.10 Discoverability + indexing.
//
// In-memory keyword search across CL artifacts. Builds a token-index by
// walking artifacts + tokenizing content. Supports prefix-match queries +
// per-class filtering.
//
// MVP design: simple inverted-index (token → []artifactPaths). Adequate
// for CL scale (~10s-100s of artifacts; thousands of unique tokens).
// T-2 + Phase V can extend with LSP-grade semantic search if needed.

package cl

import (
	"sort"
	"strings"
)

// SearchHit is one match from a search query.
type SearchHit struct {
	Path    string
	Class   Class
	ID      string
	Excerpt string // first 80 chars of the matching line
}

// SearchIndex is the in-memory keyword index built from the CL.
type SearchIndex struct {
	cl        *CL
	tokens    map[string]map[string]bool // token → set of artifact paths
	artifacts map[string]*Artifact       // path → artifact (with content for excerpt)
}

// BuildSearchIndex walks the CL + builds an inverted-index over tokenized
// artifact content. Skips runtime-ephemera per Walk semantics. Tokens are
// case-folded + alphanumeric (punctuation stripped).
func (c *CL) BuildSearchIndex() (*SearchIndex, error) {
	idx := &SearchIndex{
		cl:        c,
		tokens:    make(map[string]map[string]bool),
		artifacts: make(map[string]*Artifact),
	}
	err := c.Walk(func(a *Artifact) error {
		full, err := c.Read(a.Path)
		if err != nil {
			return nil
		}
		idx.artifacts[full.Path] = full
		for _, tok := range tokenize(string(full.Content)) {
			if idx.tokens[tok] == nil {
				idx.tokens[tok] = make(map[string]bool)
			}
			idx.tokens[tok][full.Path] = true
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return idx, nil
}

// Search returns all artifacts containing the given query (case-insensitive
// token match). If classFilter is non-empty, restricts results to that class.
// Results are sorted by path for deterministic ordering.
func (idx *SearchIndex) Search(query string, classFilter Class) []SearchHit {
	q := strings.ToLower(strings.TrimSpace(query))
	if q == "" {
		return nil
	}

	// Find paths containing the query token (or any token starting with q for prefix match)
	matchedPaths := make(map[string]bool)
	for tok, paths := range idx.tokens {
		if tok == q || strings.HasPrefix(tok, q) {
			for p := range paths {
				matchedPaths[p] = true
			}
		}
	}

	var hits []SearchHit
	for path := range matchedPaths {
		a := idx.artifacts[path]
		if classFilter != "" && a.Class != classFilter {
			continue
		}
		hits = append(hits, SearchHit{
			Path:    path,
			Class:   a.Class,
			ID:      a.ID,
			Excerpt: firstMatchingLine(string(a.Content), q),
		})
	}
	sort.Slice(hits, func(i, j int) bool { return hits[i].Path < hits[j].Path })
	return hits
}

// TokenCount returns the total number of unique tokens in the index.
func (idx *SearchIndex) TokenCount() int { return len(idx.tokens) }

// ArtifactCount returns the number of artifacts in the index.
func (idx *SearchIndex) ArtifactCount() int { return len(idx.artifacts) }

// tokenize splits content into lowercase alphanumeric tokens of length >= 2.
// Strips punctuation; preserves dashes + underscores within tokens.
func tokenize(content string) []string {
	var tokens []string
	var cur strings.Builder
	for _, r := range content {
		switch {
		case r >= 'a' && r <= 'z',
			r >= '0' && r <= '9',
			r == '-' || r == '_':
			cur.WriteRune(r)
		case r >= 'A' && r <= 'Z':
			cur.WriteRune(r + 32) // case-fold to lowercase
		default:
			if cur.Len() >= 2 {
				tokens = append(tokens, cur.String())
			}
			cur.Reset()
		}
	}
	if cur.Len() >= 2 {
		tokens = append(tokens, cur.String())
	}
	return tokens
}

// firstMatchingLine returns the first line in content that contains query
// (case-insensitive substring match), truncated to 80 chars. Returns empty
// string if no match found.
func firstMatchingLine(content, query string) string {
	q := strings.ToLower(query)
	for _, line := range strings.Split(content, "\n") {
		if strings.Contains(strings.ToLower(line), q) {
			line = strings.TrimSpace(line)
			if len(line) > 80 {
				return line[:77] + "..."
			}
			return line
		}
	}
	return ""
}
