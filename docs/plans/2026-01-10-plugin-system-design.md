# Bot-HQ Plugin System - Design & Implementation Plan

## Overview

Major architectural overhaul to extract GitHub integration into a plugin system. Bot-HQ becomes a lightweight task orchestration engine where all external integrations live in plugins.

**Core Principle:** Bot-HQ works completely standalone. Users create tasks manually, agents work, approvals keep/discard commits locally. Plugins add superpowers (GitHub PRs, time tracking, Slack integration, etc.).

---

## Architecture

### High-Level Structure

```
┌─────────────────────────────────────────────────────────────────┐
│                          bot-hq Core                             │
├─────────────────────────────────────────────────────────────────┤
│  Tabs: Tasks | Pending | Terminal | Settings | Claude | Plugins │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Plugin Runtime                         │   │
│  │  • Loads plugins from ~/.bot-hq/plugins/                  │   │
│  │  • Registers MCP servers                                  │   │
│  │  • Fires event hooks                                      │   │
│  │  • Renders UI contributions                               │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
┌───────────────┐     ┌───────────────┐     ┌───────────────┐
│ github-plugin │     │clockify-plugin│     │ slack-plugin  │
└───────────────┘     └───────────────┘     └───────────────┘
```

### What Stays in Core
- Task CRUD (manual creation via dialog)
- Agent spawning and management
- Approval flow (accept keeps commits, decline discards)
- Workspace management (without GitHub-specific fields)
- Logs, terminal, settings

### What Moves to Plugins
- GitHub issue sync
- PR creation
- GitHub remote configuration
- Any external service integration

---

## Plugin Structure

### Directory Layout

```
~/.bot-hq/plugins/<plugin-name>/
├── plugin.json            # Required: manifest
├── server.ts              # Required: MCP server entry
├── extensions.ts          # Optional: hooks & actions
├── ui/                    # Optional: React components
│   ├── tabs/              # Full page tabs
│   ├── workspace-settings.tsx
│   ├── task-badge.tsx
│   └── task-actions.tsx
├── README.md              # Required: documentation
└── package.json           # For dependencies
```

### Manifest Schema (plugin.json)

```json
{
  "$schema": "https://bot-hq.dev/schemas/plugin.json",
  "name": "plugin-name",
  "version": "1.0.0",
  "description": "What this plugin does",
  "author": {
    "name": "Author Name",
    "url": "https://github.com/username"
  },
  "repository": "https://github.com/username/bot-hq-plugin-name",
  "license": "MIT",

  "bot-hq": {
    "minVersion": "1.0.0"
  },

  "mcp": {
    "entry": "./server.ts",
    "transport": "stdio",
    "tools": ["tool_1", "tool_2"]
  },

  "extensions": "./extensions.ts",

  "ui": {
    "tabs": [
      {
        "id": "tab-id",
        "label": "Tab Label",
        "icon": "icon-name",
        "component": "./ui/tabs/my-tab.tsx"
      }
    ],
    "workspaceSettings": "./ui/workspace-settings.tsx",
    "taskBadge": "./ui/task-badge.tsx",
    "taskActions": "./ui/task-actions.tsx"
  },

  "settings": {
    "settingKey": {
      "type": "string | number | boolean | select",
      "label": "Human readable label",
      "description": "Help text",
      "default": "default value",
      "options": ["for", "select", "type"]
    }
  },

  "credentials": {
    "API_KEY": {
      "type": "secret",
      "label": "API Key",
      "description": "Get this from your account settings",
      "required": true
    }
  },

  "permissions": [
    "workspace:read",
    "workspace:write",
    "task:read",
    "task:write",
    "task:create",
    "approval:read"
  ]
}
```

---

## Plugin Extensions API

### Actions & Hooks (extensions.ts)

```typescript
import { PluginContext, PluginExtensions } from '@bot-hq/plugin-sdk';

export default function(ctx: PluginContext): PluginExtensions {
  return {
    // Actions contribute UI buttons (user explicitly triggers)
    actions: {
      approval: [
        {
          id: 'unique-action-id',
          label: 'Action Label',
          description: (approval, task, workspace) =>
            `Dynamic description`,
          icon: 'icon-name',
          defaultChecked: false,
          handler: async (approval, task, workspace) => {
            await ctx.mcp.call('tool_name', { param: 'value' });
            return { success: true };
          }
        }
      ],
      task: [...],
      workspace: [...]
    },

    // Hooks run automatically (background)
    hooks: {
      onTaskCreated: async (task) => { },
      onTaskUpdated: async (task, changes) => { },
      onAgentStart: async (agent, task) => {
        return { context: 'Injected into agent prompt' };
      },
      onAgentComplete: async (agent, task) => { },
      onApprovalCreated: async (approval) => { },
      onApprovalAccepted: async (approval, task) => { },
      onApprovalRejected: async (approval, task) => { },
    }
  };
}
```

