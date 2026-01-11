# Plugin System Phase 1: Core Infrastructure

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the plugin runtime that can load, register, and execute plugins from `~/.bot-hq/plugins/`

**Architecture:** Directory-based plugins with JSON manifests. Plugin runtime scans directories on startup, validates manifests, and registers MCP servers. Plugins store data in scoped database tables.

**Tech Stack:** Next.js 16, Drizzle ORM, SQLite, TypeScript, Node.js child_process for MCP servers

---

## Task 1: Database Schema - Plugin Tables

**Files:**
- Modify: `src/lib/db/schema.ts`
- Create: `src/lib/db/migrations/001_plugin_tables.sql` (for reference)

**Step 1: Add plugins table to schema**

Add after the `pendingDevices` table definition in `src/lib/db/schema.ts`:

```typescript
// Installed plugins
export const plugins = sqliteTable("plugins", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull().unique(),
  version: text("version").notNull(),
  enabled: integer("enabled", { mode: "boolean" }).notNull().default(true),
  manifest: text("manifest").notNull(), // Full plugin.json cached
  settings: text("settings").notNull().default("{}"), // User-configured settings
  credentials: text("credentials"), // Encrypted secrets
  installedAt: integer("installed_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});
```

**Step 2: Add plugin_workspace_data table**

```typescript
// Plugin data scoped to workspace
export const pluginWorkspaceData = sqliteTable("plugin_workspace_data", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  pluginId: integer("plugin_id").notNull().references(() => plugins.id, { onDelete: "cascade" }),
  workspaceId: integer("workspace_id").notNull().references(() => workspaces.id, { onDelete: "cascade" }),
  data: text("data").notNull().default("{}"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("plugin_workspace_plugin_idx").on(table.pluginId),
  index("plugin_workspace_workspace_idx").on(table.workspaceId),
]);
```

**Step 3: Add plugin_task_data table**

```typescript
// Plugin data scoped to task
export const pluginTaskData = sqliteTable("plugin_task_data", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  pluginId: integer("plugin_id").notNull().references(() => plugins.id, { onDelete: "cascade" }),
  taskId: integer("task_id").notNull().references(() => tasks.id, { onDelete: "cascade" }),
  data: text("data").notNull().default("{}"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("plugin_task_plugin_idx").on(table.pluginId),
  index("plugin_task_task_idx").on(table.taskId),
]);
```

**Step 4: Add plugin_store table**

```typescript
// Plugin global key-value store
export const pluginStore = sqliteTable("plugin_store", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  pluginId: integer("plugin_id").notNull().references(() => plugins.id, { onDelete: "cascade" }),
  key: text("key").notNull(),
  value: text("value"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("plugin_store_plugin_idx").on(table.pluginId),
  index("plugin_store_key_idx").on(table.pluginId, table.key),
]);
```

**Step 5: Add type exports**

```typescript
export type Plugin = typeof plugins.$inferSelect;
export type NewPlugin = typeof plugins.$inferInsert;
export type PluginWorkspaceData = typeof pluginWorkspaceData.$inferSelect;
export type PluginTaskData = typeof pluginTaskData.$inferSelect;
export type PluginStoreEntry = typeof pluginStore.$inferSelect;
```

**Step 6: Update tasks table - add source fields**

Modify the existing `tasks` table to add plugin source tracking:

```typescript
// In the tasks table definition, add these fields:
  sourcePluginId: integer("source_plugin_id").references(() => plugins.id),
  sourceRef: text("source_ref"), // Plugin-specific reference (issue #, message ID)
```

**Step 7: Run build to verify schema compiles**

Run: `npm run build`
Expected: Build succeeds with no type errors

**Step 8: Commit**

```bash
git add src/lib/db/schema.ts
git commit -m "feat(db): add plugin tables and task source fields"
```

---

## Task 2: Plugin Types

**Files:**
- Create: `src/lib/plugins/types.ts`

**Step 1: Create plugin manifest types**

