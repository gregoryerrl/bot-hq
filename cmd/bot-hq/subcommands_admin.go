package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/autoinstall"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/mcp"
	"github.com/gregoryerrl/bot-hq/internal/projects"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/gregoryerrl/bot-hq/internal/toolgate"
	"github.com/gregoryerrl/bot-hq/internal/webui"
)

func runMCP() {
	home, _ := os.UserHomeDir()
	configPath := filepath.Join(home, ".bot-hq", "config.toml")
	cfg, err := hub.LoadConfig(configPath)
	if err != nil {
		// Config errors are fatal — no way to find the DB without config
		fmt.Fprintf(os.Stderr, "config error: %v\n", err)
		os.Exit(1)
	}

	// Phase M M-1 c2 — auto-install duo Stop-hook + PreToolUse-Bash hook
	// at MCP server startup (idempotent + non-clobbering + best-effort).
	// Closes Phase L Finding-1 (installer-not-run on this machine; cost
	// detection-time of ~1 day on Phase L close) by removing the manual
	// `bot-hq install-duo-hook` + `bot-hq install-toolgate-hook`
	// invocation gap. Subcommands remain available for explicit re-install.
	if home != "" {
		if botHQPath, execErr := os.Executable(); execErr == nil && botHQPath != "" {
			settingsPath := filepath.Join(home, ".claude", "settings.json")
			autoinstall.Run(settingsPath, botHQPath, os.Stderr)
		}
	}

	db, err := hub.OpenDB(cfg.Hub.DBPath)
	if err != nil {
		// Start MCP server with an error-only tool set so clients get a
		// proper JSON-RPC response instead of an unexpected EOF.
		fmt.Fprintf(os.Stderr, "db error: %v\n", err)
		mcp.RunStdioServerWithError(fmt.Sprintf("database unavailable: %v", err))
		os.Exit(1)
	}
	defer db.Close()

	if n, err := db.ReconcileCoderGhosts(); err == nil && n > 0 {
		fmt.Fprintf(os.Stderr, "reconciled %d ghost coder agent(s) to offline\n", n)
	}

	if err := mcp.RunStdioServer(db); err != nil {
		fmt.Fprintf(os.Stderr, "mcp error: %v\n", err)
		os.Exit(1)
	}
}

