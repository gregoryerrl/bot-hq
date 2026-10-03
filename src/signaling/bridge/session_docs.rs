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
/// Is `slug` a name a CUSTOM document may carry? The names the pane gives
/// other kinds of document are reserved: the four phase names, the `@<n>`
/// archive slots, the `<phase>-eyes` reviewer co-docs. Length bounded so a tab
/// label stays a label.
fn custom_slug_check(slug: &str) -> Result<()> {
    let s = slug.trim();
    if s.is_empty() || s != slug {
        anyhow::bail!("a document name cannot be empty or padded with spaces");
    }
    if s.chars().count() > 64 {
        anyhow::bail!("a document name is at most 64 characters");
    }
    let is_phase_name =
        |n: &str| matches!(n.to_ascii_lowercase().as_str(), "investigate" | "plan" | "apply" | "verify");
    if is_phase_name(s) {
        anyhow::bail!("`{s}` is a phase name — the I/P/A/V documents are the participants'");
    }
    if let Some((_, n)) = s.rsplit_once('@') {
        if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
            anyhow::bail!("`{s}` looks like an archived version (`name@<n>`); pick another name");
        }
    }
    if let Some(base) = s.strip_suffix("-eyes") {
        if is_phase_name(base) {
            anyhow::bail!("`{s}` is a reviewer co-document name; pick another name");
        }
    }
    Ok(())
}

fn effective_slug<'a>(slug: &'a str, phase: Option<&'a str>) -> &'a str {
    phase.unwrap_or(slug)
}

/// Cap on archived versions per phase doc. Past this the oldest archive is
/// dropped and the rest shift down, so the newest versions survive — bounded
/// storage beats an unbounded loop on a doc rewritten hundreds of times.
const MAX_DOC_ARCHIVES: u32 = 50;

/// Cap on archived versions per UNTAGGED (custom) doc — lower, because a
/// custom doc is rewritten far more often than a phase doc (feedback #37: an
/// EOD draft went through 15 full rewrites, and a correction applied at rev 6
/// was silently reverted with no earlier revision left to diff against).
const MAX_UNTAGGED_DOC_ARCHIVES: u32 = 10;

/// Is `slug` an archived version (`name@<n>`)?
pub(crate) fn is_archive_slug(slug: &str) -> bool {
    slug.rsplit_once('@')
        .is_some_and(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// `session_doc_read`'s selective views (feedback #37: every mechanical check
/// of a doc used to round-trip its whole body through the transcript):
/// `lines` = "a-b" / "a-" / "a" (1-based, inclusive; "a-" runs to the end)
/// narrows the body; `grep` then returns only the matching lines,
/// case-insensitively, with their numbers.
///
/// The open-ended form is what a cut handoff doc's footer tells a compacted
/// participant to call (`agents::handoff::render`: `lines: "N-"`). It used to
/// be refused, so the first step after a compaction returned an error (EYES'
/// advisory `d2ced691`).
pub(crate) fn doc_excerpt(
    body: &str,
    grep: Option<&str>,
    lines: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    doc_excerpt_within(body, grep, lines, DOC_REPLY_BUDGET_BYTES)
}

/// How many bytes of document BODY one session-doc reply carries at most
/// (feedback #75 / #77). A bare `session_doc_search` once returned 476,583
/// characters — every doc's whole body — which overflowed the tool result and
/// spilled to a one-line JSON file the agent then had to slice by hand; one
/// rewritable doc per phase means a long build's `plan` or `apply` grows
/// without bound. Past the budget a reply says what was left out and how to
/// read it, instead of spilling. About 12k tokens: room for an ordinary plan
/// whole, not for a day's appended slices.
pub(crate) const DOC_REPLY_BUDGET_BYTES: usize = 48_000;

/// Headings listed for a doc whose body is not returned.
const OUTLINE_MAX_HEADINGS: usize = 80;

/// One markdown heading of a doc: its 1-based line, level (1–6) and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocHeading {
    pub line: usize,
    pub level: usize,
    pub text: String,
}

/// The ATX headings of `body` (`#` … `######`, then a space), in order, with
/// fenced code skipped — a `# comment` inside a code block is not a section.
pub(crate) fn doc_headings(body: &str) -> Vec<DocHeading> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    for (i, line) in body.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            fence = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            fence = Some("~~~");
            continue;
        }
        let hashes = line.bytes().take_while(|&b| b == b'#').count();
        if (1..=6).contains(&hashes) {
            let rest = &line[hashes..];
            if rest.starts_with(' ') || rest.starts_with('\t') {
                out.push(DocHeading { line: i + 1, level: hashes, text: rest.trim().to_string() });
            }
        }
    }
    out
}

