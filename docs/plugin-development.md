# Plugin Development Guide

This guide explains how to create plugins for Bot-HQ.

## Overview

Bot-HQ plugins can:

- **Provide MCP tools** - Extend agent capabilities with new tools
- **Add UI elements** - Sidebar tabs, task badges, workspace settings
- **Define actions** - Approval actions, task actions, workspace actions
- **Hook into events** - React to task/agent/approval lifecycle events
- **Store data** - Per-plugin, per-workspace, and per-task data

## Directory Structure

Plugins live in `~/.bot-hq/plugins/<plugin-name>/`. A minimal plugin structure:

```
my-plugin/
├── plugin.json          # Required: Plugin manifest
├── dist/
│   ├── server.js        # Optional: MCP server
│   └── extensions.js    # Optional: Actions & hooks
└── package.json         # For npm dependencies
```

## Plugin Manifest (plugin.json)

The manifest defines your plugin's capabilities:

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "description": "A helpful description",
  "author": {
    "name": "Your Name",
    "url": "https://example.com"
  },
  "license": "MIT",

  "bot-hq": {
    "minVersion": "1.0.0"
  },

  "mcp": {
    "entry": "./dist/server.js",
    "transport": "stdio",
    "tools": ["tool_name"]
  },

  "extensions": "./dist/extensions.js",

  "settings": {
    "mySetting": {
      "type": "boolean",
      "label": "Enable feature",
      "description": "Toggle this feature on/off",
      "default": false
    }
  },

  "credentials": {
    "API_KEY": {
      "type": "secret",
      "label": "API Key",
      "description": "Your API key",
      "required": true
    }
  },

  "permissions": [
    "task:read",
    "task:write"
  ]
}
```

### Manifest Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Lowercase alphanumeric with hyphens only |
| `version` | Yes | Semantic version string |
| `description` | No | Human-readable description |
| `author` | No | Author info with name and optional url |
| `bot-hq.minVersion` | No | Minimum Bot-HQ version required |
| `mcp` | No | MCP server configuration |
| `extensions` | No | Path to extensions module |
| `settings` | No | Plugin settings definitions |
| `credentials` | No | Credential definitions |
| `permissions` | No | Required permissions |

### Settings Types

```json
{
  "settings": {
    "text": {
      "type": "string",
      "label": "Text Setting",
      "default": "default value"
    },
    "number": {
      "type": "number",
      "label": "Number Setting",
      "default": 42
    },
    "toggle": {
      "type": "boolean",
      "label": "Boolean Setting",
      "default": true
    },
    "choice": {
      "type": "select",
      "label": "Select Setting",
      "options": ["option1", "option2", "option3"],
      "default": "option1"
    }
  }
}
```

## MCP Server Implementation

MCP servers extend agent capabilities with new tools. Create a TypeScript server:

```typescript
// src/server.ts
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const server = new Server(
  { name: "my-plugin", version: "1.0.0" },
  { capabilities: { tools: {} } }
);

// Define your tools
server.setRequestHandler("tools/list", async () => ({
  tools: [
    {
      name: "my_tool",
      description: "Does something useful",
      inputSchema: {
        type: "object",
        properties: {
          param: { type: "string", description: "A parameter" }
        },
        required: ["param"]
      }
    }
  ]
}));

// Handle tool calls
server.setRequestHandler("tools/call", async (request) => {
  const { name, arguments: args } = request.params;

  if (name === "my_tool") {
    const result = doSomething(args.param);
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
  }

  throw new Error(`Unknown tool: ${name}`);
});

// Start the server
const transport = new StdioServerTransport();
await server.connect(transport);
```

### Accessing Credentials

Credentials are passed as environment variables:

```typescript
const apiKey = process.env.API_KEY;
if (!apiKey) {
  throw new Error("API_KEY not configured");
}
```

## Extensions (Actions & Hooks)

Extensions add actions to Bot-HQ UI and hook into lifecycle events:

```typescript
// src/extensions.ts
import type { PluginContext, PluginExtensions } from "./plugin-types.js";

