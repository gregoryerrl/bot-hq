package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/gregoryerrl/bot-hq/internal/autoinstall"
	"github.com/gregoryerrl/bot-hq/internal/brian"
	"github.com/gregoryerrl/bot-hq/internal/daemoncron"
	"github.com/gregoryerrl/bot-hq/internal/discord"
	"github.com/gregoryerrl/bot-hq/internal/gemma"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/mcp"
	"github.com/gregoryerrl/bot-hq/internal/outboundhook"
	"github.com/gregoryerrl/bot-hq/internal/projects"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/rain"
	"github.com/gregoryerrl/bot-hq/internal/sessions"
	"github.com/gregoryerrl/bot-hq/internal/sessionstarthook"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/gregoryerrl/bot-hq/internal/toolgate"
	"github.com/gregoryerrl/bot-hq/internal/ui"
	"github.com/gregoryerrl/bot-hq/internal/voicemirror"
	"github.com/gregoryerrl/bot-hq/internal/webui"
)

func main() {
	if len(os.Args) > 1 {
		switch os.Args[1] {
		case "mcp":
			runMCP()
			return
		case "status":
			runStatus()
			return
		case "audit-pane-drift":
			runAuditPaneDrift()
			return
		case "outbound-miss-hook":
			runOutboundMissHook()
			return
		case "install-trio-hook":
			runInstallTrioHook()
			return
		case "tool-permission-hook":
			runToolPermissionHook()
			return
		case "install-toolgate-hook":
			runInstallToolgateHook()
			return
		case "preflight-check":
			runPreflightCheck()
			return
		case "voice-mirror-hook":
			runVoiceMirrorHook()
			return
		case "install-voice-mirror-hook":
			runInstallVoiceMirrorHook()
			return
		case "session-prune":
			runSessionPrune()
			return
		case "session-search":
			runSessionSearch()
			return
		case "session-load":
			runSessionLoad()
			return
		case "webui":
			runWebUI()
			return
		case "context-switch":
			runContextSwitch()
			return
		case "session-open":
			runSessionOpen()
			return
		case "install-session-start-hook":
			runInstallSessionStartHook()
			return
		case "audit-rules-canonical":
			runAuditRulesCanonical()
			return
		case "emit-compact-notice":
			runEmitCompactNotice()
			return
		case "emit-resume":
			runEmitResume()
			return
		case "config":
			runConfig(os.Args[2:])
			return
		case "version":
			// Ensure config directory and default config exist
			home, _ := os.UserHomeDir()
			hub.LoadConfig(filepath.Join(home, ".bot-hq", "config.toml"))
			fmt.Printf("bot-hq v%s\n", protocol.Version)
			return
		default:
			fmt.Fprintf(os.Stderr, "unknown command: %s\nUsage: bot-hq [mcp|status|audit-pane-drift|audit-rules-canonical|outbound-miss-hook|install-trio-hook|tool-permission-hook|install-toolgate-hook|preflight-check|voice-mirror-hook|install-voice-mirror-hook|session-load|session-prune|session-search|webui|context-switch|session-open|install-session-start-hook|emit-compact-notice|emit-resume|version]\n", os.Args[1])
			os.Exit(1)
		}
	}
	runHub()
}