/// A doc's outline as JSON, for a reply that leaves the body out: up to
/// [`OUTLINE_MAX_HEADINGS`] headings, each `{line, level, text}`.
pub(crate) fn doc_outline(body: &str) -> Vec<serde_json::Value> {
    doc_headings(body)
        .into_iter()
        .take(OUTLINE_MAX_HEADINGS)
        .map(|h| serde_json::json!({ "line": h.line, "level": h.level, "text": h.text }))
        .collect()
}

/// The section under the first heading whose text contains `needle`
/// (case-insensitive): its heading, and the 1-based inclusive line range from
/// that heading to the line before the next heading of the same or a higher
/// level — so a `##` section takes its `###` children with it.
pub(crate) fn doc_section(body: &str, needle: &str) -> Option<(DocHeading, usize, usize)> {
    let headings = doc_headings(body);
    let needle = needle.trim().to_lowercase();
    let at = headings.iter().position(|h| h.text.to_lowercase().contains(&needle))?;
    let start = headings[at].clone();
    let end = headings[at + 1..]
        .iter()
        .find(|h| h.level <= start.level)
        .map(|h| h.line - 1)
        .unwrap_or_else(|| body.lines().count().max(start.line));
    Some((start.clone(), start.line, end))
}

/// [`doc_excerpt`] with the byte budget as a parameter, so a test can exercise
/// the cut without a 48 KB fixture.
pub(crate) fn doc_excerpt_within(
    body: &str,
    grep: Option<&str>,
    lines: Option<&str>,
    budget: usize,
) -> anyhow::Result<serde_json::Value> {
    let all: Vec<&str> = body.lines().collect();
    let total = all.len();
    let (from, to) = match lines {
        None => (1, total.max(1)),
        Some(spec) => {
            let spec = spec.trim();
            let parse = |n: &str| {
                n.trim().parse::<usize>().map_err(|_| {
                    anyhow::anyhow!("`lines` must be \"a-b\", \"a-\" or \"a\" (1-based), got {spec:?}")
                })
            };
            let (a, b) = match spec.split_once('-') {
                // "a-": from line a to the end of the doc.
                Some((a, b)) if b.trim().is_empty() => (parse(a)?, usize::MAX),
                Some((a, b)) => (parse(a)?, parse(b)?),
                None => {
                    let a = parse(spec)?;
                    (a, a)
                }
            };
            if a == 0 || b < a {
                anyhow::bail!("`lines` must be \"a-b\" with 1 <= a <= b, got {spec:?}");
            }
            (a, b.min(total.max(1)))
        }
    };
    let window = || all.iter().enumerate().skip(from - 1).take(to.saturating_sub(from - 1));
    match grep {
        Some(pattern) => {
            let needle = pattern.to_lowercase();
            let mut spent = 0usize;
            let mut matches: Vec<serde_json::Value> = Vec::new();
            let mut left_out = 0usize;
            for (i, l) in window().filter(|(_, l)| l.to_lowercase().contains(&needle)) {
                if !matches.is_empty() && spent + l.len() > budget {
                    left_out += 1;
                    continue;
                }
                spent += l.len();
                matches.push(serde_json::json!({ "line": i + 1, "text": l }));
            }
            let mut out = serde_json::json!({ "total_lines": total, "matches": matches });
            if left_out > 0 {
                out["note"] = serde_json::json!(format!(
                    "{left_out} more matching line(s) left out at the reply budget ({budget} \
                     bytes); narrow the pattern or add `lines`"
                ));
            }
            Ok(out)
        }
        None => {
            // Whole lines up to the budget. The first line of the window is
            // always returned, however long, so the reply is never empty.
            let mut text = String::new();
            let mut last = from.saturating_sub(1);
            for (i, l) in window() {
                if last >= from && text.len() + 1 + l.len() > budget {
                    break;
                }
                if last >= from {
                    text.push('\n');
                }
                text.push_str(l);
                last = i + 1;
            }
            let shown_to = if last >= from { last } else { to };
            let mut out = serde_json::json!({
                "total_lines": total,
                "lines": format!("{from}-{shown_to}"),
                "body": text,
            });
            if last >= from && last < to {
                out["note"] = serde_json::json!(format!(
                    "cut at the reply budget ({budget} bytes): lines {from}-{last} of the \
                     {from}-{to} asked for. Continue with lines: \"{}-\"",
                    last + 1
                ));
            }
            Ok(out)
        }
    }
}

/// What a reviewer's co-doc write did (`session_doc_write_eyes`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodocWrite {
    pub id: i64,
    /// The doc that was written: `<phase>-eyes`.
    pub slug: String,
    /// The writer had a standing phase vote and this write withdrew it.
    pub vote_withdrawn: bool,
}

/// What an in-place edit of a session doc did (`session_doc_edit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocEdit {
    pub id: i64,
    pub slug: String,
    /// The doc's phase tag, unchanged by the edit.
    pub phase: Option<String>,
    pub occurrences: usize,
    pub bytes_before: usize,
    pub bytes_after: usize,
    /// Where the body as it was before the edit is kept (`<slug>@<n>`).
    pub archived_as: Option<String>,
}

