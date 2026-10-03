//! The zsh `"$var:x"` trap (feedback #96).
//!
//! Under zsh, `$name:` followed by one of zsh's modifier letters is not "the
//! parameter, then a colon": zsh applies the modifier. `git show "$R:app/x"`
//! asks git for the absolute path of `$R` followed by `pp/x` (`:a`), and
//! `"$r:tasks.md"` becomes the tail of `$r` followed by `asks.md` (`:t`). Under
//! bash the same text is literal, which is why it reads as correct. The general
//! rules warn about it, and a reviewer still hit it twice in one session
//! (s-123f01aa, 2026-10-02): the second time a `for` loop over `git show
//! "$c:path"` with `2>/dev/null` printed nothing at all, and "nothing" nearly
//! became "the setting did not exist in those commits". A check that reports
//! clean while doing nothing is the dangerous kind.
//!
//! [`check`] finds those places so a hook can refuse the command BEFORE it
//! runs, with the corrected form (`"${R}:app/x"`, which every shell reads as a
//! literal colon). It is a scanner, not a parser: it follows quoting, `$(…)`,
//! backticks, `${…}` and heredocs well enough to skip what zsh does not
//! expand, and anything it cannot follow (an unterminated quote) yields NO
//! hits. A lint that refused a command because it misread it would block work
//! on its own mistake.
//!
//! What counts as a modifier was measured on zsh 5.9 (s-3158eb35, 2026-10-03),
//! and [`tests::the_table_agrees_with_the_installed_zsh`] measures it again
//! wherever zsh is installed:
//! - `a A c e h l P q Q r s t u` apply, and `&` inside quotes (unquoted, `&` is
//!   the shell's own operator and ends the word first);
//! - `f`, `w` and `g` apply when another modifier follows them (`:fh`, `:wt`,
//!   `:gs/a/b/`), and are literal otherwise (`:fo`);
//! - `F` and `W` take a delimited argument and apply irregularly (`:Footer`
//!   applies, `:Fixtures` does not), so any `F`/`W` followed by more of the
//!   word is flagged: the braced form is right in every shell, so the cost of
//!   over-flagging is one rewrite;
//! - every parameter form takes a modifier: names, `$1`, `$?`, `$#`, `$$`,
//!   `$@`, `$*`, `$!`, `$-`, and subscripts (`$a[1]:t`).

use std::path::Path;

/// One place zsh would apply a modifier instead of reading a colon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Byte offset of the `$`.
    pub at: usize,
    /// Byte offset of the `:` (one past the parameter).
    pub colon: usize,
    /// The parameter as written after the `$`: `R`, `1`, `?`, `a[1]`.
    pub param: String,
    /// The modifier zsh reads at the colon: `a`, `fh`, `gs`, `Fo`.
    pub modifier: String,
}

/// What [`check`] found: the hits, and the command with every hit braced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trap {
    pub hits: Vec<Hit>,
    /// The command with each `$name:` rewritten as `${name}:`.
    pub corrected: String,
}

/// The places in `command` where zsh would apply a modifier after `$name:`,
/// or `None` when there are none — or when the scanner cannot follow the
/// command (fail open: see the module doc).
pub fn check(command: &str) -> Option<Trap> {
    let mut scanner = Scanner {
        s: command.as_bytes(),
        hits: Vec::new(),
        pending: Vec::new(),
    };
    scanner.code(0, Until::End).ok()?;
    if scanner.hits.is_empty() {
        return None;
    }
    let mut hits = scanner.hits;
    hits.sort_by_key(|h| h.at);
    hits.dedup_by_key(|h| h.at);
    let mut corrected = String::with_capacity(command.len() + 2 * hits.len());
    let mut from = 0;
    for h in &hits {
        // `$` and `:` are ASCII, so both offsets are char boundaries.
        corrected.push_str(&command[from..h.at]);
        corrected.push_str("${");
        corrected.push_str(&h.param);
        corrected.push('}');
        from = h.colon;
    }
    corrected.push_str(&command[from..]);
    Some(Trap { hits, corrected })
}

