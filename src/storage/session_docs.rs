//! `session_documents` table: per-session scratch docs, optionally tagged
//! with an IPAV phase for the session view's document tabs.

use super::*;

impl Storage {
    /// Upsert a per-session document by (session_id, slug). On conflict the
    /// body is overwritten, `phase` is replaced, and `updated_at` is refreshed;
    /// `created_at` is preserved. `phase` is the IPAV phase tag — one of
    /// `investigate` / `plan` / `apply` / `verify` — used by the session view's
    /// document tabs and by phase-filtered searches. Untagged docs (`None`)
    /// are session-scoped scratch invisible to tabs and phase searches.
    /// Returns the row id.
    pub async fn upsert_session_document(
        &self,
        session_id: &str,
        slug: &str,
        body: &str,
        phase: Option<&str>,
    ) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let res = sqlx::query(
            "INSERT INTO session_documents (session_id, slug, body, created_at, updated_at, phase) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(session_id, slug) DO UPDATE SET \
               body = excluded.body, \
               updated_at = excluded.updated_at, \
               phase = excluded.phase",
        )
        .bind(session_id)
        .bind(slug)
        .bind(body)
        .bind(&now)
        .bind(&now)
        .bind(phase)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upsert session_documents session={session_id} slug={slug}"))?;
        // last_insert_rowid is 0 on the UPDATE branch; re-fetch when needed.
        let id = if res.last_insert_rowid() != 0 {
            res.last_insert_rowid()
        } else {
            let row: (i64,) = sqlx::query_as(
                "SELECT id FROM session_documents WHERE session_id = ? AND slug = ?",
            )
            .bind(session_id)
            .bind(slug)
            .fetch_one(&self.pool)
            .await?;
            row.0
        };
        Ok(id)
    }

    /// Search a session's documents. Optional `query` is a case-insensitive
    /// substring filter across slug + body. Optional `phase` filters to a
    /// specific IPAV phase tag. Ordered newest-first.
    pub async fn session_documents_for(
        &self,
        session_id: &str,
        query: Option<&str>,
        phase: Option<&str>,
    ) -> Result<Vec<SessionDocument>> {
        let like = query.map(|q| format!("%{}%", q.to_lowercase()));
        let mut sql = String::from(
            "SELECT id, session_id, slug, body, created_at, updated_at, phase \
             FROM session_documents WHERE session_id = ?",
        );
        if like.is_some() {
            sql.push_str(" AND (LOWER(slug) LIKE ? OR LOWER(body) LIKE ?)");
        }
        if phase.is_some() {
            sql.push_str(" AND phase = ?");
        }
        sql.push_str(" ORDER BY updated_at DESC");

        let mut q = sqlx::query_as::<_, SessionDocument>(&sql).bind(session_id);
        if let Some(l) = like.as_deref() {
            q = q.bind(l).bind(l);
        }
        if let Some(p) = phase {
            q = q.bind(p);
        }
        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// Convenience wrapper: all docs tagged with `phase` for `session_id`,
    /// newest-first. Used by the session view's IPAV document tabs.
    pub async fn session_documents_for_phase(
        &self,
        session_id: &str,
        phase: &str,
    ) -> Result<Vec<SessionDocument>> {
        self.session_documents_for(session_id, None, Some(phase))
            .await
    }

    /// Fetch one document by (session_id, slug). None when not found.
    pub async fn session_document_by_slug(
        &self,
        session_id: &str,
        slug: &str,
    ) -> Result<Option<SessionDocument>> {
        let row = sqlx::query_as::<_, SessionDocument>(
            "SELECT id, session_id, slug, body, created_at, updated_at, phase \
             FROM session_documents \
             WHERE session_id = ? AND slug = ?",
        )
        .bind(session_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}

#[cfg(test)]
mod session_doc_tests {
    use super::*;

    async fn seeded() -> (Storage, &'static str, &'static str) {
        let s = Storage::memory().await.unwrap();
        s.create_session("sess-a", "a", None).await.unwrap();
        s.create_session("sess-b", "b", None).await.unwrap();
        (s, "sess-a", "sess-b")
    }

    #[tokio::test]
    async fn upsert_then_read_by_slug() {
        let (s, a, _) = seeded().await;
        let id = s
            .upsert_session_document(a, "plan-v1", "first body", None)
            .await
            .unwrap();
        assert!(id > 0);
        let doc = s
            .session_document_by_slug(a, "plan-v1")
            .await
            .unwrap()
            .expect("doc should exist");
        assert_eq!(doc.slug, "plan-v1");
        assert_eq!(doc.body, "first body");
        assert_eq!(doc.session_id, "sess-a");
    }

    #[tokio::test]
    async fn upsert_is_idempotent_overwrites_body() {
        let (s, a, _) = seeded().await;
        let id1 = s
            .upsert_session_document(a, "findings", "v1", None)
            .await
            .unwrap();
        let id2 = s
            .upsert_session_document(a, "findings", "v2", None)
            .await
            .unwrap();
        assert_eq!(id1, id2, "same slug should return same row id");
        let doc = s
            .session_document_by_slug(a, "findings")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.body, "v2");
        // Only one row total for this slug.
        let all = s.session_documents_for(a, None, None).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn search_filters_by_query_across_slug_and_body() {
        let (s, a, _) = seeded().await;
        s.upsert_session_document(a, "plan-v1", "rewrites broadcast.rs", None)
            .await
            .unwrap();
        s.upsert_session_document(a, "findings-perf", "irrelevant", None)
            .await
            .unwrap();
        let hits_slug = s
            .session_documents_for(a, Some("plan"), None)
            .await
            .unwrap();
        assert_eq!(hits_slug.len(), 1);
        assert_eq!(hits_slug[0].slug, "plan-v1");
        let hits_body = s
            .session_documents_for(a, Some("broadcast"), None)
            .await
            .unwrap();
        assert_eq!(hits_body.len(), 1);
        assert_eq!(hits_body[0].slug, "plan-v1");
        let no_query = s.session_documents_for(a, None, None).await.unwrap();
        assert_eq!(no_query.len(), 2);
    }

    #[tokio::test]
    async fn docs_are_isolated_per_session() {
        let (s, a, b) = seeded().await;
        s.upsert_session_document(a, "plan", "for a", None)
            .await
            .unwrap();
        let in_b = s.session_documents_for(b, None, None).await.unwrap();
        assert!(in_b.is_empty(), "session B sees no docs from A: {in_b:?}");
        let read_in_b = s.session_document_by_slug(b, "plan").await.unwrap();
        assert!(read_in_b.is_none(), "session B can't read A's slug");
    }

    #[tokio::test]
    async fn unknown_slug_returns_none() {
        let (s, a, _) = seeded().await;
        let row = s
            .session_document_by_slug(a, "nope")
            .await
            .unwrap();
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn phase_filter_returns_only_matching_docs() {
        let (s, a, _) = seeded().await;
        s.upsert_session_document(a, "plan-v1", "x", Some("plan"))
            .await
            .unwrap();
        s.upsert_session_document(a, "find-1", "y", Some("investigate"))
            .await
            .unwrap();
        s.upsert_session_document(a, "scratch", "z", None)
            .await
            .unwrap();
        let plans = s.session_documents_for_phase(a, "plan").await.unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].slug, "plan-v1");
        assert_eq!(plans[0].phase.as_deref(), Some("plan"));
        let all = s.session_documents_for(a, None, None).await.unwrap();
        assert_eq!(all.len(), 3, "no filter returns all docs including untagged");
    }

    #[tokio::test]
    async fn upsert_overwrites_phase() {
        let (s, a, _) = seeded().await;
        s.upsert_session_document(a, "doc", "v1", Some("plan"))
            .await
            .unwrap();
        s.upsert_session_document(a, "doc", "v2", Some("apply"))
            .await
            .unwrap();
        let doc = s
            .session_document_by_slug(a, "doc")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.body, "v2");
        assert_eq!(doc.phase.as_deref(), Some("apply"));
    }
}
