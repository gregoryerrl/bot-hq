//! The trust hash Codex reports for bot-hq's gate hook in a managed Codex
//! home (0103): cached after the first spawn so later spawns grant it up front.

use super::*;

/// A cached trust grant for one hook key in one Codex home.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CodexHookTrustRow {
    pub home: String,
    pub key: String,
    pub trusted_hash: String,
    pub codex_version: String,
    pub updated_at: String,
}

impl Storage {
    /// Candidate keys as Codex reported them, restricted to the exact selected
    /// credential slot. The caller verifies physical hook identity before use;
    /// SQL must not canonicalize the home or manufacture a new trust key.
    pub async fn codex_hook_trust_candidates(&self, home: &str) -> Result<Vec<CodexHookTrustRow>> {
        sqlx::query_as::<_, CodexHookTrustRow>(
            "SELECT home, key, trusted_hash, codex_version, updated_at FROM codex_hook_trust \
             WHERE home = ? ORDER BY updated_at DESC, key",
        )
        .bind(home)
        .fetch_all(&self.pool)
        .await
        .context("reading the Codex hook trust candidates")
    }

    pub async fn codex_hook_trust(&self, home: &str, key: &str) -> Result<Option<CodexHookTrustRow>> {
        let row = sqlx::query_as::<_, CodexHookTrustRow>(
            "SELECT home, key, trusted_hash, codex_version, updated_at FROM codex_hook_trust \
             WHERE home = ? AND key = ?",
        )
        .bind(home)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .context("reading the Codex hook trust cache")?;
        Ok(row)
    }

    /// Record (or refresh) the hash for a home + key.
    pub async fn set_codex_hook_trust(
        &self,
        home: &str,
        key: &str,
        trusted_hash: &str,
        codex_version: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO codex_hook_trust (home, key, trusted_hash, codex_version, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(home, key) DO UPDATE SET \
                trusted_hash = excluded.trusted_hash, \
                codex_version = excluded.codex_version, \
                updated_at = excluded.updated_at",
        )
        .bind(home)
        .bind(key)
        .bind(trusted_hash)
        .bind(codex_version)
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .context("writing the Codex hook trust cache")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0103: a grant round-trips, a refresh replaces the hash in place (one
    /// row per home + key), and an unknown home answers nothing.
    #[tokio::test]
    async fn the_trust_cache_round_trips_and_refreshes_in_place() {
        let s = Storage::memory().await.unwrap();
        let (home, key) = ("/Users/me/.codex-bot-hq", "/Users/me/.codex-bot-hq/hooks.json:pre_tool_use:0:0");
        assert!(s.codex_hook_trust(home, key).await.unwrap().is_none());
        s.set_codex_hook_trust(home, key, "sha256:aaa", "0.163.0").await.unwrap();
        let row = s.codex_hook_trust(home, key).await.unwrap().unwrap();
        assert_eq!((row.trusted_hash.as_str(), row.codex_version.as_str()), ("sha256:aaa", "0.163.0"));
        assert!(row.updated_at.ends_with('Z'));
        s.set_codex_hook_trust(home, key, "sha256:bbb", "0.164.0").await.unwrap();
        let row = s.codex_hook_trust(home, key).await.unwrap().unwrap();
        assert_eq!(row.trusted_hash, "sha256:bbb");
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM codex_hook_trust")
            .fetch_one(&s.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        assert!(s.codex_hook_trust("/elsewhere", key).await.unwrap().is_none());
    }
}
