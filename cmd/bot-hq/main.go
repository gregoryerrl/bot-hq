package main

import (
	"fmt"
	"io"
	"log"
	"os"
	"path/filepath"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/gregoryerrl/bot-hq/internal/brian"
	"github.com/gregoryerrl/bot-hq/internal/discord"
	"github.com/gregoryerrl/bot-hq/internal/gemma"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/live"
	"github.com/gregoryerrl/bot-hq/internal/mcp"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/rain"
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
		case "version":
			// Ensure config directory and default config exist
			home, _ := os.UserHomeDir()
			hub.LoadConfig(filepath.Join(home, ".bot-hq", "config.toml"))
			fmt.Printf("bot-hq v%s\n", protocol.Version)
			return
		default:
			fmt.Fprintf(os.Stderr, "unknown command: %s\nUsage: bot-hq [mcp|status|version]\n", os.Args[1])
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
