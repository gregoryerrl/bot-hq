# Bot-HQ Design Document

**Date:** 2026-01-06
**Status:** Approved
**Author:** Gregory Errl + Claude

---

## Overview

Bot-HQ is a local-first workflow automation system that uses AI agents to automate the GitHub issue → PR workflow. It runs on your Mac, accessible from any device via Tailscale.

### Problem Statement

Current workflow with Claude Code is manual and blocking:
1. Check GitHub issues manually across multiple repos
2. Create branches manually
3. Open Claude Code, explain context
4. Monitor Claude Code until done
5. Create PRs manually
6. Repeat for each repo (context switching)

### Solution

A dashboard that:
- Fetches issues from all repos automatically
- Lets you assign issues to AI agents with one click
- Agents work in parallel, autonomously
- You batch-review results when convenient
- Approval required for external actions (git push, deploys)

---

## Architecture

### System Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      Your Devices                           │
│         (Mac, Phone, Tablet via Tailscale)                  │
└─────────────────────────┬───────────────────────────────────┘
                          │ HTTPS (local network)
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                     Bot-HQ Server                           │
│                  (Next.js on your Mac)                      │
│                                                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Web UI      │  │ API Layer   │  │ Agent Orchestrator  │  │
│  │ - Taskboard │  │ - GitHub    │  │ - Manager Agent     │  │
│  │ - Pending   │  │ - Anthropic │  │ - Repo Agents       │  │
│  │ - Logs      │  │ - SQLite    │  │ - Claude Code proc  │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                          │
          ┌───────────────┼───────────────┐
          ▼               ▼               ▼
    ~/Projects/     ~/Projects/     ~/Projects/
    nokona-config   helena-theme    nokona-theme
```

### Agent Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Manager Agent                           │
│                    (Claude Haiku API)                       │
│                                                             │
│  Responsibilities:                                          │
│  • Fetch issues from GitHub for all repos                   │
│  • Parse issue priority/complexity                          │
│  • Monitor repo agent health (stuck? errored?)              │
│  • Compact/clear agent context when needed                  │
│  • Cross-repo awareness                                     │
│  • Generate summaries on request                            │
│                                                             │
│  Does NOT: Write code, make commits, touch files            │
└─────────────────────────────────────────────────────────────┘
                          │ Delegates
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                     Repo Agents                             │
│              (Claude Code subprocess per repo)              │
│                                                             │
│  Each agent can:                                            │
│  • Read/write files in its workspace                        │
│  • Run tests, builds, linters                               │
│  • Create branches, commits                                 │
│  • Create draft PRs                                         │
│                                                             │
│  Needs approval for:                                        │
│  • git push                                                 │
│  • deploy commands                                          │
│  • external API writes                                      │
└─────────────────────────────────────────────────────────────┘
```

### Workspace System

Workspaces allow repos to access linked directories with defined permissions:

```yaml
workspaces:
  - name: "helena-theme"
    repo: "~/Projects/Sites/helena-theme"
    linked:
      - path: "~/Projects/Sites/helena-wordpress"
        access: "read"
      - path: "~/Projects/Sites/helena-wordpress/wp-content/themes/helena"
        access: "write"
    build_command: "npm run build"

  - name: "nokona-configurator"
    repo: "~/Projects/nokona-configurator"
    linked: []
```

**Access levels:**
- `repo` (primary): Full read/write/git access
- `linked.read`: Read-only context access
- `linked.write`: Write access (for build outputs)

---

## Core Workflow

### Issue → PR Cycle

```
1. ISSUE SYNC (automatic, every 15 min)
   Manager fetches open issues from GitHub → stores in SQLite → appears on Taskboard

2. YOU ASSIGN (manual, in Taskboard)
   Click issue → "Assign to Agent" → issue queued

3. AGENT ANALYZES (automatic)
   Repo agent reads issue + codebase → posts plan → waits for approval

4. YOU APPROVE PLAN (manual, in Taskboard)
   Review approach → Approve/Reject → agent starts coding

5. AGENT WORKS (automatic, Claude Code subprocess)
   Creates linked branch (gh issue develop) → implements → runs tests → commits

6. PUSH APPROVAL (in Pending Board)
   Agent requests push → you see diff summary → Approve/Reject

7. PR CREATED (automatic after approval)
   Agent pushes → creates draft PR → links to issue → done
```

### Your Touchpoints

1. **Assign issues** - batch, takes seconds
2. **Approve plans** - quick scan, optional for simple ones
3. **Approve pushes + review PRs** - batch review when convenient

---

## UI Components

### Taskboard (Main View)

- Issues grouped by repo, collapsible
- State badges: New, Queued, Analyzing, Plan Ready, In Progress, PR Draft, Done
- Inline actions: Assign, Approve Plan, View PR
- Filter by repo, state, priority
- Sort by priority, date, repo

### Pending Board

- Queue of actions needing approval
- Categories: git push, external command, deploy
- Shows: diff summary, files changed, test results
- Actions: Approve, Reject, View Full Diff, Ask Agent Why

### Logboard

- Real-time activity stream (SSE)
- Filterable by repo, log type
- Searchable
- Log types: agent, test, sync, approval, error, health

### Chat Panel (Manager)

- Collapsible side panel (not primary interface)
- For custom requests only:
  - "Summarize today's work"
  - "Generate learning doc from PR #42"
  - "Assign all high-priority issues"

---

## Data Model

