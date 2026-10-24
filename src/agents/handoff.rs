//! The pinned handoff doc a participant gets back after a context compaction
//! (feedback #54 / #63 / #67 / #74 / #81).
//!
//! claude-code compacts a participant's context when it fills: everything
//! before becomes a summary, and what the user said an hour ago survives only
//! if the summary happened to keep it. In five sessions the user ran the
//! hand-over by hand ("write a handoff in a session doc, then make HANDS read
//! it after autocompact") and a peer had to notice the compaction to brief the
//! compacted participant. This module is the mechanical version:
//!
//! - the doc is an ordinary CUSTOM session document, `handoff-<participant
//!   slug>` — no new tool, and the user can read and edit it in its own tab;
//! - bot-hq mirrors it to a FILE on every write (the session-doc store is the
//!   one choke point — see `Storage::sync_handoff_file`), already rendered to
//!   what the participant should read;
//! - a `SessionStart` hook with matcher `compact`, injected at spawn, prints
//!   that file, and claude-code puts a hook's stdout into the context.
//!
//! A file rather than a database read in the hook: the hook then cannot fail
//! on a locked database or a schema it does not know, and what it prints is
//! exactly what the app last rendered.
//!
//! # The size budget is in BYTES
//!
//! claude-code replaces hook output over about 10,000 characters with a 2 KB
//! preview and a file path (probed on CLI 2.1.284, s-d43b3630: 9.4 KB arrived
//! whole, 11.5 KB did not). The probe's filler was ASCII, so whether the cap
//! counts characters, UTF-16 units or bytes is unmeasured. A byte budget is
//! safe under all three: a string's byte length is never smaller than its
//! length in either other unit.

use std::path::{Path, PathBuf};

/// Slug prefix of a handoff doc: `handoff-hands` is `hands`'s.
pub const SLUG_PREFIX: &str = "handoff-";

/// First words of every rendered handoff file. The stream translator
/// recognises bot-hq's own `SessionStart:compact` hook response by them, which
/// is how a compaction row can say the doc WAS put back rather than assume it.
pub const INJECT_MARKER: &str = "[bot-hq] Your context was just compacted.";

/// Follows [`INJECT_MARKER`] when the participant has no handoff doc.
pub const NO_DOC_MARKER: &str = "You have no pinned handoff doc";

/// Everything the hook prints stays under this many BYTES (see the module doc).
pub const FILE_BUDGET_BYTES: usize = 9_000;

/// The handoff doc slug of `participant`.
pub fn doc_slug(participant: &str) -> String {
    format!("{SLUG_PREFIX}{participant}")
}

/// The participant a handoff slug belongs to — `None` for any other slug, for
/// the bare prefix, and for an archived version (`handoff-hands@3`).
pub fn participant_of(slug: &str) -> Option<&str> {
    let rest = slug.strip_prefix(SLUG_PREFIX)?;
    (!rest.is_empty() && !rest.contains('@')).then_some(rest)
}

/// `<local_dir>/handoffs`, where the rendered files live.
pub fn dir_under(local_dir: &Path) -> PathBuf {
    local_dir.join("handoffs")
}

/// Is `s` usable as one path segment? Session ids (`s-d43b3630`) and
/// participant slugs (`hands`, `hands-2`) are; anything that could leave the
/// directory is not, and such a pair simply gets no file.
fn safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The rendered file for one participant of one session, or `None` when either
/// name is not a safe path segment.
pub fn file_path(handoff_dir: &Path, session_id: &str, participant: &str) -> Option<PathBuf> {
    (safe_segment(session_id) && safe_segment(participant))
        .then(|| handoff_dir.join(session_id).join(format!("{participant}.md")))
}

/// What the hook prints after a compaction. `doc` is the handoff doc's body
/// and its `updated_at`, when one exists.
///
/// With a doc: a header, the body up to the budget (cut on a char boundary),
/// and — only when it was cut — where to read the rest. Without one: what is
/// still written down, and how to start a doc. Both open with
/// [`INJECT_MARKER`], and the whole text is at most [`FILE_BUDGET_BYTES`].
pub fn render(session_id: &str, participant: &str, doc: Option<(&str, &str)>) -> String {
    let slug = doc_slug(participant);
    let Some((body, updated_at)) = doc.filter(|(b, _)| !b.trim().is_empty()) else {
        return format!(
            "{INJECT_MARKER} {NO_DOC_MARKER} (`{slug}`) in session {session_id}.\n\
             What survives a compaction is what is written down. Before you act, re-read the \
             session's phase documents (session_doc_search), the project's focus.md and the \
             repository's git state; do not rely on the summary for instructions the user gave.\n\
             Then write `{slug}` (session_doc_write, no phase) so the next compaction has one.\n"
        );
    };
    let header = format!(
        "{INJECT_MARKER} Below is your pinned handoff doc `{slug}` (session \
         {session_id}{WRITTEN_PREFIX}{updated_at}), put back by bot-hq.\n\
         It speaks as of that time, written by you or by the user. The summary above was written \
         at the compaction and was asked to end with what changed since then; what the user said \
         since then, if anything, follows this doc verbatim. The user's verbatim words win over the \
         summary's account of them; for anything else after that time the summary wins; for \
         anything before it, this doc's own words win, above all the user's instructions it \
         quotes. Then check the live state: the phase documents (session_doc_search), the \
         project's focus.md and git.\n---\n"
    );
    // Room for the longest footer: two 20-digit byte counts and a line number.
    const FOOTER_RESERVE: usize = 240;
    let room = FILE_BUDGET_BYTES.saturating_sub(header.len() + FOOTER_RESERVE + AGE_STAMP_RESERVE);
    if body.len() <= room {
        return format!("{header}{body}\n---\n[bot-hq] End of `{slug}`.\n");
    }
    let cut = crate::text::floor_char_boundary(body, room);
    let shown = &body[..cut];
    // The line the cut fell in is re-read whole, so the reader never has to
    // stitch half a line.
    let resume_line = shown.matches('\n').count() + 1;
    format!(
        "{header}{shown}\n---\n[bot-hq] `{slug}` is longer than what fits here ({cut} of {total} \
         bytes shown). Read the rest: session_doc_read(slug: \"{slug}\", lines: \
         \"{resume_line}-\").\n",
        total = body.len(),
    )
}

