# Bot-HQ

A control center for managing Claude Code agents. Monitor tasks, approve dangerous commands, and manage your AI coding assistants from any device.

## Features

- **Task Management**: Sync GitHub issues as tasks, assign to agents, track progress
- **Agent Control**: Start/stop Claude Code agents, monitor their work in real-time
- **Approval System**: Approve or reject dangerous commands (git push, npm publish) before execution
- **Multi-Device Access**: Access from phone/tablet via Tailscale with device authorization
- **Claude Code Settings**: View and manage global Claude Code configuration, plugins, skills, MCP servers
- **Real-time Logs**: Stream agent activity and errors live

## Prerequisites

- Node.js 18+
- Claude Code CLI installed and authenticated (`claude` command available)
- GitHub CLI authenticated (`gh auth login`) for issue sync
- Tailscale (optional, for remote device access)

## Quick Start

### 1. Clone and Install

```bash
git clone https://github.com/yourusername/bot-hq.git
cd bot-hq
npm install
```

### 2. Configure Environment

Create `.env.local`:

```env
PORT=7890
GITHUB_TOKEN=your_github_token  # Optional: for private repos
```

### 3. Initialize Database

```bash
npm run db:push
```

### 4. Start the Server

```bash
npm run dev
```

Open http://localhost:7890 (or your configured port)

## Setup Workspaces

1. Go to **Settings → Workspaces**
2. Click **Add Workspace**
3. Enter:
   - Name: e.g., "my-project"
   - Path: `/path/to/your/repo`
   - GitHub Remote: `owner/repo` (for issue sync)

## Agent Configuration

Each workspace can have custom agent rules:

- **Approval Rules**: Commands requiring human approval (e.g., `git push`, `npm publish`)
- **Blocked Commands**: Commands the agent cannot run (e.g., `rm -rf /`)

### Approval Hook

The approval hook intercepts dangerous commands. It's automatically configured at `.claude/settings.json` in your workspace:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "node \"/path/to/bot-hq/.claude/hooks/approval-gate.js\""
          }
        ]
      }
    ]
  }
}
```

## Remote Access (Tailscale)

Access Bot-HQ from your phone or other devices:

### 1. Install Tailscale

- Mac: Download from [App Store](https://apps.apple.com/app/tailscale/id1475387142) or `brew install tailscale`
- Phone: Install from App Store / Play Store

### 2. Start Server for Remote Access

```bash
npm run dev -- -H 0.0.0.0
```

### 3. Get Tailscale IP

```bash
tailscale ip -4
```

### 4. Access from Phone

1. Open `http://<tailscale-ip>:7890` on your phone
2. You'll see an "Unauthorized" page with a pairing code
3. On your Mac (localhost), go to **Settings → Devices**
4. Approve the pending device
5. Phone is now authorized

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Bot-HQ UI                            │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │ Taskboard│ │ Pending  │ │   Logs   │ │ Settings │       │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      Next.js API                            │
│  /api/agents  /api/approvals  /api/sync  /api/workspaces   │
└─────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
      ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
      │   SQLite DB  │ │ Claude Code  │ │  GitHub API  │
      │  (Drizzle)   │ │    Agent     │ │  (Issues)    │
      └──────────────┘ └──────────────┘ └──────────────┘
```

## Database Schema

| Table | Purpose |
|-------|---------|
| `workspaces` | Repository configurations |
| `tasks` | GitHub issues / manual tasks |
| `approvals` | Pending command approvals |
| `logs` | Agent activity logs |
| `agent_sessions` | Running agent processes |
| `authorized_devices` | Approved remote devices |
| `pending_devices` | Devices waiting for approval |

## Scripts

```bash
npm run dev          # Start dev server
npm run build        # Production build
npm run start        # Start production server
npm run db:push      # Push schema changes to DB
npm run db:studio    # Open Drizzle Studio (DB browser)
```

## Tech Stack

- **Framework**: Next.js 15 (App Router)
- **Database**: SQLite + Drizzle ORM
- **UI**: Tailwind CSS + shadcn/ui
- **Agent**: Claude Code CLI (headless mode)
- **Auth**: Cookie-based device authorization

## Troubleshooting

### Agent not starting
- Ensure `claude` CLI is installed and authenticated
- Check the workspace path exists
- Look at Logs tab for errors

### Approvals not appearing
- Verify `.claude/settings.json` has the hook configured
- Check that `BOT_HQ_URL` or `PORT` env var matches your server

### Remote device can't connect
- Ensure Tailscale is running on both devices
- Server must be started with `-H 0.0.0.0`
- Check firewall settings

### Database errors
- Run `npm run db:push` to sync schema
- Delete `data/bot-hq.db` and re-run to reset

## License

MIT
