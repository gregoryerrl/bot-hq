//! The reviewer's turn digest (feedback #76, carrying #61's resolved gates).
//!
//! A reviewer reads only its peers' prose: their tool calls never reach it, so
//! a Context Library write or a session-doc write leaves nothing in the channel
//! to review (s-90ac85d7: the reviewer found a wrong claim carried into a CL
//! file only by listing the library and grepping it), and a gate the user
//! answered while the reviewer worked is a status it can contradict (s-25d449df:
//! "waiting on your approval of X" three times after X had run). The user's
//! pick (2026-10-12, `decisions-needed` item 11): a digest of the peers' CL and
//! session-doc writes and the gates resolved since the reviewer's last turn,
//! delivered ONLY to the reviewer, at the start of its dealt turn, as the
//! preface of the page the ring writes to its stdin — never as chat rows.
//!
//! The window is `[cursor, now]` — inclusive at the start, so an item stamped
//! in the same millisecond as the last digest is repeated rather than lost; the
//! cursor is the `now` of the last digest
//! this participant was handed (in memory; committed only once the page
//! landed, so a drain a user message cut short loses nothing), and when there
//! is none — the first turn after a spawn or a relaunch — the time of the row
//! its channel cursor rests on, the last row it read. A gate reported while its
//! command was still running is reported again, finished, on the next digest.

use std::collections::VecDeque;

use super::SignalingBridge;

/// The most CL writes remembered per session; the oldest fall off.
pub(crate) const CL_WRITE_LOG_CAP: usize = 200;
/// The most items one digest lists — the NEWEST; earlier ones are counted.
pub(crate) const DIGEST_MAX_ITEMS: usize = 30;
/// The most bytes one digest puts in front of the page.
pub(crate) const DIGEST_BUDGET_BYTES: usize = 6_000;
/// How much of a gate's command and of its output's first line the digest shows.
const DIGEST_LINE_BYTES: usize = 120;

/// One agent write to the Context Library, as `write_cl` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClWriteRecord {
    pub at: String,
    pub agent: String,
    pub project: String,
    pub file_path: String,
    pub kind: &'static str,
    pub bytes: usize,
    /// Which lines the write changed (`cl_write::changed_lines`), when known.
    pub changed: Option<String>,
}

/// Where a participant's last digest ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DigestCursor {
    /// The `now` of the last digest handed over (RFC3339-Z).
    pub at: String,
    /// Gates that digest reported as still running: re-read next time.
    pub running: Vec<String>,
}

/// A digest ready to go in front of a page, with the cursor its landing commits.
#[derive(Debug, Clone)]
pub struct TurnDigest {
    pub text: String,
    cursor: DigestCursor,
}

/// One digest item, in time order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DigestItem {
    ClWrite(ClWriteRecord),
    Doc {
        at: String,
        writer: String,
        slug: String,
        phase: Option<String>,
        bytes: i64,
    },
    Gate {
        at: String,
        id8: String,
        asked_by: String,
        state: String,
        command: String,
        output: Option<String>,
        /// Reported before, while running; this is its finish.
        finished: bool,
    },
}

impl DigestItem {
    fn at(&self) -> &str {
        match self {
            DigestItem::ClWrite(r) => &r.at,
            DigestItem::Doc { at, .. } | DigestItem::Gate { at, .. } => at,
        }
    }

    fn render(&self) -> String {
        match self {
            DigestItem::ClWrite(r) => {
                let changed = r.changed.as_deref().map(|c| format!("; {c}")).unwrap_or_default();
                format!(
                    "- {} · {} wrote the CL: {}/{} ({}, {} bytes{changed})",
                    short_time(&r.at),
                    r.agent,
                    r.project,
                    r.file_path,
                    r.kind,
                    r.bytes
                )
            }
            DigestItem::Doc { at, writer, slug, phase, bytes } => {
                let who = if writer == "user" { "the user".to_string() } else { writer.clone() };
                let tag = phase.as_deref().map(|p| format!(" ({p} phase)")).unwrap_or_default();
                format!("- {} · {who} changed session doc `{slug}`{tag} ({bytes} bytes)", short_time(at))
            }
            DigestItem::Gate { at, id8, asked_by, state, command, output, finished } => {
                let verb = if *finished { "finished" } else { "resolved" };
                let out = output.as_deref().map(|o| format!(" — output: {o}")).unwrap_or_default();
                format!("- {} · gate {id8} ({asked_by}) {verb}: {state} · `{command}`{out}", short_time(at))
            }
        }
    }
}

