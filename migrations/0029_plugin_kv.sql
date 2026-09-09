-- Per-plugin key/value storage backing the plugin_kv_get / plugin_kv_set
-- catalog commands (plugin runtime v1). Namespacing is enforced server-side:
-- the proxy stamps plugin_id from the authenticated mount, never from args.
-- CASCADE keeps state from outliving an uninstall (reinstalls start clean).
CREATE TABLE IF NOT EXISTS plugin_kv (
    plugin_id  TEXT NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (plugin_id, key)
);
