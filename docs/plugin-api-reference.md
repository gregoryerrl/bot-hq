# Plugin API Reference

Complete API reference for Bot-HQ plugins.

## Table of Contents

- [Plugin Manifest](#plugin-manifest)
- [PluginContext Interface](#plugincontext-interface)
- [Extensions API](#extensions-api)
- [Hook Signatures](#hook-signatures)
- [Action Handlers](#action-handlers)
- [MCP Tool Format](#mcp-tool-format)

---

## Plugin Manifest

### PluginManifest

```typescript
interface PluginManifest {
  name: string;                    // Required: lowercase alphanumeric with hyphens
  version: string;                 // Required: semantic version
  description?: string;            // Human-readable description
  author?: {
    name: string;
    url?: string;
  };
  repository?: string;             // URL to source repository
  license?: string;                // SPDX license identifier

  "bot-hq"?: {
    minVersion?: string;           // Minimum Bot-HQ version
  };

  mcp?: McpConfig;                 // MCP server configuration
  extensions?: string;             // Path to extensions module
  ui?: UiConfig;                   // UI contributions
  settings?: Record<string, PluginSettingDefinition>;
  credentials?: Record<string, PluginCredentialDefinition>;
  permissions?: string[];          // Required permissions
}
```

### McpConfig

```typescript
interface McpConfig {
  entry: string;                   // Path to MCP server entry file
  transport: "stdio";              // Only stdio is supported
  tools?: string[];                // List of tool names (for documentation)
}
```

### UiConfig

```typescript
interface UiConfig {
  tabs?: PluginTabDefinition[];    // Sidebar tabs
  workspaceSettings?: string;      // Workspace settings component
  taskBadge?: string;              // Task card badge component
  taskActions?: string;            // Task card actions component
}
```

### PluginTabDefinition

```typescript
interface PluginTabDefinition {
  id: string;                      // Unique tab identifier
  label: string;                   // Display label
  icon: string;                    // Lucide icon name
  component: string;               // Component path
}
```

### PluginSettingDefinition

```typescript
interface PluginSettingDefinition {
  type: "string" | "number" | "boolean" | "select";
  label: string;                   // Display label
  description?: string;            // Help text
  default?: string | number | boolean;
  options?: string[];              // For select type only
}
```

### PluginCredentialDefinition

```typescript
interface PluginCredentialDefinition {
  type: "secret";                  // Always "secret"
  label: string;                   // Display label
  description?: string;            // Help text
  required?: boolean;              // Whether credential is required
}
```

---

## PluginContext Interface

The context object provided to extensions for interacting with Bot-HQ.

```typescript
interface PluginContext {
  mcp: McpContext;
  store: StoreContext;
  workspaceData: WorkspaceDataContext;
  taskData: TaskDataContext;
  settings: Record<string, unknown>;
  credentials: Record<string, string>;
  log: LogContext;
}
```

### McpContext

Call MCP server tools:

```typescript
interface McpContext {
  call(tool: string, params: Record<string, unknown>): Promise<unknown>;
}
```

**Example:**
```typescript
const result = await ctx.mcp.call("github_sync_issues", {
  owner: "acme",
  repo: "widgets"
});
```

### StoreContext

Plugin-level persistent storage:

```typescript
interface StoreContext {
  get(key: string): Promise<unknown>;
  set(key: string, value: unknown): Promise<void>;
  delete(key: string): Promise<void>;
}
```

**Example:**
```typescript
await ctx.store.set("lastSync", Date.now());
const lastSync = await ctx.store.get("lastSync");
await ctx.store.delete("lastSync");
```

### WorkspaceDataContext

Per-workspace storage:

```typescript
interface WorkspaceDataContext {
  get(workspaceId: number): Promise<unknown>;
  set(workspaceId: number, data: unknown): Promise<void>;
}
```

**Example:**
```typescript
await ctx.workspaceData.set(workspace.id, {
  owner: "acme",
  repo: "widgets"
});
const config = await ctx.workspaceData.get(workspace.id);
```

### TaskDataContext

Per-task storage:

```typescript
interface TaskDataContext {
  get(taskId: number): Promise<unknown>;
  set(taskId: number, data: unknown): Promise<void>;
}
```

**Example:**
```typescript
await ctx.taskData.set(task.id, { prNumber: 42 });
const data = await ctx.taskData.get(task.id);
```

### LogContext

Structured logging:

```typescript
interface LogContext {
  info(msg: string): void;
  warn(msg: string): void;
  error(msg: string): void;
}
```

**Example:**
```typescript
ctx.log.info("Processing started");
ctx.log.warn("Rate limit approaching");
ctx.log.error("API call failed");
```

---

## Extensions API

### PluginExtensions

The shape returned by extension modules:

```typescript
interface PluginExtensions {
  actions?: {
    approval?: PluginAction[];     // Actions on approval screen
    task?: PluginAction[];         // Actions on task cards
    workspace?: PluginAction[];    // Actions on workspace
  };
  hooks?: PluginHooks;
}
```

### Extension Module Format

Extensions must export a default function:

```typescript
// extensions.ts
import type { PluginContext, PluginExtensions } from "./plugin-types.js";

export default function(ctx: PluginContext): PluginExtensions {
  return {
    actions: { /* ... */ },
    hooks: { /* ... */ }
  };
}
```

---

## Action Handlers

### PluginAction

```typescript
interface PluginAction {
  id: string;                      // Unique action identifier
  label: string;                   // Display label
  description?: string | ((context: ActionContext) => string);
  icon?: string;                   // Lucide icon name
  defaultChecked?: boolean;        // For checkbox actions
  handler: (context: ActionContext) => Promise<ActionResult>;
}
```

### ActionContext

Context passed to action handlers:

```typescript
interface ActionContext {
  approval?: {
    id: number;
    branchName: string;
    baseBranch: string;
    commitMessages: string[];
    diffSummary?: unknown;
  };
  task?: {
    id: number;
    title: string;
    description: string;
    state: string;
  };
  workspace?: {
    id: number;
    name: string;
    repoPath: string;
  };
  pluginContext: PluginContext;
}
```

### ActionResult

Return value from action handlers:

```typescript
interface ActionResult {
  success: boolean;                // Whether action succeeded
  message?: string;                // Success message to display
  error?: string;                  // Error message to display
  data?: unknown;                  // Optional return data
}
```

**Example Handler:**
```typescript
{
  id: "my-action",
  label: "Do Something",
  handler: async (context: ActionContext): Promise<ActionResult> => {
    const { task, pluginContext } = context;

    if (!task) {
      return { success: false, error: "No task context" };
    }

    try {
      await pluginContext.mcp.call("my_tool", { taskId: task.id });
      return { success: true, message: "Action completed" };
    } catch (error) {
      return {
        success: false,
        error: error instanceof Error ? error.message : "Unknown error"
      };
    }
  }
}
```

---

## Hook Signatures

### PluginHooks

```typescript
interface PluginHooks {
  onTaskCreated?: (task: TaskHookData) => Promise<void>;
  onTaskUpdated?: (task: TaskHookData, changes: Partial<TaskHookData>) => Promise<void>;
  onAgentStart?: (agent: AgentHookData, task: TaskHookData) => Promise<{ context?: string } | void>;
  onAgentComplete?: (agent: AgentHookData, task: TaskHookData) => Promise<void>;
  onApprovalCreated?: (approval: ApprovalHookData) => Promise<void>;
  onApprovalAccepted?: (approval: ApprovalHookData, task: TaskHookData) => Promise<void>;
  onApprovalRejected?: (approval: ApprovalHookData, task: TaskHookData) => Promise<void>;
}
```

### TaskHookData

```typescript
interface TaskHookData {
  id: number;
  workspaceId: number;
  title: string;
  description: string;
  state: string;                   // "new" | "queued" | "in_progress" | "pending_review" | "pr_created" | "done"
  priority: number;
  branchName?: string;
}
```

### AgentHookData

```typescript
interface AgentHookData {
  sessionId: number;
  workspaceId: number;
  taskId: number;
  status: string;
}
```

### ApprovalHookData

```typescript
interface ApprovalHookData {
  id: number;
  taskId: number;
  workspaceId: number;
  branchName: string;
  baseBranch: string;
  commitMessages: string[];
  status: string;                  // "pending" | "approved" | "rejected"
}
```

**Example Hooks:**
```typescript
hooks: {
  onTaskCreated: async (task) => {
    ctx.log.info(`Task created: ${task.title}`);
    // Sync with external system
  },

  onAgentStart: async (agent, task) => {
    // Return additional context for the agent
    return {
      context: `Working on: ${task.title}\nPriority: ${task.priority}`
    };
  },

  onApprovalAccepted: async (approval, task) => {
    // Post-approval automation
    ctx.log.info(`Approval accepted for: ${task.title}`);
  }
}
```

---

## MCP Tool Format

### Tool Definition

Tools are defined using JSON Schema:

```typescript
{
  name: "tool_name",
  description: "What this tool does",
  inputSchema: {
    type: "object",
    properties: {
      param1: {
        type: "string",
        description: "First parameter"
      },
      param2: {
        type: "number",
        description: "Second parameter"
      },
      optional: {
        type: "boolean",
        description: "Optional parameter"
      }
    },
    required: ["param1", "param2"]
  }
}
```

### Tool Response Format

Tools return content arrays:

```typescript
{
  content: [
    {
      type: "text",
      text: JSON.stringify(result)
    }
  ]
}
```

**Example MCP Server:**
```typescript
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const server = new Server(
  { name: "my-plugin", version: "1.0.0" },
  { capabilities: { tools: {} } }
);

server.setRequestHandler("tools/list", async () => ({
  tools: [
    {
      name: "my_tool",
      description: "Performs an operation",
      inputSchema: {
        type: "object",
        properties: {
          input: { type: "string", description: "Input value" }
        },
        required: ["input"]
      }
    }
  ]
}));

server.setRequestHandler("tools/call", async (request) => {
  const { name, arguments: args } = request.params;

  if (name === "my_tool") {
    const result = processInput(args.input);
    return {
      content: [{ type: "text", text: JSON.stringify(result) }]
    };
  }

  throw new Error(`Unknown tool: ${name}`);
});

const transport = new StdioServerTransport();
await server.connect(transport);
```

---

## Permissions

Available permissions:

| Permission | Description |
|------------|-------------|
| `workspace:read` | Read workspace data |
| `workspace:write` | Modify workspace data |
| `task:read` | Read task data |
| `task:write` | Modify task data |
| `task:create` | Create new tasks |
| `approval:read` | Read approval data |
| `approval:write` | Modify approvals |
| `agent:read` | Read agent sessions |
| `agent:start` | Start agent sessions |
| `agent:stop` | Stop agent sessions |

---

## Error Codes

Standard error codes returned by the plugin action API:

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `INVALID_REQUEST_BODY` | 400 | Malformed JSON in request |
| `MISSING_PLUGIN_NAME` | 400 | pluginName not provided |
| `MISSING_ACTION_ID` | 400 | actionId not provided |
| `MISSING_APPROVAL_ID` | 400 | approvalId not provided |
| `APPROVAL_NOT_FOUND` | 404 | Approval ID doesn't exist |
| `TASK_NOT_FOUND` | 404 | Task doesn't exist |
| `WORKSPACE_NOT_FOUND` | 404 | Workspace doesn't exist |
| `PLUGIN_NOT_FOUND` | 404 | Plugin not installed |
| `PLUGIN_DISABLED` | 400 | Plugin is disabled |
| `ACTION_NOT_FOUND` | 404 | Action doesn't exist |
| `CONTEXT_CREATION_ERROR` | 500 | Failed to create plugin context |
| `ACTIONS_RETRIEVAL_ERROR` | 500 | Failed to get plugin actions |
| `ACTION_EXECUTION_ERROR` | 500 | Action handler threw error |
| `ACTION_TIMEOUT` | 504 | Action exceeded timeout |
| `INVALID_ACTION_RESULT` | 500 | Handler returned invalid result |
| `UNEXPECTED_ERROR` | 500 | Unhandled exception |