/// `2026-10-13T01:02:03.456Z` → `01:02:03Z` (the date is the page's; the
/// items are from the last turn or so); anything else unchanged.
fn short_time(at: &str) -> String {
    let b = at.as_bytes();
    let shaped = b.len() >= 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && at.ends_with('Z')
        && b[11..19].iter().enumerate().all(|(i, c)| matches!(i, 2 | 5) || c.is_ascii_digit());
    if shaped {
        format!("{}Z", &at[11..19])
    } else {
        at.to_string()
    }
}

/// The first line of a gated command, cut to [`DIGEST_LINE_BYTES`].
fn one_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.len() <= DIGEST_LINE_BYTES {
        return line.to_string();
    }
    let cut = crate::text::floor_char_boundary(line, DIGEST_LINE_BYTES);
    format!("{}…", &line[..cut])
}

/// The first non-empty line of a gate's output — after the row's `Output:`
/// marker, skipping the exit-code footer — or `None` when there is none.
pub(crate) fn output_first_line(content: &str) -> Option<String> {
    let body = super::action_gate::gate_output_body(content);
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("[action_gate →"))?;
    Some(one_line(line))
}

/// Render the digest: the newest [`DIGEST_MAX_ITEMS`] items within
/// [`DIGEST_BUDGET_BYTES`], oldest first, earlier ones counted. `None` for no
/// items.
pub(crate) fn render_digest(now: &str, items: &[DigestItem]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let header = format!(
        "[bot-hq · {}] Since your last turn — your peers' Context Library and session-doc writes, and the \
         gates resolved (only you receive this; it is not a chat row):",
        &now[..now.len().min(16)]
    );
    let footer_room = 120;
    let mut lines: VecDeque<String> = VecDeque::new();
    let mut used = header.len();
    let mut skipped = 0usize;
    // Newest first into the front, so the budget keeps the latest.
    for item in items.iter().rev() {
        let line = item.render();
        if lines.len() >= DIGEST_MAX_ITEMS || used + line.len() + 1 + footer_room > DIGEST_BUDGET_BYTES {
            skipped += 1;
            continue;
        }
        used += line.len() + 1;
        lines.push_front(line);
    }
    let mut out = header;
    for line in lines {
        out.push('\n');
        out.push_str(&line);
    }
    if skipped > 0 {
        out.push_str(&format!(
            "\n… {skipped} earlier item{} not shown — `gate_history`, `session_doc_search` and the library's \
             git log have them.",
            if skipped == 1 { "" } else { "s" }
        ));
    }
    Some(out)
}

impl SignalingBridge {
    /// Remember an agent's CL write for the digests of the session's reviewers.
    pub(crate) fn log_cl_write(&self, session_id: &str, record: ClWriteRecord) {
        let mut log = self.cl_write_log.lock().unwrap_or_else(|p| p.into_inner());
        let entries = log.entry(session_id.to_string()).or_default();
        if entries.len() >= CL_WRITE_LOG_CAP {
            entries.pop_front();
        }
        entries.push_back(record);
    }

    /// The CL writes of `session_id` at or after `since`, in time order.
    fn cl_writes_since(&self, session_id: &str, since: &str) -> Vec<ClWriteRecord> {
        self.cl_write_log
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .map(|log| log.iter().filter(|r| r.at.as_str() >= since).cloned().collect())
            .unwrap_or_default()
    }

