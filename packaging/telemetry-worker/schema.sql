-- bot-hq telemetry — one flat events table. Apply with:
--   npx wrangler d1 execute bot-hq-telemetry --remote --file=schema.sql
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  install_id TEXT NOT NULL,
  app_version TEXT NOT NULL,
  os TEXT NOT NULL,
  arch TEXT NOT NULL,
  kind TEXT NOT NULL,
  at TEXT NOT NULL,
  data TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_kind_received ON events (kind, received_at);
CREATE INDEX IF NOT EXISTS idx_events_install ON events (install_id, received_at);
