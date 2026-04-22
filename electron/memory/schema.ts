import { sqliteTable, text, integer } from 'drizzle-orm/sqlite-core'

export const conversations = sqliteTable('conversations', {
  id: text('id').primaryKey(),
  startedAt: text('started_at').notNull(),
  endedAt: text('ended_at'),
  projectPath: text('project_path'),
  summary: text('summary')
})

export const messages = sqliteTable('messages', {
  id: text('id').primaryKey(),
  conversationId: text('conversation_id')
    .notNull()
    .references(() => conversations.id),
  role: text('role').notNull(),
  content: text('content').notNull(),
  timestamp: text('timestamp').notNull(),
  tokenCount: integer('token_count')
})

export const memories = sqliteTable('memories', {
  id: text('id').primaryKey(),
  category: text('category').notNull(),
  content: text('content').notNull(),
  sourceConversationId: text('source_conversation_id'),
  createdAt: text('created_at').notNull(),
  lastAccessedAt: text('last_accessed_at'),
  accessCount: integer('access_count').default(0)
})

export const projects = sqliteTable('projects', {
  id: text('id').primaryKey(),
  name: text('name').notNull(),
  path: text('path').notNull().unique(),
  description: text('description'),
  lastFocusedAt: text('last_focused_at'),
  fileTreeSnapshot: text('file_tree_snapshot'),
  keyFiles: text('key_files'),
  conventions: text('conventions'),
  createdAt: text('created_at').notNull()
})

export const toolExecutions = sqliteTable('tool_executions', {
  id: text('id').primaryKey(),
  conversationId: text('conversation_id').references(() => conversations.id),
  toolName: text('tool_name').notNull(),
  parameters: text('parameters').notNull(),
  result: text('result'),
  success: integer('success').notNull(),
  durationMs: integer('duration_ms'),
  executedAt: text('executed_at').notNull()
})

export const claudeSessions = sqliteTable('claude_sessions', {
  id: text('id').primaryKey(),
  projectPath: text('project_path'),
  pid: integer('pid'),
  tmuxTarget: text('tmux_target'),
  mode: text('mode').notNull(),
  status: text('status').notNull(),
  lastOutput: text('last_output'),
  lastCheckedAt: text('last_checked_at'),
  startedAt: text('started_at').notNull(),
  endedAt: text('ended_at')
})