```typescript
// src/lib/plugins/types.ts

export interface PluginManifest {
  name: string;
  version: string;
  description: string;
  author?: {
    name: string;
    url?: string;
  };
  repository?: string;
  license?: string;

  "bot-hq"?: {
    minVersion?: string;
  };

  mcp?: {
    entry: string;
    transport: "stdio";
    tools?: string[];
  };

  extensions?: string;

  ui?: {
    tabs?: PluginTabDefinition[];
    workspaceSettings?: string;
    taskBadge?: string;
    taskActions?: string;
  };

  settings?: Record<string, PluginSettingDefinition>;
  credentials?: Record<string, PluginCredentialDefinition>;
  permissions?: string[];
}

export interface PluginTabDefinition {
  id: string;
  label: string;
  icon: string;
  component: string;
}

export interface PluginSettingDefinition {
  type: "string" | "number" | "boolean" | "select";
  label: string;
  description?: string;
  default?: string | number | boolean;
  options?: string[]; // For select type
}

export interface PluginCredentialDefinition {
  type: "secret";
  label: string;
  description?: string;
  required?: boolean;
}

export interface LoadedPlugin {
  name: string;
  version: string;
  path: string;
  manifest: PluginManifest;
  enabled: boolean;
  dbId?: number;
}

export interface PluginContext {
  mcp: {
    call: (tool: string, params: Record<string, unknown>) => Promise<unknown>;
  };
  store: {
    get: (key: string) => Promise<unknown>;
    set: (key: string, value: unknown) => Promise<void>;
    delete: (key: string) => Promise<void>;
  };
  workspaceData: {
    get: (workspaceId: number) => Promise<unknown>;
    set: (workspaceId: number, data: unknown) => Promise<void>;
  };
  taskData: {
    get: (taskId: number) => Promise<unknown>;
    set: (taskId: number, data: unknown) => Promise<void>;
  };
  settings: Record<string, unknown>;
  credentials: Record<string, string>;
  log: {
    info: (msg: string) => void;
    warn: (msg: string) => void;
    error: (msg: string) => void;
  };
}

export interface PluginAction {
  id: string;
  label: string;
  description?: string | ((context: ActionContext) => string);
  icon?: string;
  defaultChecked?: boolean;
  handler: (context: ActionContext) => Promise<ActionResult>;
}

export interface ActionContext {
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

export interface ActionResult {
  success: boolean;
  message?: string;
  error?: string;
  data?: unknown;
}

export interface PluginHooks {
  onTaskCreated?: (task: TaskHookData) => Promise<void>;
  onTaskUpdated?: (task: TaskHookData, changes: Partial<TaskHookData>) => Promise<void>;
  onAgentStart?: (agent: AgentHookData, task: TaskHookData) => Promise<{ context?: string } | void>;
  onAgentComplete?: (agent: AgentHookData, task: TaskHookData) => Promise<void>;
  onApprovalCreated?: (approval: ApprovalHookData) => Promise<void>;
  onApprovalAccepted?: (approval: ApprovalHookData, task: TaskHookData) => Promise<void>;
  onApprovalRejected?: (approval: ApprovalHookData, task: TaskHookData) => Promise<void>;
}

export interface PluginExtensions {
  actions?: {
    approval?: PluginAction[];
    task?: PluginAction[];
    workspace?: PluginAction[];
  };
  hooks?: PluginHooks;
}

export interface TaskHookData {
  id: number;
  workspaceId: number;
  title: string;
  description: string;
  state: string;
  priority: number;
  branchName?: string;
}

export interface AgentHookData {
  sessionId: number;
  workspaceId: number;
  taskId: number;
  status: string;
}

export interface ApprovalHookData {
  id: number;
  taskId: number;
  workspaceId: number;
  branchName: string;
  baseBranch: string;
  commitMessages: string[];
  status: string;
}
```

**Step 2: Run build to verify types**

Run: `npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/lib/plugins/types.ts
git commit -m "feat(plugins): add plugin type definitions"
```

---

## Task 3: Plugin Loader

**Files:**
- Create: `src/lib/plugins/loader.ts`

**Step 1: Create the plugin loader**

```typescript
// src/lib/plugins/loader.ts

import { readdir, readFile, access, stat } from "fs/promises";
import { join } from "path";
import { homedir } from "os";
import { LoadedPlugin, PluginManifest } from "./types";

const PLUGINS_DIR = join(homedir(), ".bot-hq", "plugins");

export async function getPluginsDirectory(): Promise<string> {
  return PLUGINS_DIR;
}

export async function ensurePluginsDirectory(): Promise<void> {
  const { mkdir } = await import("fs/promises");
  await mkdir(PLUGINS_DIR, { recursive: true });
}

export async function loadPluginManifest(pluginPath: string): Promise<PluginManifest | null> {
  const manifestPath = join(pluginPath, "plugin.json");

  try {
    await access(manifestPath);
    const content = await readFile(manifestPath, "utf-8");
    const manifest = JSON.parse(content) as PluginManifest;

    // Basic validation
    if (!manifest.name || !manifest.version) {
      console.error(`Invalid manifest at ${manifestPath}: missing name or version`);
      return null;
    }

    return manifest;
  } catch (error) {
    console.error(`Failed to load manifest from ${pluginPath}:`, error);
    return null;
  }
}

export async function discoverPlugins(): Promise<LoadedPlugin[]> {
  await ensurePluginsDirectory();

  const plugins: LoadedPlugin[] = [];

  try {
    const entries = await readdir(PLUGINS_DIR, { withFileTypes: true });

    for (const entry of entries) {
      if (!entry.isDirectory()) continue;

      const pluginPath = join(PLUGINS_DIR, entry.name);
      const manifest = await loadPluginManifest(pluginPath);

      if (manifest) {
        plugins.push({
          name: manifest.name,
          version: manifest.version,
          path: pluginPath,
          manifest,
          enabled: true, // Default to enabled, will be updated from DB
        });
      }
    }
  } catch (error) {
    console.error("Failed to discover plugins:", error);
  }

  return plugins;
}

export async function validateManifest(manifest: PluginManifest): Promise<string[]> {
  const errors: string[] = [];

  if (!manifest.name) {
    errors.push("Missing required field: name");
  }

  if (!manifest.version) {
    errors.push("Missing required field: version");
  }

  if (manifest.name && !/^[a-z0-9-]+$/.test(manifest.name)) {
    errors.push("Plugin name must be lowercase alphanumeric with hyphens only");
  }

  if (manifest.mcp) {
    if (!manifest.mcp.entry) {
      errors.push("MCP config missing entry point");
    }
    if (manifest.mcp.transport && manifest.mcp.transport !== "stdio") {
      errors.push("Only stdio transport is supported");
    }
  }

  return errors;
}

export async function pluginExists(name: string): Promise<boolean> {
  const pluginPath = join(PLUGINS_DIR, name);
  try {
    const stats = await stat(pluginPath);
    return stats.isDirectory();
  } catch {
    return false;
  }
}
```