/// How the "review notes landed" row opens, for `who` and one co-doc. The
/// dedupe below looks for exactly this, so the two cannot drift.
fn codoc_notice_opening(who: &str, slug: &str) -> String {
    format!("[System: {who} wrote review notes to `{slug}`")
}

/// Has the "review notes landed" row for this co-doc already been posted in
/// the writer's CURRENT run — its unbroken stretch of rows at the end of the
/// channel? Host rows do not break a run; another participant's row or the
/// user's message does, and the next write after one posts a fresh row.
///
/// Read from the stored rows rather than kept in memory, so it needs no
/// turn-boundary signal and survives a relaunch. A failed read answers
/// `false`: an extra row is the cheaper mistake. Only the last 200 rows are
/// read, so a run longer than that since its notice posts a second one —
/// harmless, and cheaper than an unbounded scan on every co-doc write.
async fn codoc_noticed_this_run(
    storage: &crate::storage::Storage,
    session_id: &str,
    author_slug: &str,
    opening: &str,
) -> bool {
    use crate::storage::MessageKind;
    let Ok(tail) = storage.messages_tail(session_id, None, 200).await else {
        return false;
    };
    for row in tail.iter().rev() {
        let host_row = row.kind == MessageKind::SystemNotice.as_str()
            || row.kind == MessageKind::PhaseChange.as_str();
        if host_row {
            if row.content.starts_with(opening) {
                return true;
            }
            continue;
        }
        if row.author != author_slug {
            return false;
        }
    }
    false
}