/// When the doc in a rendered handoff file was last written — the `updated_at`
/// [`render`] put in its header — or `None` for the no-doc text and for any
/// other text. Read from the header only (up to the first `---` line), so a
/// doc body that quotes "last written" cannot answer for it.
pub fn written_at(rendered: &str) -> Option<&str> {
    if !rendered.trim_start().starts_with(INJECT_MARKER) {
        return None;
    }
    let header = rendered.split("\n---\n").next()?;
    let from = header.find(WRITTEN_PREFIX)? + WRITTEN_PREFIX.len();
    let rest = &header[from..];
    let end = rest.find(|c: char| c == ')' || c == ',' || c.is_whitespace()).unwrap_or(rest.len());
    let at = &rest[..end];
    (!at.is_empty()).then_some(at)
}

/// What the header says right before the doc's `updated_at`.
const WRITTEN_PREFIX: &str = ", last written ";

/// Bytes [`render`] leaves free for the age [`stamp_age`] adds when the hook
/// prints the file — the longest stamp (a 20-digit day count, hours, the
/// wording) is 59 bytes — so the PRINTED text, not just the rendered file,
/// stays under [`FILE_BUDGET_BYTES`].
const AGE_STAMP_RESERVE: usize = 64;

/// `rendered` with the doc's age at `now` stamped into its header, right after
/// the time [`render`] wrote there: "(session s-1, last written T — 38 minutes
/// before this compaction)". The `compact-handoff` hook prints it right after
/// the compaction, so the age IS the gap between the doc's last write and the
/// compaction (EYES D4): the participant learns how much of the session the doc
/// cannot know about. Unchanged when there is no doc, the time does not parse,
/// or it lies in the future (a skewed clock).
pub fn stamp_age(rendered: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Some(at) = written_at(rendered) else {
        return rendered.to_string();
    };
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(at) else {
        return rendered.to_string();
    };
    let secs = (now - then.with_timezone(&chrono::Utc)).num_seconds();
    let needle = format!("{WRITTEN_PREFIX}{at}");
    let (Ok(secs), Some(pos)) = (u64::try_from(secs), rendered.find(&needle)) else {
        return rendered.to_string();
    };
    let end = pos + needle.len();
    format!("{} — {} before this compaction{}", &rendered[..end], human_age(secs), &rendered[end..])
}

/// `secs` as the age a reader takes in at a glance: minutes under two hours,
/// hours and minutes under two days, days and hours beyond.
fn human_age(secs: u64) -> String {
    fn n(count: u64, unit: &str) -> String {
        if count == 1 { format!("1 {unit}") } else { format!("{count} {unit}s") }
    }
    let minutes = secs / 60;
    match minutes {
        0 => "under a minute".to_string(),
        m if m < 120 => n(m, "minute"),
        m if m < 48 * 60 => match m % 60 {
            0 => n(m / 60, "hour"),
            r => format!("{} {}", n(m / 60, "hour"), n(r, "minute")),
        },
        m => match (m / 60) % 24 {
            0 => n(m / (24 * 60), "day"),
            h => format!("{} {}", n(m / (24 * 60), "day"), n(h, "hour")),
        },
    }
}

/// What the `PreCompact` hook prints: instructions claude-code appends to the
/// prompt that writes the compaction summary (probed on CLI 2.1.291,
/// s-079dec53 — a hook's stdout on exit 0 becomes the summary's custom
/// instructions; the text never enters the compacted context itself).
///
/// The summary is the one artifact written AT the compaction, so this is how
/// what happened after the handoff doc's last write survives it: the summary
/// is asked, ADDITIVELY, for a closing section about everything since then
/// (EYES D3 — not "carry on from the doc", which would let the summary drop
/// what only it holds: the put-back is cut at [`FILE_BUDGET_BYTES`], and a doc
/// the user edited is not in the transcript at all). `written_at` is the doc's
/// last write, `None` when the participant has none.
///
/// Plain text that never starts with `{`: claude-code parses hook output that
/// does as JSON, and `{"decision":"block"}` would BLOCK the compaction.
pub fn compaction_instructions(session_id: &str, participant: &str, written_at: Option<&str>) -> String {
    let slug = doc_slug(participant);
    let lists = "every instruction or correction the user gave, verbatim, with its time; each decision \
                 and who took it; what was done and verified, with its evidence (commit, test run, \
                 document, gate id); what is in flight and the exact next step; question and gate ids \
                 still open; anything the participant was told not to do";
    match written_at {
        Some(at) => format!(
            "[bot-hq] This conversation is participant `{participant}` in bot-hq session {session_id}. \
             After this compaction bot-hq puts its handoff doc `{slug}` (last written {at}) back into \
             its context beside your summary, and the user's own messages from after that time, \
             verbatim. Write the summary as you normally would. Then end it with a section headed \
             \"Since `{slug}` was last written ({at})\" that lists, from after that time: {lists}; and \
             each statement in `{slug}` you know to be no longer true, with what replaced it. Quote \
             the user exactly: where your quotation and the verbatim text differ, the verbatim text \
             wins."
        ),
        None => format!(
            "[bot-hq] This conversation is participant `{participant}` in bot-hq session {session_id}. \
             It has no handoff doc, so your summary and the user's own messages of this session, \
             which bot-hq puts back verbatim beside it, are all it carries across this compaction. \
             Write the summary as you normally would. Then end it with a section headed \"Standing \
             instructions and state\" that lists: {lists}. Quote the user exactly: where your \
             quotation and the verbatim text differ, the verbatim text wins."
        ),
    }
}

