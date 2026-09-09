CREATE TABLE IF NOT EXISTS plugins (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    manifest_json TEXT NOT NULL,
    dir_path TEXT NOT NULL,
    installed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
