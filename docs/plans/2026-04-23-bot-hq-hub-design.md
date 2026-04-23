# Bot-HQ Hub — Design Document

## Overview

Bot-HQ is a multi-agent communication hub. A native Go binary that lets AI agents (Claude Code sessions, voice interface, Discord bot, Brain) communicate peer-to-peer through a shared SQLite database.

**Bot-HQ Live** is a web-based voice interface served by the hub on-demand. It connects to Gemini Live API only when a browser tab is open.

**No Electron. No Node.js runtime for the hub. Single binary.**

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                       BOT-HQ HUB                         │
│                (Go binary, ~5MB, ~2MB RAM)                │
│                                                          │
│  ┌──────────┐  ┌───────────┐  ┌────────────────────┐    │
│  │Bubbletea │  │ SQLite    │  │ WebSocket Server   │    │
│  │ TUI      │  │ hub.db    │  │ (serves Live,      │    │
│  │ (tabs,   │  │ WAL mode  │  │  proxies Gemini)   │    │
│  │  feed,   │  │ pure Go   │  │                    │    │
│  │  input)  │  │           │  │                    │    │
│  └──────────┘  └─────┬─────┘  └────────────────────┘    │
│                      │                                   │
│              update_hook                                 │
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

## Tech Stack

| Component | Technology | Why |
|-----------|-----------|-----|
| Hub binary | **Go** | Single binary, instant startup, low memory, excellent concurrency |
| Terminal UI | **Bubbletea + Lipgloss + Bubbles** | Best-in-class TUI framework, beautiful out of the box |
| Database | **modernc.org/sqlite** | Pure Go SQLite, no CGO, WAL mode, update_hook support |
| MCP Server | **mark3labs/mcp-go** | Go MCP SDK, stdio transport |
| WebSocket | **gorilla/websocket** | Battle-tested, low overhead |
| Gemini proxy | **gorilla/websocket** | Direct WebSocket proxy to Gemini Live API |
| Live web UI | **Vanilla HTML/JS** | Embedded in Go binary via `embed`, zero build step |
| Config | **TOML** | `~/.bot-hq/config.toml` |

## Components

### 1. Hub (Native Terminal App)

- **Binary**: `bot-hq` — single executable, ~5MB
- **RAM**: ~2MB idle (vs ~200MB Electron, ~30MB Node.js)
- **Startup**: <50ms (vs ~2s Electron, ~200ms Node.js)
- **Responsibilities**:
  - Hosts SQLite database at `~/.bot-hq/hub.db`
  - Watches for new messages via SQLite `update_hook` (C-level callback, instant)
  - Dispatches notifications to agents via goroutines (concurrent, non-blocking)
  - Serves Live web app on `localhost:3847` (embedded static files)
  - Forks itself as stdio MCP server for Claude Code clients
  - Renders terminal UI with Bubbletea

### 2. Bot-HQ Live (Web App)

- **Runtime**: Browser tab, static files embedded in Go binary
- **On-demand**: Gemini Live API connects only when tab opens, disconnects on close
- **Audio path**:
  ```
  Browser mic → getUserMedia → PCM → WebSocket → Hub → Gemini Live API
  Gemini audio → Hub → WebSocket → Browser → AudioContext playback
  ```
- **API key stays server-side**: Browser never sees it
- **Registers as agent** on open, unregisters on close

### 3. MCP Server (stdio, per-client)

- **Same binary**: `bot-hq mcp` — hub binary in MCP server mode
- **Transport**: stdio (fastest — direct pipe IPC, no HTTP)
- **Shares SQLite**: All instances read/write same `~/.bot-hq/hub.db`
- **Claude Code config**:
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
- **No separate install**: Same binary serves as both hub and MCP server

### 4. Brain (Optional Agent)

- **What**: A Claude Code session with orchestration instructions
- **When**: Started manually when you want Discord bot or multi-agent orchestration
- **How**: Just another Claude Code session that connects to the hub via MCP
- **Not always running**: Start when needed, stop when not

### 5. Discord Bot

- **Managed by Brain**: When Brain is active, it handles Discord messages
- **Connects to hub**: Discord messages become hub messages
- **Bot-HQ branded**: Messages appear from "Bot-HQ" bot in Discord
- **Could also be built into hub**: As a Go goroutine with discordgo library

## Database Schema

SQLite at `~/.bot-hq/hub.db`, WAL mode, busy timeout 5s.