func runStatus() {
	home, _ := os.UserHomeDir()
	dbPath := filepath.Join(home, ".bot-hq", "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "db error: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	agents, err := db.ListAgents("")
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	online := 0
	for _, a := range agents {
		if a.Status == protocol.StatusOnline || a.Status == protocol.StatusWorking {
			online++
		}
		fmt.Printf("  %s  %-15s %-10s %s\n", statusDot(a.Status), a.Name, a.Status, a.Project)
	}
	fmt.Printf("\n[%d agents, %d online]\n", len(agents), online)
}

// runAuditPaneDrift cross-references registered agent tmux_targets
// against live `tmux list-panes` output and reports ghost-targets
// (registered but not present in tmux). Slice-5 H-22-bis instrumentation
// for Class A failure-mode candidate (b): pane regenerated under same
// agent_id without Meta refresh leaves the hub sending to a dead target.
//
// Output: tab-separated text, one row per agent, headers + body. Designed
// for hub_send relay during retry-exhaust events. Staleness column
// (age_seconds) is the smoking-gun field for stale-mapping diagnosis.
func runAuditPaneDrift() {
	home, _ := os.UserHomeDir()
	dbPath := filepath.Join(home, ".bot-hq", "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "db error: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	agents, err := db.ListAgents("")
	if err != nil {
		fmt.Fprintf(os.Stderr, "list agents: %v\n", err)
		os.Exit(1)
	}

	livePanes, err := tmuxpkg.ListPanes()
	if err != nil {
		fmt.Fprintf(os.Stderr, "list panes: %v\n", err)
		os.Exit(1)
	}
	// tmux ListPanes returns full `session:window.pane` triples, but
	// agents register tmux_target as session-name-only (so `tmux send-keys
	// -t <session>` routes to the active pane). Normalize both sides to
	// the session-name prefix for comparison; otherwise every registered
	// target reports missing even when the session is alive.
	liveTargets := make(map[string]struct{}, len(livePanes))
	liveSessions := make(map[string]struct{}, len(livePanes))
	for _, p := range livePanes {
		liveTargets[p.Target] = struct{}{}
		if i := strings.IndexByte(p.Target, ':'); i >= 0 {
			liveSessions[p.Target[:i]] = struct{}{}
		}
	}

	now := time.Now().UTC()
	fmt.Printf("agent_id\ttype\tregistered_target\tlive_status\tlast_seen\tnow\tage_seconds\n")
	for _, a := range agents {
		var meta struct {
			TmuxTarget string `json:"tmux_target"`
		}
		if a.Meta != "" {
			json.Unmarshal([]byte(a.Meta), &meta)
		}
		target := meta.TmuxTarget
		live := "-"
		if target != "" {
			sessionPart := target
			if i := strings.IndexByte(target, ':'); i >= 0 {
				sessionPart = target[:i]
			}
			if _, ok := liveTargets[target]; ok {
				live = "alive"
			} else if _, ok := liveSessions[sessionPart]; ok {
				// Session present, exact pane index may have shifted —
				// dispatch still works (sends to active pane). Treat as alive.
				live = "alive"
			} else {
				live = "missing"
			}
		}
		if target == "" {
			target = "(none)"
		}
		ageSec := int64(now.Sub(a.LastSeen.UTC()).Seconds())
		fmt.Printf("%s\t%s\t%s\t%s\t%s\t%s\t%d\n",
			a.ID,
			string(a.Type),
			target,
			live,
			a.LastSeen.UTC().Format(time.RFC3339),
			now.Format(time.RFC3339),
			ageSec,
		)
	}
}

// runAuditRulesCanonical is the Phase O CLI subcommand that audits all
// per-project YAMLs under ~/.bot-hq/projects/ for canonical nested form
// compliance. Read-only — does not modify any files. Reports each file's
// status (CANONICAL | DRIFT | ERROR) + summary. Exits 0 if all CANONICAL,
// 1 if any DRIFT, 2 if any ERROR.
//
// Usage:
//
//	bot-hq audit-rules-canonical            # audits ~/.bot-hq/projects/
//	bot-hq audit-rules-canonical <dir>      # audits custom dir
//
// Phase O drain — provides the runtime-audit primitive deferred from #6
// (msg 14663 R39 pushback: literal-on-disk verify-tests violate test-
// isolation; runtime-audit-via-CLI is the correct mechanism class).
func runAuditRulesCanonical() {
	dir := ""
	if len(os.Args) > 2 {
		dir = os.Args[2]
	}
	if dir == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(2)
		}
		dir = filepath.Join(home, ".bot-hq", "projects")
	}

	results, err := projects.AuditCanonical(dir)
	if err != nil {
		fmt.Fprintf(os.Stderr, "audit-rules-canonical: %v\n", err)
		os.Exit(2)
	}

	fmt.Print(projects.FormatAuditResults(results))

	exit := 0
	for _, r := range results {
		switch r.Status {
		case projects.StatusError:
			if exit < 2 {
				exit = 2
			}
		case projects.StatusDrift:
			if exit < 1 {
				exit = 1
			}
		}
	}
	os.Exit(exit)
}

// runPreflightCheck is the standalone CLI entry point for the M-1 (i)
// preflight self-check primitives. Reads ~/.claude/settings.json (or
// custom path via arg), runs RunPreflight, prints human-readable Verdict
// to stdout, exits with status 0/1/2 (PASS/WARNING/CRITICAL).
//
// Usage:
//
//	bot-hq preflight-check            # checks ~/.claude/settings.json
//	bot-hq preflight-check <path>     # checks custom settings path
//
// Phase M M-1 (i) — preflight self-check Layer-5 CLI subcommand per
// design-spike v1.1 §3 L5.
func runPreflightCheck() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		p, err := toolgate.DefaultSettingsPath()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve settings path: %v\n", err)
			os.Exit(2)
		}
		settingsPath = p
	}

	v := toolgate.RunPreflight(settingsPath)

	fmt.Printf("preflight: %s\n", v.Status)
	if v.AgentID != "" {
		fmt.Printf("agent-id: %s\n", v.AgentID)
	} else {
		fmt.Printf("agent-id: (BOT_HQ_AGENT_ID absent)\n")
	}
	for _, f := range v.Findings {
		fmt.Printf("  - %s\n", f)
	}
	if v.Status != toolgate.StatusPass {
		fmt.Printf("remediation: bot-hq install-toolgate-hook && export BOT_HQ_AGENT_ID=<brian|rain> && claude session-restart\n")
		fmt.Printf("skill: ~/.claude/skills/phase-rules-detail/SKILL.md § R-NN PRE-FLIGHT-HOOK-CHECK\n")
	}

	switch v.Status {
	case toolgate.StatusPass:
		os.Exit(0)
	case toolgate.StatusWarning:
		os.Exit(1)
	case toolgate.StatusCritical:
		os.Exit(2)
	default:
		os.Exit(2)
	}
}

