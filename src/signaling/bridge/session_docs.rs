//! Per-session scratch documents (the `session_doc_*` MCP tools). Thin
//! async wrappers over the storage layer; empty/None results when storage
//! isn't wired (test bridges built via `new()`).

use super::*;

impl SignalingBridge {
    /// Agent-callable: upsert a per-session scratch document by slug.
    /// Optional `phase` tags the doc for the IPAV document tab + phase-filtered search.
    pub async fn session_doc_write(
        &self,
        session_id: &str,
        slug: &str,
        body: &str,
        phase: Option<&str>,
    ) -> Result<i64> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Err(anyhow::anyhow!("storage not configured"));
        };
        storage
            .upsert_session_document(session_id, slug, body, phase)
            .await
    }

    /// Agent-callable: search this session's docs (slug + body substring).
    /// Optional `phase` restricts results to docs tagged with that IPAV phase.
    pub async fn session_doc_search(
        &self,
        session_id: &str,
        query: Option<&str>,
        phase: Option<&str>,
    ) -> Result<Vec<crate::storage::SessionDocument>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage
            .session_documents_for(session_id, query, phase)
            .await
    }

    /// Agent-callable: read one session doc by slug.
    pub async fn session_doc_read(
        &self,
        session_id: &str,
        slug: &str,
    ) -> Result<Option<crate::storage::SessionDocument>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(None);
        };
        storage.session_document_by_slug(session_id, slug).await
    }
}