impl SignalingBridge {
    /// How one participant of a session is NAMED (rc3 D10's display rule), or
    /// `None` when storage isn't wired, the roster has no such slug, or the read
    /// failed. Every one of those is a reason to write an unattributed heading
    /// rather than to guess a name or to fail the write.
    /// Is `slug` a participant of `session_id`? `false` on a missing storage or
    /// a failed read: the caller uses this to REFUSE a write, and a refusal
    /// must never come from a read that did not happen.
    pub(crate) async fn is_session_participant(&self, session_id: &str, slug: &str) -> bool {
        let Some(storage) = self.storage.lock().await.clone() else {
            return false;
        };
        matches!(storage.participant_by_slug(session_id, slug).await, Ok(Some(_)))
    }

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
        cap: u32,
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
        // one, up to fifty round-trips per phase-doc rewrite. Slot rule: the
        // first free number; past the cap, the rotation below.
        let occupied = storage
            .session_document_archive_slots(session_id, slug)
            .await
            .unwrap_or_default();
        // The first free number; with every slot taken, the oldest archive is
        // dropped and the rest shift down, so the NEWEST `cap` versions survive.
        let n = match (1..=cap).find(|n| !occupied.contains(n)) {
            Some(free) => free,
            None => {
                storage
                    .rotate_session_document_archives(session_id, slug, cap)
                    .await
                    .ok()?;
                cap
            }
        };
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
        // **F10: what an AGENT writes is redacted, before anything else sees it**
        // — before an append composes `{prev}` + new (so a prefix the user
        // typed into a custom doc stays as written) and before the archive
        // step. The user's own save, `session_doc_save_custom`, is not.
        let body = crate::policy::secret_scan::redact(body);
        let body: &str = &body;
        let id = {
            let Some(storage) = self.storage.lock().await.clone() else {
                return Err(anyhow::anyhow!("storage not configured"));
            };
            // **A phase doc keeps its phase when the caller omits one** (round
            // 13, found live: an executor's `mode=append` without `phase` on
            // the `apply` doc nulled the tag through `phase = excluded.phase`,
            // and the doc vanished from the A tab and every
            // `session_doc_search(phase=…)` — the reviewer read "[]" over a
            // 7 KB changelog). An untagged write whose SLUG lands on an
            // existing phase-tagged row adopts that row's phase instead of
            // stripping it; genuinely untagged scratch (no such row) is
            // unchanged.
            //
            // **An ADOPTED phase keys on the row's OWN slug** (EYES' advisory
            // `9a1602f1`, s-d43b3630). The adoption used to feed
            // `effective_slug`, which keys on the PHASE — so an untagged write
            // to `plan-eyes` (a row tagged `plan`) landed in the `plan` doc:
            // the reviewer extending its own co-doc appended into the
            // executor's plan, and the reply still named `plan-eyes`. The row
            // the slug names is the row to write; only an EXPLICIT phase
            // re-keys. The round-13 case is unchanged — `apply` keys `apply`
            // either way.
            let adopted;
            let (phase, key) = match phase {
                Some(p) => (Some(p), effective_slug(slug, Some(p))),
                None => {
                    adopted = storage
                        .session_document_by_slug(session_id, slug)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|d| d.phase);
                    (adopted.as_deref(), slug)
                }
            };
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
            // prefix into the archive on every slice. Untagged (custom) docs
            // archive too, at a lower cap (feedback #37).
            if !append {
                let cap = if phase.is_some() { MAX_DOC_ARCHIVES } else { MAX_UNTAGGED_DOC_ARCHIVES };
                Self::archive_superseded_doc(&storage, session_id, key, body, cap).await;
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

    /// The USER's save of a CUSTOM document (round 12 — `ideas.md`: custom
    /// session documents editable and creatable by the user; the I/P/A/V docs
    /// stay the agents'). Creates or replaces the untagged doc `slug`. Refused
    /// for a slug that is, or would collide with, something that is not a
    /// custom document: empty / over 64 chars, a phase name, an archive slot
    /// (`name@3`), a reviewer co-doc (`plan-eyes`), or an existing PHASE-tagged
    /// doc under that slug — an upsert would silently turn a phase doc into a
    /// custom one. Emits `DocChanged` like every other write.
    pub async fn session_doc_save_custom(
        &self,
        session_id: &str,
        slug: &str,
        body: &str,
    ) -> Result<i64> {
        custom_slug_check(slug)?;
        // F10: stored exactly as the user typed it — the one doc write that is
        // not redacted (the user's pick `7e3308f1`).
        let id = {
            let Some(storage) = self.storage.lock().await.clone() else {
                return Err(anyhow::anyhow!("storage not configured"));
            };
            if let Some(existing) = storage.session_document_by_slug(session_id, slug).await? {
                if existing.phase.is_some() {
                    anyhow::bail!(
                        "`{slug}` is a phase document ({}); phase documents are written by the \
                         participants, not edited here",
                        existing.phase.as_deref().unwrap_or("?")
                    );
                }
            }
            storage
                .upsert_session_document(session_id, slug, body, None)
                .await?
        };
        let _ = self.event_tx.send(SignalingEvent::DocChanged {
            session_id: session_id.to_string(),
        });
        Ok(id)
    }

    /// The USER's delete of a CUSTOM document (round 12). A phase doc is
    /// refused with a reason; an unknown slug is a no-op `false`. Emits
    /// `DocChanged` when a row went.
    pub async fn session_doc_delete_custom(&self, session_id: &str, slug: &str) -> Result<bool> {
        let deleted = {
            let Some(storage) = self.storage.lock().await.clone() else {
                return Err(anyhow::anyhow!("storage not configured"));
            };
            if let Some(existing) = storage.session_document_by_slug(session_id, slug).await? {
                if existing.phase.is_some() {
                    anyhow::bail!(
                        "`{slug}` is a phase document ({}); phase documents cannot be deleted",
                        existing.phase.as_deref().unwrap_or("?")
                    );
                }
            }
            storage.delete_session_document(session_id, slug).await?
        };
        if deleted {
            let _ = self.event_tx.send(SignalingEvent::DocChanged {
                session_id: session_id.to_string(),
            });
        }
        Ok(deleted)
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
    ///
    /// **A co-doc write does not move the phase-vote fingerprint** (feedback
    /// #70 / #95 — see `Storage::phase_artifact_fingerprint`), so the other
    /// participants' votes stand. Two things replace what the invalidation
    /// used to do:
    /// - the WRITER's own vote is withdrawn, so a reviewer that records a new
    ///   objection has to vote again before the phase can move;
    /// - one system row per co-doc per run of the writer's turn tells the
    ///   others that review notes landed (a reviewer appending five slices
    ///   posts one row, not five).
    pub async fn session_doc_write_eyes(
        &self,
        session_id: &str,
        phase: &str,
        body: &str,
        author_slug: &str,
        append: bool,
    ) -> Result<CodocWrite> {
        // F10: the reviewer's text is redacted like any agent write — first,
        // before the append composes it under the existing review.
        let body = crate::policy::secret_scan::redact(body);
        let body: &str = &body;
        let slug = format!("{phase}-eyes");
        let author = self.participant_display_name(session_id, author_slug).await;
        let heading = match &author {
            Some(name) => format!("### Review findings — {name}"),
            None => "### Review findings".to_string(),
        };
        let (id, storage) = {
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
                Self::archive_superseded_doc(&storage, session_id, &slug, &composed, MAX_DOC_ARCHIVES)
                    .await;
            }
            let id = storage
                .upsert_session_document(session_id, &slug, &composed, Some(phase))
                .await?;
            (id, storage)
        };
        let _ = self.event_tx.send(SignalingEvent::DocChanged {
            session_id: session_id.to_string(),
        });
        let vote_withdrawn = self
            .after_codoc_write(&storage, session_id, author_slug, author.as_deref(), &slug)
            .await;
        Ok(CodocWrite { id, slug, vote_withdrawn })
    }

    /// [`Self::after_codoc_write`] for a caller that holds only the session and
    /// the writer's slug — the in-place edit's handler. `false` when storage
    /// is not wired.
    pub(crate) async fn after_session_codoc_write(
        &self,
        session_id: &str,
        author_slug: &str,
        slug: &str,
    ) -> bool {
        let Some(storage) = self.storage.lock().await.clone() else {
            return false;
        };
        let name = self.participant_display_name(session_id, author_slug).await;
        self.after_codoc_write(&storage, session_id, author_slug, name.as_deref(), slug)
            .await
    }

    /// What every write to a reviewer co-doc ends with — a write, an append or
    /// an in-place edit: the writer's OWN phase vote is withdrawn, and the
    /// "review notes landed" row is posted once per co-doc per run. Returns
    /// whether a vote was withdrawn.
    ///
    /// Best-effort throughout: a failed retraction leaves the vote standing,
    /// which is the pre-existing state for every other participant, and
    /// neither it nor a notice that did not post may fail the doc write.
    pub(crate) async fn after_codoc_write(
        &self,
        storage: &crate::storage::Storage,
        session_id: &str,
        author_slug: &str,
        author_name: Option<&str>,
        slug: &str,
    ) -> bool {
        let vote_withdrawn = match storage.participant_by_slug(session_id, author_slug).await {
            Ok(Some(writer)) => match storage.retract_phase_votes(writer.id).await {
                Ok(withdrawn) => withdrawn > 0,
                Err(e) => {
                    tracing::warn!(%session_id, %author_slug, ?e, "withdrawing the co-doc writer's vote failed");
                    false
                }
            },
            _ => false,
        };
        let who = author_name.unwrap_or(author_slug);
        let opening = codoc_notice_opening(who, slug);
        if !codoc_noticed_this_run(storage, session_id, author_slug, &opening).await {
            let notice = format!(
                "{opening} (read them with session_doc_read). They are notes on the work, not \
                 a change to it: the other participants' phase votes stand.]"
            );
            if crate::core::post_system_notice(
                storage,
                Some(self),
                session_id,
                crate::storage::MessageKind::SystemNotice,
                notice,
                None,
            )
            .await
            .is_none()
            {
                tracing::warn!(%session_id, %slug, "the review-notes notice was not posted");
            }
        }
        vote_withdrawn
    }

    /// Agent-callable: correct a passage of an EXISTING session doc in place
    /// (feedback #58 / #77) — `cl_edit_file`'s contract on a session doc:
    /// exactly `expect` non-overlapping occurrences of `old` become `new`, or
    /// nothing changes and the error names the count found.
    ///
    /// The two ways to change a doc used to be replace and append. On a 121 KB
    /// plan that meant re-emitting the whole body to fix one table, or
    /// appending "corrected: replaces the table above" under the wrong one —
    /// the stale-claim-above-its-correction shape the rules warn against.
    ///
    /// **It archives, like a replace** (EYES, plan point 4): an edit destroys
    /// `old`, and the archive exists to preserve a body about to be replaced
    /// (feedback #37 — a correction silently reverted with no earlier revision
    /// to diff against). Append is exempt only because it removes nothing.
    ///
    /// The doc keeps its phase tag. `new` is redacted like any agent write
    /// (F10); `old` is matched against the stored — already redacted — text.
    /// An archived version (`name@<n>`) is read-only.
    pub async fn session_doc_edit(
        &self,
        session_id: &str,
        slug: &str,
        old: &str,
        new: &str,
        expect: usize,
    ) -> Result<DocEdit> {
        if old.is_empty() {
            anyhow::bail!("old_string is empty — give the exact text to replace");
        }
        if old == new {
            anyhow::bail!("old_string and new_string are identical — nothing to change");
        }
        if expect == 0 {
            anyhow::bail!("expect_occurrences must be at least 1");
        }
        if is_archive_slug(slug) {
            anyhow::bail!("`{slug}` is an archived version — it is the record of an earlier body and is not edited");
        }
        let new = crate::policy::secret_scan::redact(new);
        let (id, doc_phase, before, after, archived_as) = {
            let Some(storage) = self.storage.lock().await.clone() else {
                return Err(anyhow::anyhow!("storage not configured"));
            };
            let Some(doc) = storage.session_document_by_slug(session_id, slug).await? else {
                anyhow::bail!(
                    "no session doc `{slug}` — session_doc_edit corrects an existing doc; create \
                     it with session_doc_write"
                );
            };
            let edited = super::util::replace_exactly(&doc.body, old, &new, expect).map_err(|m| {
                anyhow::anyhow!(
                    "found {} occurrence(s) of old_string in `{slug}`, expected {expect} — {}",
                    m.found,
                    m.hint
                )
            })?;
            let cap = if doc.phase.is_some() { MAX_DOC_ARCHIVES } else { MAX_UNTAGGED_DOC_ARCHIVES };
            let archived_as = Self::archive_superseded_doc(&storage, session_id, slug, &edited, cap).await;
            let id = storage
                .upsert_session_document(session_id, slug, &edited, doc.phase.as_deref())
                .await?;
            (id, doc.phase, doc.body.len(), edited.len(), archived_as)
        };
        let _ = self.event_tx.send(SignalingEvent::DocChanged {
            session_id: session_id.to_string(),
        });
        Ok(DocEdit {
            id,
            slug: slug.to_string(),
            phase: doc_phase,
            occurrences: expect,
            bytes_before: before,
            bytes_after: after,
            archived_as,
        })
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

    const SECTIONS: &str = "# Plan\nintro\n## A. Compaction\na1\n### A1. Detect\na1 detail\n```\n# not a heading\n```\n## B. Tools\nb1\n#nospace\n#### B deep\nb deep\n# Risks\nr1";

    /// The outline a reply carries in place of a body it leaves out: ATX
    /// headings of every level, in order, with fenced code and `#tag` lines
    /// left alone.
    #[test]
    fn doc_headings_reads_every_level_and_skips_fenced_code() {
        let got: Vec<(usize, usize, String)> =
            doc_headings(SECTIONS).into_iter().map(|h| (h.line, h.level, h.text)).collect();
        assert_eq!(
            got,
            vec![
                (1, 1, "Plan".to_string()),
                (3, 2, "A. Compaction".to_string()),
                (5, 3, "A1. Detect".to_string()),
                (10, 2, "B. Tools".to_string()),
                (13, 4, "B deep".to_string()),
                (15, 1, "Risks".to_string()),
            ]
        );
        assert_eq!(doc_outline(SECTIONS)[1], serde_json::json!({"line": 3, "level": 2, "text": "A. Compaction"}));
        assert!(doc_headings("no headings here\njust prose").is_empty());
    }

    /// A section is its heading down to the line before the next heading of
    /// the same or a higher level — sub-sections ride with it — matched by a
    /// case-insensitive substring of the heading's text, first match wins.
    #[test]
    fn doc_section_takes_its_subsections_and_stops_at_a_sibling() {
        let (h, from, to) = doc_section(SECTIONS, "compaction").unwrap();
        assert_eq!((h.text.as_str(), from, to), ("A. Compaction", 3, 9), "takes A1 and the code block");
        let (h, from, to) = doc_section(SECTIONS, "a1.").unwrap();
        assert_eq!((h.text.as_str(), from, to), ("A1. Detect", 5, 9));
        let (h, from, to) = doc_section(SECTIONS, "B. TOOLS").unwrap();
        assert_eq!((h.text.as_str(), from, to), ("B. Tools", 10, 14), "its deeper heading rides along");
        let (_, from, to) = doc_section(SECTIONS, "risks").unwrap();
        assert_eq!((from, to), (15, 16), "the last section runs to the end");
        let (h, from, to) = doc_section(SECTIONS, "plan").unwrap();
        assert_eq!((h.text.as_str(), from, to), ("Plan", 1, 14), "an H1 runs to the next H1");
        assert!(doc_section(SECTIONS, "no such heading").is_none());
        assert!(doc_section(SECTIONS, "not a heading").is_none(), "code is not a heading");
    }

    /// Feedback #77: a range that does not fit the reply is cut on a LINE and
    /// the note names where to continue; an open-ended range runs to the end;
    /// a single line longer than the budget is still returned.
    #[test]
    fn an_excerpt_over_the_budget_is_cut_on_a_line_and_says_where_to_continue() {
        let body: String = (1..=20).map(|i| format!("line {i:02} ........\n")).collect();
        let v = doc_excerpt_within(&body, None, Some("3-"), 60).unwrap();
        assert_eq!(v["lines"], "3-5", "three 16-byte lines and two newlines are 50 bytes; a fourth would be 67: {v}");
        assert_eq!(v["body"], "line 03 ........\nline 04 ........\nline 05 ........");
        assert!(
            v["note"].as_str().unwrap().contains("Continue with lines: \"6-\""),
            "the note names the next line: {v}"
        );
        // Within the budget: no note, and "a-" reaches the last line.
        let v = doc_excerpt_within(&body, None, Some("18-"), 60).unwrap();
        assert_eq!(v["lines"], "18-20");
        assert!(v.get("note").is_none(), "{v}");
        // One line over the budget is returned whole rather than nothing.
        let v = doc_excerpt_within("a very long single line that is over budget\nnext", None, Some("1-2"), 10).unwrap();
        assert_eq!(v["lines"], "1-1");
        assert_eq!(v["body"], "a very long single line that is over budget");
        // grep: matches past the budget are counted, not silently dropped.
        let v = doc_excerpt_within(&body, Some("line"), None, 40).unwrap();
        assert_eq!(v["matches"].as_array().unwrap().len(), 2, "{v}");
        assert!(v["note"].as_str().unwrap().starts_with("18 more matching line(s)"), "{v}");
    }

    /// The bridge half of EYES' advisory `9a1602f1`: an untagged write whose
    /// slug names a phase-TAGGED row adopts that row's phase and writes THAT
    /// row. It used to re-key on the adopted phase, so `plan-eyes` (tagged
    /// `plan`) resolved to the `plan` doc. Feeding the adopted phase back into
    /// `effective_slug` turns this red.
    #[tokio::test]
    async fn an_adopted_phase_keys_on_the_rows_own_slug() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.upsert_session_document("s1", "plan", "THE PLAN", Some("plan")).await.unwrap();
        storage.upsert_session_document("s1", "plan-eyes", "the review", Some("plan")).await.unwrap();

        bridge.session_doc_write("s1", "plan-eyes", "one more point", None, true).await.unwrap();

        let plan = storage.session_document_by_slug("s1", "plan").await.unwrap().unwrap();
        assert_eq!(plan.body, "THE PLAN", "the executor's doc is not the row that slug names");
        let review = storage.session_document_by_slug("s1", "plan-eyes").await.unwrap().unwrap();
        assert!(review.body.starts_with("the review") && review.body.contains("one more point"), "{}", review.body);
        assert_eq!(review.phase.as_deref(), Some("plan"), "and it keeps its phase tag");
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
        let eyes_slug = bridge
            .session_doc_write_eyes("s1", "plan", "the reviewer's notes", "eyes", false)
            .await
            .unwrap()
            .slug;
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
    async fn an_untagged_write_to_a_phase_docs_slug_adopts_its_phase() {
        // Round 13, observed live in s-9bbff909: three `apply`-doc appends
        // without `phase` nulled the tag (`phase = excluded.phase`), so
        // `session_doc_search(phase="apply")` returned [] over a written
        // changelog. The write must adopt the existing row's phase, for
        // append AND replace alike — and stay findable by phase.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        bridge
            .session_doc_write("s1", "apply", "changelog v1", Some("apply"), false)
            .await
            .unwrap();
        // The live failure: an append that omits phase.
        bridge
            .session_doc_write("s1", "apply", "batch 2 landed", None, true)
            .await
            .unwrap();
        let doc = storage
            .session_document_by_slug("s1", "apply")
            .await
            .unwrap()
            .expect("the apply doc exists");
        assert_eq!(doc.phase.as_deref(), Some("apply"), "append kept the tag");
        assert!(doc.body.contains("changelog v1") && doc.body.contains("batch 2 landed"));

        // Replace without phase adopts too (and still archives the old body).
        bridge
            .session_doc_write("s1", "apply", "changelog v2", None, false)
            .await
            .unwrap();
        let doc = storage
            .session_document_by_slug("s1", "apply")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.phase.as_deref(), Some("apply"), "replace kept the tag");

        // The retrieval the reviewer actually ran.
        let found = bridge
            .session_doc_search("s1", None, Some("apply"))
            .await
            .unwrap();
        assert!(
            found.iter().any(|d| d.slug == "apply"),
            "phase-filtered search finds the doc again: {found:?}"
        );

        // A genuinely untagged scratch doc is untouched by the adoption rule.
        bridge
            .session_doc_write("s1", "scratch", "notes", None, false)
            .await
            .unwrap();
        let doc = storage
            .session_document_by_slug("s1", "scratch")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.phase, None, "scratch stays untagged");
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

        // Untagged (custom) docs archive on replace too (feedback #37): a
        // correction reverted by a later rewrite must stay detectable.
        bridge.session_doc_write("s1", "scratch", "v1", None, false).await.unwrap();
        bridge.session_doc_write("s1", "scratch", "v2", None, false).await.unwrap();
        assert_eq!(
            bridge.session_doc_read("s1", "scratch@1").await.unwrap().map(|d| d.body).as_deref(),
            Some("v1")
        );
        // …at the lower cap, keeping the NEWEST ten: v1..v18 were superseded,
        // so @1..@10 hold v9..v18 (the recent middle survives).
        for i in 3..20 {
            bridge.session_doc_write("s1", "scratch", &format!("v{i}"), None, false).await.unwrap();
        }
        let body = |n: u32| {
            let bridge = &bridge;
            async move {
                bridge.session_doc_read("s1", &format!("scratch@{n}")).await.unwrap().map(|d| d.body)
            }
        };
        assert_eq!(body(1).await.as_deref(), Some("v9"));
        assert_eq!(body(10).await.as_deref(), Some("v18"));
        assert!(body(11).await.is_none());
    }