/// What the agent reads when [`check`] refuses its command: why it is not a
/// gate stop, what zsh would do at each hit, and the corrected command.
///
/// It opens by saying this is NOT a Tool Gate refusal, because the general
/// rules tell an agent never to reword a gated command: an agent that took
/// this for one would route it to `action_gate`, which applies the same check
/// and refuses it again.
pub fn refusal(command: &str, trap: &Trap) -> String {
    let mut out = String::from(
        "Not a Tool Gate stop: zsh would change this command before it runs, so \
         bot-hq did not run it. Rewriting the command IS the fix here (action_gate \
         and terminal_exec apply the same check).\n\
         Under zsh, `$name:` followed by a modifier letter is not \"the parameter, \
         then a colon\": zsh applies the modifier.\n",
    );
    for h in &trap.hits {
        let first = h.modifier.trim_start_matches(['f', 'w', 'g']).chars().next();
        let what = first.map(modifier_meaning).unwrap_or("a modifier");
        out.push_str(&format!(
            "  - `${}:{}` — zsh reads `:{}` ({what}).\n",
            h.param, h.modifier, h.modifier
        ));
    }
    const SHOW_WHOLE: usize = 1500;
    if trap.corrected.len() <= SHOW_WHOLE {
        out.push_str(&format!(
            "Brace the parameter, which every shell reads as a literal colon:\n{}\n",
            trap.corrected
        ));
    } else {
        // A long script: the changed lines, not the whole body twice.
        out.push_str("Brace the parameter, which every shell reads as a literal colon. The lines to change:\n");
        let (before, after) = (command.lines(), trap.corrected.lines());
        for (b, a) in before.zip(after).filter(|(b, a)| b != a) {
            out.push_str(&format!("  {} \n  → {}\n", clip(b, 200), clip(a, 200)));
        }
    }
    if let Some(h) = trap.hits.first() {
        out.push_str(&format!(
            "(If you meant zsh's modifier, put it inside the braces: `${{{}:{}}}`.)",
            h.param, h.modifier
        ));
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn modifier_meaning(c: char) -> &'static str {
    match c {
        'a' => "the absolute path",
        'A' | 'P' => "the real path",
        'c' => "the command's path",
        'e' => "the extension only",
        'h' => "the head: everything before the last `/`",
        'l' => "lowercase",
        'q' => "quote it",
        'Q' => "remove quoting",
        'r' => "remove the extension",
        's' => "a substitution",
        't' => "the tail: everything after the last `/`",
        'u' => "uppercase",
        '&' => "repeat the last substitution",
        'F' | 'W' => "a modifier with an argument",
        _ => "a modifier",
    }
}

/// Whether the claude-code CLI's Bash tool runs zsh, given its environment —
/// the CLI's own order (2.1.284): `CLAUDE_CODE_SHELL` when it names bash or
/// zsh, else `SHELL` when it names one of them, else zsh when it is installed.
/// Under bash the trap does not exist, and refusing `"$R:app"` there would
/// refuse a correct command.
pub fn agent_shell_is_zsh(
    code_shell: Option<&str>,
    shell: Option<&str>,
    zsh_present: impl FnOnce() -> bool,
) -> bool {
    let base = |s: &str| {
        Path::new(s)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(s)
            .to_string()
    };
    if let Some(cs) = code_shell.map(str::trim).filter(|s| !s.is_empty()) {
        let name = base(cs);
        if name.contains("zsh") {
            return true;
        }
        if name.contains("bash") {
            return false;
        }
    }
    if let Some(sh) = shell.map(str::trim).filter(|s| !s.is_empty()) {
        if sh.contains("bash") {
            return false;
        }
        if sh.contains("zsh") {
            return true;
        }
    }
    zsh_present()
}

/// [`agent_shell_is_zsh`] for THIS process's environment — the hook inherits
/// the agent CLI's.
pub fn agent_shell_is_zsh_here() -> bool {
    let code_shell = std::env::var("CLAUDE_CODE_SHELL").ok();
    let shell = std::env::var("SHELL").ok();
    agent_shell_is_zsh(code_shell.as_deref(), shell.as_deref(), zsh_installed)
}

/// The refusal for `command` when it will run under `shell` and carries the
/// trap — `None` under any shell but zsh, where the same text is literal.
/// What `action_gate` (its gate shell) and `terminal_exec` (the terminal's
/// `$SHELL`) ask before they park or run anything.
pub fn zsh_trap(shell: &str, command: &str) -> Option<String> {
    if !is_zsh(shell) {
        return None;
    }
    let trap = check(command)?;
    Some(refusal(command, &trap))
}

/// Whether a shell path or name is zsh (`/bin/zsh`, `zsh`, `zsh-5.9`).
pub fn is_zsh(shell: &str) -> bool {
    Path::new(shell)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains("zsh"))
}