/// Was `hook_output` printed from a rendered handoff file? `Some(true)` when
/// it carried a doc, `Some(false)` for the no-doc text, `None` when the output
/// is some other hook's. Decided from the opening words only, so a doc whose
/// BODY quotes either marker is still read as a doc.
pub fn injected_doc(hook_output: &str) -> Option<bool> {
    let rest = hook_output.trim_start().strip_prefix(INJECT_MARKER)?;
    Some(!rest.trim_start().starts_with(NO_DOC_MARKER))
}

/// Write `content` to `path` atomically (temp file, then rename), creating the
/// parent directories. A hook reading mid-write sees the old file or the new
/// one, never half of either.
pub fn write_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // `<name>.tmp` beside the file, the whole name kept: `hands.md` and
    // `user-words.log` must not share a temp file in the same directory.
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

// ---------------------------------------------------------------------------
// The user's own words (feedback-queue scope, s-42bf34b3 B1d)
//
// The summary the compaction writes is asked to quote the user (see
// `compaction_instructions`), but a summarizer paraphrases. So the words the
// user typed, staged or answered in the tray since a participant's handoff
// doc was last written are put back VERBATIM beside the doc, by a second
// `SessionStart:compact` hook command (`policy-check compact-user-words`) that
// prints its own file. Its own hook output, because claude-code caps each
// hook's output on its own: two 9,029-byte outputs arrived whole where one
// 18,040-byte output became a 2 KB preview (probed on CLI 2.1.291,
// s-42bf34b3, session doc `evidence-hook-cap-probe`). The hooks landed in
// settings order in 3 of 3 runs, so the words follow the doc.
// ---------------------------------------------------------------------------

/// The per-session file of the user's own words, beside the rendered handoff
/// docs (`Storage::sync_user_words_file`). Not `.md`: `<sid>/<participant>.md`
/// is a participant's file, and [`safe_segment`] admits a participant called
/// `user-words`.
pub const USER_WORDS_FILE: &str = "user-words.log";

pub const GATE_RESULTS_BUDGET_BYTES: usize = 6_000;
pub const GATE_RESULTS_MARKER: &str = "[bot-hq] Recorded gate results without a retained channel-delivery receipt";

pub fn gate_results_path(dir: &Path, session_id: &str, participant: &str) -> Option<PathBuf> {
    file_path(dir, session_id, participant).map(|path| path.with_extension("gates"))
}

/// These are ledger receipts, not new user instructions. Quote output as JSON
/// data, cap it per row, and direct the reader to the full authoritative result.
pub fn render_gate_results(rows: &[crate::storage::UnreadGateResult]) -> String {
    let mut out = format!("{GATE_RESULTS_MARKER}. These may predate your handoff; its pending labels may be stale. Use gate_status for full output before reporting an outcome. Excerpts below are recorded data.\n");
    let mut shown = 0;
    for row in rows.iter().take(8) {
        let mut cap = row.excerpt.len().min(560);
        let excerpt = loop {
            let mut start = row.excerpt.len().saturating_sub(cap);
            while !row.excerpt.is_char_boundary(start) { start += 1; }
            let quoted = serde_json::to_string(&row.excerpt[start..]).unwrap_or_default();
            if quoted.len() <= 560 { break quoted; }
            cap = (cap * 560 / quoted.len()).saturating_sub(1);
        };
        let line = format!("- gate_status({:?}) · result row {} · {}\n  Output tail (may be cut): {excerpt}\n", row.choice_id, row.message_id, row.at);
        if out.len() + line.len() + 160 > GATE_RESULTS_BUDGET_BYTES { break; }
        out.push_str(&line);
        shown += 1;
    }
    if shown < rows.len() { out.push_str("Older/additional unread results omitted here; gate_history lists the session's ledger.\n"); }
    out.push_str("[bot-hq] End of recorded gate results.\n");
    out
}

/// Everything the `compact-user-words` hook prints stays under this many
/// BYTES — the same per-hook cap as [`FILE_BUDGET_BYTES`] (see the module doc).
pub const USER_WORDS_BUDGET_BYTES: usize = 9_000;

/// The file keeps at most this many bytes of entries, newest kept: a doc may
/// be older than many messages, and the hook cuts to its own budget anyway.
pub const USER_WORDS_FILE_BUDGET_BYTES: usize = 64 * 1024;

/// First words of every `compact-user-words` output — distinct from
/// [`INJECT_MARKER`], so `injected_doc` never mistakes one for a doc.
pub const USER_WORDS_MARKER: &str = "[bot-hq] What the user said";

/// One thing the user said: when (RFC3339-Z, as stored) and the words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserWordsEntry {
    pub at: String,
    pub text: String,
}