**Step 2: Run build to verify**

Run: `npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/lib/plugins/loader.ts
git commit -m "feat(plugins): add plugin discovery and manifest loading"
```

---

## Task 4: Plugin Registry

**Files:**
- Create: `src/lib/plugins/registry.ts`

**Step 1: Create the plugin registry**

```typescript
// src/lib/plugins/registry.ts

import { eq } from "drizzle-orm";
import { getDb } from "@/lib/db";
import { plugins } from "@/lib/db/schema";
import { LoadedPlugin, PluginManifest } from "./types";
import { discoverPlugins, validateManifest } from "./loader";

class PluginRegistry {
  private plugins: Map<string, LoadedPlugin> = new Map();
  private initialized = false;

  async initialize(): Promise<void> {
    if (this.initialized) return;

    const discovered = await discoverPlugins();
    const db = getDb();

    for (const plugin of discovered) {
      // Validate manifest
      const errors = await validateManifest(plugin.manifest);
      if (errors.length > 0) {
        console.error(`Plugin ${plugin.name} has invalid manifest:`, errors);
        continue;
      }

      // Check if plugin exists in database
      const existing = await db
        .select()
        .from(plugins)
        .where(eq(plugins.name, plugin.name))
        .get();

      if (existing) {
        // Update from database
        plugin.dbId = existing.id;
        plugin.enabled = existing.enabled;

        // Update manifest if version changed
        if (existing.version !== plugin.version) {
          await db
            .update(plugins)
            .set({
              version: plugin.version,
              manifest: JSON.stringify(plugin.manifest),
              updatedAt: new Date(),
            })
            .where(eq(plugins.id, existing.id));
        }
      } else {
        // Insert new plugin
        const result = await db
          .insert(plugins)
          .values({
            name: plugin.name,
            version: plugin.version,
            manifest: JSON.stringify(plugin.manifest),
            enabled: true,
          })
          .returning({ id: plugins.id });

        plugin.dbId = result[0].id;
      }

      this.plugins.set(plugin.name, plugin);
    }

    this.initialized = true;
    console.log(`Plugin registry initialized with ${this.plugins.size} plugins`);
  }

  getPlugin(name: string): LoadedPlugin | undefined {
    return this.plugins.get(name);
  }

  getAllPlugins(): LoadedPlugin[] {
    return Array.from(this.plugins.values());
  }

  getEnabledPlugins(): LoadedPlugin[] {
    return this.getAllPlugins().filter(p => p.enabled);
  }

  async setEnabled(name: string, enabled: boolean): Promise<boolean> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return false;

    const db = getDb();
    await db
      .update(plugins)
      .set({ enabled, updatedAt: new Date() })
      .where(eq(plugins.id, plugin.dbId));

    plugin.enabled = enabled;
    return true;
  }

  async updateSettings(name: string, settings: Record<string, unknown>): Promise<boolean> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return false;

    const db = getDb();
    await db
      .update(plugins)
      .set({ settings: JSON.stringify(settings), updatedAt: new Date() })
      .where(eq(plugins.id, plugin.dbId));

    return true;
  }

  async getSettings(name: string): Promise<Record<string, unknown>> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return {};

    const db = getDb();
    const result = await db
      .select({ settings: plugins.settings })
      .from(plugins)
      .where(eq(plugins.id, plugin.dbId))
      .get();

    return result ? JSON.parse(result.settings) : {};
  }

  async updateCredentials(name: string, credentials: Record<string, string>): Promise<boolean> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return false;

    // TODO: Encrypt credentials before storing
    const db = getDb();
    await db
      .update(plugins)
      .set({ credentials: JSON.stringify(credentials), updatedAt: new Date() })
      .where(eq(plugins.id, plugin.dbId));

    return true;
  }

  async getCredentials(name: string): Promise<Record<string, string>> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return {};

    const db = getDb();
    const result = await db
      .select({ credentials: plugins.credentials })
      .from(plugins)
      .where(eq(plugins.id, plugin.dbId))
      .get();

    // TODO: Decrypt credentials
    return result?.credentials ? JSON.parse(result.credentials) : {};
  }

  isInitialized(): boolean {
    return this.initialized;
  }
}

// Singleton instance
let registryInstance: PluginRegistry | null = null;

export function getPluginRegistry(): PluginRegistry {
  if (!registryInstance) {
    registryInstance = new PluginRegistry();
  }
  return registryInstance;
}

export async function initializePlugins(): Promise<void> {
  const registry = getPluginRegistry();
  await registry.initialize();
}
```

