package main

import (
	"fmt"
	"os"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/sessions"
)

// runSessionSummary is the CLI mirror for hub_session_summary.
//
// Usage:
//
//	bot-hq session-summary                     # today
//	bot-hq session-summary --date 2026-05-10   # specific date
//
// Prints the EOD-shaped aggregated markdown to stdout.
func runSessionSummary() {
	when := time.Now().UTC()
	args := os.Args[2:]
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--date":
			if i+1 >= len(args) {
				fmt.Fprintf(os.Stderr, "session-summary --date: value required\n")
				os.Exit(1)
			}
			parsed, err := time.Parse("2006-01-02", args[i+1])
			if err != nil {
				fmt.Fprintf(os.Stderr, "session-summary --date: invalid YYYY-MM-DD: %v\n", err)
				os.Exit(1)
			}
			when = parsed.UTC()
			i++
		default:
			fmt.Fprintf(os.Stderr, "session-summary: unknown arg %q\nUsage: bot-hq session-summary [--date YYYY-MM-DD]\n", args[i])
			os.Exit(1)
		}
	}

	md, err := sessions.SummarizeDate(when)
	if err != nil {
		fmt.Fprintf(os.Stderr, "session-summary: %v\n", err)
		os.Exit(1)
	}
	fmt.Print(md)
}

// runSessionMigrateStale is a one-shot CLI for migrating perpetually-
// active pre-W session manifests to closed-auto-migrated. After Phase W
// has run once successfully, subsequent runs are no-ops.
//
// Usage:
//
//	bot-hq session-migrate-stale            # cutoff = today (UTC)
//	bot-hq session-migrate-stale --cutoff 2026-05-09  # custom cutoff
func runSessionMigrateStale() {
	cutoff := time.Now().UTC().Truncate(24 * time.Hour)
	args := os.Args[2:]
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--cutoff":
			if i+1 >= len(args) {
				fmt.Fprintf(os.Stderr, "session-migrate-stale --cutoff: value required\n")
				os.Exit(1)
			}
			parsed, err := time.Parse("2006-01-02", args[i+1])
			if err != nil {
				fmt.Fprintf(os.Stderr, "session-migrate-stale --cutoff: invalid YYYY-MM-DD: %v\n", err)
				os.Exit(1)
			}
			cutoff = parsed.UTC()
			i++
		default:
			fmt.Fprintf(os.Stderr, "session-migrate-stale: unknown arg %q\nUsage: bot-hq session-migrate-stale [--cutoff YYYY-MM-DD]\n", args[i])
			os.Exit(1)
		}
	}

	migrated, err := sessions.MigrateStaleActive(cutoff)
	if err != nil {
		fmt.Fprintf(os.Stderr, "session-migrate-stale: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("session-migrate-stale: %d session(s) closed with status=closed-auto-migrated (cutoff=%s)\n",
		len(migrated), cutoff.Format("2006-01-02"))
	for _, id := range migrated {
		fmt.Println("  " + id)
	}
}
