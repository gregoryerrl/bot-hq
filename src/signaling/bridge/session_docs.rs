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
        let id = {
            let storage_guard = self.storage.lock().await;
            let Some(storage) = storage_guard.as_ref() else {
                return Err(anyhow::anyhow!("storage not configured"));
            };
            storage
                .upsert_session_document(session_id, effective_slug(slug, phase), body, phase)
                .await?
        };
        // Notify the UI so the doc pane refreshes without a manual tab-switch.
        let _ = self.event_tx.send(SignalingEvent::DocChanged {
            session_id: session_id.to_string(),
        });
        Ok(id)
    }

    /// EYES-callable: contribute findings to a phase WITHOUT clobbering Brian's
    /// single per-phase doc. Brian's `session_doc_write` overwrites the whole
    /// body on each upsert, so appending an EYES section into his doc would be
    /// lost the next time he rewrites it. Instead this writes a co-located,
    /// attributed doc keyed by `<phase>-eyes` and tagged with the SAME `phase`,
    /// so it renders in the same IPAV tab alongside HANDS's doc. Rewritable
    /// (Rain owns this slug — repeated writes overwrite her own doc, no header
    /// spam) and clobber-proof in both directions. Returns the row id + slug.
    pub async fn session_doc_write_eyes(
        &self,
        session_id: &str,
        phase: &str,
        body: &str,
    ) -> Result<(i64, String)> {
        let slug = format!("{phase}-eyes");
        let attributed = format!("### EYES findings (Rain)\n\n{body}");
        let id = {
            let storage_guard = self.storage.lock().await;
            let Some(storage) = storage_guard.as_ref() else {
                return Err(anyhow::anyhow!("storage not configured"));
            };
            storage
                .upsert_session_document(session_id, &slug, &attributed, Some(phase))
                .await?
        };
        let _ = self.event_tx.send(SignalingEvent::DocChanged {
            session_id: session_id.to_string(),
        });
        Ok((id, slug))
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
    use super::*;

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

    #[tokio::test]
    async fn eyes_doc_survives_brian_rewrite() {
        // The justification for the co-located design over read-append-write:
        // Brian's `session_doc_write` overwrites his whole doc body, so an EYES
        // section appended INTO his doc would be lost on his next rewrite. The
        // `<phase>-eyes` doc is a separate row — it survives Brian rewriting his
        // plan, and his doc survives Rain rewriting hers. Clobber-proof both ways.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge
            .session_doc_write("s1", "plan", "brian v1", Some("plan"))
            .await
            .unwrap();
        let (_, eyes_slug) = bridge
            .session_doc_write_eyes("s1", "plan", "rain's review")
            .await
            .unwrap();
        assert_eq!(eyes_slug, "plan-eyes");

        // Brian rewrites his plan doc — Rain's findings must survive.
        bridge
            .session_doc_write("s1", "plan", "brian v2", Some("plan"))
            .await
            .unwrap();

        let docs = bridge
            .session_doc_search("s1", None, Some("plan"))
            .await
            .unwrap();
        assert_eq!(docs.len(), 2, "both Brian's plan and Rain's plan-eyes persist");
        let eyes = docs
            .iter()
            .find(|d| d.slug == "plan-eyes")
            .expect("eyes doc survives Brian's rewrite");
        assert!(
            eyes.body.contains("rain's review"),
            "Rain's findings survive Brian's rewrite"
        );
        assert!(eyes.body.contains("### EYES findings (Rain)"));
        let plan = docs.iter().find(|d| d.slug == "plan").unwrap();
        assert_eq!(plan.body, "brian v2", "Brian's doc updated, not clobbered by Rain");
    }
}