export default function(ctx: PluginContext): PluginExtensions {
  return {
    actions: {
      // Actions on approval screen
      approval: [
        {
          id: "my-action",
          label: "My Action",
          description: "Does something on approval",
          icon: "zap",  // Lucide icon name
          defaultChecked: true,
          handler: async (context) => {
            const { approval, task, workspace, pluginContext } = context;

            try {
              // Your logic here
              return { success: true, message: "Action completed" };
            } catch (error) {
              return { success: false, error: error.message };
            }
          }
        }
      ],

      // Actions on task cards
      task: [
        {
          id: "task-action",
          label: "Task Action",
          handler: async (context) => {
            return { success: true, message: "Done" };
          }
        }
      ]
    },

    hooks: {
      onTaskCreated: async (task) => {
        ctx.log.info(`New task: ${task.title}`);
      },

      onApprovalAccepted: async (approval, task) => {
        ctx.log.info(`Approval accepted: ${task.title}`);
      }
    }
  };
}
```

### Action Context

Actions receive a context object:

```typescript
interface ActionContext {
  approval?: {
    id: number;
    branchName: string;
    baseBranch: string;
    commitMessages: string[];
  };
  task?: {
    id: number;
    title: string;
    description: string;
  };
  workspace?: {
    id: number;
    name: string;
    repoPath: string;
  };
  pluginContext: PluginContext;
}
```

### Action Results

Return structured results:

```typescript
interface ActionResult {
  success: boolean;
  message?: string;
  error?: string;
  data?: unknown;
}
```

### Available Hooks

| Hook | When Called |
|------|-------------|
| `onTaskCreated` | New task is created |
| `onTaskUpdated` | Task is modified |
| `onAgentStart` | Agent session starts |
| `onAgentComplete` | Agent session completes |
| `onApprovalCreated` | New approval is created |
| `onApprovalAccepted` | Approval is accepted |
| `onApprovalRejected` | Approval is rejected |

## Plugin Context API

The plugin context provides access to Bot-HQ features:

### MCP Tool Calls

Call your MCP server tools:

```typescript
const result = await ctx.mcp.call("my_tool", { param: "value" });
```

### Data Storage

**Plugin-level storage:**
```typescript
await ctx.store.set("key", { data: "value" });
const data = await ctx.store.get("key");
await ctx.store.delete("key");
```

**Workspace-level storage:**
```typescript
await ctx.workspaceData.set(workspaceId, { config: "value" });
const config = await ctx.workspaceData.get(workspaceId);
```

**Task-level storage:**
```typescript
await ctx.taskData.set(taskId, { status: "pending" });
const status = await ctx.taskData.get(taskId);
```

### Settings and Credentials

```typescript
// Settings (as defined in manifest)
const autoSync = ctx.settings.autoSync;

// Credentials (for use in extensions, not MCP server)
const apiKey = ctx.credentials.API_KEY;
```

### Logging

```typescript
ctx.log.info("Informational message");
ctx.log.warn("Warning message");
ctx.log.error("Error message");
```

## Development Workflow

### 1. Create Plugin Structure

```bash
mkdir -p ~/.bot-hq/plugins/my-plugin
cd ~/.bot-hq/plugins/my-plugin
npm init -y
```

### 2. Install Dependencies

```bash
npm install @modelcontextprotocol/sdk
npm install -D typescript tsx
```

### 3. Configure TypeScript

```json
// tsconfig.json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "outDir": "./dist",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true
  },
  "include": ["src/**/*"]
}
```

### 4. Build and Test

```bash
npm run build
# Restart Bot-HQ to load the plugin
```

### 5. View Logs

Plugin logs appear in the Bot-HQ console and can be viewed in the UI.

## Best Practices

### Error Handling

- Always wrap async operations in try/catch
- Return meaningful error messages
- Don't expose sensitive data in errors

```typescript
try {
  const result = await riskyOperation();
  return { success: true, data: result };
} catch (error) {
  return {
    success: false,
    error: error instanceof Error ? error.message : "Unknown error"
  };
}
```

### Graceful Degradation

- Check for required configuration before operations
- Provide helpful error messages for missing setup

```typescript
const config = await pluginContext.workspaceData.get(workspace.id);
if (!config?.apiEndpoint) {
  return {
    success: false,
    error: "API endpoint not configured. Go to Settings > Workspaces."
  };
}
```

### Performance

- Avoid blocking operations in hooks
- Use appropriate timeouts for external calls
- Cache data when appropriate

### Security

- Never log credentials
- Validate input parameters
- Use the permissions system appropriately

## Installation

To install a plugin, copy it to the plugins directory:

```bash
cp -r my-plugin ~/.bot-hq/plugins/
```

Or create an install script:

```bash
#!/bin/bash
PLUGIN_DIR="$HOME/.bot-hq/plugins/my-plugin"
mkdir -p "$PLUGIN_DIR"
cp -r dist "$PLUGIN_DIR/"
cp plugin.json "$PLUGIN_DIR/"
cp package.json "$PLUGIN_DIR/"
echo "Plugin installed!"
```

After installation, restart Bot-HQ and enable the plugin in the Plugins page.

## Example: GitHub Plugin

See `plugins/github/` in the Bot-HQ repository for a complete example that demonstrates:

- MCP server with multiple tools
- Approval action for creating PRs
- Task action for viewing on GitHub
- Workspace-level configuration storage
- Credential management for GitHub tokens