    /// The digest for `slug`'s dealt turn, or `None`: not a reviewer, no
    /// storage, or nothing happened since its last turn. The caller puts
    /// `text` in front of the page and, once the page LANDED, commits the
    /// cursor with [`Self::commit_turn_digest`].
    pub async fn turn_digest(&self, session_id: &str, slug: &str, participant_id: i64) -> Option<TurnDigest> {
        if !self.is_reviewer(session_id, slug) {
            return None;
        }
        let storage = self.storage.lock().await.clone()?;
        let now = crate::storage::now_utc();
        let key = (session_id.to_string(), slug.to_string());
        let previous = self
            .digest_cursor
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .cloned();
        let since = match &previous {
            Some(c) => c.at.clone(),
            None => {
                // The last row this participant read, or everything when it
                // has read nothing (a participant new to the session).
                let cursor = storage.cursor_for(participant_id).await.unwrap_or(0);
                if cursor > 0 {
                    storage.message_created_at(cursor).await.ok().flatten().unwrap_or_default()
                } else {
                    String::new()
                }
            }
        };
        let mut items: Vec<DigestItem> = Vec::new();
        for r in self.cl_writes_since(session_id, &since) {
            if r.agent != slug {
                items.push(DigestItem::ClWrite(r));
            }
        }
        if let Ok(docs) = storage.session_documents_changed_since(session_id, &since).await {
            for d in docs {
                let writer = d.last_writer.clone().unwrap_or_else(|| "someone".to_string());
                if writer == slug {
                    continue;
                }
                items.push(DigestItem::Doc {
                    at: d.updated_at,
                    writer,
                    slug: d.slug,
                    phase: d.phase,
                    bytes: d.bytes,
                });
            }
        }
        let mut running: Vec<String> = Vec::new();
        // Gates reported running last time: their finish, if it came.
        for id in previous.as_ref().map(|c| c.running.clone()).unwrap_or_default() {
            let Ok(Some(row)) = storage.get_tray_entry(&id).await else { continue };
            let state = self.gate_ledger_state(&storage, &row).await;
            if state.contains("RUNNING") {
                running.push(id);
                continue;
            }
            let output = self.gate_output_line(&storage, &row).await;
            items.push(DigestItem::Gate {
                at: now.clone(),
                id8: row.choice_id.chars().take(8).collect(),
                asked_by: row.agent.clone(),
                state,
                command: one_line(&crate::policy::secret_scan::redact_string(
                    row.command_text.clone().unwrap_or_default(),
                )),
                output,
                finished: true,
            });
        }
        if let Ok(gates) = storage.gates_answered_since(session_id, &since).await {
            for row in gates {
                let state = self.gate_ledger_state(&storage, &row).await;
                if state.contains("RUNNING") && !running.contains(&row.choice_id) {
                    running.push(row.choice_id.clone());
                }
                let output = self.gate_output_line(&storage, &row).await;
                items.push(DigestItem::Gate {
                    at: row.answered_at.clone().unwrap_or_else(|| now.clone()),
                    id8: row.choice_id.chars().take(8).collect(),
                    asked_by: row.agent.clone(),
                    state,
                    command: one_line(&crate::policy::secret_scan::redact_string(
                        row.command_text.clone().unwrap_or_default(),
                    )),
                    output,
                    finished: false,
                });
            }
        }
        items.sort_by(|a, b| a.at().cmp(b.at()));
        let cursor = DigestCursor { at: now.clone(), running };
        match render_digest(&now, &items) {
            Some(text) => Some(TurnDigest { text, cursor }),
            None => {
                // Nothing to say — still move the window, so a later digest does
                // not re-read the same empty span, and keep the running list.
                self.digest_cursor
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(key, cursor);
                None
            }
        }
    }

    /// The page carrying `digest` landed: its window is closed.
    pub fn commit_turn_digest(&self, session_id: &str, slug: &str, digest: TurnDigest) {
        self.digest_cursor
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((session_id.to_string(), slug.to_string()), digest.cursor);
    }

