import { sqliteTable, text, integer } from "drizzle-orm/sqlite-core";

// Workspaces (repos + linked directories)
export const workspaces = sqliteTable("workspaces", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull().unique(),
  repoPath: text("repo_path").notNull(),
  githubRemote: text("github_remote"),
  linkedDirs: text("linked_dirs"), // JSON string
  buildCommand: text("build_command"),
  agentConfig: text("agent_config"), // JSON string storing AgentConfig
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

// Tasks (issues + manual tasks)
export const tasks = sqliteTable("tasks", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id")
    .notNull()
    .references(() => workspaces.id),
  githubIssueNumber: integer("github_issue_number"),
  title: text("title").notNull(),
  description: text("description"),
  state: text("state", {
    enum: [
      "new",
      "queued",
      "analyzing",
      "plan_ready",
      "in_progress",
      "pr_draft",
      "done",
    ],
  })
    .notNull()
    .default("new"),
  priority: integer("priority").default(0),
  agentPlan: text("agent_plan"),
  branchName: text("branch_name"),
  prUrl: text("pr_url"),
  assignedAt: integer("assigned_at", { mode: "timestamp" }),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

// Pending approvals
export const approvals = sqliteTable("approvals", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  taskId: integer("task_id").references(() => tasks.id),
  type: text("type", {
    enum: ["git_push", "external_command", "deploy"],
  }).notNull(),
  command: text("command"),
  reason: text("reason"),
  diffSummary: text("diff_summary"),
  status: text("status", {
    enum: ["pending", "approved", "rejected"],
  })
    .notNull()
    .default("pending"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  resolvedAt: integer("resolved_at", { mode: "timestamp" }),
});

// Logs
export const logs = sqliteTable("logs", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id").references(() => workspaces.id),
  taskId: integer("task_id").references(() => tasks.id),
  type: text("type", {
    enum: ["agent", "test", "sync", "approval", "error", "health"],
  }).notNull(),
  message: text("message").notNull(),
  details: text("details"), // JSON string
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

// Agent sessions
export const agentSessions = sqliteTable("agent_sessions", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id")
    .notNull()
    .references(() => workspaces.id),
  taskId: integer("task_id").references(() => tasks.id),
  pid: integer("pid"),
  status: text("status", {
    enum: ["running", "idle", "stopped", "error"],
  })
    .notNull()
    .default("idle"),
  contextSize: integer("context_size"),
  startedAt: integer("started_at", { mode: "timestamp" }),
  lastActivityAt: integer("last_activity_at", { mode: "timestamp" }),
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

// Type exports
export type Workspace = typeof workspaces.$inferSelect;
export type NewWorkspace = typeof workspaces.$inferInsert;
export type Task = typeof tasks.$inferSelect;
export type NewTask = typeof tasks.$inferInsert;
export type Approval = typeof approvals.$inferSelect;
export type NewApproval = typeof approvals.$inferInsert;
export type Log = typeof logs.$inferSelect;
export type NewLog = typeof logs.$inferInsert;
export type AgentSession = typeof agentSessions.$inferSelect;
export type NewAgentSession = typeof agentSessions.$inferInsert;
export type AuthorizedDevice = typeof authorizedDevices.$inferSelect;
export type NewAuthorizedDevice = typeof authorizedDevices.$inferInsert;
