//! `retrieval_events` table: append-only log of `cl_retrieve` calls (Stage 4b
//! measurement). One row per ranked retrieval so we can answer "is the CL
//! helping" with data — tokens-per-task, stale-hit rate, empty-return rate.
//! Logging is best-effort: the bridge swallows insert errors so a logging
//! failure never blocks a retrieval.

use super::*;

/// Divide `n` by `d` as f64, returning 0.0 when `d == 0` (empty scope).
fn ratio(n: i64, d: i64) -> f64 {
    if d == 0 {
        0.0
    } else {
        n as f64 / d as f64
    }
}

impl Storage {
    /// Append one retrieval event. `session_id`/`agent` are audit context and may
    /// be absent (host/UI-invoked retrievals). `returned_atoms` is a compact JSON
    /// array describing what was handed back. Returns the new row id.
    #[allow(clippy::too_many_arguments)]
    pub async fn log_retrieval_event(
        &self,
        session_id: Option<&str>,
        agent: Option<&str>,
        project_id: &str,
        query: &str,
        atom_count: i64,
        tokens_returned: i64,
        budget_tokens: i64,
        stale_count: i64,
        returned_atoms: &str,
    ) -> Result<i64> {
        let now = now_utc();
        let row_id: i64 = sqlx::query_scalar(
            "INSERT INTO retrieval_events \
                (session_id, agent, project_id, query, atom_count, tokens_returned, \
                 budget_tokens, stale_count, returned_atoms, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(session_id)
        .bind(agent)
        .bind(project_id)
        .bind(query)
        .bind(atom_count)
        .bind(tokens_returned)
        .bind(budget_tokens)
        .bind(stale_count)
        .bind(returned_atoms)
        .bind(&now)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("logging retrieval_event for {project_id}"))?;
        Ok(row_id)
    }

    /// Aggregate retrieval telemetry, optionally scoped to a project and/or a
    /// `created_at >= since` (RFC3339) window. A single query covers every filter
    /// combination via the `(? IS NULL OR col = ?)` idiom; ratios are derived in
    /// Rust.
    pub async fn retrieval_stats(
        &self,
        project_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<RetrievalStats> {
        let (event_count, distinct_sessions, total_tokens, total_atoms, stale_hits, empty_returns): (
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = sqlx::query_as(
            "SELECT \
                COUNT(*), \
                COUNT(DISTINCT session_id), \
                COALESCE(SUM(tokens_returned), 0), \
                COALESCE(SUM(atom_count), 0), \
                COALESCE(SUM(stale_count), 0), \
                COALESCE(SUM(CASE WHEN atom_count = 0 THEN 1 ELSE 0 END), 0) \
             FROM retrieval_events \
             WHERE (? IS NULL OR project_id = ?) AND (? IS NULL OR created_at >= ?)",
        )
        .bind(project_id)
        .bind(project_id)
        .bind(since)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .context("aggregating retrieval_stats")?;

        Ok(RetrievalStats {
            event_count,
            distinct_sessions,
            total_tokens,
            total_atoms,
            stale_hits,
            empty_returns,
            avg_tokens_per_event: ratio(total_tokens, event_count),
            avg_tokens_per_session: ratio(total_tokens, distinct_sessions),
            stale_hit_rate: ratio(stale_hits, total_atoms),
            empty_return_rate: ratio(empty_returns, event_count),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem() -> Storage {
        Storage::memory().await.unwrap()
    }

    #[tokio::test]
    async fn log_and_aggregate_retrieval_events() {
        let s = mem().await;
        // Two events in project "p": one 3-atom/300-tok with a stale hit, one
        // empty (retrieval miss). A third event in "other" must not leak in.
        s.log_retrieval_event(Some("s1"), Some("brian"), "p", "commit hooks", 3, 300, 3000, 1, "[]")
            .await
            .unwrap();
        s.log_retrieval_event(Some("s1"), Some("brian"), "p", "no match here", 0, 0, 3000, 0, "[]")
            .await
            .unwrap();
        s.log_retrieval_event(Some("s2"), Some("rain"), "other", "x", 5, 500, 3000, 0, "[]")
            .await
            .unwrap();

        let stats = s.retrieval_stats(Some("p"), None).await.unwrap();
        assert_eq!(stats.event_count, 2);
        assert_eq!(stats.distinct_sessions, 1);
        assert_eq!(stats.total_tokens, 300);
        assert_eq!(stats.total_atoms, 3);
        assert_eq!(stats.stale_hits, 1);
        assert_eq!(stats.empty_returns, 1);
        assert_eq!(stats.avg_tokens_per_event, 150.0);
        assert_eq!(stats.avg_tokens_per_session, 300.0);
        assert!((stats.stale_hit_rate - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(stats.empty_return_rate, 0.5);

        // Unscoped aggregation spans both projects.
        let all = s.retrieval_stats(None, None).await.unwrap();
        assert_eq!(all.event_count, 3);
        assert_eq!(all.distinct_sessions, 2);
        assert_eq!(all.total_tokens, 800);
    }

    #[tokio::test]
    async fn retrieval_stats_empty_scope_is_zero_not_nan() {
        let s = mem().await;
        let stats = s.retrieval_stats(Some("nobody"), None).await.unwrap();
        assert_eq!(stats.event_count, 0);
        assert_eq!(stats.avg_tokens_per_event, 0.0);
        assert_eq!(stats.avg_tokens_per_session, 0.0);
        assert_eq!(stats.stale_hit_rate, 0.0);
        assert_eq!(stats.empty_return_rate, 0.0);
    }

    #[tokio::test]
    async fn retrieval_stats_since_filters_by_window() {
        let s = mem().await;
        s.log_retrieval_event(Some("s1"), Some("brian"), "p", "q", 1, 100, 3000, 0, "[]")
            .await
            .unwrap();
        // A far-future `since` excludes everything already written.
        let stats = s.retrieval_stats(Some("p"), Some("2999-01-01T00:00:00Z")).await.unwrap();
        assert_eq!(stats.event_count, 0);
        // A far-past `since` includes it.
        let stats = s.retrieval_stats(Some("p"), Some("2000-01-01T00:00:00Z")).await.unwrap();
        assert_eq!(stats.event_count, 1);
    }
}