**Step 2: Run build to verify**

Run: `npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/lib/plugins/registry.ts
git commit -m "feat(plugins): add plugin registry with database sync"
```

---

## Task 5: Plugin Data Store

**Files:**
- Create: `src/lib/plugins/store.ts`

**Step 1: Create the plugin data store**

```typescript
// src/lib/plugins/store.ts

import { eq, and } from "drizzle-orm";
import { getDb } from "@/lib/db";
import {
  pluginStore,
  pluginWorkspaceData,
  pluginTaskData,
} from "@/lib/db/schema";

export class PluginDataStore {
  constructor(private pluginId: number) {}

  // Global key-value store
  async get(key: string): Promise<unknown> {
    const db = getDb();
    const result = await db
      .select({ value: pluginStore.value })
      .from(pluginStore)
      .where(
        and(
          eq(pluginStore.pluginId, this.pluginId),
          eq(pluginStore.key, key)
        )
      )
      .get();

    return result?.value ? JSON.parse(result.value) : undefined;
  }

  async set(key: string, value: unknown): Promise<void> {
    const db = getDb();
    const serialized = JSON.stringify(value);

    // Upsert
    const existing = await db
      .select({ id: pluginStore.id })
      .from(pluginStore)
      .where(
        and(
          eq(pluginStore.pluginId, this.pluginId),
          eq(pluginStore.key, key)
        )
      )
      .get();

    if (existing) {
      await db
        .update(pluginStore)
        .set({ value: serialized, updatedAt: new Date() })
        .where(eq(pluginStore.id, existing.id));
    } else {
      await db.insert(pluginStore).values({
        pluginId: this.pluginId,
        key,
        value: serialized,
      });
    }
  }

  async delete(key: string): Promise<void> {
    const db = getDb();
    await db
      .delete(pluginStore)
      .where(
        and(
          eq(pluginStore.pluginId, this.pluginId),
          eq(pluginStore.key, key)
        )
      );
  }

  // Workspace-scoped data
  async getWorkspaceData(workspaceId: number): Promise<unknown> {
    const db = getDb();
    const result = await db
      .select({ data: pluginWorkspaceData.data })
      .from(pluginWorkspaceData)
      .where(
        and(
          eq(pluginWorkspaceData.pluginId, this.pluginId),
          eq(pluginWorkspaceData.workspaceId, workspaceId)
        )
      )
      .get();

    return result?.data ? JSON.parse(result.data) : undefined;
  }

  async setWorkspaceData(workspaceId: number, data: unknown): Promise<void> {
    const db = getDb();
    const serialized = JSON.stringify(data);

    const existing = await db
      .select({ id: pluginWorkspaceData.id })
      .from(pluginWorkspaceData)
      .where(
        and(
          eq(pluginWorkspaceData.pluginId, this.pluginId),
          eq(pluginWorkspaceData.workspaceId, workspaceId)
        )
      )
      .get();

    if (existing) {
      await db
        .update(pluginWorkspaceData)
        .set({ data: serialized, updatedAt: new Date() })
        .where(eq(pluginWorkspaceData.id, existing.id));
    } else {
      await db.insert(pluginWorkspaceData).values({
        pluginId: this.pluginId,
        workspaceId,
        data: serialized,
      });
    }
  }

  // Task-scoped data
  async getTaskData(taskId: number): Promise<unknown> {
    const db = getDb();
    const result = await db
      .select({ data: pluginTaskData.data })
      .from(pluginTaskData)
      .where(
        and(
          eq(pluginTaskData.pluginId, this.pluginId),
          eq(pluginTaskData.taskId, taskId)
        )
      )
      .get();

    return result?.data ? JSON.parse(result.data) : undefined;
  }

  async setTaskData(taskId: number, data: unknown): Promise<void> {
    const db = getDb();
    const serialized = JSON.stringify(data);

    const existing = await db
      .select({ id: pluginTaskData.id })
      .from(pluginTaskData)
      .where(
        and(
          eq(pluginTaskData.pluginId, this.pluginId),
          eq(pluginTaskData.taskId, taskId)
        )
      )
      .get();

    if (existing) {
      await db
        .update(pluginTaskData)
        .set({ data: serialized, updatedAt: new Date() })
        .where(eq(pluginTaskData.id, existing.id));
    } else {
      await db.insert(pluginTaskData).values({
        pluginId: this.pluginId,
        taskId,
        data: serialized,
      });
    }
  }
}
```

**Step 2: Run build to verify**

Run: `npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/lib/plugins/store.ts
git commit -m "feat(plugins): add plugin data store for scoped storage"
```

---

## Task 6: Plugin Context Factory

**Files:**
- Create: `src/lib/plugins/context.ts`

**Step 1: Create the plugin context factory**

