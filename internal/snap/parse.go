package snap

import (
	"errors"
	"strings"
)

var (
	ErrNoSNAPBlock     = errors.New("snap: no SNAP: marker found")
	ErrMalformedFields = errors.New("snap: block missing required fields or out of order")
)

// Parse extracts a SNAP block from a message body. The block must begin with
// a line whose trimmed contents are exactly "SNAP:", followed by four labeled
// lines in fixed order: Branches, Agents, Pending, Next. Anything before the
// SNAP: marker is treated as preamble and ignored.
//
// Parsing is whitespace-tolerant on the inter-label spacing — Format emits
// canonical 10-char alignment, but Parse accepts any spacing after the colon.
// List fields are split paren-aware via splitDepth0; see package doc.
func Parse(body string) (SNAP, error) {
	var s SNAP
	lines := strings.Split(body, "\n")

	start := -1
	for i, ln := range lines {
		if strings.TrimSpace(ln) == "SNAP:" {
			start = i + 1
			break
		}
	}
	if start == -1 {
		return s, ErrNoSNAPBlock
	}
	if start+4 > len(lines) {
		return s, ErrMalformedFields
	}

	bLine := lines[start]
	aLine := lines[start+1]
	pLine := lines[start+2]
	nLine := lines[start+3]

	if !strings.HasPrefix(bLine, "Branches:") ||
		!strings.HasPrefix(aLine, "Agents:") ||
		!strings.HasPrefix(pLine, "Pending:") ||
		!strings.HasPrefix(nLine, "Next:") {
		return s, ErrMalformedFields
	}

	s.Branches = splitDepth0(strings.TrimSpace(bLine[len("Branches:"):]))
	s.Agents = splitDepth0(strings.TrimSpace(aLine[len("Agents:"):]))
	s.Pending = strings.TrimSpace(pLine[len("Pending:"):])
	s.Next = strings.TrimSpace(nLine[len("Next:"):])
	return s, nil
}

// splitDepth0 splits s on commas at paren-depth 0. Commas nested inside
// parentheses are preserved as part of the surrounding item. Empty input
// returns nil (not an empty slice with one empty string), so a SNAP with no
// branches round-trips cleanly.
func splitDepth0(s string) []string {
	if s == "" {
		return nil
	}
	var items []string
	depth := 0
	start := 0
	for i, r := range s {
		switch r {
		case '(':
			depth++
		case ')':
			if depth > 0 {
				depth--
			}
		case ',':
			if depth == 0 {
				items = append(items, strings.TrimSpace(s[start:i]))
				start = i + 1
			}
		}
	}
	items = append(items, strings.TrimSpace(s[start:]))
	return items
}
