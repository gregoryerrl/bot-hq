# Workspace Agent Configuration - Design Document

## Overview

Add per-workspace agent configuration management to Bot-HQ, allowing users to configure approval rules, blocked commands, custom instructions, and allowed paths for each repository's Claude Code agent.

## Goals

1. Centralized UI to manage `.claude/settings.json` per workspace
2. Approval hooks that gate write operations (git push, npm publish) through Bot-HQ
3. Easy-to-use interface with manual sync to workspace

## Data Model

### Schema Addition

Add `agentConfig` JSON column to `workspaces` table:

```typescript
agentConfig: text("agent_config"), // JSON string
```

### AgentConfig Type

```typescript
interface AgentConfig {
  approvalRules: string[];      // Patterns requiring approval
  blockedCommands: string[];    // Always denied patterns
  customInstructions: string;   // Free-text instructions for agent
  allowedPaths: string[];       // Glob patterns (empty = all allowed)
}
```

### Default Config

```json
{
  "approvalRules": ["git push", "git force-push", "npm publish", "yarn publish"],
  "blockedCommands": ["rm -rf /", "sudo rm", ":(){ :|:& };:"],
  "customInstructions": "",
  "allowedPaths": []
}
```

## UI Design

### Route

`/settings/workspaces/[id]/page.tsx`

### Page Layout

- Header with back button, workspace name and path
- Four configuration sections:
  1. **Approval Rules** - Chip/tag list with add/remove
  2. **Blocked Commands** - Chip/tag list with add/remove
  3. **Custom Instructions** - Textarea
  4. **Allowed Paths** - Chip/tag list with add/remove
- Footer with "Save Changes" and "Sync to Workspace" buttons

### Components

- `WorkspaceConfigPage` - Main page component
- `RuleListEditor` - Reusable chip/tag list editor (used for 3 sections)

## API Endpoints

```
GET  /api/workspaces/[id]/config      → Fetch workspace config
PUT  /api/workspaces/[id]/config      → Save config to database
POST /api/workspaces/[id]/config/sync → Write .claude/settings.json to disk
GET  /api/workspaces/by-path          → Lookup workspace by repo path (for hook)
```

## Sync Logic

When "Sync to Workspace" is clicked:

1. Read config from DB
2. Generate `.claude/settings.json`:
   ```json
   {
     "hooks": {
       "PreToolUse": [{
         "matcher": "Bash",
         "hooks": [{
           "type": "command",
           "command": "node {BOT_HQ_PATH}/.claude/hooks/approval-gate.js"
         }]
       }]
     }
   }
   ```
3. Create `.claude/` directory if missing
4. Write `settings.json` file
5. Return success/error status

## Approval Hook Script

**Location:** `bot-hq/.claude/hooks/approval-gate.js` (shared across all workspaces)

### Flow

1. Hook receives `{ tool_name, tool_input: { command }, cwd }`
2. Fetch workspace config: `GET /api/workspaces/by-path?path={cwd}`
3. If command matches `blockedCommands` → deny immediately
4. If command matches `approvalRules` → create approval, poll until resolved
5. Otherwise → allow

### Timeout

5 minutes default - if no human response, deny by default.

## File Structure

```
src/
├── app/
│   ├── settings/
│   │   └── workspaces/
│   │       └── [id]/
│   │           └── page.tsx          # Config page
│   └── api/
│       └── workspaces/
│           ├── [id]/
│           │   └── config/
│           │       ├── route.ts      # GET/PUT config
│           │       └── sync/
│           │           └── route.ts  # POST sync
│           └── by-path/
│               └── route.ts          # GET by path
├── components/
│   └── settings/
│       ├── workspace-config-page.tsx
│       └── rule-list-editor.tsx
└── lib/
    └── agents/
        └── config-types.ts           # AgentConfig type
.claude/
└── hooks/
    └── approval-gate.js              # Shared hook script
```