fn zsh_installed() -> bool {
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("zsh").is_file()))
        .unwrap_or(false);
    on_path
        || ["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh", "/opt/homebrew/bin/zsh"]
            .iter()
            .any(|p| Path::new(p).is_file())
}

// ---------------------------------------------------------------------------
// The scanner
// ---------------------------------------------------------------------------

/// Where a stretch of shell code ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Until {
    /// The end of the command.
    End,
    /// The `)` closing a `$(` (or `$((`).
    Paren,
    /// The closing backtick.
    Backtick,
    /// The `}` closing a `${`.
    Brace,
}

/// How the text around a `$` is quoted — it decides whether `&` can be a
/// modifier (unquoted, it is the shell's operator and ends the word first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quoting {
    Unquoted,
    /// Double quotes, an unquoted heredoc's body, or a `${…}` word.
    Quoted,
}

/// A heredoc announced on the current line; its body starts at the next line.
struct Heredoc {
    delim: Vec<u8>,
    strip_tabs: bool,
    /// `<<EOF` expands parameters in its body; `<<'EOF'` does not.
    expands: bool,
}

/// `Err(())` = the scanner cannot follow the command; [`check`] then reports
/// nothing.
type Scan = Result<usize, ()>;

struct Scanner<'a> {
    s: &'a [u8],
    hits: Vec<Hit>,
    pending: Vec<Heredoc>,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The characters that end an unquoted word in the shell's own lexer.
fn is_meta(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'<' | b'>' | b'(' | b')')
}