func statusDot(s protocol.AgentStatus) string {
	switch s {
	case protocol.StatusOnline:
		return "\033[32m●\033[0m" // green
	case protocol.StatusWorking:
		return "\033[33m●\033[0m" // yellow
	default:
		return "\033[90m●\033[0m" // gray
	}
}

// runWebUI starts the Phase N v3 Clive workspace HTTP server on
// 127.0.0.1:<port> (default :3849; override via BOT_HQ_WEBUI_PORT env or
// --port flag). Reads from the canonical-store at ~/.bot-hq/. v3b read-
// only MVP — write capability + Clive integration land in v3c.
//
// Usage:
//
//	bot-hq webui                       # default port :3849
//	bot-hq webui --port 8080           # override port
//	BOT_HQ_WEBUI_PORT=8080 bot-hq webui
func runWebUI() {
	args := os.Args[2:]
	port := 0 // 0 = use NewServer default (env or DefaultPort)
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--port":
			if i+1 >= len(args) {
				fmt.Fprintf(os.Stderr, "webui: --port requires a value\n")
				os.Exit(1)
			}
			p, err := strconv.Atoi(args[i+1])
			if err != nil || p < 0 || p > 65535 {
				fmt.Fprintf(os.Stderr, "webui: invalid port %q\n", args[i+1])
				os.Exit(1)
			}
			port = p
			i++
		default:
			fmt.Fprintf(os.Stderr, "webui: unknown flag %q\nUsage: bot-hq webui [--port N]\n", args[i])
			os.Exit(1)
		}
	}

	home, err := os.UserHomeDir()
	if err != nil {
		fmt.Fprintf(os.Stderr, "webui: home dir: %v\n", err)
		os.Exit(1)
	}
	dbPath := filepath.Join(home, ".bot-hq", "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "webui: open hub.db: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	var opts []webui.Option
	if port > 0 {
		opts = append(opts, webui.WithPort(port))
	}
	srv, err := webui.NewServer(db, opts...)
	if err != nil {
		fmt.Fprintf(os.Stderr, "webui: construct: %v\n", err)
		os.Exit(1)
	}

	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	defer cancel()
	if err := srv.Start(ctx); err != nil && !errors.Is(err, context.Canceled) {
		fmt.Fprintf(os.Stderr, "webui: serve: %v\n", err)
		os.Exit(1)
	}
}

