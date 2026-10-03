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
        "{INJECT_MARKER} Below is your pinned handoff doc `{slug}` (session {session_id}, last \
         written {updated_at}), put back by bot-hq.\n\
         It was written before the compaction, by you or by the user. Where it and the summary \
         disagree, trust the doc, then check the live state: the phase documents \
         (session_doc_search), the project's focus.md and git.\n---\n"
    );
    // Room for the longest footer: two 20-digit byte counts and a line number.
    const FOOTER_RESERVE: usize = 240;
    let room = FILE_BUDGET_BYTES.saturating_sub(header.len() + FOOTER_RESERVE);
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
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn write_file_creates_the_directory_and_replaces_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let path = file_path(&dir_under(tmp.path()), "s-1", "hands").unwrap();
        write_file(&path, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        write_file(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(!path.with_extension("md.tmp").exists(), "the temp file was renamed away");
    }
}