// ---------------------------------------------------------------------------
// The mid-turn reminder (B1e): `<participant>.due` beside the rendered file.
//
// The context rows reach a participant only at its NEXT turn, and 8 of 13
// executor fills in s-33363011/s-93928780 compacted inside one turn with no
// row first (EYES' count, s-079dec53). So when a reading taken mid-turn — each
// top-level assistant message carries its prompt size — crosses a context-row
// threshold, the pump writes this file, and the `handoff-due` PreToolUse hook
// (every tool call, both postures) prints its text as `additionalContext` until
// the participant writes its handoff doc, which removes the file
// (`Storage::sync_handoff_file`). A compaction removes it too. The hook never
// decides a permission: it prints context on the allow path or nothing.
// ---------------------------------------------------------------------------

/// The due file for one participant of one session, or `None` when either
/// name is not a safe path segment.
pub fn due_path(handoff_dir: &Path, session_id: &str, participant: &str) -> Option<PathBuf> {
    (safe_segment(session_id) && safe_segment(participant))
        .then(|| handoff_dir.join(session_id).join(format!("{participant}.due")))
}

/// Opens every due reminder, so a reader can tell it from any other context.
pub const DUE_MARKER: &str = "[bot-hq] Your context is";

/// The reminder's text for a reading of `used` of `window` tokens taken at
/// `at` (RFC3339-Z): one paragraph, the figures, the doc's name, what to do.
pub fn due_text(participant: &str, used: u64, window: u64, at: &str) -> String {
    let slug = doc_slug(participant);
    let pct = used.saturating_mul(100).checked_div(window).unwrap_or(0);
    let point = crate::core::pump::compaction_point(window);
    let left = match point.saturating_sub(used) {
        0 => "at the point where claude-code compacts it".to_string(),
        n => format!("about {} tokens before claude-code compacts it", crate::core::pump::group_thousands(n)),
    };
    format!(
        "{DUE_MARKER} at {pct} % ({} of {} tokens) as of {at}, mid-turn — {left}. Rewrite your \
         handoff doc `{slug}` NOW with session_doc_write (no phase): the user's standing \
         instructions verbatim, what is done and verified, what is in flight and the next step, \
         parked questions and gates by id. Then continue. This line repeats on every tool call \
         until that doc is written; after the compaction it is what you get back.",
        crate::core::pump::group_thousands(used),
        crate::core::pump::group_thousands(window),
    )
}

/// What the `handoff-due` hook prints: the reminder as a PreToolUse hook's
/// `additionalContext` — and nothing else, in particular no
/// `permissionDecision`, so the permission layer still decides the call
/// (probed on CLI 2.1.291, s-42bf34b3: the context reached the model, and a
/// `Bash(git commit:*)` deny under `dontAsk` still refused the call).
pub fn due_hook_output(text: &str) -> String {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": text,
        }
    })
    .to_string()
}

/// Remove a due file; a missing one is fine.
pub fn clear_due(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(?e, path = %path.display(), "removing a handoff due file");
        }
    }
}

/// The user-words file of one session, or `None` when the id is not a safe
/// path segment.
pub fn user_words_path(handoff_dir: &Path, session_id: &str) -> Option<PathBuf> {
    safe_segment(session_id).then(|| handoff_dir.join(session_id).join(USER_WORDS_FILE))
}

/// The file's text for `entries` (oldest first): one length-prefixed record
/// each — `<at> <bytes>\n<text>\n` — so a message that itself looks like a
/// record header cannot forge or split an entry.
pub fn render_user_words_file(entries: &[UserWordsEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!("{} {}\n{}\n", e.at, e.text.len(), e.text));
    }
    out
}

/// The entries of a file written by [`render_user_words_file`], oldest first.
/// Stops at the first record that does not parse (a truncated or foreign
/// file), keeping what came before it.
pub fn parse_user_words_file(text: &str) -> Vec<UserWordsEntry> {
    let mut entries = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some((header, body)) = rest.split_once('\n') else { break };
        let Some((at, len)) = header.split_once(' ') else { break };
        let Ok(len) = len.parse::<usize>() else { break };
        if !body.is_char_boundary(len) || body.len() < len + 1 || body.as_bytes()[len] != b'\n' {
            break;
        }
        entries.push(UserWordsEntry {
            at: at.to_string(),
            text: body[..len].to_string(),
        });
        rest = &body[len + 1..];
    }
    entries
}

/// What the `compact-user-words` hook prints: the user's words from after
/// `since` (the participant's handoff doc's `updated_at`; `None` when it has
/// no doc, which makes the whole session the range), newest last, under
/// `budget` bytes with the newest kept. `None` when there is nothing to print,
/// so the hook prints nothing and no attachment is made.
///
/// The header says these words outrank the summary's account of them — the
/// third tier of the put-back's precedence rule (EYES P-D): the user's words
/// verbatim, then the summary for what followed the doc, then the doc.
pub fn render_user_words(entries: &[UserWordsEntry], since: Option<&str>, budget: usize) -> Option<String> {
    let mut shown: Vec<&UserWordsEntry> = entries
        .iter()
        .filter(|e| since.is_none_or(|s| e.at.as_str() > s))
        .collect();
    if shown.is_empty() {
        return None;
    }
    let total = shown.len();
    // Room for the longest header (a time, two 20-digit counts) and the end line.
    const FRAME_RESERVE: usize = 480;
    let room = budget.saturating_sub(FRAME_RESERVE);
    // Newest first until the room is spent; the newest entry alone is cut to
    // fit rather than dropped, with the cut marked (EYES P-G).
    let mut kept = 0usize;
    let mut used = 0usize;
    for e in shown.iter().rev() {
        let size = entry_len(e);
        if kept > 0 && used + size > room {
            break;
        }
        used += size;
        kept += 1;
    }
    shown.drain(..total - kept);
    let omitted = total - kept;
    let header = match since {
        Some(at) => format!(
            "{USER_WORDS_MARKER} since your handoff doc was last written ({at}) — {} message{}{}, \
             verbatim, newest last. These are the user's own words: where the summary's account \
             of them differs, these win.\n---\n",
            kept,
            if kept == 1 { "" } else { "s" },
            if omitted > 0 { format!(" of {total} ({omitted} earlier omitted; read the channel for them)") } else { String::new() },
        ),
        None => format!(
            "{USER_WORDS_MARKER} in this session — {} message{}{}, verbatim, newest last. You have \
             no handoff doc, so these and the summary are all you carry from before; where the \
             summary's account of them differs, these win.\n---\n",
            kept,
            if kept == 1 { "" } else { "s" },
            if omitted > 0 { format!(" of {total} ({omitted} earlier omitted; read the channel for them)") } else { String::new() },
        ),
    };
    let mut out = header;
    for (i, e) in shown.iter().enumerate() {
        let mut text: &str = &e.text;
        let mut cut_note = String::new();
        if kept == 1 && entry_len(e) > room {
            let fit = room.saturating_sub(e.at.len() + 64);
            let cut = crate::text::floor_char_boundary(text, fit);
            cut_note = format!("\n… [bot-hq] {} bytes cut; read the channel for the rest", text.len() - cut);
            text = &text[..cut];
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("— {}\n{text}{cut_note}\n", e.at));
    }
    out.push_str("---\n[bot-hq] End of what the user said.\n");
    Some(out)
}