### Plugin Context API

```typescript
interface PluginContext {
  // Call MCP tools
  mcp: {
    call: (tool: string, params: object) => Promise<any>;
  };

  // Global key-value store
  store: {
    get: (key: string) => Promise<any>;
    set: (key: string, value: any) => Promise<void>;
    delete: (key: string) => Promise<void>;
  };

  // Workspace-scoped data
  workspaceData: {
    get: (workspaceId: number) => Promise<any>;
    set: (workspaceId: number, data: any) => Promise<void>;
  };

  // Task-scoped data
  taskData: {
    get: (taskId: number) => Promise<any>;
    set: (taskId: number, data: any) => Promise<void>;
  };

  // Plugin settings & credentials
  settings: Record<string, any>;
  credentials: Record<string, string>;

  // Logging
  log: {
    info: (msg: string) => void;
    warn: (msg: string) => void;
    error: (msg: string) => void;
  };

  // Task management
  tasks: {
    create: (task: NewTask) => Promise<Task>;
    update: (taskId: number, changes: Partial<Task>) => Promise<Task>;
  };
}
```

---

## Database Schema Changes

### Remove GitHub-specific fields

```sql
-- workspaces: remove github_remote
ALTER TABLE workspaces DROP COLUMN github_remote;

-- tasks: remove github_issue_number, add generic source
ALTER TABLE tasks DROP COLUMN github_issue_number;
ALTER TABLE tasks ADD COLUMN source_plugin_id INTEGER;
ALTER TABLE tasks ADD COLUMN source_ref TEXT;
```

### New Plugin Tables

```sql
-- Installed plugins
CREATE TABLE plugins (
  id INTEGER PRIMARY KEY,
  name TEXT UNIQUE NOT NULL,
  version TEXT NOT NULL,
  enabled INTEGER DEFAULT 1,
  manifest TEXT NOT NULL,
  settings TEXT DEFAULT '{}',
  credentials TEXT,  -- encrypted
  installed_at TEXT DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT DEFAULT CURRENT_TIMESTAMP
);

-- Plugin data per workspace
CREATE TABLE plugin_workspace_data (
  id INTEGER PRIMARY KEY,
  plugin_id INTEGER NOT NULL,
  workspace_id INTEGER NOT NULL,
  data TEXT DEFAULT '{}',
  created_at TEXT DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (plugin_id) REFERENCES plugins(id) ON DELETE CASCADE,
  FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE,
  UNIQUE(plugin_id, workspace_id)
);

-- Plugin data per task
CREATE TABLE plugin_task_data (
  id INTEGER PRIMARY KEY,
  plugin_id INTEGER NOT NULL,
  task_id INTEGER NOT NULL,
  data TEXT DEFAULT '{}',
  created_at TEXT DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (plugin_id) REFERENCES plugins(id) ON DELETE CASCADE,
  FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
  UNIQUE(plugin_id, task_id)
);

-- Plugin data per approval
CREATE TABLE plugin_approval_data (
  id INTEGER PRIMARY KEY,
  plugin_id INTEGER NOT NULL,
  approval_id INTEGER NOT NULL,
  data TEXT DEFAULT '{}',
  created_at TEXT DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (plugin_id) REFERENCES plugins(id) ON DELETE CASCADE,
  FOREIGN KEY (approval_id) REFERENCES approvals(id) ON DELETE CASCADE,
  UNIQUE(plugin_id, approval_id)
);

-- Plugin global key-value store
CREATE TABLE plugin_store (
  id INTEGER PRIMARY KEY,
  plugin_id INTEGER NOT NULL,
  key TEXT NOT NULL,
  value TEXT,
  created_at TEXT DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (plugin_id) REFERENCES plugins(id) ON DELETE CASCADE,
  UNIQUE(plugin_id, key)
);
```

---

## UI Changes

### New/Modified Pages

1. **Plugins Tab** (new) - Manage installed plugins, settings, credentials
2. **Tasks Tab** - Add "Create Task" button, remove sync button (plugin provides)
3. **Approval Dialog** - Add plugin action checkboxes
4. **Workspace Settings** - Plugin-contributed tabs
5. **Sidebar** - Plugin-contributed tabs section

