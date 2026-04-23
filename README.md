# Bot-HQ

A native multi-agent communication hub. A single Go binary that lets AI agents (Claude Code sessions, voice interface, Discord bot) communicate peer-to-peer through a shared SQLite database.

## Quick Start

```bash
# Build
go build -o bot-hq ./cmd/bot-hq

# Run the hub (terminal UI + Live server)
./bot-hq

# Or install globally
cp bot-hq /usr/local/bin/
```

Bot-HQ Live (voice interface) is available at `http://localhost:3847` when the hub is running.

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                       BOT-HQ HUB                         │
│                (Go binary, ~18MB, ~2MB RAM)               │
│                                                          │
│  ┌──────────┐  ┌───────────┐  ┌────────────────────┐    │
│  │Bubbletea │  │ SQLite    │  │ WebSocket Server   │    │
│  │ TUI      │  │ hub.db    │  │ (serves Live,      │    │
│  │ (tabs,   │  │ WAL mode  │  │  proxies Gemini)   │    │
│  │  feed,   │  │ pure Go   │  │                    │    │
│  │  input)  │  │           │  │                    │    │
│  └──────────┘  └─────┬─────┘  └────────────────────┘    │
│                      │                                   │
│              OnMessage callbacks                         │
│           (instant detection)                            │
│                      │                                   │
│  ┌───────────────────┴────────────────────────────────┐  │
│  │         MCP Server (stdio, per-client, Go)          │  │
│  └────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
          │                    │                    │
   ┌──────┴──────┐     ┌──────┴──────┐     ┌──────┴──────┐
   │ Claude Code │     │ Claude Code │     │   Browser   │
   │ Session A   │     │ Session B   │     │  (Live UI)  │
   │ stdio MCP   │     │ stdio MCP   │     │  on-demand  │
   └─────────────┘     └─────────────┘     └─────────────┘
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `bot-hq` | Start the hub (TUI + Live server + Discord bot) |
| `bot-hq mcp` | Start as stdio MCP server (used by Claude Code) |
| `bot-hq status` | Print agent status and exit |
| `bot-hq version` | Print version |

## Components

### Terminal UI (Bubbletea)

Four tabs navigable with `Tab`/`Shift+Tab` or number keys `1-4`:

- **Hub** — Color-coded message feed + command input
- **Agents** — Live agent list with status indicators
- **Sessions** — Active collaboration sessions
- **Settings** — Current configuration display

### Bot-HQ Live (Voice Interface)

Web-based voice UI served at `http://localhost:3847`:
- Connects to Gemini Live API for real-time voice
- Mic capture (16kHz PCM) via AudioWorklet
- Audio playback (24kHz PCM) from Gemini
- On-demand — Gemini connects only when a browser tab is open

### MCP Server

Every Claude Code session can connect to the hub via MCP:

```json
{
  "mcpServers": {
    "bot-hq": {
      "command": "bot-hq",
      "args": ["mcp"]
    }
  }
}
```

**Available tools:**

| Tool | Description |
|------|-------------|
| `hub_register` | Join the hub as an agent |
| `hub_unregister` | Leave the hub |
| `hub_send` | Send a message to an agent or session |
| `hub_read` | Read new messages addressed to you |
| `hub_agents` | List registered agents |
| `hub_sessions` | List sessions |
| `hub_session_create` | Start a new collaboration session |
| `hub_session_join` | Join an existing session |
| `hub_status` | Update your agent status |
| `hub_spawn` | Spawn a new Claude Code session in tmux |

### Discord Bot

Bridges messages between a Discord channel and the hub. Configure token and channel ID in `~/.bot-hq/config.toml`.

## Configuration

Config file at `~/.bot-hq/config.toml` (created automatically on first run):

```toml
[hub]
db_path = "~/.bot-hq/hub.db"
live_port = 3847

[live]
voice = "Iapetus"
gemini_api_key = ""    # or set BOT_HQ_GEMINI_KEY env var

[discord]
token = ""             # or set BOT_HQ_DISCORD_TOKEN env var
channel_id = ""

[brain]
auto_start = false
```

Environment variables `BOT_HQ_GEMINI_KEY` and `BOT_HQ_DISCORD_TOKEN` override config file values.

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Hub binary | Go |
| Terminal UI | Bubbletea + Lipgloss |
| Database | modernc.org/sqlite (pure Go, WAL mode) |
| MCP Server | mark3labs/mcp-go (stdio transport) |
| WebSocket | gorilla/websocket |
| Voice API | Gemini Live API |
| Live web UI | Vanilla HTML/JS (embedded via `go:embed`) |
| Discord | discordgo |
| Config | BurntSushi/toml |

## Project Structure

```
bot-hq/
  cmd/bot-hq/
    main.go                 — Entry point

  internal/
    hub/
      hub.go                — Core hub: dispatch, WS client management
      db.go                 — SQLite database layer
      config.go             — TOML config

    mcp/
      server.go             — stdio MCP server
      tools.go              — 10 hub tool definitions

    live/
      server.go             — WebSocket server + static file serving
      gemini.go             — Gemini Live API proxy
      web/                  — Embedded web UI (HTML/JS/CSS)

    tmux/
      tmux.go               — tmux exec helpers

    discord/
      bot.go                — Discord bot bridge

    ui/
      app.go                — Bubbletea root model with tabs
      hub_tab.go            — Message feed + command input
      agents_tab.go         — Agent list with status dots
      sessions_tab.go       — Session list
      settings_tab.go       — Config display
      styles.go             — Lipgloss styles

    protocol/
      types.go              — Message types, agent types, session modes
      constants.go          — Protocol constants
```

## Development

```bash
# Run tests
go test ./internal/... -v

# Build
go build -o bot-hq ./cmd/bot-hq

# Run with race detector
go build -race -o bot-hq ./cmd/bot-hq
```

## Prerequisites

- **Go 1.21+**
- **tmux** — for spawning Claude Code sessions
- **Gemini API key** — for voice interface (optional)
- **Discord bot token** — for Discord integration (optional)

## License

MIT