```sql
CREATE TABLE agents (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  type        TEXT NOT NULL,        -- "coder", "voice", "brain", "discord"
  status      TEXT NOT NULL,        -- "online", "working", "idle", "offline"
  project     TEXT,                 -- current project path
  meta        TEXT,                 -- JSON blob (capabilities, tmux session, etc.)
  registered  INTEGER NOT NULL,     -- unix timestamp ms
  last_seen   INTEGER NOT NULL      -- unix timestamp ms
);

CREATE TABLE sessions (
  id          TEXT PRIMARY KEY,     -- uuid
  mode        TEXT NOT NULL,        -- "brainstorm", "implement", "chat"
  purpose     TEXT NOT NULL,        -- "fix login bug in bcc-ad-manager"
  agents      TEXT NOT NULL,        -- JSON array of agent IDs
  status      TEXT NOT NULL,        -- "active", "paused", "done"
  created     INTEGER NOT NULL,
  updated     INTEGER NOT NULL
);

CREATE TABLE messages (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id  TEXT,                 -- nullable (direct messages have no session)
  from_agent  TEXT NOT NULL,
  to_agent    TEXT,                 -- nullable (broadcast if null)
  type        TEXT NOT NULL,        -- "handshake", "question", "response",
                                   -- "command", "update", "result", "error"
  content     TEXT NOT NULL,
  created     INTEGER NOT NULL
);

CREATE INDEX idx_messages_to ON messages(to_agent, id);
CREATE INDEX idx_messages_session ON messages(session_id, id);
CREATE INDEX idx_messages_created ON messages(created);
```

## MCP Tools

| Tool | Parameters | Purpose |
|------|-----------|---------|
| `hub_register` | id, name, type, project? | Join the hub |
| `hub_unregister` | id | Leave the hub |
| `hub_send` | to?, session_id?, type, content | Send message to agent or session |
| `hub_read` | since_id?, limit? | Read new messages for me |
| `hub_agents` | status? | List agents (optionally filter by status) |
| `hub_sessions` | status? | List sessions |
| `hub_session_create` | mode, purpose, agents | Start a new session |
| `hub_session_join` | session_id | Join an existing session |
| `hub_status` | status, project? | Update my status |
| `hub_spawn` | project, prompt? | Start new Claude Code in tmux |

## Communication Speed

All communication is in-process or over unix pipes. No HTTP anywhere.

### Path 1: Claude Code → Bot-HQ Live
```
Claude Code calls hub_send (stdio pipe → Go MCP handler → SQLite write)
  ↓ ~1ms
Hub goroutine detects via update_hook (in-process channel, instant)
  ↓ 0μs
Hub pushes to Live via WebSocket (goroutine, non-blocking)
  ↓ <1ms
Gemini speaks it

Total overhead: ~1ms
```

### Path 2: Bot-HQ Live → Claude Code
```
User speaks → Gemini transcribes → WebSocket to Hub
  ↓ <1ms
Hub writes to SQLite + exec tmux send-keys (concurrent goroutines)
  ↓ ~1ms
Claude Code sees text input immediately

Total overhead: ~1ms
```

### Path 3: Claude Code ↔ Claude Code
```
Claude A calls hub_send → stdio → SQLite write
  ↓ ~1ms
Hub goroutine detects via update_hook
Hub checks target's tmux status:
  - At prompt? → exec tmux send-keys (immediate)
  - Busy? → Queued in SQLite, delivered on next hub_read
  ↓ ~1ms

Total overhead: ~2ms when idle
```

### Path 4: Gemini Live API Connection
```
Browser tab opens  → Hub dials Gemini WebSocket
Browser tab closes → Hub closes Gemini WebSocket

No connection when not in use. Zero resources.
```

### Why Go is faster here:
- **Goroutines**: Each agent connection is a goroutine (~4KB stack vs ~1MB thread)
- **No GC pressure**: SQLite is pure Go, no CGO boundary crossing
- **Channel-based dispatch**: update_hook → Go channel → goroutine fan-out. Lock-free.
- **Single binary**: No runtime startup, no module resolution, no JIT warmup

## Voice Collaboration Scenario

```
1. User (voice): "Let's brainstorm the login fix"
2. Bot-HQ Live → hub_send → Hub spawns Claude Code in tmux
3. Claude Code receives /bot-hq skill, enters hub mode
4. Claude Code → hub_send(type: "question", "What auth method?")
5. Hub → WebSocket → Live speaks: "It's asking what auth method"
6. User: "JWT with refresh tokens"
7. Live → Hub → types into Claude Code's tmux pane
   (Bot-HQ types as it speaks — user sees it on screen)
8. Claude Code continues brainstorming via hub_send...
9. Claude Code → hub_send(type: "status", "implementing")
10. Live speaks: "Claude's starting implementation" → goes passive
11. User watches Claude Code work in tmux directly
12. User: "Wait, cancel that"
13. Live → Hub → sends Ctrl+C to tmux
14. Claude Code stops
```

## Terminal UI (Bubbletea)

### Hub Tab (default)
```
┌─ Hub ─── Agents ─── Sessions ─── Settings ───────────────┐
│                                                            │
│  [09:41:02] + claude-abc joined (bcc-ad-manager)           │  green
│  [09:41:03] + live joined                                  │  green
│  [09:41:05] live → claude-abc HANDSHAKE brainstorm         │  blue
│  [09:41:08] claude-abc → live: What auth method?           │  purple
│  [09:41:15] live → claude-abc: JWT with refresh tokens     │  blue
│  [09:41:20] claude-abc STATUS working                      │  gray
│  [09:42:30] claude-abc → live: DONE fixed auth.ts          │  purple
│  [09:43:01] claude-xyz → claude-abc: Check this API?       │  purple
│  [09:43:05] ERROR claude-def disconnected unexpectedly     │  red
│                                                            │
│  ▌                                                         │
└────────────────────────────────────────────────────────────┘
```

