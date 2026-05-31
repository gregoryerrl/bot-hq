//! Per-session scratch documents (the `session_doc_*` MCP tools). Thin
//! async wrappers over the storage layer; empty/None results when storage
//! isn't wired (test bridges built via `new()`).

use super::*;

/// Resolve the storage slug for a doc write. Phase-tagged docs are keyed by
/// their phase, so there is exactly ONE rewritable doc per IPAV phase: an
/// agent that varies the slug across a phase (`plan-v1`, `plan-v2`) still
/// overwrites the single `plan` doc rather than accumulating versions.
/// Untagged scratch docs keep their caller-chosen slug (many allowed per
/// session).
fn effective_slug<'a>(slug: &'a str, phase: Option<&'a str>) -> &'a str {
    phase.unwrap_or(slug)
}

impl SignalingBridge {
    /// Agent-callable: upsert a per-session scratch document. Phase-tagged
    /// writes are keyed by phase (one rewritable doc per IPAV phase — see
    /// `effective_slug`); untagged writes are keyed by `slug`.
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
            .upsert_session_document(session_id, effective_slug(slug, phase), body, phase)
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

#[cfg(test)]
mod tests {
    use super::effective_slug;

    #[test]
    fn phase_tagged_writes_collapse_to_one_slug_per_phase() {
        // Varying the slug within a phase still resolves to the phase name,
        // so repeated writes overwrite one row instead of versioning.
        assert_eq!(effective_slug("plan-v1", Some("plan")), "plan");
        assert_eq!(effective_slug("plan-v2", Some("plan")), "plan");
        assert_eq!(effective_slug("findings-x", Some("investigate")), "investigate");
    }

    #[test]
    fn untagged_scratch_keeps_caller_slug() {
        assert_eq!(effective_slug("findings-broadcast", None), "findings-broadcast");
        assert_eq!(effective_slug("notes", None), "notes");
    }
}