```typescript
// src/lib/plugins/context.ts

import { PluginContext, LoadedPlugin } from "./types";
import { PluginDataStore } from "./store";
import { getPluginRegistry } from "./registry";
import { getMcpManager } from "./mcp-manager";

export function createPluginContext(plugin: LoadedPlugin): PluginContext {
  if (!plugin.dbId) {
    throw new Error(`Plugin ${plugin.name} is not registered in database`);
  }

  const store = new PluginDataStore(plugin.dbId);
  const registry = getPluginRegistry();

  return {
    mcp: {
      call: async (tool: string, params: Record<string, unknown>) => {
        const manager = getMcpManager();
        return manager.callTool(plugin.name, tool, params);
      },
    },

    store: {
      get: (key: string) => store.get(key),
      set: (key: string, value: unknown) => store.set(key, value),
      delete: (key: string) => store.delete(key),
    },

    workspaceData: {
      get: (workspaceId: number) => store.getWorkspaceData(workspaceId),
      set: (workspaceId: number, data: unknown) => store.setWorkspaceData(workspaceId, data),
    },

    taskData: {
      get: (taskId: number) => store.getTaskData(taskId),
      set: (taskId: number, data: unknown) => store.setTaskData(taskId, data),
    },

    get settings() {
      // Lazy load settings
      return registry.getSettings(plugin.name) as unknown as Record<string, unknown>;
    },

    get credentials() {
      // Lazy load credentials
      return registry.getCredentials(plugin.name) as unknown as Record<string, string>;
    },

    log: {
      info: (msg: string) => console.log(`[${plugin.name}] INFO:`, msg),
      warn: (msg: string) => console.warn(`[${plugin.name}] WARN:`, msg),
      error: (msg: string) => console.error(`[${plugin.name}] ERROR:`, msg),
    },
  };
}
```

**Step 2: Run build to verify**

Run: `npm run build`
Expected: Build may fail due to missing mcp-manager - that's okay, we'll create it next

**Step 3: Commit (if build passes) or continue to next task**

```bash
git add src/lib/plugins/context.ts
git commit -m "feat(plugins): add plugin context factory"
```

---

## Task 7: MCP Server Manager

**Files:**
- Create: `src/lib/plugins/mcp-manager.ts`

**Step 1: Create the MCP server manager**

```typescript
// src/lib/plugins/mcp-manager.ts

import { spawn, ChildProcess } from "child_process";
import { join } from "path";
import { LoadedPlugin } from "./types";
import { getPluginRegistry } from "./registry";

interface McpServer {
  plugin: LoadedPlugin;
  process: ChildProcess | null;
  status: "stopped" | "starting" | "running" | "error";
  pendingCalls: Map<string, {
    resolve: (value: unknown) => void;
    reject: (error: Error) => void;
  }>;
  callId: number;
}

class McpManager {
  private servers: Map<string, McpServer> = new Map();

  async startServer(plugin: LoadedPlugin): Promise<void> {
    if (!plugin.manifest.mcp) {
      throw new Error(`Plugin ${plugin.name} has no MCP configuration`);
    }

    const existing = this.servers.get(plugin.name);
    if (existing?.status === "running") {
      return;
    }

    const server: McpServer = {
      plugin,
      process: null,
      status: "starting",
      pendingCalls: new Map(),
      callId: 0,
    };
    this.servers.set(plugin.name, server);

    const entryPath = join(plugin.path, plugin.manifest.mcp.entry);

    // Get credentials from registry to pass as env
    const registry = getPluginRegistry();
    const credentials = await registry.getCredentials(plugin.name);

    const env = {
      ...process.env,
      ...credentials,
    };

    // Spawn the MCP server process
    const child = spawn("npx", ["tsx", entryPath], {
      cwd: plugin.path,
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });

    server.process = child;

    let buffer = "";

    child.stdout?.on("data", (data) => {
      buffer += data.toString();

      // Process complete JSON-RPC messages
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const message = JSON.parse(line);
          this.handleMessage(plugin.name, message);
        } catch (e) {
          console.error(`[${plugin.name}] Failed to parse MCP message:`, line);
        }
      }
    });

    child.stderr?.on("data", (data) => {
      console.error(`[${plugin.name}] MCP stderr:`, data.toString());
    });

    child.on("exit", (code) => {
      console.log(`[${plugin.name}] MCP server exited with code ${code}`);
      server.status = "stopped";
      server.process = null;

      // Reject any pending calls
      for (const [id, { reject }] of server.pendingCalls) {
        reject(new Error(`MCP server exited with code ${code}`));
      }
      server.pendingCalls.clear();
    });

    child.on("error", (error) => {
      console.error(`[${plugin.name}] MCP server error:`, error);
      server.status = "error";
    });

    // Wait for server to be ready (simplified - real impl would wait for initialize response)
    await new Promise(resolve => setTimeout(resolve, 1000));
    server.status = "running";

    console.log(`[${plugin.name}] MCP server started`);
  }

  async stopServer(pluginName: string): Promise<void> {
    const server = this.servers.get(pluginName);
    if (!server?.process) return;

    server.process.kill("SIGTERM");
    server.status = "stopped";
    server.process = null;
  }

  async callTool(
    pluginName: string,
    tool: string,
    params: Record<string, unknown>
  ): Promise<unknown> {
    const server = this.servers.get(pluginName);

    if (!server) {
      // Try to start the server
      const plugin = getPluginRegistry().getPlugin(pluginName);
      if (!plugin) {
        throw new Error(`Plugin ${pluginName} not found`);
      }
      await this.startServer(plugin);
      return this.callTool(pluginName, tool, params);
    }

    if (server.status !== "running") {
      throw new Error(`MCP server for ${pluginName} is not running`);
    }

    const id = String(++server.callId);

    const request = {
      jsonrpc: "2.0",
      id,
      method: "tools/call",
      params: {
        name: tool,
        arguments: params,
      },
    };

    return new Promise((resolve, reject) => {
      server.pendingCalls.set(id, { resolve, reject });
      server.process?.stdin?.write(JSON.stringify(request) + "\n");

      // Timeout after 30 seconds
      setTimeout(() => {
        if (server.pendingCalls.has(id)) {
          server.pendingCalls.delete(id);
          reject(new Error(`MCP call to ${tool} timed out`));
        }
      }, 30000);
    });
  }

  private handleMessage(pluginName: string, message: { id?: string; result?: unknown; error?: { message: string } }): void {
    const server = this.servers.get(pluginName);
    if (!server) return;

    if (message.id) {
      const pending = server.pendingCalls.get(message.id);
      if (pending) {
        server.pendingCalls.delete(message.id);
        if (message.error) {
          pending.reject(new Error(message.error.message));
        } else {
          pending.resolve(message.result);
        }
      }
    }
  }

  getServerStatus(pluginName: string): string {
    return this.servers.get(pluginName)?.status || "not_loaded";
  }

  async stopAll(): Promise<void> {
    for (const [name] of this.servers) {
      await this.stopServer(name);
    }
  }
}

// Singleton instance
let managerInstance: McpManager | null = null;

export function getMcpManager(): McpManager {
  if (!managerInstance) {
    managerInstance = new McpManager();
  }
  return managerInstance;
}
```