    #[test]
    fn doc_excerpt_greps_and_slices_without_the_whole_body() {
        let body = "one\nTwo alpha\nthree\nfour ALPHA\nfive";
        let g = doc_excerpt(body, Some("alpha"), None).unwrap();
        assert_eq!(g["total_lines"], 5);
        assert_eq!(g["matches"][0]["line"], 2);
        assert_eq!(g["matches"][1]["line"], 4);
        let l = doc_excerpt(body, None, Some("2-3")).unwrap();
        assert_eq!(l["body"], "Two alpha\nthree");
        let both = doc_excerpt(body, Some("alpha"), Some("3-5")).unwrap();
        assert_eq!(both["matches"].as_array().unwrap().len(), 1, "grep runs inside the window");
        assert!(doc_excerpt(body, None, Some("4-2")).is_err());
        assert!(doc_excerpt(body, None, Some("x")).is_err());
        assert!(is_archive_slug("plan@3") && !is_archive_slug("notes@home") && !is_archive_slug("plan"));
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

    /// Round 12: the user's save/delete of CUSTOM documents. The reserved
    /// names are refused, a phase doc under the slug is neither overwritten
    /// nor deleted, and every real write/delete tells the UI.
    #[tokio::test]
    async fn the_user_saves_and_deletes_custom_documents_only() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.upsert_session_document("s1", "plan", "the plan", Some("plan")).await.unwrap();
        let mut sub = bridge.subscribe();

        // Reserved names.
        for bad in ["", "  ", "plan", "Apply", "notes@3", "plan-eyes", &"x".repeat(65)] {
            assert!(
                bridge.session_doc_save_custom("s1", bad, "body").await.is_err(),
                "`{bad}` must be refused"
            );
        }
        // A phase doc under the slug is not silently converted.
        assert!(bridge.session_doc_save_custom("s1", "plan", "hijack").await.is_err());
        assert_eq!(
            storage.session_document_by_slug("s1", "plan").await.unwrap().unwrap().body,
            "the plan"
        );
        assert!(!matches!(sub.try_recv(), Ok(SignalingEvent::DocChanged { .. })), "refusals emit nothing");

        // Create, then edit, then delete — each a DocChanged.
        bridge.session_doc_save_custom("s1", "checklist", "- [ ] a").await.unwrap();
        assert!(matches!(sub.try_recv(), Ok(SignalingEvent::DocChanged { .. })));
        bridge.session_doc_save_custom("s1", "checklist", "- [x] a").await.unwrap();
        assert!(matches!(sub.try_recv(), Ok(SignalingEvent::DocChanged { .. })));
        let doc = storage.session_document_by_slug("s1", "checklist").await.unwrap().unwrap();
        assert_eq!(doc.body, "- [x] a");
        assert!(doc.phase.is_none(), "a custom doc stays untagged");
        assert!(bridge.session_doc_delete_custom("s1", "checklist").await.unwrap());
        assert!(matches!(sub.try_recv(), Ok(SignalingEvent::DocChanged { .. })));
        assert!(storage.session_document_by_slug("s1", "checklist").await.unwrap().is_none());
        // Deleting again: no-op, no event. Deleting a phase doc: refused.
        assert!(!bridge.session_doc_delete_custom("s1", "checklist").await.unwrap());
        assert!(!matches!(sub.try_recv(), Ok(SignalingEvent::DocChanged { .. })));
        assert!(bridge.session_doc_delete_custom("s1", "plan").await.is_err());
        assert!(storage.session_document_by_slug("s1", "plan").await.unwrap().is_some());
    }