**Color coding:**
- Green: system events (join, leave)
- Blue: Bot-HQ Live
- Purple: Claude Code sessions (shades per session — purple, violet, magenta)
- Orange: Discord / Brain
- Red: errors, stops, cancellations
- Gray: status updates
- Yellow: handshakes, session mode changes

**Command input** at bottom:
- `@claude-abc stop` → sends command to agent
- `@brain start discord` → tells brain to start
- `spawn bcc-ad-manager` → starts new Claude Code session in tmux

### Agents Tab
```
● claude-abc    working    bcc-ad-manager     2m ago
● live          listening                     now
● brain         idle       discord            5m ago
○ claude-def    offline    bot-hq             12m ago

[3 online, 1 offline]
```

### Sessions Tab
```
#7f3a  brainstorm  live ↔ claude-abc   "login fix"    active
#b2e1  implement   claude-xyz          "refactor"     active
#9c4d  chat        discord ↔ brain     "general"      paused

[2 active, 1 paused]
```

Click/select a session to filter Hub tab to only that session's messages.

### Settings Tab
```
HUB
  Database      ~/.bot-hq/hub.db
  Live Port     3847

BOT-HQ LIVE
  Voice         Iapetus
  Microphone    [System Default]

DISCORD
  Token         ●●●●●●●●
  Channel       #bot (Gregory's server)

BRAIN
  Auto-start    [ ]
```

## Project Structure

```
bot-hq/
  cmd/
    bot-hq/
      main.go               — Entry point: `bot-hq` starts hub, `bot-hq mcp` starts MCP server

  internal/
    hub/
      hub.go                — Core hub: SQLite, update_hook, agent dispatch
      db.go                 — Database schema, migrations, queries
      config.go             — TOML config at ~/.bot-hq/config.toml

    mcp/
      server.go             — stdio MCP server (hub_register, hub_send, etc.)
      tools.go              — Tool definitions and handlers

    live/
      proxy.go              — WebSocket server: browser ↔ hub ↔ Gemini
      gemini.go             — Gemini Live API WebSocket client

    tmux/
      tmux.go               — tmux exec helpers (send-keys, capture-pane, etc.)

    discord/
      bot.go                — Discord bot (optional, using discordgo)

    ui/
      app.go                — Bubbletea root model: tabs, layout, key handling
      hub_tab.go            — Message feed + command input
      agents_tab.go         — Agent list with status dots
      sessions_tab.go       — Session list
      settings_tab.go       — Config editor
      styles.go             — Lipgloss styles, color constants

    protocol/
      types.go              — Message types, agent types, session modes
      constants.go          — Protocol constants

  web/
    live/
      index.html            — Bot-HQ Live web UI (embedded via go:embed)
      app.js                — Audio capture, WebSocket, playback
      style.css             — Voice UI styling

  go.mod
  go.sum
```

## CLI Commands

```
bot-hq              — Start the hub (terminal UI + Live server + MCP listener)
bot-hq mcp          — Start as stdio MCP server (used by Claude Code)
bot-hq status       — Print hub status (agents, sessions) and exit
bot-hq version      — Print version
```

## Config

`~/.bot-hq/config.toml`:

```toml
[hub]
db_path = "~/.bot-hq/hub.db"
live_port = 3847

[live]
voice = "Iapetus"
gemini_api_key = "..."    # or read from env BOT_HQ_GEMINI_KEY

[discord]
token = "..."              # or read from env BOT_HQ_DISCORD_TOKEN
channel_id = "1496436363761946796"

[brain]
auto_start = false
```

## /bot-hq Skill (Global Claude Code Skill)

Installed at `~/.claude/skills/bot-hq/` so every Claude Code session has it.

When invoked, tells Claude Code:
1. You are in a hub session — communicate via `hub_send`, not terminal output
2. In brainstorm mode: send every question/thought through the hub
3. In implement mode: work silently, notify on completion
4. Respect `stop` commands immediately
5. Keep messages concise — they may be spoken aloud

## Build & Install

```bash
cd bot-hq
go build -o bot-hq ./cmd/bot-hq
mv bot-hq /usr/local/bin/   # or ~/go/bin/

# Claude Code config (global)
# ~/.claude/settings.json → mcpServers → "bot-hq": { "command": "bot-hq", "args": ["mcp"] }
```

## Migration Path

The existing Electron/TypeScript codebase (`electron/`, `src/`) remains as-is for reference and fallback. The Go hub is a new build target in the same repo. Once the Go hub is stable, the Electron code can be archived.
