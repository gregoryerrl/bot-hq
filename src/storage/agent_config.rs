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

    /// No production caller since rc3 D8 retired the Agents tab that listed
    /// these rows; read by `tests/storage_test.rs::migration_runs_on_empty_db`
    /// (an integration test — which is why this is not `#[cfg(test)]`) and the
    /// round-trip test below. Round 10 says so rather than deleting it: the
    /// integration test is the one place that pins "0060 seeds nothing here".
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
    /// spawn's model through `get_agent_config` (and the since-deleted external
    /// MCP server exposed `get_agent_configs`/`set_agent_config` to driver
    /// clients) — so deleting the wrappers would have taken their only coverage
    /// with them.
    #[tokio::test]
    async fn upsert_and_get_round_trip() {
        let storage = Storage::memory().await.unwrap();
        let cfg = AgentConfig {
            agent_name: "hands".to_string(),
            provider: "anthropic".to_string(),
            model_name: "fast-thinker-1".to_string(),
            base_url: None,
            auth_token: Some("secret".to_string()),
            updated_at: String::new(),
            context_window: Some(200_000),
        };
        storage.upsert_agent_config(&cfg).await.unwrap();

        let fetched = storage
            .get_agent_config("hands")
            .await
            .unwrap()
            .expect("the row just written");
        assert_eq!(fetched.provider, "anthropic");
        assert_eq!(fetched.model_name, "fast-thinker-1");
        assert_eq!(fetched.auth_token.as_deref(), Some("secret"));
        // Carried but unread since rc3 D9 — still written and returned, so an
        // edit through any surface can still destroy it silently.
        assert_eq!(fetched.context_window, Some(200_000));

        assert!(storage.list_agent_configs().await.unwrap().iter().any(|c| c.agent_name == "hands"));
        assert!(storage.get_agent_config("nobody").await.unwrap().is_none());
    }

    /// **A role slug is a legal `agent_configs` key again** — and until 0060 it
    /// was not.
    ///
    /// `0001_init.sql` declared `agent_name TEXT PRIMARY KEY CHECK (agent_name
    /// IN ('emma','brian','rain'))`. rc3 rosters use role slugs (`hands`,
    /// `eyes`, `hands-2`), so the CHECK rejected every key a live session could
    /// produce: `resolve_spawn_config`'s `agent_configs` tier LOOKED live and
    /// could only ever match two names nothing spawns any more, falling through
    /// to the built-in default instead of to anything a user had configured.
    ///
    /// This test used to assert the opposite — that every rc3 slug was
    /// REFUSED — and recorded the refusal as a fact to be aware of rather than
    /// a bug to fix, because `0001` is immutable and widening it needed a
    /// migration and a decision. 0060 is that migration: it rebuilds the table
    /// without the CHECK and drops the three seeded rows, none of which named a
    /// participant this app can create.
    ///
    /// Inverted rather than deleted, deliberately: the assertion is the same
    /// question asked of the opposite answer, so the tier's reachability stays
    /// pinned in both directions.
    #[tokio::test]
    async fn a_role_slug_is_a_legal_agent_config_key() {
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
            storage.upsert_agent_config(&cfg).await.unwrap_or_else(|e| {
                panic!("`{slug}` was refused ({e}) — the legacy CHECK is back, and \
                        `resolve_spawn_config`'s agent_configs tier is unreachable \
                        for every roster a session can produce")
            });
            let back = storage.get_agent_config(slug).await.unwrap();
            assert!(back.is_some(), "`{slug}` did not round-trip");
        }
    }

    /// The three names the dropped CHECK used to allow are gone from the table.
    /// Not "no longer special" — absent: they named two agents retired by D10
    /// and one removed in 0017. (Round 10: the fixture looped `hands`/`eyes`,
    /// which the CHECK never allowed and nothing ever seeded, so two of its
    /// three assertions could not fail; the literals below are the CHECK's own —
    /// bare string DATA, exempt from `retired_identifier_test` by design.)
    #[tokio::test]
    async fn the_retired_names_are_not_seeded_any_more() {
        let storage = Storage::memory().await.unwrap();
        for slug in ["emma", "brian", "rain"] {
            assert!(
                storage.get_agent_config(slug).await.unwrap().is_none(),
                "`{slug}` is still seeded"
            );
        }
    }
}