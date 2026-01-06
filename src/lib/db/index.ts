import Database from "better-sqlite3";
import { drizzle, BetterSQLite3Database } from "drizzle-orm/better-sqlite3";
import * as schema from "./schema";
import path from "path";
import fs from "fs";

const DATA_DIR = path.join(process.cwd(), "data");

let _db: BetterSQLite3Database<typeof schema> | null = null;

function getDb(): BetterSQLite3Database<typeof schema> {
  if (!_db) {
    // Ensure data directory exists
    if (!fs.existsSync(DATA_DIR)) {
      fs.mkdirSync(DATA_DIR, { recursive: true });
    }

    const sqlite = new Database(path.join(DATA_DIR, "bot-hq.db"));
    sqlite.pragma("journal_mode = WAL");
    _db = drizzle(sqlite, { schema });
  }
  return _db;
}

// Export as a getter to lazy-load the database
export const db = new Proxy({} as BetterSQLite3Database<typeof schema>, {
  get(_, prop) {
    return (getDb() as unknown as Record<string, unknown>)[prop as string];
  },
});

export * from "./schema";
