//! `agent_configs` table: per-agent provider/model/credential rows.

use super::*;

/// Full column projection for an `AgentConfig` — shared by every read so they
/// can't drift (mirrors `sessions.rs::SESSION_COLUMNS`).
///
/// `native` is NOT here, for the same reason as `models::MODEL_COLUMNS`: the
/// 0038 column survives, unread, because rc3 D9 deleted the runtime it chose.
const AGENT_CONFIG_COLUMNS: &str =
    "agent_name, provider, model_name, base_url, auth_token, updated_at, context_window";

impl Storage {
    pub async fn get_agent_config(&self, name: &str) -> Result<Option<AgentConfig>> {
        let row = sqlx::query_as::<_, AgentConfig>(&format!(
            "SELECT {AGENT_CONFIG_COLUMNS} FROM agent_configs WHERE agent_name = ?"
        ))
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_agent_configs(&self) -> Result<Vec<AgentConfig>> {
        let rows = sqlx::query_as::<_, AgentConfig>(&format!(
            "SELECT {AGENT_CONFIG_COLUMNS} FROM agent_configs ORDER BY agent_name"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn upsert_agent_config(&self, cfg: &AgentConfig) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_configs \
                 (agent_name, provider, model_name, base_url, auth_token, updated_at, context_window) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(agent_name) DO UPDATE SET \
                 provider = excluded.provider, \
                 model_name = excluded.model_name, \
                 base_url = excluded.base_url, \
                 auth_token = excluded.auth_token, \
                 updated_at = excluded.updated_at, \
                 context_window = excluded.context_window",
        )
        .bind(&cfg.agent_name)
        .bind(&cfg.provider)
        .bind(&cfg.model_name)
        .bind(&cfg.base_url)
        .bind(&cfg.auth_token)
        .bind(now_utc())
        .bind(cfg.context_window)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upserting agent_config {}", cfg.agent_name))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Moved here when the dead Tauri wrappers went** (2026-08-16): this
    /// round-trip lived in `tauri_cmd/agent_configs.rs`, whose three
    /// `#[tauri::command]`s had no caller in the frontend or anywhere else. The
    /// STORAGE methods it exercises are not dead — `core::session` resolves a
    /// spawn's model through `get_agent_config`, and the external MCP server
    /// exposes `get_agent_configs`/`set_agent_config` to driver clients — so
    /// deleting the wrappers would have taken their only coverage with them.
    #[tokio::test]
    async fn upsert_and_get_round_trip() {
        let storage = Storage::memory().await.unwrap();
        let cfg = AgentConfig {
            // `brian`, not `hands`, and that is not nostalgia — see the test
            // below for what this table can actually hold.
            agent_name: "brian".to_string(),
            provider: "anthropic".to_string(),
            model_name: "fast-thinker-1".to_string(),
            base_url: None,
            auth_token: Some("secret".to_string()),
            updated_at: String::new(),
            context_window: Some(200_000),
        };
        storage.upsert_agent_config(&cfg).await.unwrap();

        let fetched = storage
            .get_agent_config("brian")
            .await
            .unwrap()
            .expect("the row just written");
        assert_eq!(fetched.provider, "anthropic");
        assert_eq!(fetched.model_name, "fast-thinker-1");
        assert_eq!(fetched.auth_token.as_deref(), Some("secret"));
        // Carried but unread since rc3 D9 — still written and returned, so an
        // edit through any surface can still destroy it silently.
        assert_eq!(fetched.context_window, Some(200_000));

        assert!(storage.list_agent_configs().await.unwrap().iter().any(|c| c.agent_name == "brian"));
        assert!(storage.get_agent_config("nobody").await.unwrap().is_none());
    }

    /// **This fallback tier cannot serve any current roster** (audit B2-4), and
    /// the reason is a CHECK constraint in `0001_init.sql`: `agent_name IN
    /// ('emma', 'brian', 'rain')`. rc3 rosters use role slugs — `hands`, `eyes`,
    /// `hands-2` — so `resolve_spawn_config`'s third tier can never match one,
    /// and a spawn with no participant model and no role default falls through
    /// to the built-in default rather than to anything a user configured here.
    ///
    /// Recorded as a test rather than a comment because it is the kind of fact
    /// that reads as a bug later: the tier LOOKS live in `session.rs`, and it is
    /// — for two names nothing spawns any more. `0001` is applied and immutable,
    /// so changing it is a new migration and a decision, not a fix.
    #[tokio::test]
    async fn the_legacy_check_constraint_excludes_every_rc3_role_slug() {
        let storage = Storage::memory().await.unwrap();
        for slug in ["hands", "eyes", "hands-2"] {
            let cfg = AgentConfig {
                agent_name: slug.to_string(),
                provider: "anthropic".to_string(),
                model_name: "m".to_string(),
                base_url: None,
                auth_token: None,
                updated_at: String::new(),
                context_window: None,
            };
            assert!(
                storage.upsert_agent_config(&cfg).await.is_err(),
                "`{slug}` was accepted — if the CHECK was widened, \
                 `resolve_spawn_config`'s agent_configs tier just became \
                 reachable for real rosters and wants a second look"
            );
        }
    }
}
