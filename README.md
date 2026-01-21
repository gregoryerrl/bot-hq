# Bot-HQ

A control center for managing Claude Code agents. Create tasks, monitor progress, and orchestrate AI coding assistants from a single dashboard.

## Quick Start

```bash
git clone https://github.com/gregoryerrl/bot-hq.git
cd bot-hq
npm install
npm run setup
```

Edit `.env.local` and add your Anthropic API key:
```
ANTHROPIC_API_KEY=sk-ant-api03-your-key-here
```

Start the server:
```bash
npm run local
```

Open http://localhost:7890

## What It Does

- **Task Management** - Create tasks manually or sync from GitHub issues
- **Agent Orchestration** - Start Claude Code agents to work on tasks
- **Progress Tracking** - Monitor agent work with iteration-based feedback loops
- **MCP Integration** - Control bot-hq from any Claude Code session via MCP tools

## Prerequisites

- Node.js 18+
- Claude Code CLI installed (`claude` command)
- Anthropic API key

## MCP Server

Bot-HQ includes an MCP server that lets you manage tasks from any Claude Code session.

The setup script configures this automatically. To use it globally, add to your Claude settings:

```json
{
  "mcpServers": {
    "bot-hq": {
      "command": "/path/to/bot-hq/mcp-server.sh"
    }
  }
}
```

Available MCP tools:
- `task_list` - List tasks by workspace or state
- `task_get` - Get task details including feedback
- `task_create` - Create new tasks
- `task_update` - Update task state, priority, notes
- `task_assign` - Move task to queue
- `workspace_list` - List configured workspaces
- `agent_start` / `agent_stop` - Control agents
- `status_overview` - Dashboard summary

## Setting Up Workspaces

1. Open http://localhost:7890
2. Go to **Settings > Workspaces**
3. Add a workspace with:
   - **Name**: Project identifier
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

## Scripts

```bash
npm run setup      # First-time setup
npm run local      # Start dev server
npm run mcp        # Run MCP server standalone
npm run db:push    # Sync database schema
npm run db:studio  # Open database browser
npm run build      # Production build
npm run start      # Start production server
```

## Project Structure

```
bot-hq/
├── src/
│   ├── app/           # Next.js pages and API routes
│   ├── components/    # React components
│   ├── lib/           # Database, utilities
│   └── mcp/           # MCP server and tools
├── data/              # SQLite database
├── scripts/           # Setup scripts
└── plugins/           # Optional plugins (GitHub sync)
```

## Tech Stack

- Next.js 15 (App Router)
- SQLite + Drizzle ORM
- Tailwind CSS + shadcn/ui
- MCP SDK

## Troubleshooting

**Database errors**
```bash
npm rebuild better-sqlite3
npm run db:push
```

**MCP server not connecting**
```bash
# Test MCP server directly
npm run mcp
```

**Agent not starting**
- Verify `claude` CLI is installed and authenticated
- Check workspace path exists

## License

MIT
