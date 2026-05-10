package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/gregoryerrl/bot-hq/internal/cl"
)

// runCLIndex implements `bot-hq cl-index <project>` and `bot-hq cl-index --all`.
// Regenerates ~/.bot-hq/projects/<project>/INDEX.md from disk state, or all
// project INDEXes + the top-level cross-project INDEX when --all is passed.
//
// Usage:
//
//	bot-hq cl-index <project>            # regenerate one project's INDEX.md
//	bot-hq cl-index --all                # regenerate all + top-level INDEX
//
// Output: a per-project line "ok / no-op" report. Exit 0 on success.
func runCLIndex() {
	args := os.Args[2:]
	all := false
	var project string
	for _, a := range args {
		if a == "--all" {
			all = true
			continue
		}
		if project != "" {
			fmt.Fprintf(os.Stderr, "cl-index: unexpected arg %q\n", a)
			os.Exit(1)
		}
		project = a
	}

	if !all && project == "" {
		fmt.Fprintln(os.Stderr, "cl-index: project name required (or pass --all)\nUsage: bot-hq cl-index <project>\n       bot-hq cl-index --all")
		os.Exit(1)
	}

	home, err := os.UserHomeDir()
	if err != nil {
		fmt.Fprintf(os.Stderr, "cl-index: home dir: %v\n", err)
		os.Exit(1)
	}
	canonRoot := filepath.Join(home, ".bot-hq")
	c, err := cl.NewCL(canonRoot)
	if err != nil {
		fmt.Fprintf(os.Stderr, "cl-index: %v\n", err)
		os.Exit(1)
	}

	if all {
		changed, topChanged, err := c.IndexAll()
		if err != nil {
			fmt.Fprintf(os.Stderr, "cl-index --all: %v\n", err)
			os.Exit(1)
		}
		projects, _ := c.ListProjects()
		for _, p := range projects {
			status := "no-op"
			for _, cp := range changed {
				if cp == p {
					status = "updated"
					break
				}
			}
			fmt.Printf("%s: %s\n", p, status)
		}
		topStatus := "no-op"
		if topChanged {
			topStatus = "updated"
		}
		fmt.Printf("(top-level INDEX.md): %s\n", topStatus)
		return
	}

	_, changed, err := c.IndexProject(project)
	if err != nil {
		fmt.Fprintf(os.Stderr, "cl-index %s: %v\n", project, err)
		os.Exit(1)
	}
	status := "no-op"
	if changed {
		status = "updated"
	}
	fmt.Printf("%s: %s\n", project, status)
}
