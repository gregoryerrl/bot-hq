package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/agents/projectctx"
	"github.com/gregoryerrl/bot-hq/internal/agents/sessionopen"
	"github.com/gregoryerrl/bot-hq/internal/webui"
)

// runContextSwitch implements `bot-hq context-switch <project>` per
// design-spike §2.4. Emits a printable export hint, re-resolves rules
// for the new project, and prints the new SessionStart prepend to stdout
// so a calling shell wrapper can re-inject it.
//
// Usage:
//
//	bot-hq context-switch <project>            # default agent inferred from $USER or "brian"
//	bot-hq context-switch <project> --agent X
//	bot-hq context-switch <project> --print-export   # only print export hint
func runContextSwitch() {
	args := os.Args[2:]
	if len(args) < 1 {
		fmt.Fprintln(os.Stderr, "context-switch: project name required\nUsage: bot-hq context-switch <project> [--agent X] [--print-export]")
		os.Exit(1)
	}
	project := args[0]
	agent := "brian"
	printExport := false
	noPivot := false
	for i := 1; i < len(args); i++ {
		switch args[i] {
		case "--agent":
			if i+1 >= len(args) {
				fmt.Fprintln(os.Stderr, "--agent requires a value")
				os.Exit(1)
			}
			agent = args[i+1]
			i++
		case "--print-export":
			printExport = true
		case "--no-pivot":
			noPivot = true
		}
	}
	prevProject := strings.TrimSpace(os.Getenv(projectctx.EnvVar))

	if printExport {
		fmt.Printf("export %s=%s\n", projectctx.EnvVar, project)
		return
	}

	// Verify project is registered (warn-only — don't block).
	home, err := os.UserHomeDir()
	if err != nil {
		fmt.Fprintf(os.Stderr, "context-switch: home dir: %v\n", err)
		os.Exit(1)
	}
	canonRoot := filepath.Join(home, ".bot-hq")
	yamlPath := filepath.Join(canonRoot, "projects", project+".yaml")
	if _, err := os.Stat(yamlPath); os.IsNotExist(err) {
		fmt.Fprintf(os.Stderr, "warning: %s not found (project not registered)\n", yamlPath)
	}

	// Try the daemon first; fall back to direct in-process build.
	if payload := tryFetchSessionOpen(project, agent); payload != nil {
		fmt.Println(sessionopen.FormatClaude(payload))
	} else {
		payload, err := sessionopen.Build(canonRoot, project, agent)
		if err != nil {
			fmt.Fprintf(os.Stderr, "context-switch: session-open build: %v\n", err)
			os.Exit(1)
		}
		fmt.Println(sessionopen.FormatClaude(payload))
	}

	fmt.Fprintf(os.Stderr, "\n# To make the switch sticky for this shell:\n# export %s=%s\n", projectctx.EnvVar, project)

	if !noPivot {
		postHubPivot(agent, project, prevProject)
	}
}

// postHubPivot announces the context-switch to the hub via POST
// /api/hub-pivot per design-spike 157ea7f §2.4 + Phase O drain #4.
// Best-effort: daemon unreachable / non-200 → warn to stderr, don't
// block the local pivot. Caller's context-switch already succeeded
// before this fires; hub notification is informational only.
func postHubPivot(agent, project, prevProject string) {
	url := fmt.Sprintf("http://127.0.0.1:%d/api/hub-pivot", webui.DefaultPort)
	body, err := json.Marshal(map[string]string{
		"agent":        agent,
		"project":      project,
		"prev_project": prevProject,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "warning: hub-pivot marshal failed: %v\n", err)
		return
	}
	req, err := http.NewRequest("POST", url, bytes.NewReader(body))
	if err != nil {
		fmt.Fprintf(os.Stderr, "warning: hub-pivot request build failed: %v\n", err)
		return
	}
	req.Header.Set("Content-Type", "application/json")
	client := &http.Client{Timeout: 2 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		fmt.Fprintf(os.Stderr, "warning: hub-pivot post failed (daemon down?): %v\n", err)
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		respBody, _ := io.ReadAll(resp.Body)
		fmt.Fprintf(os.Stderr, "warning: hub-pivot returned %d: %s\n", resp.StatusCode, strings.TrimSpace(string(respBody)))
	}
}

// runSessionOpen implements `bot-hq session-open` — invoked by the
// SessionStart hook. Reads BOT_HQ_PROJECT (or detects it) + agent flag,
// fetches /api/session-open from the local daemon (falls back to direct
// build), formats with the appropriate adapter, prints to stdout for the
// harness to inject as system-prompt prepend.
//
// Usage: bot-hq session-open [--agent X] [--project P] [--format claude|emma]
func runSessionOpen() {
	args := os.Args[2:]
	agent := ""
	project := ""
	format := "claude"
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--agent":
			if i+1 < len(args) {
				agent = args[i+1]
				i++
			}
		case "--project":
			if i+1 < len(args) {
				project = args[i+1]
				i++
			}
		case "--format":
			if i+1 < len(args) {
				format = args[i+1]
				i++
			}
		}
	}
	if project == "" {
		project = projectctx.Detect()
	}
	if agent == "" {
		agent = inferAgent()
	}

	payload := tryFetchSessionOpen(project, agent)
	if payload == nil {
		home, _ := os.UserHomeDir()
		canonRoot := filepath.Join(home, ".bot-hq")
		var err error
		payload, err = sessionopen.Build(canonRoot, project, agent)
		if err != nil {
			fmt.Fprintf(os.Stderr, "session-open: %v\n", err)
			os.Exit(1)
		}
	}
	switch format {
	case "emma":
		// Emma adapter import path is internal/agents/emma — keep this
		// command flexible without pulling that import here. The hook
		// can shell out to a future `bot-hq session-open --format emma`
		// once Emma's harness is wired (Phase O integration). For now
		// we degrade to the claude adapter.
		fmt.Print(sessionopen.FormatClaude(payload))
	default:
		fmt.Print(sessionopen.FormatClaude(payload))
	}
}

// inferAgent picks an agent from $BOT_HQ_AGENT or falls back to "brian".
func inferAgent() string {
	if a := strings.TrimSpace(os.Getenv("BOT_HQ_AGENT")); a != "" {
		return a
	}
	return "brian"
}

// tryFetchSessionOpen calls the local daemon's /api/session-open. Returns
// nil on any failure (caller falls back to in-process build).
func tryFetchSessionOpen(project, agent string) *sessionopen.Payload {
	url := fmt.Sprintf("http://127.0.0.1:%d/api/session-open?project=%s&agent=%s", webui.DefaultPort, project, agent)
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return nil
	}
	resp, err := (&http.Client{}).Do(req)
	if err != nil {
		return nil
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return nil
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil
	}
	var p sessionopen.Payload
	if err := json.Unmarshal(body, &p); err != nil {
		return nil
	}
	return &p
}
