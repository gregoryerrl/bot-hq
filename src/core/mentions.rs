//! `@mention` — the participant the USER summons, parsed out of their message.
//!
//! rc3 **D17**. A mention chooses **who wakes**, not what they receive: the
//! summoned participant still drains every row past its own cursor when the turn
//! reaches it, exactly like any other holder. So this is a wake TARGET, not
//! addressing — the thing design §1 deleted — and it parameterises a power the
//! user already had (a user message resets the cycle to the front) rather than
//! adding a new one.
//!
//! ## Only the user may mention, and this module is where that is enforced
//!
//! Not in role prose, which is a request. The parse runs on exactly one path —
//! `AppState::broadcast`, which is the only writer of an `origin = "user"` row —
//! so a participant that types `@advisor` produces text, and nothing else. There
//! is no code that could honour it.
//!
//! That is a hard rule rather than an ordering preference (D1/D17). Peer
//! mentions compose into a summon loop nothing catches: HANDS mentions the
//! advisor, the advisor mentions the reviewer, the reviewer mentions HANDS —
//! every turn substantive, so the consensus tally is cleared by each one, spin
//! detection sees different prose every time, and no pass is ever cast. The
//! 500-lap round cap would be the only floor, at one real model call per lap on
//! the most expensive role in the session.
//!
//! ## What parses
//!
//! `@` at a word boundary, followed by a run of the characters
//! [`slugify`](crate::storage::participants) can produce: lowercase ASCII
//! alphanumerics and `-`. The boundary rule is what keeps `user@example.com`
//! from summoning `example`.
//!
//! An unknown slug is ordinary prose, never an error — the caller resolves what
//! it can and drops the rest. The UI makes the case rare by construction (the
//! `@` picker offers this session's participants and nothing else), but text can
//! arrive from the plugin proxy too, and a message refused for naming a
//! participant that left the roster would be a worse failure than a word that
//! did nothing.

