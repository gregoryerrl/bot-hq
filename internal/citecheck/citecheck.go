// Phase T T-12 cycle-3: mechanical cite-check for hub_send content.
//
// R31-sub MECHANICAL-CITE-FROM-HUB_READ has graduation evidence
// (5+ pre-intervention drifts at Phase T cycle-2 → 10-fire 0-drift
// streak post-Rain-recommendation → 2-fire intra-rotation-cycle
// recurrence at msgs 17459 + 17469). Recurrence WITHIN a single
// cycle proves mechanical-self-discipline insufficient; this package
// adds a daemon-side post-validation pass to surface cite drift to
// the emitting agent.
//
// Scope: msg-id existence verification only. Detects "msg <N>" /
// "msg-<N>" / "id=<N>" patterns in content, looks up each id via
// the caller-provided MsgLookup, and returns Concerns when a cited
// msg-id does not resolve. Content-claim plausibility (sender /
// classification alignment) is fuzzier and out of scope.
//
// Wiring: informational (NOT blocking). hub_send appends concerns
// to the result so the agent can self-correct on the next emit.
// Aligns with R31-sub graduation criterion: agent gets immediate
// mechanical feedback on cite drift; the rule-text encourages
// pre-emit hub_read; the daemon catches the residual.
//
// Layering: leaf-package via Inversion-of-Control (caller provides
// MsgLookup). No internal deps. Mirrors internal/pastedetect.

package citecheck

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"
)

// MsgLookup returns whether the given hub msg-id exists. The bool is
// true when the msg exists; err is non-nil on lookup failure (e.g.,
// DB connectivity). A missing-msg lookup MUST return (false, nil) —
// not an error — so cite-check can distinguish "doesn't exist" from
// "lookup itself failed".
type MsgLookup func(id int64) (exists bool, err error)

// Concern names a single cite drift. The concern is informational —
// callers may surface to the agent via response text but should not
// block the underlying operation.
type Concern struct {
	CitedID    int64  // the msg-id cited in content
	Reason     string // short human-readable reason
	MatchSpan  string // the cited substring (e.g. "msg 17456")
}

// citePattern matches msg-id citations in content. Covers the three
// canonical cite shapes used in bot-hq peer-coord:
//
//	msg 17456     (space)
//	msg-17456     (hyphen)
//	msg17456      (no separator — observed in compact-pipe)
//	id=17456      (key=value pair)
//
// The leading word-boundary keeps "skip-msg-21" / "lastMsgID" /
// task-IDs from false-firing.
var citePattern = regexp.MustCompile(`(?i)\b(?:msg[\s-]?|id=)(\d{2,6})\b`)

// Inspect scans content for msg-id citations and verifies each via
// lookup. Returns the list of Concerns in the order they appear in
// content; returns an empty slice when content is clean.
//
// Behavior:
//   - duplicates are deduplicated (each msg-id reported at most once)
//   - looking up an id whose lookup returns err yields a Concern with
//     Reason "lookup error" (so caller still gets a signal)
//   - non-existent ids yield Concern with Reason "msg-id not found"
func Inspect(content string, lookup MsgLookup) []Concern {
	if lookup == nil {
		return nil
	}
	matches := citePattern.FindAllStringSubmatchIndex(content, -1)
	if len(matches) == 0 {
		return nil
	}
	seen := make(map[int64]struct{}, len(matches))
	var concerns []Concern
	for _, m := range matches {
		idStr := content[m[2]:m[3]]
		id, err := strconv.ParseInt(idStr, 10, 64)
		if err != nil || id <= 0 {
			continue
		}
		if _, dup := seen[id]; dup {
			continue
		}
		seen[id] = struct{}{}
		exists, err := lookup(id)
		span := content[m[0]:m[1]]
		switch {
		case err != nil:
			concerns = append(concerns, Concern{
				CitedID:   id,
				Reason:    fmt.Sprintf("lookup error: %v", err),
				MatchSpan: span,
			})
		case !exists:
			concerns = append(concerns, Concern{
				CitedID:   id,
				Reason:    "msg-id not found in hub.db",
				MatchSpan: span,
			})
		}
	}
	return concerns
}

// FormatNotice returns a one-line cite-check notice suitable for
// appending to a hub_send result. Returns the empty string when
// concerns is empty.
func FormatNotice(concerns []Concern) string {
	if len(concerns) == 0 {
		return ""
	}
	parts := make([]string, 0, len(concerns))
	for _, c := range concerns {
		parts = append(parts, fmt.Sprintf("%s (%s)", c.MatchSpan, c.Reason))
	}
	return "cite-check: " + strings.Join(parts, "; ")
}