func runHub() {
	home, _ := os.UserHomeDir()

	// 1. Load config
	configPath := filepath.Join(home, ".bot-hq", "config.toml")
	cfg, err := hub.LoadConfig(configPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "config error: %v\n", err)
		os.Exit(1)
	}

	// 2. Create and start hub
	h, err := hub.NewHub(cfg)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hub error: %v\n", err)
		os.Exit(1)
	}

	// 3. Apply DB settings (overrides config file)
	cfg.ApplyDBSettings(h.DB)
	h.Config = cfg

	if err := h.Start(); err != nil {
		fmt.Fprintf(os.Stderr, "hub start error: %v\n", err)
		os.Exit(1)
	}
	defer h.Stop()

	// Phase S S-1a-1: daemoncron — daemon-side cadence-driven hub
	// emits replicating gemma surfaces (heartbeat-ledger this commit;
	// stale-coder / plan-usage / context-cap / delivery-gap / egress-
	// audit / lifecycle / sentinel in subsequent S-1a-N sub-commits).
	// Per Rain msg 15796 PUSH-BACK A interpretation (ii) dual-emit-
	// prevention: gemma emit-call-sites short-circuit when daemoncron
	// is online (wired below post-emma-Start via SetDaemoncronOnline).
	dc := daemoncron.NewWithDefaults(h.DB)
	if err := dc.Start(); err != nil {
		log.Printf("[autostart] daemoncron FAILED: %v", err)
	} else {
		log.Printf("[autostart] daemoncron OK (heartbeat-ledger)")
		defer dc.Stop()
	}

	// 4. Redirect log output to file — TUI owns the terminal
	logFile, logErr := os.OpenFile(filepath.Join(home, ".bot-hq", "live.log"), os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0600)
	if logErr == nil {
		log.SetOutput(logFile)
		defer logFile.Close()
	} else {
		log.SetOutput(io.Discard)
	}

	// 5. Workspace webui on :3849 — the single unified web UI. Voice +
	// workspace + Clive activity + pending-actions all served from this
	// one HTTP server. Goroutine-isolated so a webui error never crashes
	// the hub. Opt-out: BOT_HQ_WEBUI_DISABLE=1. Port-conflict graceful-
	// skip:
	// Goroutine-isolated so a webui error never crashes the hub.
	// Opt-out: BOT_HQ_WEBUI_DISABLE=1. Port-conflict graceful-skip:
	// if :3849 is already bound (manual `bot-hq webui` running), the
	// inner Start() returns the bind error; we log + continue.
	webuiCtx, webuiCancel := context.WithCancel(context.Background())
	defer webuiCancel()
	if os.Getenv("BOT_HQ_WEBUI_DISABLE") != "1" {
		webuiSrv, werr := webui.NewServer(h.DB)
		if werr != nil {
			log.Printf("[autostart] webui FAILED to construct: %v", werr)
		} else {
			go func() {
				if err := webuiSrv.Start(webuiCtx); err != nil &&
					!errors.Is(err, context.Canceled) &&
					!errors.Is(err, http.ErrServerClosed) {
					// Bind-conflict / runtime error — log + continue without crashing daemon.
					log.Printf("[autostart] webui serve error (continuing): %v", err)
				}
			}()
			log.Printf("[autostart] webui OK")
		}
	} else {
		log.Printf("[autostart] webui DISABLED via BOT_HQ_WEBUI_DISABLE=1")
	}

	// 5c. Phase N v3.x-2 bootstrap auto-write defensive snapshot loop.
	// Per design-spike §2.3: every 25 hub-msgs OR every 10 minutes (whichever
	// first), write the current AgentState snapshot to projects/<p>/bootstrap.md
	// for crash-recovery. Atomic via temp+rename. Opt-out: BOT_HQ_BOOTSTRAP_DISABLE=1.
	if os.Getenv("BOT_HQ_BOOTSTRAP_DISABLE") != "1" {
		go runBootstrapDefensiveLoop(webuiCtx, h, home)
		log.Printf("[autostart] bootstrap-defensive-loop OK")
	}

	// 6. Start Discord bot if configured (Phase R R4 multi-channel support;
	// either legacy ChannelID OR Phase R HubChannelID populated suffices).
	if cfg.Discord.Token != "" && (cfg.Discord.ChannelID != "" || cfg.Discord.HubChannelID != "") {
		discordBot, err := discord.NewBot(
			cfg.Discord.Token,
			cfg.Discord.ChannelID,
			cfg.Discord.HubChannelID,
			cfg.Discord.FlagsChannelID,
			cfg.Discord.SessionsChannelID,
			h,
		)
		if err == nil {
			if err := discordBot.Start(); err == nil {
				defer discordBot.Stop()
			}
		}
	}

	// 6b. Phase T T-14 cycle-3: tmux orphan-session cleanup. On daemon-
	// restart, kill any pre-restart bot-hq-* tmux sessions before spawning
	// fresh ones — eliminates the orphan-pane confusion class that Rain
	// msg 17419 push-back A flagged (old panes look alive but are
	// orphaned from the new daemon's hub-coord routing). Best-effort:
	// failures are logged but do not block startup.
	if killed, errs := tmuxpkg.CleanupOrphanSessions(nil); len(killed) > 0 || len(errs) > 0 {
		if len(killed) > 0 {
			log.Printf("[autostart] tmux orphan-cleanup killed %d session(s): %v", len(killed), killed)
		}
		for _, e := range errs {
			log.Printf("[autostart] tmux orphan-cleanup err: %v", e)
		}
	}

	// 7. Build Brian orchestrator instance (Start deferred until after TUI
	// is ready so its first inserts reach the TUI via OnMessage).
	var brianOrch *brian.Brian
	log.Printf("[autostart] brian=%v rain=%v emma=%v", cfg.Brian.AutoStart, cfg.Rain.AutoStart, cfg.Gemma.AutoStart)
	if cfg.Brian.AutoStart {
		brianOrch = brian.New(h.DB, cfg.Brian.WorkDir)
	}

	// 8. Build TUI app + program and wire OnMessage BEFORE starting agents.
	// In-process inserts (autostart errors, internal monitors) emitted during
	// step 9 below now reach the TUI immediately; cross-process MCP inserts
	// continue to surface via the tick poll in App.Update.
	app := ui.NewApp(cfg, h.DB, brianOrch)
	uiPane := app.Pane()
	p := tea.NewProgram(app, tea.WithAltScreen())
	h.DB.OnMessage(func(msg protocol.Message) {
		p.Send(ui.MessageReceived{Message: msg})
	})

	// 9. Start Brian orchestrator
	if brianOrch != nil {
		if err := brianOrch.Start(); err != nil {
			log.Printf("[autostart] brian FAILED: %v", err)
			h.DB.InsertMessage(protocol.Message{
				FromAgent: "system",
				Type:      protocol.MsgError,
				Content:   fmt.Sprintf("Brian auto-start failed: %v", err),
			})
		} else {
			log.Printf("[autostart] brian OK")
			defer brianOrch.Stop()
		}
	}

	// 9b. Start Rain QA agent if configured
	if cfg.Rain.AutoStart {
		rainAgent := rain.New(h.DB, cfg.Rain.WorkDir)
		if err := rainAgent.Start(); err != nil {
			log.Printf("[autostart] rain FAILED: %v", err)
			h.DB.InsertMessage(protocol.Message{
				FromAgent: "system",
				Type:      protocol.MsgError,
				Content:   fmt.Sprintf("Rain auto-start failed: %v", err),
			})
		} else {
			log.Printf("[autostart] rain OK")
			defer rainAgent.Stop()
		}
	}

	// 9b-T-10. Phase T T-10 cycle-3: vault file mtime watcher. Detects
	// user-side rotation (manual edit of `~/.bot-hq/agents/<agent>/.env`)
	// and emits a hub MsgUpdate so rotation feedback reaches the user
	// without a "signal me when done" hub round-trip — eliminates the
	// paste-to-hub temptation class that recurred in this rotation cycle.
	if vw := startVaultWatcher(h); vw != nil {
		defer vw.Stop()
	}

	// 9c. Start Emma (the persistent monitor agent, backed by the gemma package + model) if configured
	if cfg.Gemma.AutoStart {
		emmaAgent := gemma.New(h.DB, cfg.Gemma)
		// Phase H slice 5 C1 (H-32): wire Emma's plan-usage producer to
		// the TUI's panestate.Manager so successful 60s polls publish
		// HubSnapshot{PlanUsagePct, PlanWindow} that strip.go reads. Set
		// before Start so the first poll's publish lands in the same
		// Manager the UI is reading from.
		if uiPane != nil {
			emmaAgent.SetHubPublisher(uiPane.SetHubSnapshot)
		}
		if err := emmaAgent.Start(); err != nil {
			log.Printf("[autostart] emma FAILED: %v", err)
			h.DB.InsertMessage(protocol.Message{
				FromAgent: "system",
				Type:      protocol.MsgError,
				Content:   fmt.Sprintf("Emma auto-start failed: %v", err),
			})
		} else {
			log.Printf("[autostart] emma OK")
			// Wire Emma's hub-reactive sentinel subscriber. OnMessage fires
			// for every in-process insert; cross-process MCP inserts surface
			// to Emma via her own boot-time replay + the live tick path is
			// not needed (sentinel is purely event-driven).
			h.DB.OnMessage(emmaAgent.OnHubMessage)
			// Phase S S-1a-1: signal emma-side that daemoncron is online
			// so gemma's heartbeat-ledger emit-call-site short-circuits
			// (interpretation (ii) dual-emit-prevention per Rain msg 15796).
			if dc != nil && dc.IsRunning() {
				emmaAgent.SetDaemoncronOnline(true)
			}
			defer emmaAgent.Stop()
		}
	}

	// 10. Run Bubbletea TUI
	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "TUI error: %v\n", err)
		os.Exit(1)
	}
}