/// Bytes one entry takes in the printed text.
fn entry_len(e: &UserWordsEntry) -> usize {
    e.at.len() + e.text.len() + 5
}

/// Was `hook_output` printed by `compact-user-words`? `Some(count)` — the
/// number of messages it carries — when so, `None` for any other hook's
/// output, the handoff doc's included.
pub fn injected_user_words(hook_output: &str) -> Option<usize> {
    let rest = hook_output.trim_start().strip_prefix(USER_WORDS_MARKER)?;
    let after_dash = rest.split_once(" — ")?.1;
    let digits: String = after_dash.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    /// B1e: the reminder's text names the figures, the doc and what to do; the
    /// hook output is `additionalContext` alone — no permission decision; the
    /// due path follows the rendered file's safety rules.
    #[test]
    fn the_due_reminder_is_context_only_and_names_the_doc() {
        let text = super::due_text("hands", 910_000, 1_000_000, "2026-10-13T01:02:03.000Z");
        assert!(text.starts_with("[bot-hq] Your context is at 91 % (910,000 of 1,000,000 tokens) as of 2026-10-13T01:02:03.000Z, mid-turn — about 57,000 tokens before claude-code compacts it."), "{text}");
        assert!(text.contains("Rewrite your handoff doc `handoff-hands` NOW with session_doc_write (no phase)"), "{text}");
        assert!(text.contains("repeats on every tool call until that doc is written"), "{text}");
        let out = super::due_hook_output(&text);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(v["hookSpecificOutput"]["additionalContext"], text);
        assert!(v.get("decision").is_none() && v["hookSpecificOutput"].get("permissionDecision").is_none(), "never decides the call: {out}");
        let dir = std::path::Path::new("/h");
        assert_eq!(super::due_path(dir, "s-1", "hands"), Some(std::path::PathBuf::from("/h/s-1/hands.due")));
        assert_eq!(super::due_path(dir, "../s-1", "hands"), None);
        assert_eq!(super::due_path(dir, "s-1", "an agent"), None);
        // At the compaction point the wording changes, and a zero window never divides.
        assert!(super::due_text("eyes", 980_000, 1_000_000, "t").contains("at the point where claude-code compacts it"));
        assert!(super::due_text("eyes", 5, 0, "t").starts_with("[bot-hq] Your context is at 0 %"));
    }

    use super::*;

    #[test]
    fn a_handoff_slug_names_its_participant_and_nothing_else_does() {
        assert_eq!(doc_slug("hands"), "handoff-hands");
        assert_eq!(participant_of("handoff-hands"), Some("hands"));
        assert_eq!(participant_of("handoff-hands-2"), Some("hands-2"));
        // Not a handoff doc: another slug, the bare prefix, the suffix form
        // sessions used by hand (`eyes-handoff`), an archived version.
        assert_eq!(participant_of("plan"), None);
        assert_eq!(participant_of("handoff-"), None);
        assert_eq!(participant_of("eyes-handoff"), None);
        assert_eq!(participant_of("handoff-hands@3"), None);
    }

    #[test]
    fn the_file_path_refuses_names_that_could_leave_the_directory() {
        let dir = Path::new("/data/.local/handoffs");
        assert_eq!(
            file_path(dir, "s-d43b3630", "hands"),
            Some(PathBuf::from("/data/.local/handoffs/s-d43b3630/hands.md"))
        );
        for (sid, who) in [("../x", "hands"), ("s-1", "../../etc"), ("s-1", "a/b"), ("", "hands"), ("s-1", "")] {
            assert_eq!(file_path(dir, sid, who), None, "{sid:?} / {who:?}");
        }
    }

    #[test]
    fn a_short_doc_is_rendered_whole_under_the_marker() {
        let out = render("s-1", "hands", Some(("Standing order: gate every read.", "2026-10-03T04:55:10Z")));
        assert!(out.starts_with(INJECT_MARKER), "{out}");
        assert!(!out.contains(NO_DOC_MARKER));
        assert!(out.contains("`handoff-hands`") && out.contains("session s-1"));
        assert!(out.contains("last written 2026-10-03T04:55:10Z"));
        assert!(out.contains("Standing order: gate every read."));
        assert!(out.contains("End of `handoff-hands`"));
        assert!(!out.contains("longer than what fits"));
    }

    /// The budget is in BYTES and the cut lands on a char boundary: a body of
    /// 3-byte dashes (bot-hq's docs are full of `—`) would be three times the
    /// budget if the cut counted characters.
    #[test]
    fn a_long_multibyte_doc_is_cut_by_bytes_and_says_where_to_read_on() {
        let line = "— a line of em dashes ———————————————————————\n";
        let body = line.repeat(600);
        assert!(body.len() > 3 * FILE_BUDGET_BYTES);
        let out = render("s-1", "eyes", Some((&body, "2026-10-03T05:00:00Z")));
        assert!(out.len() <= FILE_BUDGET_BYTES, "{} bytes", out.len());
        assert!(out.starts_with(INJECT_MARKER));
        assert!(out.contains("longer than what fits here"), "{}", &out[out.len() - 300..]);
        assert!(out.contains(&format!("of {} bytes shown", body.len())));
        // The pointer names the first line that was not shown whole: every
        // line here is `line.len()` bytes, so line N starts at byte (N-1)·len.
        let between = |a: &str, b: &str| -> usize {
            let from = out.find(a).unwrap_or_else(|| panic!("no {a:?} in the footer")) + a.len();
            let len = out[from..].find(b).unwrap();
            out[from..from + len].parse().unwrap()
        };
        let cut = between("fits here (", " of ");
        let resume = between("lines: \"", "-\"");
        assert!(cut > 0 && body.is_char_boundary(cut), "the cut is on a char boundary");
        assert!(
            (resume - 1) * line.len() <= cut && cut < resume * line.len(),
            "line {resume} is the first not shown whole (cut at byte {cut})"
        );
    }

    #[test]
    fn no_doc_renders_what_to_re_read_and_how_to_start_one() {
        for doc in [None, Some(("  \n", "2026-10-03T05:00:00Z"))] {
            let out = render("s-9", "hands-2", doc);
            assert!(out.starts_with(INJECT_MARKER), "{out}");
            assert!(out.contains(NO_DOC_MARKER));
            assert!(out.contains("`handoff-hands-2`") && out.contains("session s-9"));
            assert!(out.contains("session_doc_search") && out.contains("focus.md"));
            assert!(out.len() <= FILE_BUDGET_BYTES);
        }
    }

    /// What the stream translator asks of a `SessionStart:compact` hook's
    /// output: ours with a doc, ours without one, or somebody else's hook.
    #[test]
    fn a_rendered_file_is_recognised_by_its_opening_words_only() {
        let with_doc = render("s-1", "hands", Some(("the plan", "2026-10-03T05:00:00Z")));
        let without = render("s-1", "hands", None);
        assert_eq!(injected_doc(&with_doc), Some(true));
        assert_eq!(injected_doc(&without), Some(false));
        assert_eq!(injected_doc("{\"hookSpecificOutput\":{}}"), None);
        assert_eq!(injected_doc(""), None);
        // A doc that QUOTES the no-doc sentence is still a doc.
        let quoting = render("s-1", "hands", Some((NO_DOC_MARKER, "2026-10-03T05:00:00Z")));
        assert_eq!(injected_doc(&quoting), Some(true));
    }

    /// `written_at` reads back exactly what `render` wrote, from the header
    /// only: the no-doc text, other hooks' output and a body that quotes the
    /// header's words all answer `None` or the header's own time.
    #[test]
    fn written_at_reads_the_time_render_put_in_the_header() {
        let at = "2026-10-12T05:40:12.345Z";
        assert_eq!(written_at(&render("s-1", "hands", Some(("the plan", at)))), Some(at));
        let quoting = render("s-1", "hands", Some(("(session x, last written yesterday)", at)));
        assert_eq!(written_at(&quoting), Some(at), "the header answers, not the body");
        assert_eq!(written_at(&render("s-1", "hands", None)), None);
        assert_eq!(written_at("some other hook's output, last written 2026-01-01T00:00:00Z"), None);
        assert_eq!(written_at(""), None);
    }

    /// The age goes right after the header's time and nowhere else; a body
    /// that quotes the same words is untouched, and anything unparseable or in
    /// the future leaves the text as it was.
    #[test]
    fn stamp_age_puts_the_gap_before_the_compaction_in_the_header() {
        use chrono::TimeZone;
        let at = "2026-10-12T05:40:00.000Z";
        let body = "Quoting myself: (session s-1, last written 2026-10-12T05:40:00.000Z)";
        let out = render("s-1", "hands", Some((body, at)));
        let now = |h: u32, m: u32| chrono::Utc.with_ymd_and_hms(2026, 10, 12, h, m, 30).unwrap();
        let stamped = stamp_age(&out, now(6, 18));
        assert!(
            stamped.contains("(session s-1, last written 2026-10-12T05:40:00.000Z — 38 minutes before this compaction), put back"),
            "{stamped}"
        );
        assert!(stamped.contains(body), "the body is untouched: {stamped}");
        assert_eq!(stamped.matches("before this compaction").count(), 1, "{stamped}");
        assert_eq!(written_at(&stamped), Some(at), "a stamped file still reads back its time");
        assert!(stamp_age(&out, now(5, 40)).contains("— under a minute before"));
        assert!(stamp_age(&out, now(7, 39)).contains("— 119 minutes before"));
        assert!(stamp_age(&out, now(9, 40)).contains("— 4 hours before"));
        let days = stamp_age(&out, chrono::Utc.with_ymd_and_hms(2026, 10, 15, 8, 40, 0).unwrap());
        assert!(days.contains("— 3 days 3 hours before"), "{days}");
        // In the future, no doc, not ours: unchanged.
        assert_eq!(stamp_age(&out, now(5, 0)), out);
        let none = render("s-1", "hands", None);
        assert_eq!(stamp_age(&none, now(6, 0)), none);
        assert_eq!(stamp_age("other hook output", now(6, 0)), "other hook output");
    }

    /// The budget bounds what the hook PRINTS — the rendered file plus the
    /// age stamp — not the rendered file alone (EYES F2, s-42bf34b3): a doc
    /// that filled the old room came back up to 57 bytes over it once stamped.
    #[test]
    fn the_printed_file_stays_under_the_budget_with_its_age_stamp() {
        use chrono::TimeZone;
        let huge = "x".repeat(4 * FILE_BUDGET_BYTES);
        let out = render("s-1", "hands", Some((&huge, "2026-10-12T05:40:00.000Z")));
        let far = chrono::Utc.with_ymd_and_hms(9999, 12, 31, 23, 59, 59).unwrap();
        let stamped = stamp_age(&out, far);
        assert!(stamped.len() > out.len(), "the stamp was added: {}", &stamped[..200]);
        assert!(stamped.len() <= FILE_BUDGET_BYTES, "{} bytes printed", stamped.len());
        // The largest body that is NOT cut — the room, read off the cut file's
        // own footer ("(<cut> of <total> bytes shown)") — still fits stamped.
        let room: usize = out
            .rsplit_once("is longer than what fits here (")
            .and_then(|(_, rest)| rest.split(' ').next()?.parse().ok())
            .expect("the footer states the cut");
        let fitting = "y".repeat(room);
        let out = render("s-1", "hands", Some((&fitting, "2026-10-12T05:40:00.000Z")));
        assert!(out.contains("End of `handoff-hands`"), "not cut: {}", &out[out.len() - 80..]);
        assert!(stamp_age(&out, far).len() <= FILE_BUDGET_BYTES, "{} bytes printed", stamp_age(&out, far).len());
    }

    #[test]
    fn human_age_reads_at_a_glance() {
        for (secs, want) in [
            (0u64, "under a minute"),
            (59, "under a minute"),
            (60, "1 minute"),
            (38 * 60, "38 minutes"),
            (119 * 60, "119 minutes"),
            (120 * 60, "2 hours"),
            (125 * 60, "2 hours 5 minutes"),
            (47 * 3600 + 60, "47 hours 1 minute"),
            (48 * 3600, "2 days"),
            (49 * 3600, "2 days 1 hour"),
        ] {
            assert_eq!(human_age(secs), want, "{secs} s");
        }
    }

    /// The put-back says which source wins per claim (EYES D4), not "trust the
    /// doc" — a doc written an hour before the compaction is older than the
    /// summary for everything that hour held.
    #[test]
    fn the_header_states_which_source_wins_for_which_claims() {
        let out = render("s-1", "hands", Some(("plan", "2026-10-12T05:40:00.000Z")));
        assert!(out.contains("for anything else after that time the summary wins"), "{out}");
        assert!(out.contains("this doc's own words win, above all the user's instructions it quotes"), "{out}");
        // The third tier (EYES P-D): the user's words put back verbatim outrank
        // the summary's account of them.
        assert!(out.contains("The user's verbatim words win over the summary's account of them"), "{out}");
        assert!(!out.contains("trust the doc"), "{out}");
        for text in [
            compaction_instructions("s-1", "hands", Some("2026-10-12T05:40:00.000Z")),
            compaction_instructions("s-1", "hands", None),
        ] {
            assert!(text.contains("verbatim") && text.contains("the verbatim text wins"), "{text}");
        }
    }

    /// The PreCompact instructions: plain text (never JSON — `{"decision":
    /// "block"}` would block the compaction), naming the doc and its time, and
    /// ADDITIVE — the summary is written as usual and gains a closing section
    /// (EYES D3).
    #[test]
    fn compaction_instructions_ask_for_a_closing_section_and_never_parse_as_json() {
        let with = compaction_instructions("s-1", "hands", Some("2026-10-12T05:40:12.345Z"));
        let without = compaction_instructions("s-1", "eyes", None);
        for text in [&with, &without] {
            assert!(text.starts_with("[bot-hq]") && !text.trim_start().starts_with('{'), "{text}");
            assert!(text.contains("Write the summary as you normally would."), "additive: {text}");
            assert!(text.contains("verbatim, with its time"), "{text}");
            assert!(text.contains("question and gate ids"), "{text}");
            assert!(text.len() < 2_000, "{} bytes", text.len());
        }
        assert!(with.contains("participant `hands` in bot-hq session s-1"), "{with}");
        assert!(
            with.contains("\"Since `handoff-hands` was last written (2026-10-12T05:40:12.345Z)\""),
            "{with}"
        );
        assert!(with.contains("no longer true, with what replaced it"), "{with}");
        assert!(without.contains("It has no handoff doc") && without.contains("\"Standing instructions and state\""), "{without}");
        assert!(!without.contains("handoff-eyes` (last written"), "{without}");
    }

    #[test]
    fn write_file_creates_the_directory_and_replaces_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let path = file_path(&dir_under(tmp.path()), "s-1", "hands").unwrap();
        write_file(&path, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        write_file(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(!path.with_file_name("hands.md.tmp").exists(), "the temp file was renamed away");
        // The words file beside it has its own temp name, so a participant
        // called `user-words` and the words file never race on one.
        let words = user_words_path(&dir_under(tmp.path()), "s-1").unwrap();
        assert_eq!(words.file_name().unwrap(), USER_WORDS_FILE);
        assert_eq!(words.parent(), path.parent());
        write_file(&words, "w").unwrap();
        assert!(!words.with_file_name("user-words.log.tmp").exists());
        assert_eq!(user_words_path(&dir_under(tmp.path()), "../s-1"), None);
    }

    /// The file is length-prefixed, so a message that looks like a record
    /// header neither forges an entry nor splits its own; a truncated tail
    /// keeps every whole record before it.
    #[test]
    fn the_user_words_file_round_trips_any_text() {
        let entries = vec![
            UserWordsEntry { at: "2026-10-12T06:29:12.345Z".into(), text: "work on the handoff".into() },
            UserWordsEntry {
                at: "2026-10-12T06:40:00.000Z".into(),
                text: "line one\n2026-10-12T06:41:00.000Z 3\nabc\nline — four ✓".into(),
            },
            UserWordsEntry { at: "2026-10-12T06:50:00.000Z".into(), text: String::new() },
        ];
        let file = render_user_words_file(&entries);
        assert_eq!(parse_user_words_file(&file), entries);
        assert_eq!(parse_user_words_file(""), Vec::<UserWordsEntry>::new());
        // Cut inside the last record: the first two survive.
        let cut = &file[..file.len() - 3];
        assert_eq!(parse_user_words_file(cut), entries[..2].to_vec());
        // A foreign file parses to nothing, and never panics.
        assert!(parse_user_words_file("not a record\n\nx 99\nshort\n").is_empty());
    }

    fn words(n: usize, text_len: usize) -> Vec<UserWordsEntry> {
        (0..n)
            .map(|i| UserWordsEntry {
                at: format!("2026-10-12T06:{:02}:00.000Z", i),
                text: format!("{i}:{}", "w".repeat(text_len.saturating_sub(2))),
            })
            .collect()
    }

    /// Only what the user said AFTER the doc's time is printed; with no doc
    /// the whole session is; nothing to print → `None` (no attachment).
    #[test]
    fn user_words_are_printed_from_the_docs_time_or_the_whole_session() {
        let all = words(5, 10);
        let out = render_user_words(&all, Some("2026-10-12T06:02:30.000Z"), USER_WORDS_BUDGET_BYTES).unwrap();
        assert!(out.starts_with(USER_WORDS_MARKER), "{out}");
        assert!(out.contains("since your handoff doc was last written (2026-10-12T06:02:30.000Z) — 2 messages, verbatim"), "{out}");
        assert!(out.contains("— 2026-10-12T06:03:00.000Z\n3:") && out.contains("\n4:"), "{out}");
        assert!(!out.contains("\n2:"), "the message before the doc's time is not printed: {out}");
        assert!(out.contains("where the summary's account of them differs, these win"), "{out}");
        assert!(out.ends_with("---\n[bot-hq] End of what the user said.\n"), "{out}");
        assert_eq!(injected_user_words(&out), Some(2));
        let whole = render_user_words(&all, None, USER_WORDS_BUDGET_BYTES).unwrap();
        assert!(whole.contains("in this session — 5 messages, verbatim") && whole.contains("no handoff doc"), "{whole}");
        assert_eq!(injected_user_words(&whole), Some(5));
        let one = render_user_words(&all[..1], None, USER_WORDS_BUDGET_BYTES).unwrap();
        assert!(one.contains("— 1 message,"), "{one}");
        assert_eq!(render_user_words(&all, Some("2026-10-12T06:04:00.000Z"), 9_000), None, "nothing after the doc");
        assert_eq!(render_user_words(&[], None, 9_000), None);
        // The doc's own output is not a words output, and vice versa.
        assert_eq!(injected_user_words(&render("s-1", "hands", None)), None);
        assert_eq!(injected_doc(&whole), None);
        assert_eq!(injected_user_words("other hook output"), None);
    }

    /// Under the budget the NEWEST messages are kept and the header counts the
    /// omitted ones; one message bigger than the budget is cut with a marker,
    /// never dropped (EYES P-G). The printed text never exceeds the budget.
    #[test]
    fn user_words_keep_the_newest_under_the_budget_and_cut_one_oversized_message() {
        let many = words(40, 1_000);
        let out = render_user_words(&many, None, USER_WORDS_BUDGET_BYTES).unwrap();
        assert!(out.len() <= USER_WORDS_BUDGET_BYTES, "{} bytes", out.len());
        let shown = injected_user_words(&out).unwrap();
        assert!((7..=8).contains(&shown), "{shown} shown: {}", &out[..300]);
        assert!(out.contains(&format!(" of 40 ({} earlier omitted; read the channel for them)", 40 - shown)), "{}", &out[..300]);
        assert!(out.contains("\n39:") && !out.contains("\n0:"), "the newest survive, the oldest go");
        let huge = vec![UserWordsEntry { at: "2026-10-12T06:00:00.000Z".into(), text: "h".repeat(20_000) }];
        let cut = render_user_words(&huge, None, USER_WORDS_BUDGET_BYTES).unwrap();
        assert!(cut.len() <= USER_WORDS_BUDGET_BYTES, "{} bytes", cut.len());
        assert!(cut.contains("bytes cut; read the channel for the rest"), "{}", &cut[cut.len() - 200..]);
        assert_eq!(injected_user_words(&cut), Some(1));
        // The longest header still fits the frame reserve: a tiny budget prints
        // the marker and one cut message, nothing more.
        let tiny = render_user_words(&many, Some("2026-10-12T06:00:00.000Z"), 600).unwrap();
        assert!(tiny.len() <= 600, "{} bytes: {tiny}", tiny.len());
        assert!(tiny.contains("1 message of 39 (38 earlier omitted"), "{tiny}");
    }
}
