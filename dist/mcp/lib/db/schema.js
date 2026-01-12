"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.pendingDevices = exports.authorizedDevices = exports.settings = exports.agentSessions = exports.logs = exports.approvals = exports.tasks = exports.workspaces = void 0;
const sqlite_core_1 = require("drizzle-orm/sqlite-core");
// Workspaces (repos + linked directories)
exports.workspaces = (0, sqlite_core_1.sqliteTable)("workspaces", {
    id: (0, sqlite_core_1.integer)("id").primaryKey({ autoIncrement: true }),
    name: (0, sqlite_core_1.text)("name").notNull().unique(),
    repoPath: (0, sqlite_core_1.text)("repo_path").notNull(),
    githubRemote: (0, sqlite_core_1.text)("github_remote"),
    linkedDirs: (0, sqlite_core_1.text)("linked_dirs"), // JSON string
    buildCommand: (0, sqlite_core_1.text)("build_command"),
    agentConfig: (0, sqlite_core_1.text)("agent_config"), // JSON string storing AgentConfig
    createdAt: (0, sqlite_core_1.integer)("created_at", { mode: "timestamp" })
        .notNull()
        .$defaultFn(() => new Date()),
});
// Tasks (issues + manual tasks)
exports.tasks = (0, sqlite_core_1.sqliteTable)("tasks", {
    id: (0, sqlite_core_1.integer)("id").primaryKey({ autoIncrement: true }),
    workspaceId: (0, sqlite_core_1.integer)("workspace_id")
        .notNull()
        .references(() => exports.workspaces.id),
    githubIssueNumber: (0, sqlite_core_1.integer)("github_issue_number"),
    title: (0, sqlite_core_1.text)("title").notNull(),
    description: (0, sqlite_core_1.text)("description"),
    state: (0, sqlite_core_1.text)("state", {
        enum: [
            "new",
            "queued",
            "in_progress",
            "pending_review",
            "pr_created",
            "done",
        ],
    })
        .notNull()
        .default("new"),
    priority: (0, sqlite_core_1.integer)("priority").default(0),
    agentPlan: (0, sqlite_core_1.text)("agent_plan"),
    branchName: (0, sqlite_core_1.text)("branch_name"),
    prUrl: (0, sqlite_core_1.text)("pr_url"),
    assignedAt: (0, sqlite_core_1.integer)("assigned_at", { mode: "timestamp" }),
    updatedAt: (0, sqlite_core_1.integer)("updated_at", { mode: "timestamp" })
        .notNull()
        .$defaultFn(() => new Date()),
}, (table) => [
    (0, sqlite_core_1.index)("tasks_workspace_idx").on(table.workspaceId),
    (0, sqlite_core_1.index)("tasks_state_idx").on(table.state),
    (0, sqlite_core_1.index)("tasks_issue_idx").on(table.workspaceId, table.githubIssueNumber),
]);
// Draft PRs (pending review)
exports.approvals = (0, sqlite_core_1.sqliteTable)("approvals", {
    id: (0, sqlite_core_1.integer)("id").primaryKey({ autoIncrement: true }),
    taskId: (0, sqlite_core_1.integer)("task_id")
        .notNull()
        .references(() => exports.tasks.id),
    workspaceId: (0, sqlite_core_1.integer)("workspace_id")
        .notNull()
        .references(() => exports.workspaces.id),
    branchName: (0, sqlite_core_1.text)("branch_name").notNull(),
    baseBranch: (0, sqlite_core_1.text)("base_branch").notNull().default("main"),
    commitMessages: (0, sqlite_core_1.text)("commit_messages"), // JSON array of commit messages
    diffSummary: (0, sqlite_core_1.text)("diff_summary"), // JSON: { files: [...], additions, deletions }
    status: (0, sqlite_core_1.text)("status", {
        enum: ["pending", "approved", "rejected"],
    })
        .notNull()
        .default("pending"),
    userInstructions: (0, sqlite_core_1.text)("user_instructions"), // Feedback for "Request Changes"
    createdAt: (0, sqlite_core_1.integer)("created_at", { mode: "timestamp" })
        .notNull()
        .$defaultFn(() => new Date()),
    resolvedAt: (0, sqlite_core_1.integer)("resolved_at", { mode: "timestamp" }),
}, (table) => [
    (0, sqlite_core_1.index)("approvals_status_idx").on(table.status),
    (0, sqlite_core_1.index)("approvals_task_idx").on(table.taskId),
]);
// Logs
exports.logs = (0, sqlite_core_1.sqliteTable)("logs", {
    id: (0, sqlite_core_1.integer)("id").primaryKey({ autoIncrement: true }),
    workspaceId: (0, sqlite_core_1.integer)("workspace_id").references(() => exports.workspaces.id),
    taskId: (0, sqlite_core_1.integer)("task_id").references(() => exports.tasks.id),
    type: (0, sqlite_core_1.text)("type", {
        enum: ["agent", "test", "sync", "approval", "error", "health"],
    }).notNull(),
    message: (0, sqlite_core_1.text)("message").notNull(),
    details: (0, sqlite_core_1.text)("details"), // JSON string
    createdAt: (0, sqlite_core_1.integer)("created_at", { mode: "timestamp" })
        .notNull()
        .$defaultFn(() => new Date()),
}, (table) => [
    (0, sqlite_core_1.index)("logs_type_idx").on(table.type),
    (0, sqlite_core_1.index)("logs_task_idx").on(table.taskId),
    (0, sqlite_core_1.index)("logs_created_idx").on(table.createdAt),
    (0, sqlite_core_1.index)("logs_stream_idx").on(table.id, table.type), // For streaming queries
]);
// Agent sessions
exports.agentSessions = (0, sqlite_core_1.sqliteTable)("agent_sessions", {
    id: (0, sqlite_core_1.integer)("id").primaryKey({ autoIncrement: true }),
    workspaceId: (0, sqlite_core_1.integer)("workspace_id")
        .notNull()
        .references(() => exports.workspaces.id),
    taskId: (0, sqlite_core_1.integer)("task_id").references(() => exports.tasks.id),
    pid: (0, sqlite_core_1.integer)("pid"),
    status: (0, sqlite_core_1.text)("status", {
        enum: ["running", "idle", "stopped", "error"],
    })
        .notNull()
        .default("idle"),
    contextSize: (0, sqlite_core_1.integer)("context_size"),
    startedAt: (0, sqlite_core_1.integer)("started_at", { mode: "timestamp" }),
    lastActivityAt: (0, sqlite_core_1.integer)("last_activity_at", { mode: "timestamp" }),
}, (table) => [
    (0, sqlite_core_1.index)("sessions_status_idx").on(table.status),
    (0, sqlite_core_1.index)("sessions_task_idx").on(table.taskId),
]);
// App settings (key-value store)
exports.settings = (0, sqlite_core_1.sqliteTable)("settings", {
    key: (0, sqlite_core_1.text)("key").primaryKey(),
    value: (0, sqlite_core_1.text)("value").notNull(),
    updatedAt: (0, sqlite_core_1.integer)("updated_at", { mode: "timestamp" })
        .notNull()
        .$defaultFn(() => new Date()),
});
// Authorized devices
exports.authorizedDevices = (0, sqlite_core_1.sqliteTable)("authorized_devices", {
    id: (0, sqlite_core_1.integer)("id").primaryKey({ autoIncrement: true }),
    deviceName: (0, sqlite_core_1.text)("device_name").notNull(),
    deviceFingerprint: (0, sqlite_core_1.text)("device_fingerprint").notNull().unique(),
    tokenHash: (0, sqlite_core_1.text)("token_hash").notNull(),
    authorizedAt: (0, sqlite_core_1.integer)("authorized_at", { mode: "timestamp" })
        .notNull()
        .$defaultFn(() => new Date()),
    lastSeenAt: (0, sqlite_core_1.integer)("last_seen_at", { mode: "timestamp" }),
    isRevoked: (0, sqlite_core_1.integer)("is_revoked", { mode: "boolean" }).notNull().default(false),
});
// Pending device requests (waiting for approval)
exports.pendingDevices = (0, sqlite_core_1.sqliteTable)("pending_devices", {
    id: (0, sqlite_core_1.integer)("id").primaryKey({ autoIncrement: true }),
    pairingCode: (0, sqlite_core_1.text)("pairing_code").notNull().unique(),
    deviceInfo: (0, sqlite_core_1.text)("device_info").notNull(), // JSON: userAgent, ip, etc.
    createdAt: (0, sqlite_core_1.integer)("created_at", { mode: "timestamp" })
        .notNull()
        .$defaultFn(() => new Date()),
    expiresAt: (0, sqlite_core_1.integer)("expires_at", { mode: "timestamp" }).notNull(),
});
