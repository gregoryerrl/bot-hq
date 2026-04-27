package main

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"os"
	"path/filepath"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/gregoryerrl/bot-hq/internal/brian"
	"github.com/gregoryerrl/bot-hq/internal/discord"
	"github.com/gregoryerrl/bot-hq/internal/gemma"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/live"
	"github.com/gregoryerrl/bot-hq/internal/mcp"
	"github.com/gregoryerrl/bot-hq/internal/outboundhook"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/rain"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/gregoryerrl/bot-hq/internal/ui"
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
		case "version":
			// Ensure config directory and default config exist
			home, _ := os.UserHomeDir()
			hub.LoadConfig(filepath.Join(home, ".bot-hq", "config.toml"))
			fmt.Printf("bot-hq v%s\n", protocol.Version)
			return
		default:
			fmt.Fprintf(os.Stderr, "unknown command: %s\nUsage: bot-hq [mcp|status|audit-pane-drift|outbound-miss-hook|install-trio-hook|version]\n", os.Args[1])
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

	// 4. Redirect log output to file — TUI owns the terminal
	logFile, logErr := os.OpenFile(filepath.Join(home, ".bot-hq", "live.log"), os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0600)
	if logErr == nil {
		log.SetOutput(logFile)
		defer logFile.Close()
	} else {
		log.SetOutput(io.Discard)
	}

	// 5. Start Live web server
	liveServer := live.NewServer(h, cfg.Hub.LivePort)
	liveServer.Start()

	// 6. Start Discord bot if configured
	if cfg.Discord.Token != "" && cfg.Discord.ChannelID != "" {
		discordBot, err := discord.NewBot(cfg.Discord.Token, cfg.Discord.ChannelID, h)
		if err == nil {
			if err := discordBot.Start(); err == nil {
				defer discordBot.Stop()
			}
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
// three-clause filter, and emits an OUTBOUND-MISS alert via the hub
// when the agent produced pane text without a hub_send tool call.
// All errors are silenced — a Stop hook must never block agent exit.
func runOutboundMissHook() {
	outboundhook.RunHook(os.Stdin)
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
	fmt.Printf("Reminder: export BOT_HQ_AGENT_ID=<id> in the agent's tmux pane env so the hook knows the agent identity.\n")
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
