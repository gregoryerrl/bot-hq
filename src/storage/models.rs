//! `models` registry (saved LLM endpoints) + `app_settings` key/value store.

use super::*;

/// The projection [`Model`] is read through.
///
/// `native` is NOT here. The column survives (0036, `NOT NULL DEFAULT 0`) but
/// rc3 D9 deleted the runtime it selected, so nothing reads or writes it — an
/// upsert now leaves it at the column default. Dropping it needs a migration
/// this phase does not write.
const MODEL_COLUMNS: &str = "id, display_name, provider, model_name, base_url, auth_token, \
     created_at, updated_at, context_window, cli_settings";

/// Key in `app_settings`: "0" = repo-backed sessions run directly in the repo
/// by default instead of an isolated git worktree. Resolved via
/// [`Storage::default_worktree_enabled`]; the create dialog seeds its
/// checkbox from it.
pub const WORKTREE_DEFAULT_KEY: &str = "worktree_default";

/// The default spawn generation — the ONE name `default_spawn_model` prefers
/// in the registry and `core::session::default_agent_config` compiles in as
/// the empty-registry last resort. Single source (EYES 8790cb6a): two notions
/// of "the default model" is how a fresh install spawned everything on Haiku.
pub const DEFAULT_SPAWN_MODEL_NAME: &str = "claude-opus-5";