/// Every `@slug` in `text`, in the order written, without repeats.
///
/// Case-folded, because a slug is lowercase by construction and a user who
/// typed `@EYES` meant the same participant. Deduplicated, because two mentions
/// of one participant are one summons — the queue hands out one turn per entry
/// and the second would be a turn spent re-reading what it just answered.
pub fn parse_mention_slugs(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    for (i, _) in text.match_indices('@') {
        // The boundary. `@` must not follow an alphanumeric, which is what
        // separates a mention from the tail of an email address or a handle
        // someone pasted. Punctuation before it is fine — `(@advisor` and
        // `"@advisor` both read as a mention to a human, so they do here.
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        // Backticks escape a mention (round 13, the user's rule for every
        // composer sigil): `` `@eyes` `` is the user SHOWING the syntax, not
        // summoning. Inside-a-code-span = an odd number of backticks before
        // this position — the same heuristic the composer pickers use, so the
        // two ends of the wire agree on what is literal.
        if bytes[..i].iter().filter(|b| **b == b'`').count() % 2 == 1 {
            continue;
        }
        let rest = &text[i + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(rest.len());
        // Trailing `-` is punctuation, not slug: `@eyes-` and `@eyes--` are the
        // participant `eyes`, while `@eyes-2` is the participant `eyes-2`.
        let slug = rest[..end].trim_end_matches('-').to_ascii_lowercase();
        if slug.is_empty() {
            continue;
        }
        if !out.contains(&slug) {
            out.push(slug);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mention_is_the_slug_after_the_at_sign() {
        assert_eq!(parse_mention_slugs("@advisor take a look"), ["advisor"]);
        assert_eq!(parse_mention_slugs("ping @eyes please"), ["eyes"]);
        assert_eq!(parse_mention_slugs("@eyes-2 you next"), ["eyes-2"]);
    }

    #[test]
    fn mentions_queue_in_the_order_written_and_do_not_repeat() {
        // D17 #3: `@advisor @security` gives the advisor the next turn and the
        // security role the one after.
        assert_eq!(
            parse_mention_slugs("@advisor @security both of you"),
            ["advisor", "security"]
        );
        // Two mentions of one participant are one summons. Otherwise a user
        // writing "@advisor … and @advisor again" spends the advisor's second
        // turn re-reading its own first one.
        assert_eq!(parse_mention_slugs("@advisor and @advisor"), ["advisor"]);
        // Case-folded to the same entry, so the dedupe sees them as one.
        assert_eq!(parse_mention_slugs("@Advisor and @ADVISOR"), ["advisor"]);
    }

    #[test]
    fn an_email_address_is_not_a_summons() {
        // The boundary rule's whole job. Without it every address in a pasted
        // log hands out a turn.
        assert!(parse_mention_slugs("mail me at user@example.com").is_empty());
        assert!(parse_mention_slugs("git log --author=me@host").is_empty());
        // …and the boundary is ALPHANUMERIC, not whitespace: punctuation before
        // the `@` still reads as a mention to a human.
        assert_eq!(parse_mention_slugs("(@advisor)"), ["advisor"]);
        assert_eq!(parse_mention_slugs("\"@advisor\""), ["advisor"]);
        assert_eq!(parse_mention_slugs("- @advisor"), ["advisor"]);
    }

    #[test]
    fn punctuation_ends_a_slug_and_a_bare_at_sign_is_nothing() {
        assert_eq!(parse_mention_slugs("@advisor, thoughts?"), ["advisor"]);
        assert_eq!(parse_mention_slugs("@advisor: go"), ["advisor"]);
        // A trailing hyphen is punctuation; an interior one is slug.
        assert_eq!(parse_mention_slugs("@eyes- and @eyes-2"), ["eyes", "eyes-2"]);
        assert!(parse_mention_slugs("@ advisor").is_empty());
        assert!(parse_mention_slugs("email @ 5pm").is_empty());
        assert!(parse_mention_slugs("@@").is_empty());
    }

    #[test]
    fn a_slug_that_names_nobody_still_parses_and_is_the_callers_problem() {
        // Resolution is the caller's, and an unknown slug is ordinary prose
        // there (D1). This function's contract is the SHAPE — it cannot know
        // the roster, and a parser that needed one could not be tested without
        // a database.
        assert_eq!(parse_mention_slugs("@nobody-at-all hi"), ["nobody-at-all"]);
    }

    #[test]
    fn a_backticked_mention_is_shown_not_summoned() {
        // Round 13: backticks escape every composer sigil, `@` included.
        assert!(parse_mention_slugs("write `@eyes` to summon").is_empty());
        assert!(parse_mention_slugs("`@eyes @hands` are the two").is_empty());
        // Outside the span, mentions still parse — the count is positional.
        assert_eq!(parse_mention_slugs("run `x` then @eyes go"), ["eyes"]);
        // An unclosed backtick escapes everything after it — the safe reading
        // of a half-typed code span.
        assert!(parse_mention_slugs("see ` @eyes").is_empty());
    }

    #[test]
    fn a_mention_inside_a_word_or_a_path_is_not_one() {
        assert!(parse_mention_slugs("src/foo@v2/bar").is_empty());
        assert!(parse_mention_slugs("v1.2@beta").is_empty());
    }

    #[test]
    fn multibyte_text_does_not_split_a_character() {
        // `match_indices` gives byte offsets and the slice below is byte-based,
        // so a non-ASCII neighbour is where this would panic rather than
        // misparse. The boundary check reads ONE byte back, which is why it is
        // `is_ascii_alphanumeric` — a UTF-8 continuation byte is not ASCII, so
        // a `@` after a multibyte character is a boundary, which is the right
        // answer for "→@advisor" and the safe one otherwise.
        assert_eq!(parse_mention_slugs("→ @advisor"), ["advisor"]);
        assert_eq!(parse_mention_slugs("café @advisor"), ["advisor"]);
        assert_eq!(parse_mention_slugs("@advisor café"), ["advisor"]);
        assert!(parse_mention_slugs("日本語").is_empty());
    }
}
