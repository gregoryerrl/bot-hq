//! `models` registry (saved LLM endpoints) + `app_settings` key/value store.

use super::*;

const MODEL_COLUMNS: &str =
    "id, display_name, provider, model_name, base_url, auth_token, created_at, updated_at";

/// Key in `app_settings`: "1" = new sessions default to solo-Brian. The create
/// dialog reads it to pre-check "Disable Rain"; backend dispatch paths with no
/// dialog (Maintain CL, external driver) resolve it via
/// [`Storage::default_rain_enabled`].
pub const RAIN_DISABLED_DEFAULT_KEY: &str = "rain_disabled_default";

/// Key in `app_settings`: "0" = repo-backed sessions run directly in the repo
/// by default instead of an isolated git worktree. Resolved via
/// [`Storage::default_worktree_enabled`]; the create dialog seeds its
/// checkbox from it.
pub const WORKTREE_DEFAULT_KEY: &str = "worktree_default";

impl Storage {
    // ---- models ----------------------------------------------------------

    /// All saved models, ordered by display name.
    pub async fn list_models(&self) -> Result<Vec<Model>> {
        let rows = sqlx::query_as::<_, Model>(&format!(
            "SELECT {MODEL_COLUMNS} FROM models ORDER BY display_name COLLATE NOCASE ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .context("listing models")?;
        Ok(rows)
    }

    pub async fn get_model(&self, id: &str) -> Result<Option<Model>> {
        let row = sqlx::query_as::<_, Model>(&format!(
            "SELECT {MODEL_COLUMNS} FROM models WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Insert or update a saved model by id. `created_at` is preserved on
    /// conflict; only `updated_at` advances.
    pub async fn upsert_model(&self, m: &Model) -> Result<()> {
        let now = now_utc();
        sqlx::query(
            "INSERT INTO models \
                (id, display_name, provider, model_name, base_url, auth_token, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                display_name = excluded.display_name, \
                provider = excluded.provider, \
                model_name = excluded.model_name, \
                base_url = excluded.base_url, \
                auth_token = excluded.auth_token, \
                updated_at = excluded.updated_at",
        )
        .bind(&m.id)
        .bind(&m.display_name)
        .bind(&m.provider)
        .bind(&m.model_name)
        .bind(&m.base_url)
        .bind(&m.auth_token)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upserting model {}", m.id))?;
        Ok(())
    }

    pub async fn delete_model(&self, id: &str) -> Result<u64> {
        let res = sqlx::query("DELETE FROM models WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("deleting model {id}"))?;
        Ok(res.rows_affected())
    }

    // ---- app_settings (key/value) ---------------------------------------

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .with_context(|| format!("setting app_setting {key}"))?;
        Ok(())
    }

    /// Whether a session created WITHOUT an explicit Rain choice spawns the
    /// duo. `rain_disabled_default == "1"` → solo-Brian (false); unset, any
    /// other value, or a read error → duo (true, the historical default).
    pub async fn default_rain_enabled(&self) -> bool {
        !matches!(
            self.get_setting(RAIN_DISABLED_DEFAULT_KEY).await,
            Ok(Some(v)) if v == "1"
        )
    }

    /// Whether a repo-backed session created WITHOUT an explicit worktree
    /// choice runs in an isolated git worktree. Opt-OUT: unset or any value
    /// but "0" → worktree on (parallel sessions per project are the default);
    /// `worktree_default == "0"` → direct mode.
    pub async fn default_worktree_enabled(&self) -> bool {
        !matches!(
            self.get_setting(WORKTREE_DEFAULT_KEY).await,
            Ok(Some(v)) if v == "0"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, name: &str) -> Model {
        Model {
            id: id.into(),
            display_name: name.into(),
            provider: "anthropic".into(),
            model_name: "claude-opus-4-8".into(),
            base_url: None,
            auth_token: Some("sk-test".into()),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn upsert_get_list_delete_roundtrip() {
        let s = Storage::memory().await.unwrap();
        // Migration 0016 seeds the registry, so measure against the live
        // baseline rather than an absolute count.
        let seeded = s.list_models().await.unwrap().len();
        s.upsert_model(&model("m1", "Opus")).await.unwrap();
        s.upsert_model(&model("m2", "Sonnet")).await.unwrap();

        let got = s.get_model("m1").await.unwrap().unwrap();
        assert_eq!(got.display_name, "Opus");
        // Stored timestamp is canonical RFC3339-Z, not zone-less.
        assert!(got.created_at.ends_with('Z'), "got {}", got.created_at);

        let all = s.list_models().await.unwrap();
        assert_eq!(all.len(), seeded + 2);
        // Ordered by display name COLLATE NOCASE: "Opus" sorts before "Sonnet".
        let pos = |id: &str| all.iter().position(|m| m.id == id).unwrap();
        assert!(pos("m1") < pos("m2"));

        let removed = s.delete_model("m1").await.unwrap();
        assert_eq!(removed, 1);
        assert!(s.get_model("m1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_preserves_created_at_on_conflict() {
        let s = Storage::memory().await.unwrap();
        s.upsert_model(&model("m1", "Opus")).await.unwrap();
        let first = s.get_model("m1").await.unwrap().unwrap();
        let mut edit = model("m1", "Opus Renamed");
        edit.created_at = "ignored".into();
        s.upsert_model(&edit).await.unwrap();
        let after = s.get_model("m1").await.unwrap().unwrap();
        assert_eq!(after.display_name, "Opus Renamed");
        assert_eq!(after.created_at, first.created_at, "created_at must persist");
    }

    #[tokio::test]
    async fn default_rain_enabled_tracks_setting() {
        let s = Storage::memory().await.unwrap();
        // Unset → duo (historical default).
        assert!(s.default_rain_enabled().await);
        // "1" → solo-Brian default.
        s.set_setting(RAIN_DISABLED_DEFAULT_KEY, "1").await.unwrap();
        assert!(!s.default_rain_enabled().await);
        // Any other value → duo.
        s.set_setting(RAIN_DISABLED_DEFAULT_KEY, "0").await.unwrap();
        assert!(s.default_rain_enabled().await);
    }

    #[tokio::test]
    async fn default_worktree_enabled_is_opt_out() {
        let s = Storage::memory().await.unwrap();
        // Unset → worktree isolation on by default.
        assert!(s.default_worktree_enabled().await);
        s.set_setting(WORKTREE_DEFAULT_KEY, "0").await.unwrap();
        assert!(!s.default_worktree_enabled().await);
        s.set_setting(WORKTREE_DEFAULT_KEY, "1").await.unwrap();
        assert!(s.default_worktree_enabled().await);
    }

    #[tokio::test]
    async fn settings_roundtrip() {
        let s = Storage::memory().await.unwrap();
        assert!(s.get_setting("default_model_id").await.unwrap().is_none());
        s.set_setting("default_model_id", "m1").await.unwrap();
        assert_eq!(
            s.get_setting("default_model_id").await.unwrap().as_deref(),
            Some("m1")
        );
        // Upsert overwrites.
        s.set_setting("default_model_id", "m2").await.unwrap();
        assert_eq!(
            s.get_setting("default_model_id").await.unwrap().as_deref(),
            Some("m2")
        );
    }
}