// runEmitCompactNotice fires a MsgCompactNotice into the hub on
// behalf of the agent identified by --agent flag. Used by Claude
// Code PreCompact hook per phase-s.md S-2 §97 + Phase-S-followup-1
// F1-5 wire.
//
// Discriminator-broadcast: queries hub agents via hub.DB; tags [HR]
// when any peer has active fire-in-flight (current_task non-empty
// per Phase-R-followup-1 (f) data-model); untagged when all idle.
//
// Usage:
//
//	bot-hq emit-compact-notice --agent <id>
func runEmitCompactNotice() {
	// Defensive degrade: when invoked from a Claude Code session that lacks
	// BOT_HQ_AGENT_ID (i.e., not a duo pane), the PreCompact hook command
	// `bot-hq emit-compact-notice --agent ${BOT_HQ_AGENT_ID}` resolves to
	// `--agent` with empty / missing value. Without this guard, Go's flag
	// parser exits non-zero, which Claude Code interprets as "block the
	// compact." Non-duo sessions have no peers to notify; the right answer
	// is noop + exit 0 so compaction proceeds normally.
	if emptyAgentInvocation(os.Args[2:]) {
		fmt.Fprintln(os.Stderr, "emit-compact-notice: no BOT_HQ_AGENT_ID (non-duo session); skipping notice")
		return
	}
	flagSet := flag.NewFlagSet("emit-compact-notice", flag.ExitOnError)
	agentID := flagSet.String("agent", "", "agent ID emitting the compact notice (required)")
	if err := flagSet.Parse(os.Args[2:]); err != nil {
		fmt.Fprintf(os.Stderr, "parse flags: %v\n", err)
		os.Exit(1)
	}
	if *agentID == "" {
		fmt.Fprintln(os.Stderr, "emit-compact-notice: --agent <id> required")
		os.Exit(1)
	}

	home, _ := os.UserHomeDir()
	cfg, err := hub.LoadConfig(filepath.Join(home, ".bot-hq", "config.toml"))
	if err != nil {
		fmt.Fprintf(os.Stderr, "config: %v\n", err)
		os.Exit(1)
	}
	dbPath := cfg.Hub.DBPath
	if dbPath == "" {
		dbPath = filepath.Join(home, ".bot-hq", "hub.db")
	}
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "open hub db: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	agents, err := db.ListAgents("")
	if err != nil {
		fmt.Fprintf(os.Stderr, "list agents: %v\n", err)
		os.Exit(1)
	}
	peerActive := protocol.AnyPeerActiveFireInFlight(*agentID, agents)
	content := protocol.BuildCompactNoticeContent(*agentID, peerActive)

	msg := protocol.Message{
		FromAgent: *agentID,
		Type:      protocol.MsgCompactNotice,
		Content:   content,
	}
	id, err := db.InsertMessage(msg)
	if err != nil {
		fmt.Fprintf(os.Stderr, "insert message: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("emit-compact-notice: msg %d (peer-active=%v)\n", id, peerActive)
}

// runEmitResume fires a MsgResume into the hub. Symmetric pair to
// runEmitCompactNotice; called by post-compact resume mechanism (or
// manually by user) to signal context-reloaded so peers can resume
// cross-talk safely.
//
// Usage:
//
//	bot-hq emit-resume --agent <id>
func runEmitResume() {
	// Defensive degrade: same shape as runEmitCompactNotice — non-duo
	// sessions noop instead of failing.
	if emptyAgentInvocation(os.Args[2:]) {
		fmt.Fprintln(os.Stderr, "emit-resume: no BOT_HQ_AGENT_ID (non-duo session); skipping notice")
		return
	}
	flagSet := flag.NewFlagSet("emit-resume", flag.ExitOnError)
	agentID := flagSet.String("agent", "", "agent ID emitting the resume (required)")
	if err := flagSet.Parse(os.Args[2:]); err != nil {
		fmt.Fprintf(os.Stderr, "parse flags: %v\n", err)
		os.Exit(1)
	}
	if *agentID == "" {
		fmt.Fprintln(os.Stderr, "emit-resume: --agent <id> required")
		os.Exit(1)
	}

	home, _ := os.UserHomeDir()
	cfg, err := hub.LoadConfig(filepath.Join(home, ".bot-hq", "config.toml"))
	if err != nil {
		fmt.Fprintf(os.Stderr, "config: %v\n", err)
		os.Exit(1)
	}
	dbPath := cfg.Hub.DBPath
	if dbPath == "" {
		dbPath = filepath.Join(home, ".bot-hq", "hub.db")
	}
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "open hub db: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	content := protocol.BuildResumeContent(*agentID)
	msg := protocol.Message{
		FromAgent: *agentID,
		Type:      protocol.MsgResume,
		Content:   content,
	}
	id, err := db.InsertMessage(msg)
	if err != nil {
		fmt.Fprintf(os.Stderr, "insert message: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("emit-resume: msg %d\n", id)
}

// emptyAgentInvocation reports whether args (typically os.Args[2:])
// indicates the caller invoked the subcommand without a real --agent
// value. Triggered when a hook command like
// `bot-hq emit-compact-notice --agent ${BOT_HQ_AGENT_ID}` runs with
// the env var unset/empty (non-duo Claude Code session). Both
// shell-shapes covered: `--agent` (no following arg) and
// `--agent ""` / `--agent=""`.
func emptyAgentInvocation(args []string) bool {
	for i, a := range args {
		switch a {
		case "--agent", "-agent":
			if i+1 >= len(args) || args[i+1] == "" {
				return true
			}
		case "--agent=", "-agent=":
			return true
		}
	}
	return false
}

