package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/gregoryerrl/bot-hq/internal/contextload"
)

// runContextLoad is the CLI surface for the per-project context loader.
// Mirrors the bot_hq_context_load MCP tool — useful for shell debugging,
// session-open hooks that prefer CLI over MCP, and trio-bypass cases.
//
// Usage:
//
//	bot-hq context-load <project>
//
// Prints assembled markdown to stdout. Exits 1 on missing arg or load
// failure (treats missing project YAML / library as empty layers, not
// errors — only YAML parse / IO permission failures abort).
func runContextLoad() {
	if len(os.Args) < 3 {
		fmt.Fprintf(os.Stderr, "Usage: bot-hq context-load <project>\n")
		os.Exit(1)
	}
	project := os.Args[2]

	home, err := os.UserHomeDir()
	if err != nil {
		fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
		os.Exit(1)
	}
	canonRoot := filepath.Join(home, ".bot-hq")

	c, err := contextload.Load(canonRoot, project)
	if err != nil {
		fmt.Fprintf(os.Stderr, "context-load %s: %v\n", project, err)
		os.Exit(1)
	}

	fmt.Print(c.Markdown())
}