func runMCP() {
	home, _ := os.UserHomeDir()
	configPath := filepath.Join(home, ".bot-hq", "config.toml")
	cfg, err := hub.LoadConfig(configPath)
	if err != nil {
		// Config errors are fatal — no way to find the DB without config
		fmt.Fprintf(os.Stderr, "config error: %v\n", err)
		os.Exit(1)
	}

	// Phase M M-1 c2 — auto-install trio Stop-hook + PreToolUse-Bash hook
	// at MCP server startup (idempotent + non-clobbering + best-effort).
	// Closes Phase L Finding-1 (installer-not-run on this machine; cost
	// detection-time of ~1 day on Phase L close) by removing the manual
	// `bot-hq install-trio-hook` + `bot-hq install-toolgate-hook`
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

// runOutboundMissHook is the Claude Code Stop-hook entry. Reads the
// hook input JSON from stdin (transcript_path et al), evaluates the
// three-clause filter, emits an OUTBOUND-MISS alert via the hub when
// the agent produced pane text without a hub_send tool call, AND blocks
// the stop event via {decision:block} JSON stdout output + ExitBlock=2
// + stderr propagation when shouldFlag fires. Phase M M-2 c1 R36 OUTBOUND-
// DISCIPLINE-MECHANICAL enforcement-conversion (mirrors R33 toolgate
// gate-CHECK exit-code propagation pattern).
func runOutboundMissHook() {
	os.Exit(outboundhook.RunHook(os.Stdin, os.Stdout, os.Stderr))
}

// runInstallTrioHook installs the OUTBOUND-MISS Stop hook into the
// trio agent's Claude settings.json. Idempotent + non-clobbering.
//
// Usage:
//
//	bot-hq install-trio-hook            # writes ~/.claude/settings.json
//	bot-hq install-trio-hook <path>     # writes a custom path
//
// User must additionally export BOT_HQ_AGENT_ID=<id> in the agent's
// pane environment so the hook knows which agent it is firing for.
func runInstallTrioHook() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(1)
		}
		settingsPath = filepath.Join(home, ".claude", "settings.json")
	}
	botHQPath, err := os.Executable()
	if err != nil || botHQPath == "" {
		fmt.Fprintf(os.Stderr, "resolve bot-hq binary path: %v\n", err)
		os.Exit(1)
	}
	if err := outboundhook.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(os.Stderr, "install: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("OUTBOUND-MISS hook installed in %s\n", settingsPath)
	fmt.Printf("Hook command: %s\n", outboundhook.SettingsHookCommand(botHQPath))
	fmt.Printf("Reminder: autostart trio panes set BOT_HQ_AGENT_ID automatically. For panes launched outside autostart (manual claude exec), export BOT_HQ_AGENT_ID=<id> before launch.\n")
}