### Create Task Dialog Flow

1. User clicks "Create Task"
2. Selects workspace, enters prompt
3. Manager bot (headless Claude) analyzes codebase
4. Refined task preview shown
5. User confirms → task created

### Approval Dialog with Plugin Actions

```
┌─────────────────────────────────────────┐
│  On Accept, also:                       │
│                                         │
│  ☑ <GitHub> Create Pull Request         │
│             Push to owner/repo          │
│                                         │
│  ☑ <Clockify> Log time entry            │
│              [1h 24m] to "Project"      │
│                                         │
│  ☐ <Slack> Notify team                  │
│            Post to #dev-updates         │
│                                         │
│      [Decline] [Request Changes] [Accept]│
└─────────────────────────────────────────┘
```

### Accept vs Decline

| Action | Core Behavior | Plugin Actions |
|--------|---------------|----------------|
| Accept | Keep commits, keep branch, state → done | Run selected checkboxes |
| Decline | Discard commits, delete branch, state → new | None |

---

## Error Handling

### Plugin Isolation

- Plugin errors never break core operations
- Each plugin action wrapped in try/catch with timeout
- Core accept/decline always succeeds
- Failed plugin actions reported in UI with retry option

```typescript
async function handleAccept(approval, selectedActions) {
  // 1. Core (always succeeds)
  await keepCommitsOnBranch(approval);
  await updateTaskState(approval.taskId, 'done');

  // 2. Plugins (failures reported, not blocking)
  const results = await Promise.allSettled(
    selectedActions.map(id => executePluginAction(id, context))
  );

  // 3. Report failures
  const failures = results.filter(r => !r.success);
  if (failures.length) showWarningToast(failures);
}
```

---

## Implementation Plan

### Phase 1: Core Plugin Infrastructure
**Goal:** Plugin runtime that can load, register, and execute plugins

1. **Database migration**
   - Create plugin tables (plugins, plugin_workspace_data, etc.)
   - Add source_plugin_id, source_ref to tasks table
   - Migrate existing data (GitHub fields → plugin data)

2. **Plugin loader**
   - Scan ~/.bot-hq/plugins/ on startup
   - Parse and validate plugin.json manifests
   - Handle missing/invalid plugins gracefully

3. **Plugin registry**
   - Store loaded plugins in memory
   - Track enabled/disabled state
   - Provide API to query plugins

4. **MCP server manager**
   - Spawn plugin MCP servers on demand
   - Handle server lifecycle (start, stop, restart)
   - Route tool calls to correct server

### Phase 2: Plugin API & SDK
**Goal:** Complete API for plugins to interact with bot-hq

5. **Plugin context implementation**
   - ctx.mcp.call() - route to plugin's MCP server
   - ctx.store - plugin_store table operations
   - ctx.workspaceData - plugin_workspace_data operations
   - ctx.taskData - plugin_task_data operations
   - ctx.settings / ctx.credentials - from plugins table

6. **Event system**
   - Define all hook points
   - Fire hooks at appropriate times
   - Pass correct context to handlers
   - Handle async hooks properly

7. **Action registry**
   - Collect actions from all plugins
   - Group by location (approval, task, workspace)
   - Provide API to get actions for context

8. **@bot-hq/plugin-sdk package**
   - TypeScript types for all interfaces
   - Helper utilities
   - Documentation

### Phase 3: Core UI Updates
**Goal:** Bot-hq works standalone without plugins

9. **Remove GitHub from core**
   - Delete /lib/github/ directory
   - Delete /lib/sync/ directory
   - Remove GitHub imports from all files
   - Remove sync button from Tasks tab
   - Remove GitHub fields from workspace dialogs

10. **Create Task dialog**
    - New "Create Task" button
    - Workspace selector
    - Prompt input
    - Manager bot integration
    - Task preview and confirmation

11. **Updated approval flow**
    - Accept: keep commits locally
    - Decline: discard commits, delete branch
    - Remove PR creation from core

12. **Plugins tab**
    - List installed plugins
    - Enable/disable toggle
    - Settings dialog per plugin
    - Credentials management
    - Install from folder button

### Phase 4: Plugin UI Contributions
**Goal:** Plugins can extend the UI

13. **Approval dialog actions**
    - Query action registry for approval actions
    - Render checkboxes with plugin icon/label/description
    - Execute selected actions on accept
    - Show success/failure results