**Step 2: Run build to verify**

Run: `npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/lib/plugins/mcp-manager.ts
git commit -m "feat(plugins): add MCP server manager for plugin tool calls"
```

---

## Task 8: Plugin Events System

**Files:**
- Create: `src/lib/plugins/events.ts`

**Step 1: Create the plugin events system**

```typescript
// src/lib/plugins/events.ts

import { getPluginRegistry } from "./registry";
import { createPluginContext } from "./context";
import {
  PluginHooks,
  PluginExtensions,
  TaskHookData,
  AgentHookData,
  ApprovalHookData,
} from "./types";

type HookName = keyof PluginHooks;

class PluginEvents {
  private extensionsCache: Map<string, PluginExtensions> = new Map();

  private async loadExtensions(pluginName: string): Promise<PluginExtensions | null> {
    if (this.extensionsCache.has(pluginName)) {
      return this.extensionsCache.get(pluginName)!;
    }

    const registry = getPluginRegistry();
    const plugin = registry.getPlugin(pluginName);

    if (!plugin || !plugin.manifest.extensions) {
      return null;
    }

    try {
      // Dynamic import of extensions module
      const extensionsPath = `${plugin.path}/${plugin.manifest.extensions}`;
      const module = await import(extensionsPath);
      const context = createPluginContext(plugin);
      const extensions = module.default(context) as PluginExtensions;

      this.extensionsCache.set(pluginName, extensions);
      return extensions;
    } catch (error) {
      console.error(`Failed to load extensions for ${pluginName}:`, error);
      return null;
    }
  }

  async fireHook<T extends HookName>(
    hookName: T,
    ...args: Parameters<NonNullable<PluginHooks[T]>>
  ): Promise<Map<string, Awaited<ReturnType<NonNullable<PluginHooks[T]>>>>> {
    const results = new Map<string, Awaited<ReturnType<NonNullable<PluginHooks[T]>>>>();
    const registry = getPluginRegistry();
    const enabledPlugins = registry.getEnabledPlugins();

    await Promise.all(
      enabledPlugins.map(async (plugin) => {
        try {
          const extensions = await this.loadExtensions(plugin.name);
          const hook = extensions?.hooks?.[hookName];

          if (hook) {
            // @ts-expect-error - TypeScript can't narrow the args type properly
            const result = await hook(...args);
            results.set(plugin.name, result);
          }
        } catch (error) {
          console.error(`Hook ${hookName} failed for plugin ${plugin.name}:`, error);
        }
      })
    );

    return results;
  }

  async getApprovalActions(): Promise<Array<{
    pluginName: string;
    action: PluginExtensions["actions"] extends { approval: infer A } ? A extends Array<infer T> ? T : never : never;
  }>> {
    const actions: Array<{
      pluginName: string;
      action: NonNullable<NonNullable<PluginExtensions["actions"]>["approval"]>[number];
    }> = [];

    const registry = getPluginRegistry();
    const enabledPlugins = registry.getEnabledPlugins();

    for (const plugin of enabledPlugins) {
      const extensions = await this.loadExtensions(plugin.name);
      const pluginActions = extensions?.actions?.approval || [];

      for (const action of pluginActions) {
        actions.push({ pluginName: plugin.name, action });
      }
    }

    return actions;
  }

  async getTaskActions(): Promise<Array<{
    pluginName: string;
    action: NonNullable<NonNullable<PluginExtensions["actions"]>["task"]>[number];
  }>> {
    const actions: Array<{
      pluginName: string;
      action: NonNullable<NonNullable<PluginExtensions["actions"]>["task"]>[number];
    }> = [];

    const registry = getPluginRegistry();
    const enabledPlugins = registry.getEnabledPlugins();

    for (const plugin of enabledPlugins) {
      const extensions = await this.loadExtensions(plugin.name);
      const pluginActions = extensions?.actions?.task || [];

      for (const action of pluginActions) {
        actions.push({ pluginName: plugin.name, action });
      }
    }

    return actions;
  }

  clearCache(): void {
    this.extensionsCache.clear();
  }
}

// Singleton instance
let eventsInstance: PluginEvents | null = null;

export function getPluginEvents(): PluginEvents {
  if (!eventsInstance) {
    eventsInstance = new PluginEvents();
  }
  return eventsInstance;
}

// Convenience functions for firing specific hooks
export async function fireTaskCreated(task: TaskHookData): Promise<void> {
  await getPluginEvents().fireHook("onTaskCreated", task);
}

export async function fireTaskUpdated(task: TaskHookData, changes: Partial<TaskHookData>): Promise<void> {
  await getPluginEvents().fireHook("onTaskUpdated", task, changes);
}

export async function fireAgentStart(agent: AgentHookData, task: TaskHookData): Promise<Map<string, { context?: string } | void>> {
  return getPluginEvents().fireHook("onAgentStart", agent, task);
}

export async function fireAgentComplete(agent: AgentHookData, task: TaskHookData): Promise<void> {
  await getPluginEvents().fireHook("onAgentComplete", agent, task);
}

export async function fireApprovalCreated(approval: ApprovalHookData): Promise<void> {
  await getPluginEvents().fireHook("onApprovalCreated", approval);
}

export async function fireApprovalAccepted(approval: ApprovalHookData, task: TaskHookData): Promise<void> {
  await getPluginEvents().fireHook("onApprovalAccepted", approval, task);
}

export async function fireApprovalRejected(approval: ApprovalHookData, task: TaskHookData): Promise<void> {
  await getPluginEvents().fireHook("onApprovalRejected", approval, task);
}
```

