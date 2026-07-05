//! `plugin_kv` table: per-plugin key/value storage (plugin runtime v1).
//! Namespaced by plugin_id — the proxy layer stamps the id, plugins never
//! pass it themselves.

use super::*;

impl Storage {
    pub async fn plugin_kv_get(&self, plugin_id: &str, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM plugin_kv WHERE plugin_id = ? AND key = ?")
                .bind(plugin_id)
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .with_context(|| format!("plugin_kv_get {plugin_id}/{key}"))?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn plugin_kv_set(&self, plugin_id: &str, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO plugin_kv (plugin_id, key, value, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (plugin_id, key) DO UPDATE SET value = excluded.value, \
             updated_at = excluded.updated_at",
        )
        .bind(plugin_id)
        .bind(key)
        .bind(value)
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .with_context(|| format!("plugin_kv_set {plugin_id}/{key}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod plugin_kv_tests {
    use super::*;

    async fn store_with_plugin(id: &str) -> Storage {
        let s = Storage::memory().await.unwrap();
        s.insert_plugin(id, "Test", "0.1.0", "{}", "/tmp/x", None, false).await.unwrap();
        s
    }

    #[tokio::test]
    async fn get_missing_key_returns_none() {
        let s = store_with_plugin("p1").await;
        assert_eq!(s.plugin_kv_get("p1", "lens").await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_then_get_roundtrip_and_overwrite() {
        let s = store_with_plugin("p1").await;
        s.plugin_kv_set("p1", "lens", "graph").await.unwrap();
        assert_eq!(
            s.plugin_kv_get("p1", "lens").await.unwrap(),
            Some("graph".to_string())
        );
        s.plugin_kv_set("p1", "lens", "prose").await.unwrap();
        assert_eq!(
            s.plugin_kv_get("p1", "lens").await.unwrap(),
            Some("prose".to_string())
        );
    }

    #[tokio::test]
    async fn keys_are_namespaced_per_plugin() {
        let s = store_with_plugin("p1").await;
        s.insert_plugin("p2", "Other", "0.1.0", "{}", "/tmp/y", None, false).await.unwrap();
        s.plugin_kv_set("p1", "k", "one").await.unwrap();
        s.plugin_kv_set("p2", "k", "two").await.unwrap();
        assert_eq!(s.plugin_kv_get("p1", "k").await.unwrap(), Some("one".into()));
        assert_eq!(s.plugin_kv_get("p2", "k").await.unwrap(), Some("two".into()));
    }

    #[tokio::test]
    async fn uninstall_cascades_kv_rows() {
        let s = store_with_plugin("p1").await;
        s.plugin_kv_set("p1", "k", "v").await.unwrap();
        s.delete_plugin("p1").await.unwrap();
        // Row gone via FK CASCADE — a reinstall starts clean.
        assert_eq!(s.plugin_kv_get("p1", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_for_unknown_plugin_is_rejected_by_fk() {
        let s = Storage::memory().await.unwrap();
        assert!(s.plugin_kv_set("ghost", "k", "v").await.is_err());
    }
}
