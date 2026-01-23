# Bot-HQ

A control center for managing Claude Code agents. Create tasks, monitor progress, and orchestrate AI coding assistants from a single dashboard.

## Quick Start

```bash
# Clone and install
git clone https://github.com/gregoryerrl/bot-hq.git
cd bot-hq
npm install

# Run interactive setup
npm run setup
```

The setup wizard will guide you through:
1. Configuring your Anthropic API key
2. Setting the server port (default: 7890)
3. Configuring the projects directory
4. Initializing the database
5. Setting up the MCP server

Then start the server:
```bash
npm run local
```

Open http://localhost:7890

## Setup Options

### Interactive Setup (Recommended)
```bash
npm run setup
```
Walks you through configuration with prompts.

### Quick Setup (Non-Interactive)
```bash
npm run setup:quick
```
Uses defaults, skips prompts. Good for CI/CD or when you'll configure `.env.local` manually.

### Custom Configuration
```bash
# Use a different port
npm run setup -- --port 8080

# Specify projects directory
npm run setup -- --scope /path/to/your/projects

# Skip MCP server configuration
npm run setup -- --skip-mcp

# Combine options
npm run setup -- -y --port 8080 --scope ~/code
```

### Setup Commands Reference

| Command | Description |
|---------|-------------|
| `npm run setup` | Interactive setup wizard |
| `npm run setup:quick` | Quick setup with defaults |
| `npm run setup:verify` | Verify installation health |
| `npm run setup:reset` | Reset database and state |
| `npm run setup -- --help` | Show all setup options |

### Environment Variables

Copy `.env.example` to `.env.local` and configure:

```bash
# Required
ANTHROPIC_API_KEY=sk-ant-api03-...

# Server (optional)
BOT_HQ_PORT=7890
BOT_HQ_URL=http://localhost:7890

# Agent scope (optional)
BOT_HQ_SCOPE=/Users/you/Projects
```

See `.env.example` for all available options.

## Prerequisites

- **Node.js 18+** (20+ recommended)
- **Claude Code CLI** (`claude` command) - for agent functionality
- **Anthropic API key** - [Get one here](https://console.anthropic.com/)

## Troubleshooting

### Doctor Command

Run diagnostics and auto-fix common issues:

```bash
# Check for issues
npm run doctor

# Attempt automatic fixes
npm run doctor:fix
```

### Common Issues

**Database errors**
```bash
npm rebuild better-sqlite3
npm run db:push
```

**node-pty errors (terminal not working)**
```bash
npm rebuild node-pty
# Or run setup which fixes permissions automatically
npm run setup:quick
```

**MCP server not connecting**
```bash
# Test MCP server directly
npm run mcp

# Regenerate MCP server script
npm run setup -- --skip-db
```

**Port already in use**
```bash
# Use a different port
npm run setup -- --port 8080
# Or edit .env.local and change BOT_HQ_PORT
```

**Agent not starting**
- Verify `claude` CLI is installed: `claude --version`
- Check workspace path exists
- Ensure ANTHROPIC_API_KEY is set in `.env.local`

## Features

- **Task Management** - Create tasks manually or sync from GitHub issues
- **Agent Orchestration** - Start Claude Code agents to work on tasks
- **Progress Tracking** - Monitor agent work with iteration-based feedback loops
- **MCP Integration** - Control bot-hq from any Claude Code session via MCP tools

## MCP Server

Bot-HQ includes an MCP server that lets you manage tasks from any Claude Code session.

The setup script configures this automatically. To use it globally, add to your Claude settings (`~/.claude/settings.json`):

```json
{
  "mcpServers": {
    "bot-hq": {
      "command": "/path/to/bot-hq/mcp-server.sh"
    }
  }
}
```

### Available MCP Tools

| Tool | Description |
|------|-------------|
| `task_list` | List tasks by workspace or state |
| `task_get` | Get task details including feedback |
| `task_create` | Create new tasks |
| `task_update` | Update task state, priority, notes |
| `task_assign` | Move task to queue |
| `workspace_list` | List configured workspaces |
| `workspace_create` | Create a new workspace |
| `agent_start` | Start agent on a task |
| `agent_stop` | Stop running agent |
| `status_overview` | Dashboard summary |

## Setting Up Workspaces

1. Open http://localhost:7890
2. Go to **Workspaces** (sidebar)
3. Click **Add Workspace** with:
   - **Name**: Project identifier (e.g., "my-app")
   - **Path**: Absolute path to the repository

## How Tasks Work

Tasks follow this lifecycle:

```
new → queued → in_progress → done
                    ↓
               needs_help
                    ↓
              (with feedback)
                    ↓
                 queued (iteration 2, 3, ...)
```

When an agent completes a task, you can:
- Mark it **done** if the work is correct
- Send **feedback** to requeue with incremented iteration count

The agent receives the feedback and iteration count to improve its work.

## All Scripts

| Script | Description |
|--------|-------------|
| `npm run setup` | Interactive first-time setup |
| `npm run setup:quick` | Non-interactive setup with defaults |
| `npm run setup:verify` | Verify installation health |
| `npm run setup:reset` | Reset database and state |
| `npm run doctor` | Diagnose common issues |
| `npm run doctor:fix` | Diagnose and auto-fix issues |
| `npm run local` | Start development server |
| `npm run dev` | Start development server (alias) |
| `npm run build` | Production build |
| `npm run start` | Start production server |
| `npm run mcp` | Run MCP server standalone |
| `npm run db:push` | Sync database schema |
| `npm run db:studio` | Open database browser |
| `npm run lint` | Run ESLint |

## Project Structure

```
bot-hq/
├── src/
│   ├── app/           # Next.js pages and API routes
│   ├── components/    # React components
│   ├── lib/           # Database, utilities, manager
│   └── mcp/           # MCP server and tools
├── scripts/           # Setup and doctor scripts
├── data/              # SQLite database
├── drizzle/           # Database migrations
└── docs/              # Documentation
```

## Tech Stack

- **Next.js 16** (App Router, React 19)
- **SQLite** + Drizzle ORM
- **Tailwind CSS** + shadcn/ui
- **MCP SDK** for Claude integration
- **node-pty** for terminal sessions

## Development

```bash
# Start dev server with hot reload
npm run local

# Open database browser
npm run db:studio

# Run linter
npm run lint
```

## License

MIT