14. **Task card extensions**
    - Render plugin badges (taskBadge components)
    - Render plugin actions (taskActions components)
    - Pass correct context to components

15. **Workspace settings tabs**
    - Load workspaceSettings components from plugins
    - Render as tabs in edit workspace dialog
    - Save plugin data on form submit

16. **Sidebar plugin tabs**
    - Load tab definitions from manifests
    - Add "PLUGINS" section to sidebar
    - Render full-page plugin components

### Phase 5: GitHub Plugin
**Goal:** First official plugin, restores all GitHub functionality

17. **GitHub plugin structure**
    - plugin.json manifest
    - MCP server with tools:
      - github_sync_issues
      - github_create_pr
      - github_clone_repo
      - github_get_issue
    - Extensions with actions/hooks

18. **GitHub plugin UI**
    - GitHub tab (connected repos, sync status)
    - Workspace settings (repo, auto-sync options)
    - Task badge (issue number, labels)
    - Task action (View on GitHub link)
    - Approval action (Create PR checkbox)

19. **Migration path**
    - Auto-create GitHub plugin data from old schema
    - Preserve existing workspace GitHub configs
    - Preserve task → issue mappings

### Phase 6: Polish & Documentation

20. **Error handling & edge cases**
    - Plugin load failures
    - MCP server crashes
    - Network errors in plugin actions
    - Invalid plugin data

21. **Plugin development docs**
    - Getting started guide
    - API reference
    - Example plugins
    - Best practices

22. **Testing**
    - Unit tests for plugin runtime
    - Integration tests for plugin lifecycle
    - E2E tests for GitHub plugin

---

## File Changes Summary

### New Files
```
/src/lib/plugins/
├── index.ts              # Plugin runtime entry
├── loader.ts             # Load plugins from disk
├── registry.ts           # Plugin registry
├── mcp-manager.ts        # MCP server lifecycle
├── context.ts            # PluginContext implementation
├── events.ts             # Event/hook system
├── actions.ts            # Action registry
└── types.ts              # TypeScript interfaces

/src/components/plugins/
├── plugins-page.tsx      # Plugins tab
├── plugin-card.tsx       # Plugin list item
├── plugin-settings-dialog.tsx
└── plugin-actions.tsx    # Render action checkboxes

/src/components/taskboard/
├── create-task-dialog.tsx  # NEW

/src/app/api/plugins/
├── route.ts              # List/install plugins
├── [id]/route.ts         # Plugin CRUD
├── [id]/settings/route.ts
└── [id]/data/route.ts    # Plugin data API

~/.bot-hq/plugins/github/
├── plugin.json
├── server.ts
├── extensions.ts
└── ui/
    ├── tabs/github-tab.tsx
    ├── workspace-settings.tsx
    ├── task-badge.tsx
    └── task-actions.tsx
```

### Deleted Files
```
/src/lib/github/          # Entire directory
/src/lib/sync/            # Entire directory
```

### Modified Files
```
/src/lib/db/schema.ts     # New tables, modified tasks table
/src/components/layout/sidebar.tsx  # Plugin tabs section
/src/components/pending-board/draft-pr-card.tsx  # Plugin actions
/src/components/settings/add-workspace-dialog.tsx  # Remove GitHub
/src/components/settings/workspace-list.tsx  # Remove GitHub
/src/components/taskboard/task-list.tsx  # Create Task button
/src/components/taskboard/task-card.tsx  # Plugin extensions
/src/app/api/approvals/[id]/route.ts  # Remove PR creation
/src/app/api/workspaces/route.ts  # Remove GitHub fields
/src/lib/agents/claude-code.ts  # Plugin hook integration
```

---

## Success Criteria

1. Bot-hq runs standalone without any plugins
2. Users can create tasks manually via dialog
3. Accept keeps commits locally, Decline discards
4. Plugin system loads and runs plugins correctly
5. GitHub plugin restores all previous GitHub functionality
6. Plugin actions appear as checkboxes in approval dialog
7. Plugins can contribute sidebar tabs
8. Plugins can extend workspace settings
9. Plugin errors don't break core functionality
10. Clear documentation for plugin development

---

## Open Questions

1. **Plugin marketplace** - Where do users discover/share plugins? (Future consideration)
2. **Plugin updates** - How to handle version updates? (Manual for now)
3. **Plugin dependencies** - Can plugins depend on other plugins? (Not in v1)
4. **Sandboxing** - How much isolation do plugins need? (Trust model for now)
