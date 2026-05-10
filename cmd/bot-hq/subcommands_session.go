package main

import (
	"fmt"
	"os"
	"strconv"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/sessions"
)

// runSessionLoad is the Phase N v2 #5 N-1(b)-B CLI surface that mirrors
// the hub_session_load MCP tool per N-1 (a) Q-IV RATIFIED lean (iii)
// CLI + file. Prints the manifest content to stdout.
//
// Usage:
//
//	bot-hq session-load <session-id>            # load by id
//	bot-hq session-load --project <project>     # load most-recent for project
func runSessionLoad() {
	if len(os.Args) < 3 {
		fmt.Fprintf(os.Stderr, "Usage: bot-hq session-load <session-id>\n       bot-hq session-load --project <project>\n")
		os.Exit(1)
	}
	var id string
	if os.Args[2] == "--project" {
		if len(os.Args) < 4 {
			fmt.Fprintf(os.Stderr, "session-load --project: project key required\n")
			os.Exit(1)
		}
		project := os.Args[3]
		recent, err := sessions.MostRecentForProject(project)
		if err != nil {
			fmt.Fprintf(os.Stderr, "most-recent lookup failed: %v\n", err)
			os.Exit(1)
		}
		if recent == "" {
			fmt.Fprintf(os.Stderr, "no sessions found for project %q\n", project)
			os.Exit(1)
		}
		id = recent
	} else {
		id = os.Args[2]
	}
	content, err := sessions.LoadManifestContent(id)
	if err != nil {
		if os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "session manifest not found: %s\n", id)
			os.Exit(1)
		}
		fmt.Fprintf(os.Stderr, "load manifest failed: %v\n", err)
		os.Exit(1)
	}
	fmt.Print(content)
}

// runSessionPrune deletes session directories older than the supplied
// retention window. Drives the OQ-5 productionize-class deferral per
// phase-p.md §P-5: configurable retention + on-demand prune subcommand.
//
// Usage:
//
//	bot-hq session-prune                # uses sessions.DefaultRetentionDays
//	bot-hq session-prune --days <N>     # custom retention window in days
//	bot-hq session-prune --dry-run      # report what would be pruned, no delete
//
// Exits 0 on success (zero-or-more pruned), non-zero on error.
func runSessionPrune() {
	days := sessions.DefaultRetentionDays
	dryRun := false
	args := os.Args[2:]
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--days":
			if i+1 >= len(args) {
				fmt.Fprintf(os.Stderr, "session-prune --days: value required\n")
				os.Exit(1)
			}
			n, err := strconv.Atoi(args[i+1])
			if err != nil || n < 0 {
				fmt.Fprintf(os.Stderr, "session-prune --days: positive integer required, got %q\n", args[i+1])
				os.Exit(1)
			}
			days = n
			i++
		case "--dry-run":
			dryRun = true
		default:
			fmt.Fprintf(os.Stderr, "session-prune: unknown arg %q\n", args[i])
			os.Exit(1)
		}
	}
	now := time.Now()
	if dryRun {
		// Dry-run reports what would be pruned without deleting.
		ids, err := sessions.ListSessionIDs()
		if err != nil {
			fmt.Fprintf(os.Stderr, "list sessions: %v\n", err)
			os.Exit(1)
		}
		var wouldPrune []string
		for _, id := range ids {
			ok, err := sessions.IsWithinRetention(id, days, now)
			if err != nil {
				continue
			}
			if !ok {
				wouldPrune = append(wouldPrune, id)
			}
		}
		fmt.Printf("session-prune --dry-run --days=%d: would prune %d session(s)\n", days, len(wouldPrune))
		for _, id := range wouldPrune {
			fmt.Println("  " + id)
		}
		return
	}
	pruned, err := sessions.PruneOlderThan(days, now)
	if err != nil {
		fmt.Fprintf(os.Stderr, "prune failed: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("session-prune --days=%d: pruned %d session(s)\n", days, len(pruned))
	for _, id := range pruned {
		fmt.Println("  " + id)
	}
}

// runSessionSearch performs a cross-session manifest substring search
// per phase-p.md §P-7 (OQ-7 productionize-class). Grep-style output
// for editor / fzf / xargs piping.
//
// Usage:
//
//	bot-hq session-search <query>             # default 50 results
//	bot-hq session-search --limit <N> <query> # custom cap
//
// Exits 0 on success regardless of hit-count (so empty result is a
// normal completion); exits non-zero only on filesystem errors.
func runSessionSearch() {
	limit := 50
	args := os.Args[2:]
	var query string
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--limit":
			if i+1 >= len(args) {
				fmt.Fprintf(os.Stderr, "session-search --limit: value required\n")
				os.Exit(1)
			}
			n, err := strconv.Atoi(args[i+1])
			if err != nil || n <= 0 {
				fmt.Fprintf(os.Stderr, "session-search --limit: positive integer required, got %q\n", args[i+1])
				os.Exit(1)
			}
			limit = n
			i++
		default:
			if query != "" {
				fmt.Fprintf(os.Stderr, "session-search: unexpected positional arg %q (query already set to %q)\n", args[i], query)
				os.Exit(1)
			}
			query = args[i]
		}
	}
	if query == "" {
		fmt.Fprintf(os.Stderr, "Usage: bot-hq session-search [--limit <N>] <query>\n")
		os.Exit(1)
	}
	hits, err := sessions.SearchSessions(query, limit)
	if err != nil {
		fmt.Fprintf(os.Stderr, "search failed: %v\n", err)
		os.Exit(1)
	}
	if len(hits) == 0 {
		fmt.Fprintf(os.Stderr, "no matches for %q\n", query)
		return
	}
	fmt.Print(sessions.FormatSearchResults(hits))
}
