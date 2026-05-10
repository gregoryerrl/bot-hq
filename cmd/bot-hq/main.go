package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/gregoryerrl/bot-hq/internal/brian"
	"github.com/gregoryerrl/bot-hq/internal/daemoncron"
	"github.com/gregoryerrl/bot-hq/internal/discord"
	"github.com/gregoryerrl/bot-hq/internal/gemma"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/rain"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/gregoryerrl/bot-hq/internal/ui"
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
		case "context-load":
			runContextLoad()
			return
		case "session-lookback":
			runSessionLookback()
			return
		case "session-summary":
			runSessionSummary()
			return
		case "session-migrate-stale":
			runSessionMigrateStale()
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
			fmt.Fprintf(os.Stderr, "unknown command: %s\nUsage: bot-hq [mcp|status|audit-pane-drift|audit-rules-canonical|outbound-miss-hook|install-trio-hook|tool-permission-hook|install-toolgate-hook|preflight-check|voice-mirror-hook|install-voice-mirror-hook|session-load|session-prune|session-search|webui|context-switch|context-load|session-open|install-session-start-hook|emit-compact-notice|emit-resume|version]\n", os.Args[1])
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

	// Phase-S-followup: daemoncron — daemon-side cadence-driven hub
	// emits. Owns heartbeat-ledger, stale-coder, plan-usage / context-cap /
	// delivery-gap / egress-audit / lifecycle / ollama emit fires.
	// Gemma drives the detection loops + delegates emits unconditionally.
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

	// 5c. Phase V architecture: bootstrap timer loop removed. The CL is the
	// durable state; agents read it on-demand via the context_load tool
	// (internal/contextload) when pivoting to a project. Per-agent state
	// (last_state.json) is event-written by the agents themselves at
	// scope-affecting boundaries (commits, halts, phase transitions),
	// not on a timer. The bootstrap.md file class is now absent-by-default;
	// sessionopen handler degrades gracefully when missing (BootstrapView
	// omitted from payload). Removed: runBootstrapDefensiveLoop +
	// BOT_HQ_BOOTSTRAP_DISABLE env override.

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
			defer emmaAgent.Stop()
		}
	}

	// 10. Run Bubbletea TUI
	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "TUI error: %v\n", err)
		os.Exit(1)
	}
}
