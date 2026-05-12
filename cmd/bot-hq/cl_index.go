package main

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/gregoryerrl/bot-hq/internal/cl"
)

// runCLIndex implements `bot-hq cl-index` modes:
//
//	bot-hq cl-index <project>              # regenerate one project's INDEX.md
//	bot-hq cl-index --all                  # regenerate all + top-level INDEX
//	bot-hq cl-index --fix <project>        # seed canonical-9 + extension dirs + INDEX
//	bot-hq cl-index --fix --all            # fix every project
//	bot-hq cl-index --validate <project>   # report extension-schema findings
//	bot-hq cl-index --validate --all       # validate every project
//
// Modes are mutually exclusive (validate / fix / bare-index). Bare-index
// is the default — backwards-compatible.
//
// Exit code semantics:
//
//	0 — success (no errors)
//	1 — runtime error (file IO / yaml parse / missing project)
//	2 — `--validate` found at least one severity=error issue
func runCLIndex() {
	args := os.Args[2:]
	var (
		all      bool
		fix      bool
		validate bool
		project  string
	)
	for _, a := range args {
		switch a {
		case "--all":
			all = true
		case "--fix":
			fix = true
		case "--validate":
			validate = true
		default:
			if project != "" {
				fmt.Fprintf(os.Stderr, "cl-index: unexpected arg %q\n", a)
				os.Exit(1)
			}
			project = a
		}
	}

	if fix && validate {
		fmt.Fprintln(os.Stderr, "cl-index: --fix and --validate are mutually exclusive")
		os.Exit(1)
	}
	if !all && project == "" {
		fmt.Fprintln(os.Stderr, "cl-index: project name required (or pass --all)\nUsage: bot-hq cl-index <project>\n       bot-hq cl-index --all\n       bot-hq cl-index --fix <project> | --all\n       bot-hq cl-index --validate <project> | --all")
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

	switch {
	case validate:
		runValidate(c, project, all)
	case fix:
		runFix(c, project, all)
	default:
		runIndex(c, project, all)
	}
}

func runIndex(c *cl.CL, project string, all bool) {
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

func runFix(c *cl.CL, project string, all bool) {
	if all {
		fixed, err := c.FixAll()
		for _, p := range fixed {
			fmt.Printf("%s: fixed\n", p)
		}
		if err != nil {
			fmt.Fprintf(os.Stderr, "cl-index --fix --all: %v\n", err)
			os.Exit(1)
		}
		return
	}
	if err := c.FixProject(project); err != nil {
		fmt.Fprintf(os.Stderr, "cl-index --fix %s: %v\n", project, err)
		os.Exit(1)
	}
	fmt.Printf("%s: fixed\n", project)
}

func runValidate(c *cl.CL, project string, all bool) {
	var issues []cl.ValidationIssue
	var err error
	if all {
		issues, err = c.ValidateAll()
	} else {
		issues, err = c.ValidateProject(project)
	}
	if err != nil {
		if errors.Is(err, cl.ErrNotFound) {
			fmt.Fprintf(os.Stderr, "cl-index --validate: %v\n", err)
			os.Exit(1)
		}
		fmt.Fprintf(os.Stderr, "cl-index --validate: %v\n", err)
		os.Exit(1)
	}
	if len(issues) == 0 {
		fmt.Println("ok: no findings")
		return
	}
	errCount := 0
	for _, i := range issues {
		fmt.Println(i.String())
		if i.Severity == "error" {
			errCount++
		}
	}
	if errCount > 0 {
		fmt.Fprintf(os.Stderr, "%d error(s)\n", errCount)
		os.Exit(2)
	}
}