**Step 2: Run build to verify**

Run: `npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/lib/plugins/events.ts
git commit -m "feat(plugins): add event system for plugin hooks"
```

---

## Task 9: Plugin Module Index

**Files:**
- Create: `src/lib/plugins/index.ts`

**Step 1: Create the module index**

```typescript
// src/lib/plugins/index.ts

// Types
export * from "./types";

// Core
export { getPluginRegistry, initializePlugins } from "./registry";
export { discoverPlugins, loadPluginManifest, getPluginsDirectory } from "./loader";
export { getMcpManager } from "./mcp-manager";
export { createPluginContext } from "./context";
export { PluginDataStore } from "./store";

// Events
export {
  getPluginEvents,
  fireTaskCreated,
  fireTaskUpdated,
  fireAgentStart,
  fireAgentComplete,
  fireApprovalCreated,
  fireApprovalAccepted,
  fireApprovalRejected,
} from "./events";
```

**Step 2: Run build to verify**

Run: `npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/lib/plugins/index.ts
git commit -m "feat(plugins): add module index for public exports"
```

---

## Task 10: Plugin API Routes

**Files:**
- Create: `src/app/api/plugins/route.ts`
- Create: `src/app/api/plugins/[name]/route.ts`
- Create: `src/app/api/plugins/[name]/settings/route.ts`

**Step 1: Create list/discover plugins route**

```typescript
// src/app/api/plugins/route.ts

import { NextResponse } from "next/server";
import { getPluginRegistry, initializePlugins } from "@/lib/plugins";

export async function GET() {
  try {
    const registry = getPluginRegistry();

    if (!registry.isInitialized()) {
      await initializePlugins();
    }

    const plugins = registry.getAllPlugins().map(p => ({
      name: p.name,
      version: p.version,
      description: p.manifest.description,
      enabled: p.enabled,
      hasUI: !!p.manifest.ui,
      hasMcp: !!p.manifest.mcp,
    }));

    return NextResponse.json({ plugins });
  } catch (error) {
    console.error("Failed to list plugins:", error);
    return NextResponse.json(
      { error: "Failed to list plugins" },
      { status: 500 }
    );
  }
}
```

**Step 2: Create single plugin route**

