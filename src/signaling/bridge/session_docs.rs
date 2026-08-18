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

/// Cap on archived versions per phase doc. Past this the oldest slot is left
/// alone and the newest archive is overwritten — bounded storage beats an
/// unbounded loop on a doc rewritten hundreds of times.
const MAX_DOC_ARCHIVES: u32 = 50;

impl SignalingBridge {
    /// How one participant of a session is NAMED (rc3 D10's display rule), or
    /// `None` when storage isn't wired, the roster has no such slug, or the read
    /// failed. Every one of those is a reason to write an unattributed heading
    /// rather than to guess a name or to fail the write.
    async fn participant_display_name(&self, session_id: &str, slug: &str) -> Option<String> {
        let storage = self.storage.lock().await.clone()?;
        match storage.participant_by_slug(session_id, slug).await {
            Ok(Some(p)) => Some(storage.display_name_of(&p).await),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(%session_id, %slug, ?e, "naming the doc's author failed");
                None
            }
        }
    }

    /// Archive the current body of `slug` as an untagged scratch doc
    /// (`{slug}@{n}`) before a phase-keyed rewrite replaces it. Phase docs are
    /// deliberately single-slot (one rewritable doc per IPAV phase), which in
    /// the 2026-07-27 archive study destroyed a session's primary deliverable:
    /// a 23-finding audit lived in the `apply` doc and four later batch writes
    /// erased it. Archives are untagged so they stay out of the IPAV tabs and
    /// `phase=`-filtered searches, but remain reachable via plain
    /// `session_doc_search` / `session_doc_read`. Returns the archive slug when
    /// one was written. Only called for phase-tagged writes — untagged scratch
    /// docs are caller-managed and rewriting them is routine, not data loss.
    async fn archive_superseded_doc(
        storage: &crate::storage::Storage,
        session_id: &str,
        slug: &str,
        new_body: &str,
    ) -> Option<String> {
        let existing = storage
            .session_document_by_slug(session_id, slug)
            .await
            .ok()
            .flatten()?;
        if existing.body == new_body {
            return None;
        }
        // One read of the occupied slots (round 10) — this used to probe
        // `{slug}@1`, `{slug}@2`, … with a SELECT each until it found a free
        // one, up to fifty round-trips per phase-doc rewrite. Same slot rule as
        // before: the first free number, and past the cap the newest archive
        // (`@MAX`) is overwritten so storage stays bounded.
        let occupied = storage
            .session_document_archive_slots(session_id, slug)
            .await
            .unwrap_or_default();
        let n = (1..=MAX_DOC_ARCHIVES)
            .find(|n| !occupied.contains(n))
            .unwrap_or(MAX_DOC_ARCHIVES);
        let candidate = format!("{slug}@{n}");
        storage
            .upsert_session_document(session_id, &candidate, &existing.body, None)
            .await
            .ok()?;
        Some(candidate)
    }
    /// Agent-callable: upsert a per-session scratch document. Phase-tagged
    /// writes are keyed by phase (one rewritable doc per IPAV phase — see
    /// `effective_slug`); untagged writes are keyed by `slug`.
    ///
    /// `append` adds to the existing body instead of replacing it, under a
    /// timestamped separator. One rewritable doc per phase is right for linear
    /// work, but a phase that ships several slices had only two options: rewrite
    /// the whole doc each time (so it silently went stale when nobody did) or
    /// spawn a second doc (which the phase key forbids). Appending makes a
    /// multi-slice phase additive. Nothing is archived on an append — nothing is
    /// superseded. Filed from a live session as feedback #3, where an apply doc
    /// still cited figures three slices out of date and was the first artifact
    /// the reviewer pulled.
    pub async fn session_doc_write(
        &self,
        session_id: &str,
        slug: &str,
        body: &str,
        phase: Option<&str>,
        append: bool,
    ) -> Result<i64> {
        let id = {
            let Some(storage) = self.storage.lock().await.clone() else {
                return Err(anyhow::anyhow!("storage not configured"));
            };
            let key = effective_slug(slug, phase);
            // Append only has meaning against an existing doc; appending to a
            // missing one is just a write.
            let existing = if append {
                storage
                    .session_document_by_slug(session_id, key)
                    .await
                    .ok()
                    .flatten()
                    .map(|d| d.body)
            } else {
                None
            };
            let composed;
            let body = match existing {
                Some(prev) => {
                    composed = format!(
                        "{prev}\n\n---\n_appended {}_\n\n{body}",
                        crate::storage::now_utc()
                    );
                    composed.as_str()
                }
                None => body,
            };
            // Archiving exists to preserve a body about to be REPLACED. An
            // append replaces nothing, so archiving it would just duplicate the
            // prefix into the archive on every slice.
            if phase.is_some() && !append {
                Self::archive_superseded_doc(&storage, session_id, key, body).await;
            }
            storage
                .upsert_session_document(session_id, key, body, phase)
                .await?
        };
        // Notify the UI so the doc pane refreshes without a manual tab-switch.
        let _ = self.event_tx.send(SignalingEvent::DocChanged {
            session_id: session_id.to_string(),
        });
        Ok(id)
    }

    /// Reviewer-callable: contribute findings to a phase WITHOUT clobbering the
    /// executor's single per-phase doc. A plain `session_doc_write` overwrites
    /// the whole body on each upsert, so appending a review section into that
    /// doc would be lost the next time it is rewritten. Instead this writes a
    /// co-located, attributed doc keyed by `<phase>-eyes` and tagged with the
    /// SAME `phase`, so it renders in the same IPAV tab alongside the executor's
    /// doc. Rewritable (the reviewer owns this slug — repeated writes overwrite
    /// its own doc, no header spam) and clobber-proof in both directions.
    /// Returns the row id + slug.
    ///
    /// The `<phase>-eyes` SLUG is fixed: migration 0049's role prose promises it
    /// by name (`e.g. plan-eyes`) and migrations are immutable, so renaming it
    /// here would make a shipped prompt lie.
    ///
    /// `author_slug` is the writing participant, used only for the header.
    /// **rc3 D10: the header is a roster fact, not the constant `(Rain)`.** It
    /// resolves through [`Storage::display_name_of`], so a third role reviewing
    /// is attributed as itself instead of as somebody else; an unreadable roster
    /// degrades to an unattributed header rather than to a wrong name.
    ///
    /// `append` means the same thing it means on [`Self::session_doc_write`]:
    /// the new body lands under a timestamped separator below the existing
    /// review doc, nothing is archived. Round 9: this path took no `append` at
    /// all, so a reviewer's `mode:"append"` — the mode the descriptor sells to
    /// every caller — silently REPLACED its own findings; the participant the
    /// `<phase>-eyes` redirect exists to protect was the one it destroyed for.
    pub async fn session_doc_write_eyes(
        &self,
        session_id: &str,
        phase: &str,
        body: &str,
        author_slug: &str,
        append: bool,
    ) -> Result<(i64, String)> {
        let slug = format!("{phase}-eyes");
        let author = self.participant_display_name(session_id, author_slug).await;
        let heading = match author {
            Some(name) => format!("### Review findings — {name}"),
            None => "### Review findings".to_string(),
        };
        let id = {
            let Some(storage) = self.storage.lock().await.clone() else {
                return Err(anyhow::anyhow!("storage not configured"));
            };
            let existing = if append {
                storage
                    .session_document_by_slug(session_id, &slug)
                    .await
                    .ok()
                    .flatten()
                    .map(|d| d.body)
            } else {
                None
            };
            let composed = match existing {
                // The heading is already at the top of the existing doc; an
                // appended slice goes under the separator, not under a second
                // heading.
                Some(prev) => format!(
                    "{prev}\n\n---\n_appended {}_\n\n{body}",
                    crate::storage::now_utc()
                ),
                None => format!("{heading}\n\n{body}"),
            };
            if !append {
                Self::archive_superseded_doc(&storage, session_id, &slug, &composed).await;
            }
            storage
                .upsert_session_document(session_id, &slug, &composed, Some(phase))
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
        let Some(storage) = self.storage.lock().await.clone() else {
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
        let Some(storage) = self.storage.lock().await.clone() else {
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
    async fn the_review_doc_survives_the_executors_rewrite() {
        // The justification for the co-located design over read-append-write: a
        // plain `session_doc_write` overwrites the whole doc body, so a review
        // section appended INTO the executor's doc would be lost on its next
        // rewrite. The `<phase>-eyes` doc is a separate row — it survives the
        // executor rewriting its plan, and that doc survives the reviewer
        // rewriting its own. Clobber-proof both ways.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge
            .session_doc_write("s1", "plan", "executor v1", Some("plan"), false)
            .await
            .unwrap();
        let (_, eyes_slug) = bridge
            .session_doc_write_eyes("s1", "plan", "the reviewer's notes", "eyes", false)
            .await
            .unwrap();
        assert_eq!(eyes_slug, "plan-eyes");

        // The executor rewrites its plan doc — the review must survive.
        bridge
            .session_doc_write("s1", "plan", "executor v2", Some("plan"), false)
            .await
            .unwrap();

        let docs = bridge
            .session_doc_search("s1", None, Some("plan"))
            .await
            .unwrap();
        assert_eq!(docs.len(), 2, "the plan doc and plan-eyes both persist");
        let eyes = docs
            .iter()
            .find(|d| d.slug == "plan-eyes")
            .expect("review doc survives the executor's rewrite");
        assert!(
            eyes.body.contains("the reviewer's notes"),
            "the review survives the executor's rewrite"
        );
        // No roster on this session, so the author cannot be named — the
        // heading degrades to the unattributed form rather than guessing.
        assert!(eyes.body.contains("### Review findings"));
        let plan = docs.iter().find(|d| d.slug == "plan").unwrap();
        assert_eq!(
            plan.body, "executor v2",
            "the executor's doc updated, not clobbered by the review"
        );
    }

    /// The review doc's heading is a ROSTER FACT (rc3 D10), not the constant
    /// `(Rain)` it used to be — it is whatever the writing participant is
    /// displayed as, `role · model`.
    ///
    /// The join under test is `author slug → participant row → role + model →
    /// heading`. Every link is real here: a migrated database, a roster seeded
    /// from the roles table, and the same `display_name_of` the spawn path uses
    /// to name peers in the prompt. Asserting a literal heading string instead
    /// would pass just as happily with the name hardcoded back.
    #[tokio::test]
    async fn the_review_heading_names_the_writer_by_role_and_model() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        storage.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
        let reviewer = storage
            .participants_for_session("s1")
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.slug == "eyes")
            .expect("the seeded roster carries the EYES role");
        let expected = storage.display_name_of(&reviewer).await;

        bridge
            .session_doc_write_eyes("s1", "plan", "the review", "eyes", false)
            .await
            .unwrap();

        let doc = bridge
            .session_doc_read("s1", "plan-eyes")
            .await
            .unwrap()
            .expect("the review doc");
        assert!(
            doc.body.contains(&format!("### Review findings — {expected}")),
            "heading must name the writer as the roster displays it ({expected}); got: {}",
            doc.body.lines().next().unwrap_or("")
        );
    }

    #[tokio::test]
    async fn phase_doc_rewrite_archives_superseded_body() {
        // 2026-07-27 archive study: four batch rewrites of the `apply` doc
        // destroyed a 23-finding audit. A phase-keyed rewrite must archive the
        // old body as an untagged `{slug}@{n}` scratch doc first.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge
            .session_doc_write("s1", "apply", "the 23-finding audit", Some("apply"), false)
            .await
            .unwrap();
        bridge
            .session_doc_write("s1", "apply", "batch B changelog", Some("apply"), false)
            .await
            .unwrap();
        bridge
            .session_doc_write("s1", "apply", "batch C changelog", Some("apply"), false)
            .await
            .unwrap();

        let v1 = bridge.session_doc_read("s1", "apply@1").await.unwrap();
        let v2 = bridge.session_doc_read("s1", "apply@2").await.unwrap();
        let head = bridge.session_doc_read("s1", "apply").await.unwrap().unwrap();
        assert_eq!(v1.expect("first archive").body, "the 23-finding audit");
        assert_eq!(v2.expect("second archive").body, "batch B changelog");
        assert_eq!(head.body, "batch C changelog");

        // Archives are untagged: invisible to phase-filtered search (IPAV tabs)…
        let phase_docs = bridge.session_doc_search("s1", None, Some("apply")).await.unwrap();
        assert!(
            phase_docs.iter().all(|d| !d.slug.contains('@')),
            "archives must not surface in phase-filtered searches"
        );
        // …but reachable by plain search.
        let all = bridge.session_doc_search("s1", Some("apply@"), None).await.unwrap();
        assert_eq!(all.len(), 2, "both archives discoverable via plain search");
    }

    #[tokio::test]
    async fn append_accumulates_slices_instead_of_replacing() {
        // Feedback #3: a phase that ships several slices had only bad options —
        // rewrite the whole doc each time (so it goes stale when nobody does) or
        // open a second doc (which the phase key forbids). Append makes the
        // multi-slice case additive.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge
            .session_doc_write("s1", "apply", "slice 1: canaries", Some("apply"), false)
            .await
            .unwrap();
        bridge
            .session_doc_write("s1", "apply", "slice 2: url_clicks fix", Some("apply"), true)
            .await
            .unwrap();
        bridge
            .session_doc_write("s1", "apply", "slice 3: segment rename", Some("apply"), true)
            .await
            .unwrap();

        let head = bridge.session_doc_read("s1", "apply").await.unwrap().unwrap();
        // Every slice survives — the staleness in the report came from earlier
        // slices being replaced by later ones.
        assert!(head.body.contains("slice 1: canaries"));
        assert!(head.body.contains("slice 2: url_clicks fix"));
        assert!(head.body.contains("slice 3: segment rename"));
        assert_eq!(head.body.matches("_appended ").count(), 2, "one marker per append");

        // An append supersedes nothing, so it must not archive — otherwise each
        // slice would duplicate the whole accumulated prefix into an archive.
        let archives = bridge.session_doc_search("s1", Some("apply@"), None).await.unwrap();
        assert!(archives.is_empty(), "append must not archive; got {archives:?}");
    }

    #[tokio::test]
    async fn append_to_a_missing_doc_is_just_a_write() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge
            .session_doc_write("s1", "verify", "first ever", Some("verify"), true)
            .await
            .unwrap();
        let head = bridge.session_doc_read("s1", "verify").await.unwrap().unwrap();
        assert_eq!(head.body, "first ever", "no separator with nothing to separate");
    }

    #[tokio::test]
    async fn replace_still_archives_after_an_append() {
        // Append and replace have to coexist: a slice-appended doc that is then
        // deliberately rewritten must still preserve the accumulated body.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge
            .session_doc_write("s1", "apply", "slice 1", Some("apply"), false)
            .await
            .unwrap();
        bridge
            .session_doc_write("s1", "apply", "slice 2", Some("apply"), true)
            .await
            .unwrap();
        bridge
            .session_doc_write("s1", "apply", "full rewrite", Some("apply"), false)
            .await
            .unwrap();

        let archived = bridge.session_doc_read("s1", "apply@1").await.unwrap();
        let body = archived.expect("the rewrite archives the accumulated body").body;
        assert!(body.contains("slice 1") && body.contains("slice 2"));
        let head = bridge.session_doc_read("s1", "apply").await.unwrap().unwrap();
        assert_eq!(head.body, "full rewrite");
    }

    #[tokio::test]
    async fn same_body_rewrite_and_untagged_docs_do_not_archive() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        // Identical-body rewrite: no archive row.
        bridge.session_doc_write("s1", "plan", "same", Some("plan"), false).await.unwrap();
        bridge.session_doc_write("s1", "plan", "same", Some("plan"), false).await.unwrap();
        assert!(bridge.session_doc_read("s1", "plan@1").await.unwrap().is_none());

        // Untagged scratch docs are caller-managed: rewriting is routine, not loss.
        bridge.session_doc_write("s1", "scratch", "v1", None, false).await.unwrap();
        bridge.session_doc_write("s1", "scratch", "v2", None, false).await.unwrap();
        assert!(bridge.session_doc_read("s1", "scratch@1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn eyes_phase_doc_rewrite_archives_too() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge.session_doc_write_eyes("s1", "verify", "verdict v1", "eyes", false).await.unwrap();
        bridge.session_doc_write_eyes("s1", "verify", "verdict v2", "eyes", false).await.unwrap();

        let archived = bridge
            .session_doc_read("s1", "verify-eyes@1")
            .await
            .unwrap()
            .expect("superseded eyes verdict archived");
        assert!(archived.body.contains("verdict v1"));
        assert!(archived.phase.is_none(), "archive is untagged");
    }

    /// Round 9: `mode:"append"` reached the reviewer branch and was DROPPED —
    /// `session_doc_write_eyes` took no `append`, archived, and replaced. A
    /// reviewer appending its second slice of findings destroyed the first,
    /// which is precisely the participant the co-located doc exists to serve.
    /// RED before the fix: the second body replaced the first.
    #[tokio::test]
    async fn a_reviewers_append_keeps_the_earlier_findings() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge
            .session_doc_write_eyes("s1", "investigate", "E1 the wire is unpinned", "eyes", false)
            .await
            .unwrap();
        bridge
            .session_doc_write_eyes("s1", "investigate", "E2 a stray doc line", "eyes", true)
            .await
            .unwrap();

        let doc = bridge
            .session_doc_read("s1", "investigate-eyes")
            .await
            .unwrap()
            .expect("the review doc exists");
        assert!(doc.body.contains("E1 the wire is unpinned"), "first slice lost: {}", doc.body);
        assert!(doc.body.contains("E2 a stray doc line"), "second slice missing: {}", doc.body);
        assert!(doc.body.contains("_appended "), "no separator: {}", doc.body);
        assert_eq!(doc.body.matches("### Review findings").count(), 1, "one heading, not two");
        assert_eq!(doc.phase.as_deref(), Some("investigate"));
        // An append supersedes nothing — no archive row.
        assert!(bridge.session_doc_read("s1", "investigate-eyes@1").await.unwrap().is_none());
    }
}
