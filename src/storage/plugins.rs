//! `plugins` table: installed-plugin registry (manifest + enabled flag).

use super::*;

impl Storage {
    /// Upsert a plugin registry row. INSERT OR REPLACE means re-installing
    /// over an existing id overwrites the manifest + dir_path; the `enabled`
    /// column resets to its column default (1) which matches user intent —
    /// reinstalling a plugin re-enables it.
    pub async fn insert_plugin(
        &self,
        id: &str,
        name: &str,
        version: &str,
        manifest_json: &str,
        dir_path: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO plugins (id, name, version, manifest_json, dir_path) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(version)
        .bind(manifest_json)
        .bind(dir_path)
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting plugin {id}"))?;
        Ok(())
    }

    pub async fn delete_plugin(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM plugins WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("deleting plugin {id}"))?;
        Ok(())
    }

    pub async fn set_plugin_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        sqlx::query("UPDATE plugins SET enabled = ? WHERE id = ?")
            .bind(enabled)
            .bind(id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("setting plugin {id} enabled={enabled}"))?;
        Ok(())
    }

    pub async fn list_plugins(&self) -> Result<Vec<Plugin>> {
        let rows = sqlx::query_as::<_, Plugin>(
            "SELECT id, name, version, enabled, manifest_json, dir_path, installed_at \
             FROM plugins ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing plugins")?;
        Ok(rows)
    }
}

#[cfg(test)]
mod plugin_tests {
    use super::*;

    async fn store() -> Storage {
        Storage::memory().await.unwrap()
    }

    #[tokio::test]
    async fn insert_then_list_roundtrip() {
        let s = store().await;
        s.insert_plugin("p1", "Notes", "1.0.0", "{\"id\":\"p1\"}", "/tmp/p1")
            .await
            .unwrap();
        let rows = s.list_plugins().await.unwrap();
        assert_eq!(rows.len(), 1);
        let p = &rows[0];
        assert_eq!(p.id, "p1");
        assert_eq!(p.name, "Notes");
        assert_eq!(p.version, "1.0.0");
        assert_eq!(p.manifest_json, "{\"id\":\"p1\"}");
        assert_eq!(p.dir_path, "/tmp/p1");
        assert!(p.enabled, "default enabled = 1");
        assert!(!p.installed_at.is_empty());
    }

    #[tokio::test]
    async fn insert_or_replace_overwrites_existing_id() {
        let s = store().await;
        s.insert_plugin("p1", "Notes", "1.0.0", "{}", "/tmp/old")
            .await
            .unwrap();
        s.insert_plugin("p1", "Notes Pro", "2.0.0", "{\"x\":1}", "/tmp/new")
            .await
            .unwrap();
        let rows = s.list_plugins().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Notes Pro");
        assert_eq!(rows[0].version, "2.0.0");
        assert_eq!(rows[0].dir_path, "/tmp/new");
    }

    #[tokio::test]
    async fn set_enabled_toggles_flag() {
        let s = store().await;
        s.insert_plugin("p1", "A", "0.1", "{}", "/x").await.unwrap();
        s.set_plugin_enabled("p1", false).await.unwrap();
        let rows = s.list_plugins().await.unwrap();
        assert!(!rows[0].enabled);
        s.set_plugin_enabled("p1", true).await.unwrap();
        let rows = s.list_plugins().await.unwrap();
        assert!(rows[0].enabled);
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let s = store().await;
        s.insert_plugin("p1", "A", "0.1", "{}", "/x").await.unwrap();
        s.insert_plugin("p2", "B", "0.2", "{}", "/y").await.unwrap();
        s.delete_plugin("p1").await.unwrap();
        let rows = s.list_plugins().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "p2");
    }

    #[tokio::test]
    async fn list_orders_by_id_asc() {
        let s = store().await;
        s.insert_plugin("z", "z", "0", "{}", "/z").await.unwrap();
        s.insert_plugin("a", "a", "0", "{}", "/a").await.unwrap();
        s.insert_plugin("m", "m", "0", "{}", "/m").await.unwrap();
        let rows = s.list_plugins().await.unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "m", "z"]);
    }

    #[tokio::test]
    async fn set_enabled_on_missing_id_is_noop() {
        let s = store().await;
        s.set_plugin_enabled("ghost", false).await.unwrap();
        assert!(s.list_plugins().await.unwrap().is_empty());
    }
}