```typescript
// src/app/api/plugins/[name]/route.ts

import { NextResponse } from "next/server";
import { getPluginRegistry } from "@/lib/plugins";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ name: string }> }
) {
  try {
    const { name } = await params;
    const registry = getPluginRegistry();
    const plugin = registry.getPlugin(name);

    if (!plugin) {
      return NextResponse.json(
        { error: "Plugin not found" },
        { status: 404 }
      );
    }

    const settings = await registry.getSettings(name);

    return NextResponse.json({
      name: plugin.name,
      version: plugin.version,
      description: plugin.manifest.description,
      enabled: plugin.enabled,
      manifest: plugin.manifest,
      settings,
    });
  } catch (error) {
    console.error("Failed to get plugin:", error);
    return NextResponse.json(
      { error: "Failed to get plugin" },
      { status: 500 }
    );
  }
}

export async function PATCH(
  request: Request,
  { params }: { params: Promise<{ name: string }> }
) {
  try {
    const { name } = await params;
    const body = await request.json();
    const registry = getPluginRegistry();

    if (typeof body.enabled === "boolean") {
      await registry.setEnabled(name, body.enabled);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to update plugin:", error);
    return NextResponse.json(
      { error: "Failed to update plugin" },
      { status: 500 }
    );
  }
}
```

**Step 3: Create plugin settings route**

```typescript
// src/app/api/plugins/[name]/settings/route.ts

import { NextResponse } from "next/server";
import { getPluginRegistry } from "@/lib/plugins";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ name: string }> }
) {
  try {
    const { name } = await params;
    const registry = getPluginRegistry();
    const settings = await registry.getSettings(name);

    return NextResponse.json({ settings });
  } catch (error) {
    console.error("Failed to get plugin settings:", error);
    return NextResponse.json(
      { error: "Failed to get settings" },
      { status: 500 }
    );
  }
}

export async function PUT(
  request: Request,
  { params }: { params: Promise<{ name: string }> }
) {
  try {
    const { name } = await params;
    const body = await request.json();
    const registry = getPluginRegistry();

    await registry.updateSettings(name, body.settings || {});

    if (body.credentials) {
      await registry.updateCredentials(name, body.credentials);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to update plugin settings:", error);
    return NextResponse.json(
      { error: "Failed to update settings" },
      { status: 500 }
    );
  }
}
```

**Step 4: Run build to verify**

Run: `npm run build`
Expected: Build succeeds

**Step 5: Commit**

```bash
git add src/app/api/plugins/
git commit -m "feat(api): add plugin management API routes"
```

---

## Task 11: Initialize Plugins on App Start

**Files:**
- Modify: `src/instrumentation.ts`

**Step 1: Add plugin initialization to instrumentation**

Find the instrumentation file and add plugin initialization:

```typescript
// Add to src/instrumentation.ts

import { initializePlugins } from "@/lib/plugins";

// Add to the register function or create one:
export async function register() {
  // Existing initialization...

  // Initialize plugins
  if (process.env.NEXT_RUNTIME === "nodejs") {
    try {
      await initializePlugins();
      console.log("Plugins initialized");
    } catch (error) {
      console.error("Failed to initialize plugins:", error);
    }
  }
}
```

**Step 2: Run build to verify**

Run: `npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/instrumentation.ts
git commit -m "feat: initialize plugins on app startup"
```

---

## Task 12: Final Build and Integration Test

**Step 1: Run full build**

Run: `npm run build`
Expected: Build succeeds with no errors

**Step 2: Start dev server**

Run: `npm run dev`
Expected: Server starts, "Plugins initialized" appears in logs

**Step 3: Test API endpoint**

Run: `curl http://localhost:3000/api/plugins`
Expected: `{"plugins":[]}`

**Step 4: Create test plugin directory**

```bash
mkdir -p ~/.bot-hq/plugins/test-plugin
cat > ~/.bot-hq/plugins/test-plugin/plugin.json << 'EOF'
{
  "name": "test-plugin",
  "version": "1.0.0",
  "description": "Test plugin for development"
}
EOF
```

**Step 5: Restart dev server and test again**

Run: `curl http://localhost:3000/api/plugins`
Expected: `{"plugins":[{"name":"test-plugin","version":"1.0.0","description":"Test plugin for development","enabled":true,"hasUI":false,"hasMcp":false}]}`

**Step 6: Clean up test plugin**

```bash
rm -rf ~/.bot-hq/plugins/test-plugin
```

**Step 7: Final commit**

```bash
git add -A
git commit -m "feat(plugins): complete phase 1 - core plugin infrastructure"
```

---

## Summary

Phase 1 establishes the foundation:

| Component | File | Purpose |
|-----------|------|---------|
| Database schema | `src/lib/db/schema.ts` | Plugin tables |
| Types | `src/lib/plugins/types.ts` | TypeScript interfaces |
| Loader | `src/lib/plugins/loader.ts` | Discover plugins from disk |
| Registry | `src/lib/plugins/registry.ts` | Track loaded plugins |
| Store | `src/lib/plugins/store.ts` | Scoped data storage |
| Context | `src/lib/plugins/context.ts` | Plugin API implementation |
| MCP Manager | `src/lib/plugins/mcp-manager.ts` | Spawn/manage MCP servers |
| Events | `src/lib/plugins/events.ts` | Hook system |
| API | `src/app/api/plugins/` | REST endpoints |

**Next:** Phase 2 will build the Plugins UI tab and integrate hooks into the agent/approval flows.
