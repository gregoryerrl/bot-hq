// Phase W EOD summary aggregator: render all sessions matching a
// date as an aggregated retrospective. Optimized for duo EOD-report
// generation — the agent reads this and weaves it into user-facing
// EOD-voice content per feedback_eod_style.md.

package sessions

import (
	"fmt"
	"sort"
	"strings"
	"time"
)

// SummarizeDate returns aggregated markdown of all sessions whose
// session-id starts with the YYYY-MM-DD prefix matching `date`.
// Includes both closed and active sessions; sorts by project →
// session-id-asc within each project. Empty result returns a
// "no sessions" stub markdown so callers don't have to special-case.
func SummarizeDate(date time.Time) (string, error) {
	prefix := date.UTC().Format("2006-01-02")
	ids, err := ListSessionIDs()
	if err != nil {
		return "", err
	}
	var dayIDs []string
	for _, id := range ids {
		if strings.HasPrefix(id, prefix) {
			dayIDs = append(dayIDs, id)
		}
	}
	if len(dayIDs) == 0 {
		return fmt.Sprintf("# Session summary: %s\n\n_No sessions found for this date._\n", prefix), nil
	}
	sort.Strings(dayIDs)

	// Group by project
	byProject := map[string][]Manifest{}
	for _, id := range dayIDs {
		m, err := ReadManifest(id)
		if err != nil {
			continue // skip unreadable manifests; index rebuild can fix
		}
		byProject[m.Project] = append(byProject[m.Project], m)
	}

	// Compute totals from actually-readable manifests; counts that
	// claim N when only N-1 are renderable would be misleading.
	var totalCommits, totalFiles, totalDecisions int
	var closedCount, readableCount int
	for _, ms := range byProject {
		for _, m := range ms {
			readableCount++
			totalCommits += len(m.CommitsLanded)
			totalFiles += len(m.FilesTouched)
			totalDecisions += len(m.Decisions)
			if !m.EndTS.IsZero() {
				closedCount++
			}
		}
	}

	var b strings.Builder
	fmt.Fprintf(&b, "# Session summary: %s\n\n", prefix)
	fmt.Fprintf(&b, "**Sessions:** %d (across %d projects, %d closed / %d still active)\n",
		readableCount, len(byProject), closedCount, readableCount-closedCount)
	if skipped := len(dayIDs) - readableCount; skipped > 0 {
		fmt.Fprintf(&b, "_(%d session director%s skipped — manifest unreadable)_\n", skipped, plural(skipped, "y", "ies"))
	}
	fmt.Fprintf(&b, "**Total commits landed:** %d\n", totalCommits)
	fmt.Fprintf(&b, "**Total files touched (sum, may double-count across sessions):** %d\n", totalFiles)
	fmt.Fprintf(&b, "**Total decisions captured:** %d\n\n", totalDecisions)

	projects := make([]string, 0, len(byProject))
	for p := range byProject {
		projects = append(projects, p)
	}
	sort.Strings(projects)

	for _, p := range projects {
		title := p
		if title == "" {
			title = "(no project)"
		}
		fmt.Fprintf(&b, "## %s\n\n", title)
		for _, m := range byProject[p] {
			renderSummaryEntry(&b, m)
		}
	}

	return b.String(), nil
}

// renderSummaryEntry appends a single session block to the summary.
// Compact format: heading + 1-line metadata + outcome (trimmed to first
// 200 chars or first paragraph) + counts.
func renderSummaryEntry(b *strings.Builder, m Manifest) {
	status := m.Status
	if status == "" {
		if m.EndTS.IsZero() {
			status = "active"
		} else {
			status = "closed"
		}
	}

	timeWindow := "(no end)"
	if !m.EndTS.IsZero() && !m.StartTS.IsZero() {
		timeWindow = fmt.Sprintf("%s → %s", m.StartTS.UTC().Format("15:04Z"), m.EndTS.UTC().Format("15:04Z"))
	} else if !m.StartTS.IsZero() {
		timeWindow = fmt.Sprintf("%s → still active", m.StartTS.UTC().Format("15:04Z"))
	}

	fmt.Fprintf(b, "### %s — %s\n\n", m.ID, status)
	fmt.Fprintf(b, "**Time:** %s\n", timeWindow)

	if m.MsgCount > 0 {
		fmt.Fprintf(b, "**Hub msgs:** %d\n", m.MsgCount)
	}
	if len(m.Agents) > 0 {
		fmt.Fprintf(b, "**Agents:** %s\n", strings.Join(m.Agents, ", "))
	}

	if m.Outcome != "" {
		// First paragraph or first 200 chars, whichever shorter
		outcome := strings.TrimSpace(m.Outcome)
		if i := strings.Index(outcome, "\n\n"); i > 0 && i < 200 {
			outcome = outcome[:i]
		} else if len(outcome) > 200 {
			outcome = outcome[:200] + "..."
		}
		fmt.Fprintf(b, "\n%s\n", outcome)
	} else if m.EndTS.IsZero() {
		b.WriteString("\n_(active — no outcome yet)_\n")
	} else {
		b.WriteString("\n_(closed without outcome — pre-W manifest or migration artifact)_\n")
	}

	// Counts row only when there's at least one structured field
	if len(m.CommitsLanded)+len(m.FilesTouched)+len(m.Decisions) > 0 {
		fmt.Fprintf(b, "\n**Commits:** %d | **Files:** %d | **Decisions:** %d\n",
			len(m.CommitsLanded), len(m.FilesTouched), len(m.Decisions))
	}
	b.WriteString("\n")
}

// MigrateStaleActive scans for active sessions older than `cutoff` and
// finalizes each with status="closed-auto-migrated" + a synthetic
// outcome. Returns the list of session-ids that were migrated.
//
// `cutoff` is typically time.Now().UTC().Truncate(24h) — i.e., active
// sessions from yesterday or earlier are stale; today's active session
// is intentional in-flight work (not migration target).
//
// Idempotent: re-running after a successful migration returns an empty
// list (no active sessions older than cutoff remain).
func MigrateStaleActive(cutoff time.Time) ([]string, error) {
	ids, err := ListSessionIDs()
	if err != nil {
		return nil, err
	}
	var migrated []string
	for _, id := range ids {
		m, err := ReadManifest(id)
		if err != nil {
			continue
		}
		if !m.EndTS.IsZero() {
			continue // already closed
		}
		if m.StartTS.After(cutoff) || m.StartTS.Equal(cutoff) {
			continue // not yet stale (within or after cutoff window)
		}
		// Synthetic outcome — explicit migration label so future
		// retrospective queries can filter migrated sessions out
		// (or in) cleanly.
		_, err = Finalize(id, FinalizeOptions{
			Outcome: fmt.Sprintf(
				"Auto-migrated by Phase W sessions hardening 2026-05-10. "+
					"This session was perpetually-active under the pre-W timer-snapshot regime; "+
					"closed retroactively with no in-flight work as part of the cleanup. "+
					"Original start: %s.",
				m.StartTS.UTC().Format(time.RFC3339)),
			Status: "closed-auto-migrated",
			Now:    cutoff,
		})
		if err != nil {
			// Don't abort the migration on a single failure — log via
			// returned slice (caller can compare actual-migrated vs total
			// stale count).
			continue
		}
		migrated = append(migrated, id)
	}
	return migrated, nil
}

// plural returns sing for n=1, plur otherwise. Tiny helper for "1 entry"
// vs "N entries" pluralization in summary diagnostics.
func plural(n int, sing, plur string) string {
	if n == 1 {
		return sing
	}
	return plur
}