```sql
-- Workspaces (repos + linked directories)
workspaces
├── id
├── name
├── repo_path
├── github_remote
├── linked_dirs             -- JSON
├── build_command
└── created_at

-- Tasks (issues + manual tasks)
tasks
├── id
├── workspace_id
├── github_issue_number
├── title
├── description
├── state                   -- new|queued|analyzing|plan_ready|in_progress|pr_draft|done
├── priority
├── agent_plan
├── branch_name
├── pr_url
├── assigned_at
└── updated_at

-- Pending approvals
approvals
├── id
├── task_id
├── type                    -- git_push|external_command|deploy
├── command
├── reason
├── diff_summary
├── status                  -- pending|approved|rejected
├── created_at
└── resolved_at

-- Logs
logs
├── id
├── workspace_id
├── task_id
├── type
├── message
├── details                 -- JSON
└── created_at

-- Agent sessions
agent_sessions
├── id
├── workspace_id
├── task_id
├── pid
├── status                  -- running|idle|stopped|error
├── context_size
├── started_at
└── last_activity_at

-- Authorized devices
authorized_devices
├── id
├── device_name
├── device_fingerprint
├── token_hash
├── authorized_at
├── last_seen_at
└── is_revoked
```

---

## Security

### Layer 1: Network (Tailscale)

- Bot-HQ not exposed to public internet
- Only devices in your Tailscale network can access
- Encrypted traffic between devices

### Layer 2: Device Authorization

- First-time devices must pair via 6-digit code
- Code displayed on Mac, entered on new device
- Subsequent visits use stored token
- Devices can be revoked from settings

### Agent Permissions

- Agents can only access their workspace (repo + linked dirs)
- External actions require approval
- All actions logged to logboard

---

## Tech Stack

| Component | Technology |
|-----------|------------|
| Framework | Next.js 14+ (App Router) |
| Database | SQLite |
| ORM | Drizzle |
| UI | shadcn/ui + Tailwind |
| AI (Manager) | Anthropic API (Haiku) |
| AI (Coding) | Claude Code subprocess |
| Real-time | Server-Sent Events |
| Auth | Token-based + device fingerprint |
| Network | Tailscale |

---

## Project Structure

```
~/Projects/bot-hq/
├── src/
│   ├── app/
│   │   ├── page.tsx              # Taskboard
│   │   ├── pending/page.tsx
│   │   ├── logs/page.tsx
│   │   ├── settings/
│   │   └── api/
│   ├── components/
│   │   ├── ui/
│   │   ├── taskboard/
│   │   ├── pending-board/
│   │   ├── log-viewer/
│   │   └── chat-panel/
│   ├── lib/
│   │   ├── db/
│   │   ├── agents/
│   │   ├── github/
│   │   └── auth/
│   └── hooks/
├── data/
│   └── bot-hq.db
└── scripts/
```

---

## Cost Estimate

| Item | Cost |
|------|------|
| Claude Code Max subscription | $100/month (existing) |
| Manager + light API tasks | ~$30-60/month |
| Tailscale | Free |
| Hosting | $0 (local) |
| **Total** | **~$130-160/month** |

---

## Build Phases

### Phase 1: Foundation
- Project setup (Next.js, SQLite, Tailscale)
- Workspace management (add/configure repos)
- Device authorization
- Basic UI shell

### Phase 2: Core Workflow
- GitHub issue sync
- Taskboard with states
- Claude Code subprocess integration
- Approval system + pending board

### Phase 3: Polish
- Logboard with real-time streaming
- Manager agent (chat + orchestration)
- Notifications
- Mobile-friendly UI

---

## Self-Update Capability

Bot-HQ can be configured as one of its own workspaces, allowing agents to work on Bot-HQ itself (dogfooding).

### How It Works

```
Bot-HQ (running on main branch)
  └── Repo Agent: bot-hq
       └── Working on feature branch
       └── Modifying: ~/Projects/bot-hq/src/...
       └── Cannot affect running instance
```

### Safeguards

| Risk | Safeguard |
|------|-----------|
| Agent breaks Bot-HQ mid-run | Work on feature branches only. Running instance uses main. |
| Recursive self-modification | Agent sessions are isolated processes - changes don't affect running instance |
| Bad deploy bricks Bot-HQ | Approval required for restart/deploy commands |
| Agent corrupts database | DB directory explicitly blocked from agent access |

### Workspace Configuration

```yaml
workspaces:
  - name: "bot-hq"
    repo: "~/Projects/bot-hq"
    linked:
      - path: "~/Projects/bot-hq/data"
        access: "none"  # Block DB access
```

### Update Workflow

1. Agent works on bot-hq issue (feature branch)
2. Agent creates draft PR
3. You review and merge to main
4. You manually restart Bot-HQ to pick up changes

**Key principle:** Changes only apply after manual restart. The running instance remains stable.

---

## Future Considerations (v2+)

- **Gemini Flash for manager** - Reduce API costs further
- **Roadmap view** - High-level planning across repos
- **Auto-approve rules** - Skip approval for low-risk operations
- **Webhooks** - Real-time GitHub sync instead of polling
- **Multiple users** - Team support (if needed)

---

## Decisions Log

| Decision | Rationale |
|----------|-----------|
| Local-first over cloud | Zero hosting cost, data stays local, Mac must be on anyway |
| Tailscale over Cloudflare Tunnel | Built-in device pairing, private network |
| Claude Code over raw API | Reuse subscription, battle-tested agentic loop |
| SQLite over Postgres | Zero setup, single file, sufficient for scale |
| Semi-automated GitHub | User stays in control of what gets worked on |
| Draft PRs only | User reviews before senior sees |
| Workspace model | Handles complex directory structures (WordPress, monorepos) |