    async fn gate_output_line(
        &self,
        storage: &crate::storage::Storage,
        row: &crate::storage::SessionTrayEntry,
    ) -> Option<String> {
        let row_id = row.result_row_id?;
        let content = storage.message_content(row_id).await.ok().flatten()?;
        output_first_line(&content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cl(at: &str, agent: &str) -> DigestItem {
        DigestItem::ClWrite(ClWriteRecord {
            at: at.into(),
            agent: agent.into(),
            project: "bot-hq".into(),
            file_path: "focus.md".into(),
            kind: "append",
            bytes: 4_210,
            changed: Some("lines 88–96".into()),
        })
    }

    #[test]
    fn a_digest_lists_items_oldest_first_with_short_times() {
        let items = vec![
            cl("2026-10-13T01:02:03.456Z", "hands"),
            DigestItem::Doc {
                at: "2026-10-13T01:03:00.000Z".into(),
                writer: "user".into(),
                slug: "decisions-needed".into(),
                phase: None,
                bytes: 812,
            },
            DigestItem::Gate {
                at: "2026-10-13T01:04:00.000Z".into(),
                id8: "2f3a9c1d".into(),
                asked_by: "hands".into(),
                state: "approved, exit 0 after 3.1 s, 812 bytes of output".into(),
                command: "cargo test --lib x".into(),
                output: Some("test result: ok. 3 passed".into()),
                finished: false,
            },
        ];
        let text = render_digest("2026-10-13T01:05:00.000Z", &items).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("[bot-hq · 2026-10-13T01:05] Since your last turn"), "{}", lines[0]);
        assert!(lines[0].contains("only you receive this; it is not a chat row"));
        assert_eq!(lines[1], "- 01:02:03Z · hands wrote the CL: bot-hq/focus.md (append, 4210 bytes; lines 88–96)");
        assert_eq!(lines[2], "- 01:03:00Z · the user changed session doc `decisions-needed` (812 bytes)");
        assert_eq!(
            lines[3],
            "- 01:04:00Z · gate 2f3a9c1d (hands) resolved: approved, exit 0 after 3.1 s, 812 bytes of output · \
             `cargo test --lib x` — output: test result: ok. 3 passed"
        );
        assert_eq!(lines.len(), 4);
        assert!(render_digest("2026-10-13T01:05:00.000Z", &[]).is_none());
    }

    #[test]
    fn a_long_digest_keeps_the_newest_items_and_counts_the_rest() {
        let items: Vec<DigestItem> = (0..50)
            .map(|i| cl(&format!("2026-10-13T01:{:02}:00.000Z", i), "hands"))
            .collect();
        let text = render_digest("2026-10-13T02:00:00.000Z", &items).unwrap();
        assert!(text.len() <= DIGEST_BUDGET_BYTES, "{}", text.len());
        let body: Vec<&str> = text.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(body.len(), DIGEST_MAX_ITEMS);
        assert!(body[0].starts_with("- 01:20:00Z"), "the oldest shown is item 20: {}", body[0]);
        assert!(body.last().unwrap().starts_with("- 01:49:00Z"));
        assert!(text.ends_with("… 20 earlier items not shown — `gate_history`, `session_doc_search` and the library's git log have them."), "{text}");
    }

    #[test]
    fn the_output_line_skips_the_verdict_and_the_footer() {
        let content = "Tray answer 2f3a9c1d — approved\nOutput:\n\ntest result: ok. 3 passed\nmore\n[action_gate → exit 0 · 40 output bytes · shell zsh]";
        assert_eq!(output_first_line(content).as_deref(), Some("test result: ok. 3 passed"));
        let footer_only = "verdict\nOutput:\n[action_gate → exit 0 · 0 output bytes · shell zsh]";
        assert_eq!(output_first_line(footer_only), None);
        let long = format!("Output:\n{}", "x".repeat(300));
        assert_eq!(output_first_line(&long).unwrap().chars().count(), DIGEST_LINE_BYTES + 1);
    }

    #[test]
    fn the_cl_write_log_is_capped_per_session() {
        let b = SignalingBridge::new();
        for i in 0..(CL_WRITE_LOG_CAP + 5) {
            b.log_cl_write(
                "s1",
                ClWriteRecord {
                    at: format!("2026-10-13T00:00:{:02}.{:03}Z", i / 1000, i % 1000),
                    agent: "hands".into(),
                    project: "p".into(),
                    file_path: format!("f{i}.md"),
                    kind: "create",
                    bytes: 1,
                    changed: None,
                },
            );
        }
        let all = b.cl_writes_since("s1", "");
        assert_eq!(all.len(), CL_WRITE_LOG_CAP);
        assert_eq!(all[0].file_path, "f5.md", "the oldest five fell off");
        assert!(b.cl_writes_since("s2", "").is_empty());
        assert_eq!(b.cl_writes_since("s1", "2026-10-13T00:00:00.203Z").len(), 2, "inclusive");
    }
}