/// Key in `app_settings`: "0" = disable the Track-A workflow-adherence nudges
/// (e.g. the session-start CL-index primer) that mechanically page a model
/// toward the workflow when it doesn't reliably follow the prompt. Opt-OUT:
/// unset or any value but "0" → nudges ON (the default). Resolved via
/// [`Storage::adherence_nudges_enabled`].
pub const ADHERENCE_NUDGES_KEY: &str = "adherence_nudges";

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
                (id, display_name, provider, model_name, base_url, auth_token, created_at, updated_at, \
                 context_window, cli_settings) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                display_name = excluded.display_name, \
                provider = excluded.provider, \
                model_name = excluded.model_name, \
                base_url = excluded.base_url, \
                auth_token = excluded.auth_token, \
                updated_at = excluded.updated_at, \
                context_window = excluded.context_window, \
                cli_settings = excluded.cli_settings",
        )
        .bind(&m.id)
        .bind(&m.display_name)
        .bind(&m.provider)
        .bind(&m.model_name)
        .bind(&m.base_url)
        .bind(&m.auth_token)
        .bind(&now)
        .bind(&now)
        .bind(m.context_window)
        .bind(&m.cli_settings)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upserting model {}", m.id))?;
        Ok(())
    }

    /// The registry row spawning a given wire model id that can lend its
    /// `cli_settings` — this is how the `agent_configs` fallback tier of
    /// `resolve_spawn_config` borrows the setting for a model name it shares:
    /// that table has no such column, and a participant landing there on a model
    /// the CLI does not know would otherwise run at the 200k default, the exact
    /// bug 0079 fixes.
    ///
    /// `model_name` is not unique (0013 keys on `id`; 0073's seeds guard on
    /// `NOT EXISTS … model_name`, so duplicates are live). The common duplicate
    /// is an older seeded row beside a newer one the user gave the setting to —
    /// bare `ORDER BY rowid` picks the older, gets `None`, and the bug survives
    /// on the one path this borrow closes (EYES b528ad35; the 8790cb6a class).
    /// So the row WITH a setting wins, oldest among those as the tie-break.
    pub async fn model_by_name(&self, model_name: &str) -> Result<Option<Model>> {
        let row = sqlx::query_as::<_, Model>(&format!(
            "SELECT {MODEL_COLUMNS} FROM models WHERE model_name = ? \
             ORDER BY (cli_settings IS NULL), rowid LIMIT 1"
        ))
        .bind(model_name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_model(&self, id: &str) -> Result<u64> {
        let res = sqlx::query("DELETE FROM models WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("deleting model {id}"))?;
        Ok(res.rows_affected())
    }

    /// The model spawns fall back to when nothing else resolved (1.0.0
    /// Batch 5, corrected by EYES blocking 8790cb6a): PREFER the explicitly
    /// named default generation, and only then recency. "Newest row" alone
    /// was a proxy for "the default" and the two diverged immediately — 0073
    /// inserts haiku LAST, so on a fresh install every agent silently spawned
    /// on the weakest current model with a 200K window while the picker
    /// offered Opus 5. The name below is the single source both this resolver
    /// and the compiled last-resort constant read, so the two notions of
    /// "default" cannot split again.
    pub async fn default_spawn_model(&self) -> Result<Option<Model>> {
        if let Some(m) = sqlx::query_as::<_, Model>(
            "SELECT * FROM models WHERE model_name = ? ORDER BY rowid LIMIT 1",
        )
        .bind(DEFAULT_SPAWN_MODEL_NAME)
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(Some(m));
        }
        let row = sqlx::query_as::<_, Model>(
            "SELECT * FROM models ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
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

    pub async fn delete_setting(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM app_settings WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .with_context(|| format!("deleting app_setting {key}"))?;
        Ok(())
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

    /// Whether the Track-A workflow-adherence nudges fire. Opt-OUT: unset, any
    /// value but "0", or a read error → ON (the default); `adherence_nudges ==
    /// "0"` → OFF (revert to pure prompt-driven behavior).
    pub async fn adherence_nudges_enabled(&self) -> bool {
        !matches!(
            self.get_setting(ADHERENCE_NUDGES_KEY).await,
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
            context_window: None,
            cli_settings: None,
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

    /// 0079: `cli_settings` round-trips through upsert → get / list, on both the
    /// projected reads and the `SELECT *` fallback path, and an edit that drops
    /// it clears the column (the upsert writes it on the conflict branch too).
    #[tokio::test]
    async fn cli_settings_roundtrip_and_clear() {
        let s = Storage::memory().await.unwrap();
        let mut m = model("m1", "Fable 5.1");
        m.model_name = "claude-fable-5-1".into();
        m.cli_settings = Some(r#"{"modelOverrides":{"claude-fable-5":"claude-fable-5-1"}}"#.into());
        s.upsert_model(&m).await.unwrap();
        let got = s.get_model("m1").await.unwrap().unwrap();
        assert_eq!(got.cli_settings, m.cli_settings);
        let listed = s.list_models().await.unwrap();
        assert_eq!(
            listed.iter().find(|x| x.id == "m1").unwrap().cli_settings,
            m.cli_settings
        );
        // The `SELECT *` path (default_spawn_model's shape) projects it too.
        let star = sqlx::query_as::<_, Model>("SELECT * FROM models WHERE id = 'm1'")
            .fetch_one(&s.pool)
            .await
            .unwrap();
        assert_eq!(star.cli_settings, m.cli_settings);

        let mut cleared = model("m1", "Fable 5.1");
        cleared.cli_settings = None;
        s.upsert_model(&cleared).await.unwrap();
        assert_eq!(s.get_model("m1").await.unwrap().unwrap().cli_settings, None);
    }

    /// `model_by_name` prefers the duplicate that carries `cli_settings`
    /// (EYES b528ad35): an older seeded row beside a newer user-edited one must
    /// lend the newer row's setting, and clearing the newer row's setting must
    /// not resurrect an older row's stale one — unless that older row is the
    /// only one still carrying one, which is the borrow doing its job.
    #[tokio::test]
    async fn model_by_name_prefers_the_duplicate_that_carries_cli_settings() {
        let s = Storage::memory().await.unwrap();
        let mk = |id: &str, cli: Option<&str>| {
            let mut m = model(id, id);
            m.model_name = "claude-fable-5-1".into();
            m.cli_settings = cli.map(str::to_string);
            m
        };
        // Oldest row first (lowest rowid), no setting; the user's newer row has it.
        s.upsert_model(&mk("seeded-old", None)).await.unwrap();
        s.upsert_model(&mk("user-new", Some(r#"{"modelOverrides":{}}"#))).await.unwrap();
        let picked = s.model_by_name("claude-fable-5-1").await.unwrap().unwrap();
        assert_eq!(picked.id, "user-new", "the row with a setting wins over the older one");

        // Both carry one → oldest wins, deterministically.
        s.upsert_model(&mk("seeded-old", Some(r#"{"a":1}"#))).await.unwrap();
        let picked = s.model_by_name("claude-fable-5-1").await.unwrap().unwrap();
        assert_eq!(picked.id, "seeded-old");

        // Neither carries one → still a row (the caller borrows `None`), oldest.
        s.upsert_model(&mk("seeded-old", None)).await.unwrap();
        s.upsert_model(&mk("user-new", None)).await.unwrap();
        let picked = s.model_by_name("claude-fable-5-1").await.unwrap().unwrap();
        assert_eq!(picked.id, "seeded-old");
        assert_eq!(picked.cli_settings, None);

        assert!(s.model_by_name("claude-nowhere").await.unwrap().is_none());
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
    async fn adherence_nudges_enabled_is_opt_out() {
        let s = Storage::memory().await.unwrap();
        // Unset → nudges on by default.
        assert!(s.adherence_nudges_enabled().await);
        // "0" → disabled.
        s.set_setting(ADHERENCE_NUDGES_KEY, "0").await.unwrap();
        assert!(!s.adherence_nudges_enabled().await);
        // Any other value → on.
        s.set_setting(ADHERENCE_NUDGES_KEY, "1").await.unwrap();
        assert!(s.adherence_nudges_enabled().await);
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