impl Scanner<'_> {
    /// Shell code from `i` to where `until` says it ends; returns the offset
    /// just past that end.
    fn code(&mut self, mut i: usize, until: Until) -> Scan {
        let s = self.s;
        let mut parens = 0usize;
        let mut braces = 0usize;
        // A `#` starts a comment only at the start of a word — and never
        // inside `${…}`, where `${#v}` and `${v#x}` are operators.
        let mut word_start = true;
        while i < s.len() {
            let c = s[i];
            match c {
                b'\\' => {
                    i += 2;
                    word_start = false;
                    continue;
                }
                b'\'' => {
                    i = self.single(i + 1)?;
                    word_start = false;
                    continue;
                }
                b'"' => {
                    i = self.double(i + 1)?;
                    word_start = false;
                    continue;
                }
                b'`' => {
                    if until == Until::Backtick {
                        return Ok(i + 1);
                    }
                    i = self.code(i + 1, Until::Backtick)?;
                    word_start = false;
                    continue;
                }
                b'$' => {
                    let quoting = if until == Until::Brace { Quoting::Quoted } else { Quoting::Unquoted };
                    i = self.dollar(i, quoting)?;
                    word_start = false;
                    continue;
                }
                b'#' if word_start && until != Until::Brace => {
                    while i < s.len() && s[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                b'\n' => {
                    i = self.heredoc_bodies(i + 1)?;
                    word_start = true;
                    continue;
                }
                b'<' if until != Until::Brace && s.get(i + 1) == Some(&b'<') => {
                    if s.get(i + 2) == Some(&b'<') {
                        // A here-string: its word is ordinary text, scanned on.
                        i += 3;
                    } else {
                        i = self.heredoc_operator(i + 2)?;
                    }
                    word_start = true;
                    continue;
                }
                b'(' => {
                    parens += 1;
                }
                b')' => {
                    if until == Until::Paren && parens == 0 {
                        return Ok(i + 1);
                    }
                    parens = parens.saturating_sub(1);
                }
                b'{' if until == Until::Brace => braces += 1,
                b'}' if until == Until::Brace => {
                    if braces == 0 {
                        return Ok(i + 1);
                    }
                    braces -= 1;
                }
                _ => {}
            }
            word_start = until != Until::Brace && is_meta(c);
            i += 1;
        }
        match until {
            Until::End => Ok(s.len()),
            _ => Err(()),
        }
    }

    /// From just past an opening `'` to just past its closing `'`.
    fn single(&self, i: usize) -> Scan {
        match self.s[i.min(self.s.len())..].iter().position(|&c| c == b'\'') {
            Some(p) => Ok(i + p + 1),
            None => Err(()),
        }
    }

    /// `$'…'`: backslash escapes, no expansion.
    fn ansi_c(&self, mut i: usize) -> Scan {
        while i < self.s.len() {
            match self.s[i] {
                b'\\' => i += 2,
                b'\'' => return Ok(i + 1),
                _ => i += 1,
            }
        }
        Err(())
    }

    /// From just past an opening `"` to just past its closing `"`.
    fn double(&mut self, mut i: usize) -> Scan {
        while i < self.s.len() {
            match self.s[i] {
                b'\\' => i += 2,
                b'"' => return Ok(i + 1),
                b'`' => i = self.code(i + 1, Until::Backtick)?,
                b'$' => i = self.dollar(i, Quoting::Quoted)?,
                _ => i += 1,
            }
        }
        Err(())
    }

    /// An expanding heredoc body, `[i, end)`: like double quotes without the
    /// closing quote (a `"` in a heredoc body is just a character).
    fn heredoc_text(&mut self, mut i: usize, end: usize) -> Result<(), ()> {
        while i < end {
            match self.s[i] {
                b'\\' => i += 2,
                b'`' => i = self.code(i + 1, Until::Backtick)?,
                b'$' => i = self.dollar(i, Quoting::Quoted)?,
                _ => i += 1,
            }
        }
        Ok(())
    }

    /// At a `$`: what follows it, and whether it is a hit.
    fn dollar(&mut self, at: usize, quoting: Quoting) -> Scan {
        let s = self.s;
        let j = at + 1;
        let Some(&next) = s.get(j) else {
            return Ok(j);
        };
        match next {
            b'{' => self.code(j + 1, Until::Brace),
            b'(' => self.code(j + 1, Until::Paren),
            // `$'…'` and `$"…"` are quoting only outside double quotes; inside
            // them the `$` is literal and the quote is the string's own.
            b'\'' if quoting == Quoting::Unquoted => self.ansi_c(j + 1),
            b'"' if quoting == Quoting::Unquoted => self.double(j + 1),
            c if is_ident_start(c) => {
                let mut end = j;
                while end < s.len() && is_ident(s[end]) {
                    end += 1;
                }
                // A subscript belongs to the parameter: `$a[1]:t` applies `:t`
                // to the element.
                if s.get(end) == Some(&b'[') {
                    if let Some(close) = s[end..].iter().position(|&c| c == b']') {
                        end += close + 1;
                    }
                }
                self.colon(at, end, quoting);
                Ok(end)
            }
            c if c.is_ascii_digit() => {
                let mut end = j;
                while end < s.len() && s[end].is_ascii_digit() {
                    end += 1;
                }
                self.colon(at, end, quoting);
                Ok(end)
            }
            b'?' | b'#' | b'$' | b'@' | b'*' | b'!' | b'-' => {
                self.colon(at, j + 1, quoting);
                Ok(j + 1)
            }
            _ => Ok(j),
        }
    }

    /// A parameter `s[at+1..end]` that a `:` follows: record a hit when what
    /// follows the colon is a modifier zsh applies.
    fn colon(&mut self, at: usize, end: usize, quoting: Quoting) {
        if self.s.get(end) != Some(&b':') {
            return;
        }
        if let Some(len) = self.modifier_at(end + 1, quoting) {
            self.hits.push(Hit {
                at,
                colon: end,
                param: String::from_utf8_lossy(&self.s[at + 1..end]).into_owned(),
                modifier: String::from_utf8_lossy(&self.s[end + 1..end + 1 + len]).into_owned(),
            });
        }
    }

    /// The length of the modifier zsh would read at `j`, if any (module doc).
    fn modifier_at(&self, j: usize, quoting: Quoting) -> Option<usize> {
        match *self.s.get(j)? {
            b'a' | b'A' | b'c' | b'e' | b'h' | b'l' | b'P' | b'q' | b'Q' | b'r' | b's' | b't'
            | b'u' => Some(1),
            b'&' if quoting == Quoting::Quoted => Some(1),
            b'f' | b'w' | b'g' => self.modifier_at(j + 1, quoting).map(|n| n + 1),
            b'F' | b'W' => match self.s.get(j + 1) {
                Some(&c) if c.is_ascii_graphic() && !matches!(c, b'"' | b'\'' | b'`')
                    && !(quoting == Quoting::Unquoted && is_meta(c)) =>
                {
                    Some(2)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// After `<<` (and not `<<<`): the delimiter word, registered for the
    /// next line. Returns the offset past the word.
    fn heredoc_operator(&mut self, mut i: usize) -> Scan {
        let s = self.s;
        let strip_tabs = s.get(i) == Some(&b'-');
        if strip_tabs {
            i += 1;
        }
        while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
            i += 1;
        }
        let mut delim = Vec::new();
        let mut quoted = false;
        while i < s.len() && !is_meta(s[i]) {
            match s[i] {
                b'\'' => {
                    quoted = true;
                    let close = self.single(i + 1)?;
                    delim.extend_from_slice(&s[i + 1..close - 1]);
                    i = close;
                }
                b'"' => {
                    quoted = true;
                    let close = s[i + 1..].iter().position(|&c| c == b'"').ok_or(())? + i + 1;
                    delim.extend_from_slice(&s[i + 1..close]);
                    i = close + 1;
                }
                b'\\' => {
                    quoted = true;
                    if let Some(&c) = s.get(i + 1) {
                        delim.push(c);
                    }
                    i += 2;
                }
                c => {
                    delim.push(c);
                    i += 1;
                }
            }
        }
        if delim.is_empty() {
            return Err(());
        }
        self.pending.push(Heredoc { delim, strip_tabs, expands: !quoted });
        Ok(i)
    }

    /// At the start of a line: the bodies of the heredocs announced on the
    /// line before, in order. Returns the offset past the last one.
    fn heredoc_bodies(&mut self, mut i: usize) -> Scan {
        let s = self.s;
        for doc in std::mem::take(&mut self.pending) {
            let body_start = i;
            let mut body_end = s.len();
            let mut after = s.len();
            let mut line = i;
            while line < s.len() {
                let nl = s[line..].iter().position(|&c| c == b'\n').map(|p| line + p);
                let text_end = nl.unwrap_or(s.len());
                let mut text = &s[line..text_end];
                if doc.strip_tabs {
                    while let [b'\t', rest @ ..] = text {
                        text = rest;
                    }
                }
                if text == doc.delim.as_slice() {
                    body_end = line;
                    after = nl.map(|p| p + 1).unwrap_or(s.len());
                    break;
                }
                line = match nl {
                    Some(p) => p + 1,
                    None => s.len(),
                };
            }
            // No delimiter line: the body runs to the end, as zsh reads it
            // (with a warning).
            if doc.expands {
                self.heredoc_text(body_start, body_end)?;
            }
            i = after;
        }
        Ok(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(command: &str) -> Vec<(String, String)> {
        check(command)
            .map(|t| t.hits.into_iter().map(|h| (h.param, h.modifier)).collect())
            .unwrap_or_default()
    }

    fn hit(p: &str, m: &str) -> (String, String) {
        (p.to_string(), m.to_string())
    }

    /// The two incidents #96 reported, and the general rules' own example.
    #[test]
    fn the_reported_shapes_are_caught_and_braced() {
        let t = check(r#"git show "$R:app/Jobs/FetchMetaGeoDataJob.php""#).expect("a hit");
        assert_eq!(t.hits.len(), 1);
        assert_eq!((t.hits[0].param.as_str(), t.hits[0].modifier.as_str()), ("R", "a"));
        assert_eq!(t.corrected, r#"git show "${R}:app/Jobs/FetchMetaGeoDataJob.php""#);

        let looped = r#"for c in a b; do git show "$c:app/Jobs/x.php" | grep -n retry; done 2>/dev/null"#;
        assert_eq!(params(looped), vec![hit("c", "a")]);
        assert_eq!(
            check(looped).unwrap().corrected,
            r#"for c in a b; do git show "${c}:app/Jobs/x.php" | grep -n retry; done 2>/dev/null"#
        );

        assert_eq!(params(r#"git show "$r:tasks.md""#), vec![hit("r", "t")]);
    }

    /// What zsh reads literally must pass: a lint that blocks a correct
    /// command costs the agent a turn for nothing.
    #[test]
    fn literal_colons_pass() {
        for ok in [
            r#"echo "$PATH:/opt/bin""#,
            r#"ssh "$host:$port""#,
            r#"curl "$base:8080/x""#,
            r#"echo "$v:""#,
            r#"echo "$v::x""#,
            r#"echo "$v:x""#,
            r#"echo "$v:fo $v:wo $v:gx $v:F $v:W""#,
            r#"git show "${R}:app/x""#,
            r#"echo "${R:a}""#,
            "echo '$R:app'",
            "echo $'$R:app\\n'",
            r#"echo \$R:app"#,
            r#"awk '{print $1":"$2}' f"#,
            "echo $v:&& echo hi",
            "echo $v:& wait",
            "cat <<'EOF'\n$v:tasks\nEOF",
            "cat <<\"EOF\"\n$v:tasks\nEOF",
            "cat <<\\EOF\n$v:tasks\nEOF",
            "python3 - <<'PY'\nprint(\"$v:t\")\nPY",
            "# git show \"$R:app\"\necho ok",
            "echo plain",
            "",
        ] {
            assert_eq!(check(ok), None, "{ok:?}");
        }
    }

    #[test]
    fn every_context_zsh_expands_is_scanned() {
        let cases: &[(&str, Vec<(String, String)>)] = &[
            ("echo $v:t", vec![hit("v", "t")]),
            (r#"echo "$v:h""#, vec![hit("v", "h")]),
            (r#"echo "$v:&""#, vec![hit("v", "&")]),
            (r#"echo "$v:fh $v:wt $v:gs/a/b/""#, vec![hit("v", "fh"), hit("v", "wt"), hit("v", "gs")]),
            (r#"echo "$v:Footer""#, vec![hit("v", "Fo")]),
            (r#"echo $(basename "$v:t")"#, vec![hit("v", "t")]),
            ("echo `echo $v:h`", vec![hit("v", "h")]),
            (r#"echo ${q:-"$v:t"} ${q:-$w:e}"#, vec![hit("v", "t"), hit("w", "e")]),
            (r#"cat <<< "$v:t""#, vec![hit("v", "t")]),
            ("cat <<< $v:r", vec![hit("v", "r")]),
            ("cat <<EOF\nbody $v:tasks\nEOF\necho done", vec![hit("v", "t")]),
            ("cat <<-EOF\n\tbody $v:u\n\tEOF", vec![hit("v", "u")]),
            ("cat <<'EOF'\n$v:t\nEOF\necho \"$w:t\"", vec![hit("w", "t")]),
            ("x=$(cat <<EOF\n$v:a\nEOF\n)", vec![hit("v", "a")]),
            (r#"echo a#"$v:t""#, vec![hit("v", "t")]),
            (r#"echo "$1:t $?:t $#:t $$:t $@:t $*:t $!:t $-:t $_:t""#, vec![
                hit("1", "t"), hit("?", "t"), hit("#", "t"), hit("$", "t"), hit("@", "t"),
                hit("*", "t"), hit("!", "t"), hit("-", "t"), hit("_", "t"),
            ]),
            (r#"echo "$a[1]:t""#, vec![hit("a[1]", "t")]),
            ("echo \"→ $v:t é\"", vec![hit("v", "t")]),
        ];
        for (command, want) in cases {
            assert_eq!(&params(command), want, "{command:?}");
        }
    }

    #[test]
    fn every_hit_is_braced_and_nothing_else_changes() {
        let t = check("echo \"→ $v:t é\" '$w:t' \"$1:a\"").unwrap();
        assert_eq!(t.corrected, "echo \"→ ${v}:t é\" '$w:t' \"${1}:a\"");
        let t = check(r#"echo "$a[1]:t $?:h""#).unwrap();
        assert_eq!(t.corrected, r#"echo "${a[1]}:t ${?}:h""#);
    }

    /// Anything the scanner cannot follow passes: refusing a command on a
    /// misreading would block work on the lint's own mistake.
    #[test]
    fn what_cannot_be_followed_passes() {
        for unparsed in [
            r#"echo "unterminated $v:t"#,
            "echo 'unterminated $v:t",
            "echo $(unclosed $v:t",
            "echo ${unclosed $v:t",
            "echo `unclosed $v:t",
            "cat <<",
        ] {
            assert_eq!(check(unparsed), None, "{unparsed:?}");
        }
    }

    #[test]
    fn the_refusal_says_it_is_not_a_gate_stop_and_carries_the_fix() {
        let command = r#"git show "$R:app/x""#;
        let text = refusal(command, &check(command).unwrap());
        assert!(text.starts_with("Not a Tool Gate stop"), "{text}");
        assert!(text.contains("Rewriting the command IS the fix"), "{text}");
        assert!(text.contains(r#"git show "${R}:app/x""#), "the corrected command, verbatim: {text}");
        assert!(text.contains("`${R:a}`"), "how to write the modifier if it was meant: {text}");
        assert!(text.contains("the absolute path"), "{text}");
    }

    #[test]
    fn a_long_script_gets_its_changed_lines_not_the_whole_body() {
        let filler = "echo filler line\n".repeat(120);
        let command = format!("{filler}git show \"$R:app/x\"\n{filler}");
        let text = refusal(&command, &check(&command).unwrap());
        assert!(text.contains("The lines to change"), "{text}");
        assert!(text.contains("→ git show \"${R}:app/x\""), "{text}");
        assert!(text.len() < 2000, "{} bytes", text.len());
    }

    #[test]
    fn zsh_trap_refuses_only_under_zsh() {
        let command = r#"git show "$R:app/x""#;
        assert!(zsh_trap("/bin/zsh", command).is_some_and(|r| r.starts_with("Not a Tool Gate stop")));
        assert_eq!(zsh_trap("/bin/bash", command), None, "literal under bash");
        assert_eq!(zsh_trap("", command), None, "an unknown shell is not zsh");
        assert_eq!(zsh_trap("/bin/zsh", "git show HEAD:app/x"), None);
    }

    #[test]
    fn the_shell_is_resolved_in_the_clis_order() {
        let yes = || true;
        let no = || false;
        assert!(agent_shell_is_zsh(Some("/bin/zsh"), Some("/bin/bash"), no));
        assert!(!agent_shell_is_zsh(Some("/bin/bash"), Some("/bin/zsh"), yes));
        assert!(agent_shell_is_zsh(None, Some("/bin/zsh"), no));
        assert!(!agent_shell_is_zsh(None, Some("/usr/local/bin/bash"), yes));
        // A fish login shell: the CLI falls back to detection, zsh first.
        assert!(agent_shell_is_zsh(None, Some("/opt/homebrew/bin/fish"), yes));
        assert!(!agent_shell_is_zsh(None, Some("/opt/homebrew/bin/fish"), no));
        assert!(agent_shell_is_zsh(Some(""), None, yes));
        // An override naming neither falls through to SHELL.
        assert!(!agent_shell_is_zsh(Some("/bin/fish"), Some("/bin/bash"), yes));
        assert!(is_zsh("/bin/zsh") && is_zsh("zsh") && !is_zsh("/bin/bash") && !is_zsh("sh"));
    }

    /// The table, measured again on the zsh installed here (skipped where
    /// there is none, e.g. a bare Linux runner). For every character after
    /// `$v:` — and after the `f`/`w`/`g` prefixes — the lint must flag exactly
    /// what zsh changes, except that `F`/`W` are deliberately over-flagged
    /// (module doc).
    #[test]
    fn the_table_agrees_with_the_installed_zsh() {
        let zsh = ["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh", "/opt/homebrew/bin/zsh"]
            .into_iter()
            .find(|p| Path::new(p).is_file());
        let Some(zsh) = zsh else {
            eprintln!("zsh not installed — the measured table is pinned by the unit tests only");
            return;
        };
        let mut chars: Vec<char> = ('a'..='z').chain('A'..='Z').chain('0'..='9').collect();
        chars.extend("&/._-:,@%^*+=~#?!".chars());
        let mut probes: Vec<String> = chars.iter().map(|c| c.to_string()).collect();
        for p in ['f', 'w', 'g'] {
            probes.extend(chars.iter().map(|c| format!("{p}{c}")));
        }
        // One zsh, one subshell per probe: an expansion error ("bad
        // substitution") ends only its own subshell.
        let mut script = String::from("v=/x/y.z\n");
        for m in &probes {
            script.push_str(&format!(
                "( print -r -- \"$v:{m}pp/q\" ) 2>/dev/null || print -r -- '<error>'\n"
            ));
        }
        let out = std::process::Command::new(zsh)
            .args(["-f", "-c", &script])
            .output()
            .expect("zsh runs");
        let lines: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
        assert_eq!(lines.len(), probes.len(), "one line per probe: {lines:?}");
        for (m, got) in probes.iter().zip(&lines) {
            let changed = got != &format!("/x/y.z:{m}pp/q");
            let flagged = check(&format!("print -r -- \"$v:{m}pp/q\"")).is_some();
            let lead = m.trim_start_matches(['f', 'w', 'g']);
            if lead.starts_with(['F', 'W']) && flagged && !changed {
                continue; // the deliberate over-flag
            }
            assert_eq!(flagged, changed, "`$v:{m}`: zsh printed {got:?}");
        }
    }
}