    /// F10 (plan C4b): what an AGENT writes to a session doc is redacted —
    /// a phase doc, an append, the reviewer's co-doc — while a custom doc the
    /// USER saves is stored exactly as typed, and an agent's append to it
    /// leaves the user's text as written.
    #[tokio::test]
    async fn an_agents_doc_is_redacted_and_the_users_own_text_is_not() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        let token = format!("{}{}", "ghp_", "1234567890abcdefghijABCDEF");
        let marker = "[redacted: a GitHub access token]";
        let body = |s: &str| {
            let storage = storage.clone();
            let slug = s.to_string();
            async move {
                storage.session_document_by_slug("s1", &slug).await.unwrap().unwrap().body
            }
        };

        bridge
            .session_doc_write("s1", "plan", &format!("run with {token}"), Some("plan"), false)
            .await
            .unwrap();
        assert_eq!(body("plan").await, format!("run with {marker}"));

        bridge
            .session_doc_write_eyes("s1", "plan", &format!("the plan prints {token}"), "eyes", false)
            .await
            .unwrap();
        let review = body("plan-eyes").await;
        assert!(review.ends_with(&format!("the plan prints {marker}")), "{review}");
        assert!(!review.contains(&token));

        let typed = format!("my deploy token: {token}");
        bridge.session_doc_save_custom("s1", "notes", &typed).await.unwrap();
        assert_eq!(body("notes").await, typed, "the user's save is verbatim");

        bridge
            .session_doc_write("s1", "notes", &format!("agent adds {token}"), None, true)
            .await
            .unwrap();
        let notes = body("notes").await;
        assert!(notes.starts_with(&typed), "the user's text stays as written: {notes}");
        assert!(notes.ends_with(&format!("agent adds {marker}")), "{notes}");
        assert_eq!(notes.matches(&token).count(), 1, "{notes}");
    }
}
