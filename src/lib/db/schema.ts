import { sqliteTable, text, integer, index } from "drizzle-orm/sqlite-core";

// Workspaces (repos + linked directories)
export const workspaces = sqliteTable("workspaces", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull().unique(),
  repoPath: text("repo_path").notNull(),
  linkedDirs: text("linked_dirs"), // JSON string
  buildCommand: text("build_command"),
  agentConfig: text("agent_config"), // JSON string storing AgentConfig
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

// Tasks
export const tasks = sqliteTable("tasks", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id")
    .notNull()
    .references(() => workspaces.id),
  sourcePluginId: integer("source_plugin_id"), // Plugin that created this task
  sourceRef: text("source_ref"), // Plugin-specific reference (issue #, message ID)
  title: text("title").notNull(),
  description: text("description"),
  state: text("state", {
    enum: [
      "new",
      "queued",
      "in_progress",
      "awaiting_input",  // Manager is waiting for user input (brainstorming)
      "needs_help",  // Replaces stuck/pending_review state
      "done",
    ],
  })
    .notNull()
    .default("new"),
  priority: integer("priority").default(0),
  agentPlan: text("agent_plan"),
  branchName: text("branch_name"),
  // New fields for manager + subagent architecture
  completionCriteria: text("completion_criteria"),  // Task-specific success criteria
  iterationCount: integer("iteration_count").default(0),  // Current iteration
  maxIterations: integer("max_iterations"),  // Override global default
  feedback: text("feedback"),  // Human feedback on retry
  // Brainstorming fields - for manager awaiting user input
  waitingQuestion: text("waiting_question"),  // Question manager is asking
  waitingContext: text("waiting_context"),    // Conversation context so far
  waitingSince: integer("waiting_since", { mode: "timestamp" }),  // When started waiting
  assignedAt: integer("assigned_at", { mode: "timestamp" }),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("tasks_workspace_idx").on(table.workspaceId),
  index("tasks_state_idx").on(table.state),
]);

// REMOVED: Pending approvals table - replaced by git-native review
// The review workflow now uses task branches directly without a separate approvals table.

// Logs
export const logs = sqliteTable("logs", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id").references(() => workspaces.id),
  taskId: integer("task_id").references(() => tasks.id),
  type: text("type", {
    enum: ["agent", "test", "approval", "error", "health"],
  }).notNull(),
  message: text("message").notNull(),
  details: text("details"), // JSON string
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("logs_type_idx").on(table.type),
  index("logs_task_idx").on(table.taskId),
  index("logs_created_idx").on(table.createdAt),
  index("logs_stream_idx").on(table.id, table.type), // For streaming queries
]);

// REMOVED: Agent sessions table - replaced by single persistent manager
// The manager runs as a persistent session and spawns subagents via Task tool.

// App settings (key-value store)
export const settings = sqliteTable("settings", {
  key: text("key").primaryKey(),
  value: text("value").notNull(),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

// Authorized devices
export const authorizedDevices = sqliteTable("authorized_devices", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  deviceName: text("device_name").notNull(),
  deviceFingerprint: text("device_fingerprint").notNull().unique(),
  tokenHash: text("token_hash").notNull(),
  authorizedAt: integer("authorized_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  lastSeenAt: integer("last_seen_at", { mode: "timestamp" }),
  isRevoked: integer("is_revoked", { mode: "boolean" }).notNull().default(false),
});

// Pending device requests (waiting for approval)
export const pendingDevices = sqliteTable("pending_devices", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  pairingCode: text("pairing_code").notNull().unique(),
  deviceInfo: text("device_info").notNull(), // JSON: userAgent, ip, etc.
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  expiresAt: integer("expires_at", { mode: "timestamp" }).notNull(),
});

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

// Type exports
export type Workspace = typeof workspaces.$inferSelect;
export type NewWorkspace = typeof workspaces.$inferInsert;
export type Task = typeof tasks.$inferSelect;
export type NewTask = typeof tasks.$inferInsert;
export type Log = typeof logs.$inferSelect;
export type NewLog = typeof logs.$inferInsert;
export type AuthorizedDevice = typeof authorizedDevices.$inferSelect;
export type NewAuthorizedDevice = typeof authorizedDevices.$inferInsert;
export type PendingDevice = typeof pendingDevices.$inferSelect;
export type NewPendingDevice = typeof pendingDevices.$inferInsert;
export type Setting = typeof settings.$inferSelect;
export type NewSetting = typeof settings.$inferInsert;
export type Plugin = typeof plugins.$inferSelect;
export type NewPlugin = typeof plugins.$inferInsert;
export type PluginWorkspaceData = typeof pluginWorkspaceData.$inferSelect;
export type PluginTaskData = typeof pluginTaskData.$inferSelect;
export type PluginStoreEntry = typeof pluginStore.$inferSelect;
