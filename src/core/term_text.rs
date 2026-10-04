//! Terminal output as the screen shows it, for the text agents quote.
//!
//! `terminal_read` and `terminal_exec` returned the PTY's bytes as they came:
//! colour and mode escapes (`[1m[7m%[27m…`), zsh's prompt redraw (a `%`, a
//! row of spaces, `\r`, the prompt again) and its echo of a typed line
//! (`p\bpython3`) — and that text is what agents paste into chat and IPAV
//! docs as evidence. [`render_plain`] replays the bytes onto lines the way a
//! terminal draws them, without emulating a screen: escapes are dropped, `\r`
//! and `\b` move the cursor so later text overwrites, and erase-to-end-of-line
//! truncates. Lines stay logical (no wrapping at the terminal's width), and a
//! full-screen program (vim, less) is not emulated.

/// `bytes` (lossy UTF-8) rendered to plain lines, trailing spaces trimmed.
pub fn render_plain(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let chars: Vec<char> = text.chars().collect();
    let mut done: Vec<String> = Vec::new();
    let mut line: Vec<char> = Vec::new();
    let mut col = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '\n' => {
                done.push(finish(&line));
                line.clear();
                col = 0;
            }
            '\r' => col = 0,
            '\u{8}' => col = col.saturating_sub(1),
            '\t' => col = (col / 8 + 1) * 8,
            '\u{1b}' => i = escape(&chars, i, &mut line, &mut col),
            c if c.is_control() => {}
            c => {
                if col < line.len() {
                    line[col] = c;
                } else {
                    line.resize(col, ' ');
                    line.push(c);
                }
                col += 1;
            }
        }
    }
    done.push(finish(&line));
    done.join("\n")
}

fn finish(line: &[char]) -> String {
    line.iter().collect::<String>().trim_end().to_string()
}

/// Handle the escape sequence whose `ESC` was just read; `i` is the index
/// after it. Returns the index after the sequence.
fn escape(chars: &[char], mut i: usize, line: &mut Vec<char>, col: &mut usize) -> usize {
    let Some(&kind) = chars.get(i) else { return i };
    i += 1;
    match kind {
        '[' => {
            // CSI: parameters, intermediates, one final byte.
            let start = i;
            while chars.get(i).is_some_and(|c| ('\u{30}'..='\u{3f}').contains(c)) {
                i += 1;
            }
            let params: String = chars[start..i].iter().collect();
            while chars.get(i).is_some_and(|c| ('\u{20}'..='\u{2f}').contains(c)) {
                i += 1;
            }
            let Some(&last) = chars.get(i) else { return i };
            i += 1;
            let n = |default: usize| params.split(';').next().and_then(|p| p.parse::<usize>().ok()).unwrap_or(default);
            match last {
                // Erase in line / in display — this line's part of it.
                'K' | 'J' => match n(0) {
                    0 => line.truncate(*col),
                    1 => {
                        for cell in line.iter_mut().take(*col + 1) {
                            *cell = ' ';
                        }
                    }
                    _ => line.clear(),
                },
                'C' => *col += n(1).max(1),
                'D' => *col = col.saturating_sub(n(1).max(1)),
                'G' => *col = n(1).max(1) - 1,
                _ => {}
            }
            i
        }
        // OSC, DCS, SOS, PM, APC: a string ended by BEL or ST (`ESC \`).
        ']' | 'P' | 'X' | '^' | '_' => {
            while let Some(&c) = chars.get(i) {
                i += 1;
                if c == '\u{7}' {
                    break;
                }
                if c == '\u{1b}' && chars.get(i) == Some(&'\\') {
                    i += 1;
                    break;
                }
            }
            i
        }
        // `ESC ( B` and the like: intermediates, then one final byte.
        c if ('\u{20}'..='\u{2f}').contains(&c) => {
            while chars.get(i).is_some_and(|c| ('\u{20}'..='\u{2f}').contains(c)) {
                i += 1;
            }
            i + usize::from(i < chars.len())
        }
        // `ESC =`, `ESC 7`, `ESC M`…: one byte.
        _ => i,
    }
}

#[cfg(test)]
mod tests {
    use super::render_plain;

    fn plain(s: &str) -> String {
        render_plain(s.as_bytes())
    }

    #[test]
    fn escapes_are_dropped() {
        assert_eq!(plain("\u{1b}[1mbold\u{1b}[0m \u{1b}[?2004htext"), "bold text");
        assert_eq!(plain("\u{1b}]0;a title\u{7}after"), "after");
        assert_eq!(plain("\u{1b}]11;?\u{1b}\\x"), "x");
        assert_eq!(plain("\u{1b}P>|xterm.js\u{1b}\\y"), "y");
        assert_eq!(plain("\u{1b}(Bz\u{1b}=\u{1b}7w"), "zw");
        assert_eq!(plain("a\u{7}b\u{0}c"), "abc");
    }

    /// zsh's echo of a typed line, and its prompt redraw (`PROMPT_SP`), as
    /// captured from `zsh -i` in a PTY in s-3158eb35 (names generalised).
    #[test]
    fn zsh_redraws_read_as_the_screen_shows_them() {
        assert_eq!(plain("p\u{8}python3 x"), "python3 x");
        let startup = format!(
            "\u{1b}[1m\u{1b}[7m%\u{1b}[27m\u{1b}[1m\u{1b}[0m{}\r \r\r\u{1b}[0m\u{1b}[27m\u{1b}[24m\u{1b}[J\
             user@host dir % \u{1b}[K\u{1b}[?2004h",
            " ".repeat(79)
        );
        assert_eq!(plain(&startup), "user@host dir %");
        assert_eq!(plain("$ make\r\nok\r\n$ "), "$ make\nok\n$");
    }

    #[test]
    fn carriage_return_backspace_and_erase_move_and_cut() {
        assert_eq!(plain("downloading 10%\rdownloading 100%"), "downloading 100%");
        assert_eq!(plain("abcdef\r\u{1b}[K"), "");
        assert_eq!(plain("abcdef\rxy\u{1b}[K"), "xy");
        assert_eq!(plain("abcdef\u{1b}[3D\u{1b}[1K"), "    ef");
        assert_eq!(plain("abc\u{1b}[2K"), "");
        assert_eq!(plain("a\u{1b}[3Cb"), "a   b");
        assert_eq!(plain("abc\u{1b}[2Gx"), "axc");
        assert_eq!(plain("a\tb"), "a       b");
        assert_eq!(plain("one\n\nthree  "), "one\n\nthree");
    }
}
