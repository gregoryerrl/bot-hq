//! `plugins` table: installed-plugin registry (manifest + enabled flag).

use super::*;

impl Storage {
    /// Upsert a plugin registry row. INSERT OR REPLACE means re-installing
    /// over an existing id overwrites the manifest + dir_path; the `enabled`
    /// column resets to its column default (1) which matches user intent —
    /// reinstalling a plugin re-enables it.
    ///
    /// `csp_json` is the consent-frozen CSP grant (canonical serialization
    /// of the approved `csp_extra_origins`, or `None` when the manifest
    /// requested none) — the ONLY source serving reads CSP extras from.
    ///
    /// `linked` marks a dev-mode install: `dir_path` is then the user's
    /// SOURCE directory (served directly, never copied, never deleted).
    ///
    /// `source_path` records where a copy-mode bundle came from (local dir
    /// or manifest URL) for "Update from source" / Reinstall pre-fill;
    /// `None` for linked rows — `dir_path` IS the source.
    pub async fn insert_plugin(
        &self,
        id: &str,
        name: &str,
        version: &str,
        manifest_json: &str,
        dir_path: &str,
        source_path: Option<&str>,
        csp_json: Option<&str>,
        linked: bool,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO plugins (id, name, version, manifest_json, dir_path, source_path, csp_json, linked, installed_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(version)
        .bind(manifest_json)
        .bind(dir_path)
        .bind(source_path)
        .bind(csp_json)
        .bind(linked)
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting plugin {id}"))?;
        Ok(())
    }

    /// Refresh a plugin's CONSENTED state in place (linked re-approve).
    /// Deliberately an UPDATE, not INSERT OR REPLACE: REPLACE is
    /// DELETE+INSERT in SQLite, and the plugin_kv FK cascade would wipe the
    /// plugin's KV rows — re-approving a manifest must not destroy state.
    /// Re-approval implies wanting the plugin active → enabled resets to 1.
    pub async fn update_plugin_consent(
        &self,
        id: &str,
        name: &str,
        version: &str,
        manifest_json: &str,
        csp_json: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE plugins SET name = ?, version = ?, manifest_json = ?, csp_json = ?, enabled = 1 \
             WHERE id = ?",
        )
        .bind(name)
        .bind(version)
        .bind(manifest_json)
        .bind(csp_json)
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("updating consent for plugin {id}"))?;
        Ok(())
    }

    /// Refresh a plugin's CONSENTED install state in place (Reinstall —
    /// mode convert or same-mode refresh from a new source). Same
    /// UPDATE-not-REPLACE rationale as [`Storage::update_plugin_consent`]:
    /// the plugin_kv FK cascade must never fire on a row refresh.
    /// Reinstalling implies wanting the plugin active → enabled resets to 1.
    pub async fn update_plugin_install(
        &self,
        id: &str,
        name: &str,
        version: &str,
        manifest_json: &str,
        csp_json: Option<&str>,
        dir_path: &str,
        source_path: Option<&str>,
        linked: bool,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE plugins SET name = ?, version = ?, manifest_json = ?, csp_json = ?, dir_path = ?, source_path = ?, linked = ?, enabled = 1 \
             WHERE id = ?",
        )
        .bind(name)
        .bind(version)
        .bind(manifest_json)
        .bind(csp_json)
        .bind(dir_path)
        .bind(source_path)
        .bind(linked)
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("updating install for plugin {id}"))?;
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
            "SELECT id, name, version, enabled, manifest_json, dir_path, csp_json, linked, source_path, installed_at \
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
        s.insert_plugin("p1", "Notes", "1.0.0", "{\"id\":\"p1\"}", "/tmp/p1", None, None, false)
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
        s.insert_plugin("p1", "Notes", "1.0.0", "{}", "/tmp/old", None, None, false)
            .await
            .unwrap();
        s.insert_plugin("p1", "Notes Pro", "2.0.0", "{\"x\":1}", "/tmp/new", None, None, false)
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
        s.insert_plugin("p1", "A", "0.1", "{}", "/x", None, None, false).await.unwrap();
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
        s.insert_plugin("p1", "A", "0.1", "{}", "/x", None, None, false).await.unwrap();
        s.insert_plugin("p2", "B", "0.2", "{}", "/y", None, None, false).await.unwrap();
        s.delete_plugin("p1").await.unwrap();
        let rows = s.list_plugins().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "p2");
    }

    #[tokio::test]
    async fn list_orders_by_id_asc() {
        let s = store().await;
        s.insert_plugin("z", "z", "0", "{}", "/z", None, None, false).await.unwrap();
        s.insert_plugin("a", "a", "0", "{}", "/a", None, None, false).await.unwrap();
        s.insert_plugin("m", "m", "0", "{}", "/m", None, None, false).await.unwrap();
        let rows = s.list_plugins().await.unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "m", "z"]);
    }

    #[tokio::test]
    async fn csp_json_roundtrips_and_defaults_null() {
        let s = store().await;
        s.insert_plugin("plain", "P", "0.1", "{}", "/p", None, None, false).await.unwrap();
        s.insert_plugin(
            "cdn",
            "C",
            "0.1",
            "{}",
            "/c",
            None,
            Some(r#"{"script-src":["https://cdn.jsdelivr.net"]}"#),
            false,
        )
        .await
        .unwrap();
        let rows = s.list_plugins().await.unwrap();
        let plain = rows.iter().find(|r| r.id == "plain").unwrap();
        let cdn = rows.iter().find(|r| r.id == "cdn").unwrap();
        assert_eq!(plain.csp_json, None);
        assert_eq!(
            cdn.csp_json.as_deref(),
            Some(r#"{"script-src":["https://cdn.jsdelivr.net"]}"#)
        );
        // Re-install WITHOUT the grant clears it (INSERT OR REPLACE).
        s.insert_plugin("cdn", "C", "0.2", "{}", "/c", None, None, false).await.unwrap();
        let rows = s.list_plugins().await.unwrap();
        let cdn = rows.iter().find(|r| r.id == "cdn").unwrap();
        assert_eq!(cdn.csp_json, None);
    }

    #[tokio::test]
    async fn linked_flag_roundtrips_and_defaults_false() {
        let s = store().await;
        s.insert_plugin("copy", "C", "0.1", "{}", "/data/plugins/copy", None, None, false)
            .await
            .unwrap();
        s.insert_plugin("dev", "D", "0.1", "{}", "/home/me/dev-plugin", None, None, true)
            .await
            .unwrap();
        let rows = s.list_plugins().await.unwrap();
        assert!(!rows.iter().find(|r| r.id == "copy").unwrap().linked);
        assert!(rows.iter().find(|r| r.id == "dev").unwrap().linked);
    }

    #[tokio::test]
    async fn update_plugin_install_switches_mode_in_place_and_keeps_kv() {
        let s = store().await;
        s.insert_plugin("p1", "Notes", "1.0.0", "{}", "/data/plugins/p1", None, None, false)
            .await
            .unwrap();
        s.plugin_kv_set("p1", "state", "keepme").await.unwrap();
        s.set_plugin_enabled("p1", false).await.unwrap();

        s.update_plugin_install(
            "p1",
            "Notes",
            "1.1.0",
            "{\"v\":2}",
            Some(r#"{"script-src":["https://cdn.example"]}"#),
            "/home/me/notes-plugin",
            None,
            true,
        )
        .await
        .unwrap();

        let rows = s.list_plugins().await.unwrap();
        assert_eq!(rows.len(), 1, "UPDATE, not insert");
        let p = &rows[0];
        assert_eq!(p.version, "1.1.0");
        assert_eq!(p.dir_path, "/home/me/notes-plugin");
        assert!(p.linked);
        assert!(p.enabled, "reinstall re-enables");
        assert_eq!(
            p.csp_json.as_deref(),
            Some(r#"{"script-src":["https://cdn.example"]}"#)
        );
        // The whole point: the plugin_kv cascade never fired.
        assert_eq!(
            s.plugin_kv_get("p1", "state").await.unwrap(),
            Some("keepme".to_string())
        );
    }

    #[tokio::test]
    async fn set_enabled_on_missing_id_is_noop() {
        let s = store().await;
        s.set_plugin_enabled("ghost", false).await.unwrap();
        assert!(s.list_plugins().await.unwrap().is_empty());
    }
}
