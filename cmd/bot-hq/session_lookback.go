package main

import (
	"fmt"
	"os"

	"github.com/gregoryerrl/bot-hq/internal/sessions"
)

// runSessionLookback is the CLI surface for hub_session_lookback.
// Mirrors the MCP tool — useful for shell debugging + non-trio
// invocation paths.
//
// Usage:
//
//	bot-hq session-lookback <session-id>
//
// Prints retrospective markdown to stdout. Exits 1 on missing session
// or read failure.
func runSessionLookback() {
	if len(os.Args) < 3 {
		fmt.Fprintf(os.Stderr, "Usage: bot-hq session-lookback <session-id>\n")
		os.Exit(1)
	}
	id := os.Args[2]

	md, err := sessions.Lookback(id)
	if err != nil {
		if sessions.LookbackErrIsMissing(err) {
			fmt.Fprintf(os.Stderr, "session not found: %s\n", id)
			os.Exit(1)
		}
		fmt.Fprintf(os.Stderr, "lookback %s: %v\n", id, err)
		os.Exit(1)
	}
	fmt.Print(md)
}
