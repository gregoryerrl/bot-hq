// cite-validate is a thin CLI wrapper around internal/citeanchor (Phase T T-1.9).
// Usage: cite-validate <path>
//
// Exits 0 on all-valid + skipped; 1 if any invalid anchors found.
//
// Wires hub.DB MessageExists checker (T-2 hub.DB.MessageExists addition)
// for msg-id validation when ~/.bot-hq/hub.db exists.
package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"

	"github.com/gregoryerrl/bot-hq/internal/citeanchor"
	"github.com/gregoryerrl/bot-hq/internal/hub"
)

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: cite-validate <path>")
		os.Exit(2)
	}

	// T-2 wiring: open hub.db for msg-id checks (best-effort)
	var checker citeanchor.MsgIDChecker
	home, err := os.UserHomeDir()
	if err == nil {
		dbPath := filepath.Join(home, ".bot-hq", "hub.db")
		if db, openErr := hub.OpenDB(dbPath); openErr == nil {
			defer db.Close()
			checker = &citeanchor.HubMsgChecker{DB: db}
		}
	}

	res, err := citeanchor.ValidateFile(context.Background(), os.Args[1], checker)
	if err != nil {
		fmt.Fprintf(os.Stderr, "validate: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Anchors: %d / Valid: %d / Invalid: %d / Skipped: %d\n", len(res.Anchors), res.Valid, res.Invalid, res.Skipped)
	if res.Invalid > 0 {
		fmt.Println("\nInvalid anchors (first 20):")
		count := 0
		for _, f := range res.Findings {
			if f.Status == "invalid" {
				count++
				if count > 20 {
					break
				}
				fmt.Printf("  - [%s] %q: %s\n", f.Anchor.Class, f.Anchor.Raw, f.Message)
			}
		}
		os.Exit(1)
	}
}