// runToolPermissionHook is the PreToolUse hook entry point for the K-16
// class-split gate. Reads PreToolUse hook input from stdin, applies the
// gate, exits with 0 (allow) or 2 (block).
func runToolPermissionHook() {
	os.Exit(toolgate.RunHook(os.Stdin, os.Stderr))
}

// runVoiceMirrorHook is the Phase N v2 #3 N-2 PreToolUse hook entry
// point per R40 VOICE-MIRROR-DISCIPLINE. Reads JSON from stdin (Claude
// Code PreToolUse Write event payload), invokes voicemirror.RunHook
// which is alert-only (NOT blocking) — always exits 0.
func runVoiceMirrorHook() {
	os.Exit(voicemirror.RunHook(os.Stdin, os.Stderr))
}

// runInstallVoiceMirrorHook installs the Phase N v2 #3 N-2 PreToolUse-
// Write hook into the trio agent's Claude settings.json per R40 VOICE-
// MIRROR-DISCIPLINE. Idempotent + non-clobbering, mirroring
// runInstallToolgateHook + runInstallTrioHook patterns.
//
// Usage:
//
//	bot-hq install-voice-mirror-hook            # writes ~/.claude/settings.json
//	bot-hq install-voice-mirror-hook <path>     # writes a custom path
//
// Phase N v2 #8 close-composite — folds install subcommand per Rain
// Q2 lean (b1) at #3 N-2 PASS-2.
func runInstallVoiceMirrorHook() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(1)
		}
		settingsPath = filepath.Join(home, ".claude", "settings.json")
	}
	botHQPath, err := os.Executable()
	if err != nil || botHQPath == "" {
		fmt.Fprintf(os.Stderr, "resolve bot-hq binary path: %v\n", err)
		os.Exit(1)
	}
	if err := voicemirror.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(os.Stderr, "install: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Voice-mirror PreToolUse-Write hook installed in %s\n", settingsPath)
	fmt.Printf("Hook command: %s\n", voicemirror.SettingsHookCommand(botHQPath))
	fmt.Printf("Hook fires on Write tool calls against user-artifact paths per R40 VOICE-MIRROR-DISCIPLINE (alert-only, NOT blocking).\n")
	fmt.Printf("INCLUDE patterns: ~/Documents/*, ~/Desktop/*, ~/.bot-hq/projects/<project>/{plans,eod,clips}/*, CLAUDE.md, README.md\n")
	fmt.Printf("SKIP patterns: **/memory/**, .git/, .cache/, node_modules/\n")
	fmt.Printf("Log: ~/.bot-hq/voice-mirror-log.md (override via BOT_HQ_VOICE_MIRROR_LOG_PATH env).\n")
	fmt.Printf("Hook activation requires Claude session-restart (settings.json not hot-reloaded mid-session per Phase L Finding-3).\n")
}

