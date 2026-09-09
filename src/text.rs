//! Byte-offset helpers for `str` slicing that must never land inside a
//! multibyte character.
//!
//! Two round-9 finds motivated this module: `agents::events::short_line`
//! sliced an unparseable claude-code stdout line at byte 160, and
//! `terminal_tools` kept the last 16 KiB of lossy-decoded PTY output by byte
//! offset — both PANIC when the offset falls inside a UTF-8 sequence (a
//! progress bar's `━`, a box-drawing `─`, an emoji), and a panic in a spawned
//! task surfaces as an agent that simply stops answering. `str::floor_char_boundary`
//! is what these want, but it is unstable, so this free function carries the
//! idiom the tree already had in `storage::participants::clamped_body`.

/// The largest byte index `<= idx` (and `<= s.len()`) that is a char boundary
/// of `s`, so `&s[..floor_char_boundary(s, idx)]` (or `&s[floor..]`) never
/// panics. Mirrors the unstable `str::floor_char_boundary`.
pub fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut cut = idx;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// The smallest byte index `>= idx` (capped at `s.len()`) that is a char
/// boundary of `s` — for keeping a TAIL of at most `len - idx` bytes: the
/// partial char at the front is dropped rather than a whole extra one kept.
/// Mirrors the unstable `str::ceil_char_boundary`.
pub fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut cut = idx;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_walks_forward_to_the_next_char_start() {
        let s = "a─b"; // a(0) ─(1..4) b(4)
        assert_eq!(ceil_char_boundary(s, 1), 1);
        assert_eq!(ceil_char_boundary(s, 2), 4);
        assert_eq!(ceil_char_boundary(s, 3), 4);
        assert_eq!(ceil_char_boundary(s, 99), 5);
        for idx in 0..=s.len() + 2 {
            let _tail = &s[ceil_char_boundary(s, idx)..];
        }
    }

    #[test]
    fn a_boundary_index_is_returned_unchanged() {
        assert_eq!(floor_char_boundary("abc", 0), 0);
        assert_eq!(floor_char_boundary("abc", 2), 2);
        assert_eq!(floor_char_boundary("abc", 3), 3);
    }

    #[test]
    fn an_index_inside_a_multibyte_char_walks_back_to_its_start() {
        // "─" is 3 bytes (E2 94 80); "a─b" = a(0) ─(1..4) b(4)
        let s = "a─b";
        assert_eq!(floor_char_boundary(s, 2), 1);
        assert_eq!(floor_char_boundary(s, 3), 1);
        assert_eq!(floor_char_boundary(s, 4), 4);
    }

    #[test]
    fn an_index_past_the_end_clamps_to_len() {
        assert_eq!(floor_char_boundary("a─b", 99), 5);
        assert_eq!(floor_char_boundary("", 5), 0);
    }

    /// The property the two callers rely on: slicing at the result never panics,
    /// whatever the index and whatever the text.
    #[test]
    fn slicing_at_the_result_never_panics() {
        let s = "cargo ━━━━━━━━━━ 12/34 ─── done ✓ 🚀";
        for idx in 0..=s.len() + 3 {
            let cut = floor_char_boundary(s, idx);
            let _head = &s[..cut];
            let _tail = &s[cut..];
        }
    }
}
