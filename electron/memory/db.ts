import Database from 'better-sqlite3'
import { drizzle } from 'drizzle-orm/better-sqlite3'
import { join } from 'path'
import { mkdirSync } from 'fs'
import * as schema from './schema'

let db: ReturnType<typeof drizzle<typeof schema>> | null = null

export function getDb() {
  if (db) return db

  // Use project data dir for now (app.getPath requires app to be ready)
  const dataDir = join(__dirname, '../../data')
  mkdirSync(dataDir, { recursive: true })

  const sqlite = new Database(join(dataDir, 'bot-hq.db'))
  sqlite.pragma('journal_mode = WAL')
  sqlite.pragma('foreign_keys = ON')

  db = drizzle(sqlite, { schema })

  // Create tables if they don't exist
  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS conversations (
      id TEXT PRIMARY KEY,
      started_at TEXT NOT NULL,
      ended_at TEXT,
      project_path TEXT,
      summary TEXT
    );
    CREATE TABLE IF NOT EXISTS messages (
      id TEXT PRIMARY KEY,
      conversation_id TEXT NOT NULL REFERENCES conversations(id),
      role TEXT NOT NULL,
      content TEXT NOT NULL,
      timestamp TEXT NOT NULL,
      token_count INTEGER
    );
    CREATE TABLE IF NOT EXISTS memories (
      id TEXT PRIMARY KEY,
      category TEXT NOT NULL,
      content TEXT NOT NULL,
      source_conversation_id TEXT,
      created_at TEXT NOT NULL,
      last_accessed_at TEXT,
      access_count INTEGER DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS projects (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      path TEXT NOT NULL UNIQUE,
      description TEXT,
      last_focused_at TEXT,
      file_tree_snapshot TEXT,
      key_files TEXT,
      conventions TEXT,
      created_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS tool_executions (
      id TEXT PRIMARY KEY,
      conversation_id TEXT REFERENCES conversations(id),
      tool_name TEXT NOT NULL,
      parameters TEXT NOT NULL,
      result TEXT,
      success INTEGER NOT NULL,
      duration_ms INTEGER,
      executed_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS claude_sessions (
      id TEXT PRIMARY KEY,
      project_path TEXT,
      pid INTEGER,
      tmux_target TEXT,
      mode TEXT NOT NULL,
      status TEXT NOT NULL,
      last_output TEXT,
      last_checked_at TEXT,
      started_at TEXT NOT NULL,
      ended_at TEXT
    );
  `)

  return db
}

export { schema }
