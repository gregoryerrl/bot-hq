# Bot-HQ Documentation

> Complete reference documentation for Bot-HQ - a task orchestration and AI agent management system.

---

## Table of Contents

1. [Project Overview](#1-project-overview)
2. [Architecture](#2-architecture)
3. [Tech Stack](#3-tech-stack)
4. [Infrastructure](#4-infrastructure)
5. [Complete File Structure](#5-complete-file-structure)
6. [Installation & Setup](#6-installation--setup)
7. [Database Schema](#7-database-schema)
8. [MCP Server & Tools](#8-mcp-server--tools)
9. [Core Modules](#9-core-modules)
10. [Web Interface](#10-web-interface)
11. [API Reference](#11-api-reference)
12. [Terminal & PTY Management](#12-terminal--pty-management)
13. [Context Management](#13-context-management)
14. [Authentication](#14-authentication)
15. [Workflow & Task States](#15-workflow--task-states)
16. [Configuration Reference](#16-configuration-reference)
17. [Troubleshooting](#17-troubleshooting)

---

## 1. Project Overview

### What is Bot-HQ?

Bot-HQ is a task orchestration and AI agent management system that coordinates Claude Code agents to work on software development tasks autonomously. It serves as a control center where users:

- Create and manage development tasks
- Configure workspaces for multiple repositories
- Oversee agent execution through a web dashboard
- Integrate with GitHub for issue synchronization

### Key Capabilities

| Capability | Description |
|------------|-------------|
| **Task Management** | Create, assign, track, and review development tasks |
| **Agent Orchestration** | Coordinate Claude Code agents working on tasks |
| **Workspace Management** | Manage multiple repositories with custom configurations |
| **Git Integration** | Sync tasks from GitHub issues, track feature branches |
| **Persistent Sessions** | Terminal sessions persist across page refreshes and devices |
| **MCP Integration** | Model Context Protocol server for Claude Code communication |

### Design Philosophy

Bot-HQ implements a **hybrid autonomy model**:

| Mode | Description | When It Happens |
|------|-------------|-----------------|
| **AUTONOMOUS** | System works independently | Task execution, code generation, commits, verification |
| **HITL (Human-in-the-Loop)** | Human decision required | Task creation, assignment, brainstorming questions, final review |

**Key Principle**: Maximize autonomous work while ensuring humans retain control at critical decision points.

---

## 2. Architecture

### High-Level Architecture

```
┌─────────────────────────────────────────┐
│         Bot-HQ Web Dashboard             │
│    (Next.js UI + Task Management)        │
└────────────────┬────────────────────────┘
                 │ HTTP/SSE
                 ▼
┌─────────────────────────────────────────┐
│      Next.js API Routes (Port 7890)      │
├─────────────────────────────────────────┤
│  SQLite (WAL)    │    PTY Manager        │
│  via Drizzle ORM │    via node-pty       │
└────────────────┬─┴──────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│      MCP Server (stdio transport)        │
│  Exposes tools to Claude Code            │
└────────────────┬────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│   Persistent Manager (Claude Code)       │
│   - PTY-based terminal session           │
│   - Monitors task queue                  │
│   - Receives commands via MCP            │
└─────────────────────────────────────────┘
```

### Manager + Subagent Model

Bot-HQ uses a hierarchical agent architecture:

1. **Manager**: A persistent Claude Code session that orchestrates all work
2. **Subagents**: Individual Claude Code sessions spawned per task

The manager runs with `--dangerously-skip-permissions` to allow autonomous operation.

### Data Flow

```
User Request → Web UI → API Route → Database
                                  ↓
                           PTY Manager
                                  ↓
                           Manager Session
                                  ↓
                           MCP Tools → Subagent
```

---

## 3. Tech Stack

### Frontend

| Technology | Version | Purpose |
|------------|---------|---------|
| Next.js | 16.1.1 | React framework with App Router |
| React | 19.2.3 | UI library |
| TypeScript | 5.x | Type safety |
| Tailwind CSS | 4.x | Styling |
| shadcn/ui | - | Component library (Radix UI based) |
| xterm.js | 6.0.0 | Terminal emulator |
| lucide-react | 0.562.0 | Icons |
| sonner | 2.0.7 | Toast notifications |

### Backend

| Technology | Version | Purpose |
|------------|---------|---------|
| Node.js | 18+ (20+ recommended) | Runtime |
| SQLite | - | Database |
| Drizzle ORM | 0.45.1 | Database ORM |
| better-sqlite3 | 12.6.2 | SQLite driver |
| node-pty | 1.1.0 | PTY (pseudo-terminal) management |

### AI & Integration

| Technology | Version | Purpose |
|------------|---------|---------|
| @modelcontextprotocol/sdk | 1.25.2 | MCP server/client |
| @anthropic-ai/sdk | 0.71.2 | Anthropic API (direct calls) |

---

## 4. Infrastructure

### System Components

```
┌─────────────────────────────────────────────────────────────────────┐
│                         BOT-HQ INFRASTRUCTURE                        │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                    WEB LAYER (Next.js 16.1.1)               │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │    │
│  │  │   Pages     │  │ Components  │  │    API Routes       │  │    │
│  │  │  (React 19) │  │ (shadcn/ui) │  │ (Route Handlers)    │  │    │
│  │  └─────────────┘  └─────────────┘  └─────────────────────┘  │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                │                                     │
│                                ▼                                     │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                      SERVICE LAYER                           │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐   │    │
│  │  │  PTY Manager │  │   Auth       │  │  Bot-HQ Context  │   │    │
│  │  │  (node-pty)  │  │  (Cookies)   │  │  (.bot-hq/)      │   │    │
│  │  └──────────────┘  └──────────────┘  └──────────────────┘   │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                │                                     │
│                                ▼                                     │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                      DATA LAYER                              │    │
│  │  ┌─────────────────────────┐  ┌─────────────────────────┐   │    │
│  │  │  SQLite + Drizzle ORM   │  │  File System (.bot-hq)  │   │    │
│  │  │  data/bot-hq.db (WAL)   │  │  Markdown context files │   │    │
│  │  └─────────────────────────┘  └─────────────────────────┘   │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                    EXTERNAL INTEGRATIONS                     │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐   │    │
│  │  │ MCP Server   │  │  Claude Code │  │  GitHub/GitLab   │   │    │
│  │  │ (stdio)      │  │  (PTY)       │  │  (REST API)      │   │    │
│  │  └──────────────┘  └──────────────┘  └──────────────────┘   │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Runtime Requirements

| Component | Requirement | Notes |
|-----------|-------------|-------|
| **Node.js** | 18+ (20+ recommended) | Checked at install via preinstall script |
| **Port** | 7890 (configurable) | `BOT_HQ_PORT` environment variable |
| **Disk** | SQLite DB + buffer storage | ~100KB per active PTY session |
| **Memory** | Varies with session count | node-pty spawns child processes |

### Data Storage

| Type | Location | Purpose |
|------|----------|---------|
| **Database** | `data/bot-hq.db` | SQLite database (WAL mode) |
| **Context Files** | `$BOT_HQ_SCOPE/.bot-hq/` | Markdown context persistence |
| **Session Buffer** | In-memory (100KB/session) | PTY output for reconnection |
| **Config** | `.claude/`, `.mcp.json` | Claude Code and MCP settings |

### Network

| Endpoint | Port | Protocol | Purpose |
|----------|------|----------|---------|
| Web Server | 7890 | HTTP | Web UI and REST API |
| SSE Stream | 7890 | HTTP (SSE) | Terminal output streaming |
| MCP Server | stdin/stdout | stdio | Claude Code integration |

### Process Model

```
                    ┌─────────────────────────┐
                    │   Next.js Server        │
                    │   (Single Process)      │
                    └───────────┬─────────────┘
                                │
            ┌───────────────────┼───────────────────┐
            │                   │                   │
            ▼                   ▼                   ▼
    ┌───────────────┐   ┌───────────────┐   ┌───────────────┐
    │ Manager PTY   │   │ Session PTY   │   │ Session PTY   │
    │ (Claude Code) │   │ (Claude Code) │   │ (Claude Code) │
    │ ID: "manager" │   │ ID: uuid-1    │   │ ID: uuid-2    │
    └───────────────┘   └───────────────┘   └───────────────┘
```

---

## 5. Complete File Structure

### Project Root

```
bot-hq/
├── .claude/                          # Claude Code configuration
│   ├── hooks/
│   │   └── approval-gate.js          # Approval hook for commands
│   ├── settings.json                 # Claude settings
│   └── settings.local.json           # Local overrides
├── .mcp.json                         # MCP server configuration
├── data/                             # Runtime data (gitignored)
│   └── bot-hq.db                     # SQLite database
├── dist/                             # Compiled MCP server
├── docs/                             # Design documents & plans
│   ├── plans/                        # Architecture decisions
│   ├── plugin-api-reference.md       # Plugin API docs
│   └── plugin-development.md         # Plugin dev guide
├── drizzle/                          # Database migrations
│   └── meta/                         # Migration metadata
├── scripts/                          # Setup & maintenance scripts
│   ├── doctor.js                     # Diagnostic tool
│   └── setup.js                      # Setup wizard
├── src/                              # Application source code
├── AGENTS.md                         # AI agent context file
├── CHROME-TESTING.md                 # Browser testing guide
├── README.md                         # Project readme
├── documentation.md                  # This file
├── drizzle.config.ts                 # Drizzle ORM configuration
├── next.config.ts                    # Next.js configuration
├── next-env.d.ts                     # Next.js TypeScript declarations
├── package.json                      # Dependencies & scripts
├── tsconfig.json                     # TypeScript configuration
└── vitest.config.ts                  # Test configuration
```

### Source Code (`src/`)

```
src/
├── __tests__/                        # Test files
│   ├── lib/
│   │   ├── auth.test.ts
│   │   ├── config-types.test.ts
│   │   ├── github-types.test.ts
│   │   └── utils.test.ts
│   └── setup.ts                      # Test setup
│
├── app/                              # Next.js App Router
│   ├── api/                          # API Route Handlers
│   │   ├── auth/                     # Authentication endpoints
│   │   │   ├── approve/route.ts      # Approve pending device
│   │   │   ├── devices/
│   │   │   │   ├── route.ts          # List devices
│   │   │   │   └── [id]/route.ts     # Manage device
│   │   │   ├── pair/route.ts         # Device pairing
│   │   │   ├── pending/route.ts      # List pending devices
│   │   │   ├── poll/route.ts         # Poll auth status
│   │   │   ├── register/route.ts     # Register device
│   │   │   ├── reject/route.ts       # Reject device
│   │   │   └── verify/route.ts       # Verify token
│   │   │
│   │   ├── claude-chat/              # Claude chat API
│   │   │   ├── message/route.ts      # Send message
│   │   │   ├── new/route.ts          # New session
│   │   │   └── sessions/
│   │   │       ├── route.ts          # List sessions
│   │   │       └── [sessionId]/route.ts
│   │   │
│   │   ├── claude-settings/route.ts  # Claude settings
│   │   │
│   │   ├── docs/                     # Documentation API
│   │   │   ├── route.ts              # Doc list
│   │   │   └── [...path]/route.ts    # Doc content
│   │   │
│   │   ├── git/                      # Git operations
│   │   │   └── diff/route.ts         # Get branch diff
│   │   │
│   │   ├── git-remote/               # Git integration
│   │   │   ├── route.ts              # CRUD remotes
│   │   │   ├── [id]/route.ts         # Manage remote
│   │   │   ├── clone/route.ts        # Clone repo
│   │   │   ├── repos/route.ts        # List repos
│   │   │   └── issues/
│   │   │       ├── route.ts          # List issues
│   │   │       └── create-task/route.ts
│   │   │
│   │   ├── logs/                     # Logging API
│   │   │   ├── route.ts              # Get logs
│   │   │   ├── sources/route.ts      # Log sources
│   │   │   └── stream/route.ts       # SSE log stream
│   │   │
│   │   ├── manager/                  # Manager control
│   │   │   ├── chat/route.ts         # Chat with manager
│   │   │   ├── command/route.ts      # Send command
│   │   │   ├── status/route.ts       # Manager status
│   │   │   └── summary/route.ts      # Get summary
│   │   │
│   │   ├── manager-settings/route.ts # Manager config
│   │   ├── pick-folder/route.ts      # Folder picker
│   │   ├── settings/route.ts         # App settings
│   │   │
│   │   ├── tasks/                    # Task management
│   │   │   ├── route.ts              # List/create tasks
│   │   │   ├── awaiting/route.ts     # Awaiting input
│   │   │   └── [id]/
│   │   │       ├── route.ts          # Get/update task
│   │   │       ├── assign/route.ts   # Assign task
│   │   │       └── review/route.ts   # Review task
│   │   │
│   │   ├── terminal/                 # Terminal/PTY API
│   │   │   ├── route.ts              # List/create sessions
│   │   │   ├── manager/route.ts      # Manager session
│   │   │   └── [id]/
│   │   │       ├── route.ts          # Session control
│   │   │       └── stream/route.ts   # SSE output stream
│   │   │
│   │   └── workspaces/               # Workspace management
│   │       ├── route.ts              # List/create
│   │       ├── by-path/route.ts      # Find by path
│   │       └── [id]/
│   │           ├── route.ts          # Get/update/delete
│   │           ├── context/route.ts  # Workspace context
│   │           └── config/
│   │               ├── route.ts      # Configuration
│   │               └── sync/route.ts # Sync from remote
│   │
│   ├── claude/page.tsx               # Claude session page
│   ├── docs/page.tsx                 # Documentation viewer
│   ├── git-remote/page.tsx           # Git remote config
│   ├── layout.tsx                    # Root layout
│   ├── logs/
│   │   ├── page.tsx                  # Log viewer
│   │   ├── server/page.tsx           # Server logs
│   │   └── agent/[sessionId]/page.tsx
│   ├── page.tsx                      # Home (Taskboard)
│   ├── pending/page.tsx              # Awaiting input tasks
│   ├── settings/
│   │   ├── page.tsx                  # Settings
│   │   └── workspaces/[id]/page.tsx  # Workspace config
│   ├── unauthorized/page.tsx         # Access denied
│   └── workspaces/page.tsx           # Workspace list
│
├── components/                       # React Components
│   ├── chat-panel/                   # Floating chat panel
│   │   ├── chat-input.tsx
│   │   ├── chat-message.tsx
│   │   └── chat-panel.tsx
│   │
│   ├── claude/                       # Claude session UI
│   │   ├── chat-message.tsx          # Chat message display
│   │   ├── chat-view.tsx             # Chat view container
│   │   ├── claude-session.tsx        # Main session component
│   │   ├── mode-toggle.tsx           # View mode toggle
│   │   ├── permission-prompt.tsx     # Permission handling
│   │   ├── selection-menu.tsx        # Selection menu
│   │   ├── session-tabs.tsx          # Tab management
│   │   └── terminal-view.tsx         # Terminal display
│   │
│   ├── docs/                         # Documentation UI
│   │   ├── doc-viewer.tsx            # Markdown viewer
│   │   └── file-tree.tsx             # File navigation
│   │
│   ├── git-remote/                   # Git remote UI
│   │   ├── git-remote-settings.tsx   # Settings form
│   │   ├── issues-list.tsx           # Issue listing
│   │   ├── remote-form.tsx           # Add/edit remote
│   │   └── remote-list.tsx           # Remote listing
│   │
│   ├── layout/                       # Layout components
│   │   ├── header.tsx                # Page headers
│   │   ├── mobile-nav.tsx            # Mobile navigation
│   │   └── sidebar.tsx               # Main sidebar
│   │
│   ├── log-viewer/                   # Log display
│   │   ├── log-detail.tsx            # Log detail view
│   │   ├── log-entry.tsx             # Single log entry
│   │   ├── log-list.tsx              # Log listing
│   │   ├── log-source-card.tsx       # Source card
│   │   └── log-source-list.tsx       # Source listing
│   │
│   ├── notifications/                # Notification system
│   │   ├── awaiting-input-banner.tsx # Input needed banner
│   │   ├── notification-bell.tsx     # Bell icon
│   │   └── notification-provider.tsx # Context provider
│   │
│   ├── review/                       # Code review UI
│   │   └── diff-review-card.tsx      # Diff display
│   │
│   ├── settings/                     # Settings UI
│   │   ├── add-workspace-dialog.tsx  # Add workspace
│   │   ├── device-list.tsx           # Device management
│   │   ├── manager-settings.tsx      # Manager config
│   │   ├── pairing-display.tsx       # Pairing UI
│   │   ├── rule-list-editor.tsx      # Rule editing
│   │   ├── scope-directory.tsx       # Directory picker
│   │   └── workspace-list.tsx        # Workspace list
│   │
│   ├── taskboard/                    # Task management UI
│   │   ├── create-task-dialog.tsx    # Create task modal
│   │   ├── task-card.tsx             # Task display
│   │   └── task-list.tsx             # Task listing
│   │
│   └── ui/                           # shadcn/ui components
│       ├── alert-dialog.tsx
│       ├── badge.tsx
│       ├── button.tsx
│       ├── card.tsx
│       ├── checkbox.tsx
│       ├── dialog.tsx
│       ├── dropdown-menu.tsx
│       ├── input.tsx
│       ├── label.tsx
│       ├── scroll-area.tsx
│       ├── select.tsx
│       ├── separator.tsx
│       ├── sonner.tsx
│       ├── switch.tsx
│       ├── tabs.tsx
│       └── textarea.tsx
│
├── hooks/                            # React Hooks
│   ├── use-log-stream.ts             # Log SSE subscription
│   ├── use-manager-chat.ts           # Manager chat state
│   ├── use-media-query.ts            # Responsive detection
│   └── use-notifications.ts          # Notification state
│
├── lib/                              # Core Libraries
│   ├── agent-docs.ts                 # Documentation generator
│   ├── agents/
│   │   └── config-types.ts           # Agent config parsing
│   ├── auth/
│   │   ├── index.ts                  # Device authorization
│   │   └── pairing.ts                # Pairing code logic
│   ├── bot-hq/
│   │   ├── index.ts                  # .bot-hq management
│   │   └── templates.ts              # Default templates
│   ├── db/
│   │   ├── index.ts                  # Database connection
│   │   └── schema.ts                 # Table definitions
│   ├── manager/
│   │   └── persistent-manager.ts     # Manager lifecycle
│   ├── notifications/
│   │   └── index.ts                  # Browser notifications
│   ├── pty-manager.ts                # PTY session management
│   ├── settings.ts                   # Settings helpers
│   ├── terminal-parser.ts            # Output parsing
│   └── utils.ts                      # Utility functions
│
├── mcp/                              # MCP Server
│   ├── server.ts                     # Entry point
│   └── tools/
│       ├── agents.ts                 # Agent tools
│       ├── monitoring.ts             # Monitoring tools
│       └── tasks.ts                  # Task tools
│
├── instrumentation.ts                # Next.js instrumentation
└── middleware.ts                     # Auth middleware
```

### Configuration Files

| File | Purpose |
|------|---------|
| `package.json` | Dependencies, scripts, metadata |
| `tsconfig.json` | TypeScript compiler options |
| `drizzle.config.ts` | Drizzle ORM database configuration |
| `vitest.config.ts` | Test runner configuration |
| `components.json` | shadcn/ui component configuration |
| `.mcp.json` | MCP server configuration for Claude Code |

---

## 6. Installation & Setup

### Prerequisites

- Node.js 18+ (20+ recommended)
- npm
- Claude Code CLI installed globally

### Quick Start

```bash
# Clone the repository
git clone <repo-url>
cd bot-hq

# Install dependencies
npm install

# Interactive setup (recommended)
npm run setup

# Start development server
npm run dev
```

### Setup Commands

| Command | Description |
|---------|-------------|
| `npm run setup` | Interactive setup wizard |
| `npm run setup:quick` | Non-interactive setup with defaults |
| `npm run setup:verify` | Verify installation health |
| `npm run setup:reset` | Reset database and state |
| `npm run setup -- --help` | Show all setup options |
| `npm run setup -- --port 8080` | Custom port |
| `npm run setup -- --scope ~/code` | Custom projects directory |
| `npm run setup -- -y --skip-mcp` | Quick setup, skip MCP configuration |

### Doctor Command

```bash
# Diagnose common issues
npm run doctor

# Auto-fix problems
npm run doctor:fix
```

The doctor command checks:
- Database initialization
- node-pty compilation
- MCP server configuration
- Port availability
- Permission issues

### Development Commands

| Command | Description |
|---------|-------------|
| `npm run dev` | Start development server (port 7890) |
| `npm run local` | Alias for `npm run dev` |
| `npm run build` | Production build |
| `npm run start` | Start production server |
| `npm run mcp` | Run MCP server standalone |
| `npm run lint` | Run ESLint |

### Database Commands

| Command | Description |
|---------|-------------|
| `npm run db:push` | Push schema changes to database |
| `npm run db:generate` | Generate migration files |
| `npm run db:migrate` | Run migrations |
| `npm run db:studio` | Open Drizzle Studio (database browser) |

---

## 7. Database Schema

Bot-HQ uses SQLite with WAL (Write-Ahead Logging) mode for better concurrency. The database is stored at `data/bot-hq.db`.

### Tables Overview

```
┌──────────────────┐     ┌──────────────────┐
│    workspaces    │◄────│      tasks       │
├──────────────────┤     ├──────────────────┤
│ id (PK)          │     │ id (PK)          │
│ name (unique)    │     │ workspaceId (FK) │
│ repoPath         │     │ sourceRemoteId   │
│ linkedDirs       │     │ title            │
│ buildCommand     │     │ description      │
│ agentConfig      │     │ state            │
│ createdAt        │     │ priority         │
└──────────────────┘     │ branchName       │
         │               │ iterationCount   │
         │               │ feedback         │
         ▼               │ ...              │
┌──────────────────┐     └──────────────────┘
│   gitRemotes     │              │
├──────────────────┤              ▼
│ id (PK)          │     ┌──────────────────┐
│ workspaceId (FK) │     │      logs        │
│ provider         │     ├──────────────────┤
│ owner/repo       │     │ id (PK)          │
│ credentials      │     │ workspaceId (FK) │
│ isDefault        │     │ taskId (FK)      │
└──────────────────┘     │ type             │
                         │ message          │
                         │ createdAt        │
                         └──────────────────┘
```

### Workspaces Table

```typescript
workspaces {
  id: integer (PK, autoincrement)
  name: text (unique, not null)      // Project identifier
  repoPath: text (not null)          // Absolute path to repository
  linkedDirs: text                   // JSON array of linked directories
  buildCommand: text                 // Optional build command
  agentConfig: text                  // JSON AgentConfig
  createdAt: timestamp
}
```

### Tasks Table

```typescript
tasks {
  id: integer (PK, autoincrement)
  workspaceId: integer (FK → workspaces.id)
  sourceRemoteId: integer (FK → gitRemotes.id)  // If from GitHub issue
  sourceRef: text                    // Issue number, MR number
  title: text (not null)
  description: text
  state: enum ['new', 'queued', 'in_progress', 'awaiting_input', 'needs_help', 'done']
  priority: integer (default 0)
  agentPlan: text
  branchName: text                   // Feature branch created
  completionCriteria: text           // Success criteria for iteration
  iterationCount: integer (default 0)
  maxIterations: integer             // Override global default
  feedback: text                     // Human feedback on retry

  // Brainstorming fields
  waitingQuestion: text              // Question manager is asking
  waitingContext: text               // Conversation context
  waitingSince: timestamp            // When started waiting

  assignedAt: timestamp
  updatedAt: timestamp
}
```

### Git Remotes Table

```typescript
gitRemotes {
  id: integer (PK, autoincrement)
  workspaceId: integer (FK → workspaces.id)  // Null = global
  provider: enum ['github', 'gitlab', 'bitbucket', 'gitea', 'custom']
  name: text (not null)              // Display name
  url: text (not null)               // Base URL
  owner: text                        // Repository owner/org
  repo: text                         // Repository name
  credentials: text                  // Encrypted JSON {token: string}
  isDefault: boolean (default false)
  createdAt: timestamp
  updatedAt: timestamp
}
```

### Logs Table

```typescript
logs {
  id: integer (PK, autoincrement)
  workspaceId: integer (FK → workspaces.id)
  taskId: integer (FK → tasks.id)
  type: enum ['agent', 'test', 'approval', 'error', 'health']
  message: text (not null)
  details: text                      // JSON details
  createdAt: timestamp
}
```

### Authentication Tables

```typescript
authorizedDevices {
  id: integer (PK, autoincrement)
  deviceName: text (not null)
  deviceFingerprint: text (unique, not null)
  tokenHash: text (not null)         // SHA256 hashed token
  authorizedAt: timestamp
  lastSeenAt: timestamp
  isRevoked: boolean (default false)
}

pendingDevices {
  id: integer (PK, autoincrement)
  pairingCode: text (unique, not null)  // 6-digit code
  deviceInfo: text (not null)        // JSON: userAgent, IP, etc.
  createdAt: timestamp
  expiresAt: timestamp
}

settings {
  key: text (PK)
  value: text (not null)
  updatedAt: timestamp
}
```

---

## 8. MCP Server & Tools

### Overview

Bot-HQ includes an MCP (Model Context Protocol) server that exposes tools for Claude Code to interact with the system. The server uses stdio transport.

**Entry Point**: `src/mcp/server.ts`

**Run standalone**: `npm run mcp`

### Tool Categories

#### Task Tools (`src/mcp/tools/tasks.ts`)

| Tool | Description | Parameters |
|------|-------------|------------|
| `task_list` | List tasks with optional filters | `workspaceId?`, `state?` |
| `task_get` | Get full task details | `taskId` (required) |
| `task_create` | Create a new task | `workspaceId`, `title`, `description`, `priority?` |
| `task_update` | Update task properties | `taskId`, `priority?`, `state?`, `notes?` |
| `task_assign` | Move task from 'new' to 'queued' | `taskId` |

#### Agent Tools (`src/mcp/tools/agents.ts`)

| Tool | Description | Parameters |
|------|-------------|------------|
| `agent_list` | Get manager status | (none) |
| `agent_start` | Send task to manager | `taskId` |
| `agent_stop` | Stop task in progress | `taskId` |

#### Monitoring Tools (`src/mcp/tools/monitoring.ts`)

| Tool | Description | Parameters |
|------|-------------|------------|
| `logs_get` | Get recent logs | `taskId?`, `type?`, `limit?` |
| `status_overview` | Dashboard overview | (none) |
| `workspace_list` | List all workspaces | (none) |
| `workspace_create` | Create workspace | `name`, `repoPath`, `linkedDirs?`, `buildCommand?` |
| `workspace_delete` | Delete workspace | `workspaceId` |
| `workspace_update` | Update workspace | `workspaceId`, `linkedDirs?`, `buildCommand?` |
| `workspace_sync` | Sync issues from remote | `workspaceId?` |
| `github_list_all_issues` | List GitHub issues | (none) |
| `github_create_task_from_issue` | Create task from issue | `workspaceId`, `issueNumber`, `priority?` |

### MCP Configuration

To use Bot-HQ MCP tools in Claude Code, add to your Claude Code settings:

```json
{
  "mcpServers": {
    "bot-hq": {
      "command": "npm",
      "args": ["run", "mcp"],
      "cwd": "/path/to/bot-hq"
    }
  }
}
```

---

## 9. Core Modules

### Directory Structure

```
src/
├── app/                    # Next.js pages and API routes
│   ├── api/                # API endpoints
│   ├── claude/             # Claude session page
│   ├── workspaces/         # Workspace pages
│   └── ...
├── components/             # React components
│   ├── taskboard/          # Task management UI
│   ├── claude/             # Claude session components
│   ├── settings/           # Settings UI
│   └── ui/                 # shadcn/ui components
├── lib/                    # Core logic
│   ├── db/                 # Database (schema, connection)
│   ├── manager/            # Persistent manager logic
│   ├── bot-hq/             # .bot-hq file management
│   ├── auth/               # Authentication
│   └── ...
└── mcp/                    # MCP server
    ├── server.ts           # Entry point
    └── tools/              # Tool definitions
```

### Key Files Reference

| File | Purpose |
|------|---------|
| `src/lib/db/schema.ts` | Database table definitions |
| `src/lib/db/index.ts` | Drizzle connection (lazy-loaded singleton) |
| `src/lib/manager/persistent-manager.ts` | Manager lifecycle and control |
| `src/lib/pty-manager.ts` | PTY/terminal session management |
| `src/lib/bot-hq/index.ts` | .bot-hq file structure management |
| `src/lib/bot-hq/templates.ts` | Default prompts and templates |
| `src/lib/settings.ts` | Settings management |
| `src/lib/terminal-parser.ts` | Parse terminal output, detect prompts |
| `src/lib/auth/index.ts` | Device authorization |
| `src/middleware.ts` | Authentication middleware |

---

## 10. Web Interface

### Page Routes

| Route | Page | Purpose |
|-------|------|---------|
| `/` | Taskboard | Main dashboard - task management |
| `/claude` | Claude | Manager terminal & chat interface |
| `/workspaces` | Workspaces | Workspace list & management |
| `/git-remote` | Git Remote | Configure GitHub/GitLab remotes |
| `/pending` | Pending | Tasks awaiting input (brainstorming) |
| `/logs` | Logs | System logs viewer |
| `/logs/server` | Server Logs | Server-specific logs |
| `/logs/agent/[sessionId]` | Agent Logs | Individual agent session logs |
| `/settings` | Settings | Global settings configuration |
| `/settings/workspaces/[id]` | Workspace Settings | Individual workspace settings |
| `/docs` | Docs | Documentation viewer |
| `/unauthorized` | Unauthorized | Access denied page |

### Key Components

#### Taskboard (`src/components/taskboard/`)
- `task-list.tsx` - List all tasks with filters
- `task-card.tsx` - Individual task display
- `create-task-dialog.tsx` - Task creation modal

#### Claude Session (`src/components/claude/`)
- `claude-session.tsx` - Main session container
- `terminal-view.tsx` - Terminal output display
- `chat-view.tsx` - Chat message display
- `session-tabs.tsx` - Tab management for multiple sessions
- `permission-prompt.tsx` - Permission request handling

#### Settings (`src/components/settings/`)
- `workspace-list.tsx` - Manage workspaces
- `add-workspace-dialog.tsx` - Add workspace modal
- `scope-directory.tsx` - Directory picker

---

## 11. API Reference

### Task Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/tasks` | List all tasks |
| POST | `/api/tasks` | Create a task |
| GET | `/api/tasks/[id]` | Get task details |
| PATCH | `/api/tasks/[id]` | Update a task |
| POST | `/api/tasks/[id]/assign` | Assign task to queue |
| POST | `/api/tasks/[id]/review` | Submit task review/feedback |
| GET | `/api/tasks/awaiting` | Get tasks awaiting input |

### Workspace Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/workspaces` | List all workspaces |
| POST | `/api/workspaces` | Create a workspace |
| GET | `/api/workspaces/[id]/context` | Get workspace context |
| PUT | `/api/workspaces/[id]/context` | Update workspace context |
| GET | `/api/workspaces/[id]/config` | Get workspace configuration |
| POST | `/api/workspaces/[id]/config/sync` | Sync from remote |
| GET | `/api/workspaces/by-path` | Find workspace by path |

### Terminal Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/terminal` | List all terminal sessions |
| POST | `/api/terminal` | Create new terminal session |
| GET | `/api/terminal/[id]` | Get session info |
| DELETE | `/api/terminal/[id]` | Kill session |
| GET | `/api/terminal/[id]/stream` | SSE stream for output |
| GET | `/api/terminal/manager` | Get manager status |
| POST | `/api/terminal/manager` | Send input to manager |

### Authentication Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/auth/register` | Register pending device |
| GET | `/api/auth/poll` | Poll authorization status |
| POST | `/api/auth/verify` | Verify device token |
| GET | `/api/auth/pending` | List pending devices (admin) |
| POST | `/api/auth/approve` | Approve device (admin) |
| POST | `/api/auth/reject` | Reject device (admin) |
| POST | `/api/auth/pair` | Device pairing |

### Git Remote Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/git-remote` | List all remotes |
| POST | `/api/git-remote` | Create a remote |
| GET | `/api/git-remote/issues` | List issues from remotes |
| POST | `/api/git-remote/issues/sync` | Sync issues |

### Git Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/git/diff` | Get git diff for a branch |

---

## 12. Terminal & PTY Management

### PTY Manager (`src/lib/pty-manager.ts`)

The PTY manager handles terminal sessions using `node-pty`. Key features:

- **Session Creation**: Spawns Claude Code with `--dangerously-skip-permissions`
- **Buffer Storage**: Stores last 100KB of output for reconnecting clients
- **Persistent Sessions**: Sessions don't timeout; they persist until killed or server stops
- **Manager Session**: Special session ID `"manager"` for the persistent manager

### Session Configuration

```typescript
{
  name: "xterm-256color",
  cols: 120,
  rows: 30,
  env: {
    TERM: "xterm-256color",
    COLORTERM: "truecolor"
  }
}
```

### Reconnection

When clients reconnect to an existing session:
1. SSE stream sends `{ type: "connected", sessionId }` message
2. Immediately followed by `{ type: "buffer", data: "..." }` with stored output
3. Then continues with live streaming

### Terminal Parser (`src/lib/terminal-parser.ts`)

Parses terminal output to detect:
- Permission prompts from Claude Code
- Selection menus
- Awaiting input states
- ANSI escape code stripping

---

## 13. Context Management

### .bot-hq Directory Structure

Bot-HQ uses markdown files for context persistence, stored in the `BOT_HQ_SCOPE` directory:

```
.bot-hq/
├── MANAGER_PROMPT.md          # Customizable manager instructions
├── QUEUE.md                   # Task queue status
├── .manager-status            # Manager running state flag
│
└── workspaces/
    └── {workspace-name}/
        ├── WORKSPACE.md       # Project context
        │                      # - Tech stack, dependencies
        │                      # - Code patterns and standards
        │                      # - Important file locations
        │
        ├── STATE.md           # Current working state
        │                      # - What was last worked on
        │                      # - Any pending items
        │
        └── tasks/
            └── {task-id}/
                └── PROGRESS.md    # Per-task work log
                                   # - Status, iteration count
                                   # - Work log entries
                                   # - Blockers encountered
```

### Why Markdown Files?

| Benefit | Description |
|---------|-------------|
| Human-readable | Easy to inspect and edit manually |
| Version-controllable | Git-friendly, can track changes |
| Survives restarts | No complex serialization needed |
| Easy injection | Can be directly included in prompts |

### Context Functions

```typescript
// Initialize .bot-hq structure
initializeBotHqStructure(): Promise<void>

// Initialize workspace context
initializeWorkspaceContext(workspaceName: string): Promise<void>

// Get/save manager prompt
getManagerPrompt(): Promise<string>
saveManagerPrompt(content: string): Promise<void>

// Get/save workspace context
getWorkspaceContext(workspaceName: string): Promise<string>
saveWorkspaceContext(workspaceName: string, content: string): Promise<void>

// Task progress
getTaskProgress(workspaceName: string, taskId: number): Promise<string | null>
cleanupTaskFiles(workspaceName: string, taskId: number): Promise<void>
```

---

## 14. Authentication

### Authentication Model

Bot-HQ uses device-based authentication:

1. **Localhost**: Full access without authentication
2. **Remote Access**: Requires device token in cookies

### Authentication Flow

```
┌─────────────────────────────────────────────────────────┐
│ 1. Remote client requests access                         │
│    → Generates fingerprint, requests pairing             │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│ 2. Server creates pending device with 6-digit code      │
│    → Code expires after time window                      │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│ 3. Admin (localhost) approves device                     │
│    → Server generates token                              │
│    → Stores token hash in database                       │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│ 4. Client receives token                                 │
│    → Stores in device_token cookie                       │
│    → Uses for subsequent requests                        │
└─────────────────────────────────────────────────────────┘
```

### Middleware (`src/middleware.ts`)

```typescript
PUBLIC_ROUTES = [
  "/unauthorized",
  "/api/auth/register",
  "/api/auth/check",
  "/api/auth/poll"
]

ADMIN_ONLY_ROUTES = [
  "/api/auth/pending",
  "/api/auth/approve",
  "/api/auth/reject"
]
```

**Flow**:
1. Check if localhost → allow all access
2. Block admin-only routes for non-localhost
3. Allow public routes
4. Check `device_token` and `device_id` cookies
5. Redirect to `/unauthorized` if missing

---

## 15. Workflow & Task States

### Task State Machine

```
    ┌───────────────┐                   ┌─────────────────┐
    │               │                   │                 │
    │    [new]      │───── assign ─────►│   [queued]      │
    │               │                   │                 │
    │  Human creates│                   │  Ready for      │
    │  task         │                   │  manager        │
    │               │                   │                 │
    └───────────────┘                   └────────┬────────┘
                                                 │
                                            start
                                                 │
                                                 ▼
                                        ┌─────────────────┐
                                        │                 │
                                        │ [in_progress]   │◄──────────┐
                                        │                 │           │
                                        │  Agent working  │      (retry with
                                        │                 │       feedback)
                                        │                 │           │
                                        └───────┬─────────┘           │
                                                │                     │
                          ┌─────────────────────┼─────────────────┐   │
                          │                     │                 │   │
                          ▼                     ▼                 ▼   │
                 ┌─────────────┐       ┌─────────────┐   ┌───────────────┐
                 │             │       │             │   │               │
                 │[awaiting_   │       │[needs_help] │   │   [done]      │
                 │  input]     │       │             │   │               │
                 │             │       │  Human must │   │  Human        │
                 │  Answer     │       │  intervene  │   │  reviews      │
                 │  questions  │       │             │   │               │
                 │             │       │             │   │               │
                 └──────┬──────┘       └─────────────┘   └───────┬───────┘
                        │                                        │
                   (answer)                                (accept/reject)
                        │                                        │
                        └───────────► [queued] ◄─────────────────┘
                                     (if rejected)
```

### State Descriptions

| State | Mode | Description | Next Actions |
|-------|------|-------------|--------------|
| `new` | HITL | Task just created | Assign to queue |
| `queued` | AUTO | Ready for work | Start agent |
| `in_progress` | AUTO | Agent working | Wait for completion |
| `awaiting_input` | HITL | Manager needs clarification | Answer question |
| `needs_help` | HITL | Stuck after failures | Manual intervention |
| `done` | HITL | Work complete | Accept or reject |

### Iteration Loop

Tasks can be retried with feedback:

```
Task Execute → Evaluate → Criteria Met?
                              ├─ YES → DONE
                              └─ NO → iteration++
                                     → Add Feedback
                                     → iterations < max?
                                           ├─ YES → RETRY (queued)
                                           └─ NO → NEEDS_HELP
```

---

## 16. Configuration Reference

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | (required) | API key for Claude agents |
| `BOT_HQ_PORT` | 7890 | Server port |
| `BOT_HQ_URL` | http://localhost:7890 | Full URL (for MCP) |
| `BOT_HQ_SCOPE` | ~/Projects | Working directory for agents |
| `BOT_HQ_SHELL` | $SHELL | Shell for terminal sessions |
| `BOT_HQ_MAX_ITERATIONS` | 3 | Max task retry iterations |
| `BOT_HQ_ITERATION_DELAY` | 5000 | Delay between iterations (ms) |
| `DEBUG` | - | Enable debug logging (`bot-hq:*`) |
| `NODE_ENV` | development | Environment |

### Agent Configuration

Per-workspace agent configuration:

```typescript
interface AgentConfig {
  approvalRules: string[];      // Commands requiring approval
  blockedCommands: string[];    // Explicitly blocked
  customInstructions: string;   // Workspace-specific instructions
  allowedPaths: string[];       // Filesystem restrictions
}

// Default config
{
  approvalRules: ["git push", "git force-push", "npm publish"],
  blockedCommands: ["rm -rf /", "sudo rm"],
  customInstructions: "",
  allowedPaths: []
}
```

### PTY Configuration

```typescript
{
  buffer: 100 * 1024,           // 100KB buffer per session
  cols: 120,
  rows: 30,
  term: "xterm-256color"
}
```

---

## 17. Troubleshooting

### Common Issues

#### "node-pty failed to build"

```bash
# Rebuild native modules
npm rebuild node-pty

# Or run doctor
npm run doctor:fix
```

#### "Database not initialized"

```bash
# Push schema
npm run db:push

# Or reset
npm run setup:reset
```

#### "Manager not starting"

1. Check if Claude Code is installed: `claude --version`
2. Verify BOT_HQ_SCOPE exists and is writable
3. Check logs at `/logs/server`

#### "Port already in use"

```bash
# Check what's using the port
lsof -i :7890

# Use different port
BOT_HQ_PORT=8080 npm run dev
```

#### "Terminal session not responding"

1. Check PTY session exists via `/api/terminal`
2. Try killing and recreating session
3. Check server logs for errors

### Verification

```bash
# Full system check
npm run setup:verify

# Diagnose issues
npm run doctor
```

### Debug Mode

```bash
DEBUG=bot-hq:* npm run dev
```

---

## Appendix: Credits & Inspirations

Bot-HQ's architecture draws from two key inspirations:

| Feature | Inspired By | Description |
|---------|-------------|-------------|
| **Criteria-Driven Iteration** | Ralph Wiggum technique ([fstandhartinger](https://github.com/fstandhartinger/ralph-wiggum), [ghuntley](https://github.com/ghuntley/how-to-ralph-wiggum)) | The iteration loop where tasks are retried with feedback until completion criteria are met |
| **Smart Context Management** | GSD (Get Shit Done) pattern ([b-r-a-n/gsd-claude](https://github.com/b-r-a-n/gsd-claude)) | The `.bot-hq/` directory structure with markdown files for context persistence |

---

*Last Updated: 2026-02-02*