// runInstallSessionStartHook installs the Phase N v3.x-1.5 SessionStart
// hook into the trio agent's Claude settings.json. The installed hook
// command invokes `bot-hq session-open` at session-start, which fetches
// the daemon's /api/session-open and prints markdown the harness
// prepends as system-prompt context (overview + bootstrap + resolved
// rules + tasks). Idempotent + non-clobbering, mirroring
// runInstallTrioHook + runInstallToolgateHook + runInstallVoiceMirrorHook.
//
// Usage:
//
//	bot-hq install-session-start-hook            # writes ~/.claude/settings.json
//	bot-hq install-session-start-hook <path>     # writes a custom path
//
// Phase N v3.x-1.5 design-spike (157ea7f) §2.2 specifies the hook
// invocation surface. v3.x-2 implementation landed the session-open
// subcommand (cmd/bot-hq/context_switch.go runSessionOpen); this
// subcommand wires it into Claude settings.json so the hook fires
// automatically at SessionStart instead of requiring manual
// settings.json editing.
func runInstallSessionStartHook() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(1)
		}
		settingsPath = filepath.Join(home, ".claude", "settings.json")
	}
	botHQPath, err := os.Executable()
	if err != nil || botHQPath == "" {
		fmt.Fprintf(os.Stderr, "resolve bot-hq binary path: %v\n", err)
		os.Exit(1)
	}
	if err := sessionstarthook.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(os.Stderr, "install: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("SessionStart hook installed in %s\n", settingsPath)
	fmt.Printf("Hook command: %s\n", sessionstarthook.SettingsHookCommand(botHQPath))
	fmt.Printf("Hook fires at Claude SessionStart and prepends bot-hq session-open output (overview + bootstrap + resolved rules + tasks) as system-prompt context.\n")
	fmt.Printf("Project context: $BOT_HQ_PROJECT env var (authoritative); falls back to cwd-inference / 'bot-hq' default.\n")
	fmt.Printf("Agent context: $BOT_HQ_AGENT env var; falls back to 'brian'.\n")
	fmt.Printf("Hook activation requires Claude session-restart (settings.json not hot-reloaded mid-session per Phase L Finding-3).\n")
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

// runInstallToolgateHook installs the K-16 PreToolUse class-split gate
// hook into the trio agent's Claude settings.json. Idempotent +
// non-clobbering, mirroring runInstallTrioHook's pattern.
//
// Usage:
//
//	bot-hq install-toolgate-hook            # writes ~/.claude/settings.json
//	bot-hq install-toolgate-hook <path>     # writes a custom path
func runInstallToolgateHook() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(1)
		}
		settingsPath = filepath.Join(home, ".claude", "settings.json")
	}
	botHQPath, err := os.Executable()
	if err != nil || botHQPath == "" {
		fmt.Fprintf(os.Stderr, "resolve bot-hq binary path: %v\n", err)
		os.Exit(1)
	}
	if err := toolgate.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(os.Stderr, "install: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Toolgate PreToolUse-Bash hook installed in %s\n", settingsPath)
	fmt.Printf("Hook command: %s\n", toolgate.SettingsHookCommand(botHQPath))
	fmt.Printf("Gates active per BOT_HQ_AGENT_ID:\n")
	fmt.Printf("  rain → K-16 class-split (HANDS-execute blocked) + K-13 R12 commit-gate\n")
	fmt.Printf("  brian (or non-rain trio member) → L-5 R33 pre-commit + pre-push + pre-merge gate-CHECK\n")
	fmt.Printf("Hook activation requires Claude session-restart (settings.json not hot-reloaded mid-session).\n")
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
