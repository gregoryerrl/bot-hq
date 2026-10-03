//! What an OUTWARD command publishes — the content the reviewer must read
//! before it parks for the user.
//!
//! Two questions, answered from the command line alone, before anything runs:
//! - [`is_outward`]: does any simple command in it run `gh` or `curl` — the
//!   tools that publish under the user's identity? Quote-aware and
//!   wrapper-aware: `echo "gh issue"` is not outward; `GH_TOKEN=x gh …`,
//!   `env gh …`, `command gh …` and `bash -c "gh …"` are.
//! - [`extract`]: which files and inline strings does it publish? A body-flag
//!   table PER SUBCOMMAND (feedback #35: `-F` is `--body-file` to
//!   `gh issue create`, `--notes-file` to `gh release create` and `--field` to
//!   `gh api`), and a refusal — never a silent "content-free" — for every form
//!   whose content cannot be read ahead of the run: a body computed by the
//!   shell (`$VAR`, `$(…)`), read from stdin, written by the same command, or
//!   carried by a file-bearing flag the table does not know.
//!
//! A refusal here is the reviewer's guarantee, not a limitation to route
//! around: the alternative is a publish that parks for the user with its
//! content unread.
//!
//! One deliberate exception: a shell running a script bot-hq cannot read
//! (`./gen | bash`, `eval "$CMD"`) is OUTWARD but not refused — it routes to
//! review as content-free, and the reviewer and the user read the command
//! line itself. Refusing it would add no safety (the same script written to a
//! file and run as `bash file.sh` is not opaque) and would leave a gated,
//! non-publishing pipeline un-runnable even with the user's approval.

/// One shell word as the shell would pass it, plus whether any part of it is
/// computed at run time (`$VAR`, `$(…)`, `` `…` ``, `<(…)`) — a value bot-hq
/// cannot read ahead. `op` marks an unquoted redirection operator (`>`, `<<`,
/// `<<<`, …), kept as its own word so a caller can tell `> file` from an
/// argument.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Word {
    text: String,
    dynamic: bool,
    op: bool,
}

/// One simple command's words, the heredoc BODIES it declared (in order), and
/// whether its stdin is the previous segment's stdout (`a | b`).
#[derive(Debug, Clone, Default)]
struct Segment {
    words: Vec<Word>,
    heredocs: Vec<String>,
    piped_from_prev: bool,
}

/// Split `command` into simple-command segments the way a POSIX shell would:
/// `'…'` literal; `"…"` with `\` escaping `" \ $ \``; `\` outside quotes;
/// `;`, `&`, `|`, `&&`, `||`, `(`, `)` and newlines end a segment; `#` at a
/// word start comments to end of line. Heredoc bodies (`<<DELIM` … `DELIM`)
/// are CAPTURED on the segment that declared them, never parsed as commands
/// here — whether they are a script depends on who reads them (see
/// [`simple_commands`]). `$(…)`, `${…}`, backticks and `<(…)`/`>(…)` are
/// consumed whole and mark the word dynamic.
fn segments(command: &str) -> Vec<Segment> {
    let chars: Vec<char> = command.chars().collect();
    let mut segs: Vec<Segment> = Vec::new();
    let mut seg = Segment::default();
    let mut cur = String::new();
    let mut dynamic = false;
    let mut started = false;
    // (delimiter, strip leading tabs, index of the declaring segment)
    let mut pending_heredocs: Vec<(String, bool, usize)> = Vec::new();
    let mut next_piped = false;
    let mut i = 0;

    fn finish(cur: &mut String, dynamic: &mut bool, started: &mut bool, seg: &mut Segment) {
        if *started {
            seg.words.push(Word { text: std::mem::take(cur), dynamic: *dynamic, op: false });
        }
        *dynamic = false;
        *started = false;
    }
    // Consume a balanced `open … close` span starting AT `open` into `cur`.
    fn consume_balanced(chars: &[char], mut i: usize, cur: &mut String, open: char, close: char) -> usize {
        let mut depth = 0usize;
        while i < chars.len() {
            let c = chars[i];
            cur.push(c);
            i += 1;
            if c == open {
                depth += 1;
            } else if c == close {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
        }
        i
    }
    // Consume a `$(…)` / `${…}` / `` `…` `` span starting at `i` (pointing at
    // `$` or a backtick) into `cur`; returns the index after it.
    fn consume_expansion(chars: &[char], mut i: usize, cur: &mut String) -> usize {
        if chars[i] == '`' {
            cur.push('`');
            i += 1;
            while i < chars.len() && chars[i] != '`' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    cur.push(chars[i]);
                    i += 1;
                }
                cur.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                cur.push('`');
                i += 1;
            }
            return i;
        }
        cur.push('$');
        i += 1;
        match chars.get(i) {
            Some('(') => consume_balanced(chars, i, cur, '(', ')'),
            Some('{') => consume_balanced(chars, i, cur, '{', '}'),
            _ => i, // `$NAME` — the name reads as ordinary word chars
        }
    }
    let end_segment = |seg: &mut Segment, segs: &mut Vec<Segment>, piped_next: bool, next_piped: &mut bool| {
        let mut done = std::mem::take(seg);
        done.piped_from_prev = *next_piped;
        segs.push(done);
        *next_piped = piped_next;
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => {
                finish(&mut cur, &mut dynamic, &mut started, &mut seg);
                i += 1;
            }
            '\n' => {
                finish(&mut cur, &mut dynamic, &mut started, &mut seg);
                end_segment(&mut seg, &mut segs, false, &mut next_piped);
                i += 1;
                // Heredoc bodies start on the line after their operator; each
                // is captured onto the segment that declared it.
                for (delim, strip_tabs, owner) in std::mem::take(&mut pending_heredocs) {
                    let mut body = String::new();
                    loop {
                        let start = i;
                        while i < chars.len() && chars[i] != '\n' {
                            i += 1;
                        }
                        let raw: String = chars[start..i].iter().collect();
                        let at_end = i >= chars.len();
                        if !at_end {
                            i += 1;
                        }
                        let line = if strip_tabs { raw.trim_start_matches('\t').to_string() } else { raw };
                        if line == delim {
                            break;
                        }
                        body.push_str(&line);
                        body.push('\n');
                        if at_end {
                            break;
                        }
                    }
                    if let Some(s) = segs.get_mut(owner) {
                        s.heredocs.push(body);
                    }
                }
            }
            ';' | '&' | '|' | '(' | ')' => {
                finish(&mut cur, &mut dynamic, &mut started, &mut seg);
                // A single `|` (or `|&`) pipes into the next segment; `||`,
                // `&&`, `;`, `&` and parens do not.
                let pipe = c == '|' && chars.get(i + 1) != Some(&'|');
                end_segment(&mut seg, &mut segs, pipe, &mut next_piped);
                i += 1;
                if c == '&' || c == '|' {
                    while i < chars.len() && matches!(chars[i], '&' | '|') {
                        i += 1;
                    }
                }
            }
            '<' | '>' if chars.get(i + 1) == Some(&'(') => {
                // Process substitution `<(…)` / `>(…)`: a command whose output
                // becomes a path — dynamic, and recursed into by the caller.
                finish(&mut cur, &mut dynamic, &mut started, &mut seg);
                started = true;
                dynamic = true;
                cur.push(c);
                i = consume_balanced(&chars, i + 1, &mut cur, '(', ')');
            }
            '<' | '>' => {
                // A leading fd number (`2>`) was a separate word already; the
                // operator itself becomes an `op` word.
                finish(&mut cur, &mut dynamic, &mut started, &mut seg);
                let mut op = String::new();
                op.push(c);
                i += 1;
                while i < chars.len() && matches!(chars[i], '<' | '>' | '|') {
                    op.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() && chars[i] == '&' {
                    // `>&2`, `<&0`: an fd dup, not a segment separator.
                    op.push('&');
                    i += 1;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '-') {
                        op.push(chars[i]);
                        i += 1;
                    }
                }
                let heredoc = op.starts_with("<<") && !op.starts_with("<<<");
                let strip_tabs = heredoc && i < chars.len() && chars[i] == '-';
                if strip_tabs {
                    op.push('-');
                    i += 1;
                }
                seg.words.push(Word { text: op, dynamic: false, op: true });
                if heredoc {
                    // The delimiter word follows (quotes around it are removed).
                    while i < chars.len() && matches!(chars[i], ' ' | '\t') {
                        i += 1;
                    }
                    let mut delim = String::new();
                    while i < chars.len() && !matches!(chars[i], ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' | '(' | ')') {
                        if !matches!(chars[i], '\'' | '"' | '\\') {
                            delim.push(chars[i]);
                        }
                        i += 1;
                    }
                    seg.words.push(Word { text: delim.clone(), dynamic: false, op: false });
                    pending_heredocs.push((delim, strip_tabs, segs.len()));
                }
            }
            '#' if !started => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\\' => {
                started = true;
                if let Some(&next) = chars.get(i + 1) {
                    if next != '\n' {
                        cur.push(next);
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            '\'' => {
                started = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                started = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    match chars[i] {
                        '\\' if matches!(chars.get(i + 1), Some('"' | '\\' | '$' | '`' | '\n')) => {
                            if chars[i + 1] != '\n' {
                                cur.push(chars[i + 1]);
                            }
                            i += 2;
                        }
                        '$' | '`' => {
                            dynamic = true;
                            i = consume_expansion(&chars, i, &mut cur);
                        }
                        other => {
                            cur.push(other);
                            i += 1;
                        }
                    }
                }
                i += 1;
            }
            '$' | '`' => {
                started = true;
                dynamic = true;
                i = consume_expansion(&chars, i, &mut cur);
            }
            other => {
                started = true;
                cur.push(other);
                i += 1;
            }
        }
    }
    finish(&mut cur, &mut dynamic, &mut started, &mut seg);
    end_segment(&mut seg, &mut segs, false, &mut next_piped);
    segs
}

/// The command texts inside a dynamic word's substitutions — `$(…)`,
/// backticks, `<(…)`, `>(…)` — so `URL=$(gh pr create …)` is analysed like
/// the `gh` it runs. `${…}` is a parameter, not a command.
fn substitution_bodies(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let paren_open = match chars[i] {
            '$' | '<' | '>' => chars.get(i + 1) == Some(&'('),
            _ => false,
        };
        if paren_open {
            let mut depth = 0usize;
            let mut j = i + 1;
            let start = j + 1;
            while j < chars.len() {
                match chars[j] {
                    '(' => depth += 1,
                    ')' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            out.push(chars[start.min(j)..j.min(chars.len())].iter().collect());
            i = j + 1;
        } else if chars[i] == '`' {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && chars[j] != '`' {
                j += 1;
            }
            out.push(chars[start..j.min(chars.len())].iter().collect());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Words that only precede the real command: shell keywords and grouping
/// (`!`, `{`, `then`, `do`, …), `NAME=value` assignments, and the transparent
/// wrappers `env`, `command`, `exec`, `builtin`, `nohup`, `time`, `sudo` with
/// their flags. Other wrappers (`timeout 30 gh`, `xargs gh`, `nice gh`,
/// `find -exec gh`) are caught by [`simple_commands`]' any-position rule.
fn skips_to_command(words: &[Word]) -> usize {
    let mut i = 0;
    while i < words.len() {
        let w = &words[i];
        if w.op {
            // `> out gh …` is legal shell; skip the operator and its target.
            i += 2;
            continue;
        }
        let t = w.text.as_str();
        if matches!(
            t,
            "!" | "{" | "}" | "if" | "then" | "else" | "elif" | "fi" | "do" | "done" | "while"
                | "until" | "coproc"
        ) {
            i += 1;
            continue;
        }
        let is_assignment = t
            .split_once('=')
            .is_some_and(|(name, _)| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        if is_assignment {
            i += 1;
            continue;
        }
        // The transparent wrappers, each with ITS OWN value-taking flags
        // (`time -p` and `command -p` are booleans; `sudo -p PROMPT` and
        // `exec -a NAME` take values) — one shared list would swallow the
        // command word as a flag value (EYES 7cc26d55).
        let value_flags: Option<&[&str]> = match t {
            "command" | "builtin" | "nohup" | "time" => Some(&[]),
            "exec" => Some(&["-a"]),
            "env" => Some(&["-u", "-C", "-S"]),
            "sudo" => Some(&["-u", "-g", "-p", "-C", "-D", "-h", "-r", "-t", "-U", "-T"]),
            _ => None,
        };
        if let Some(value_flags) = value_flags {
            i += 1;
            while i < words.len() && words[i].text.starts_with('-') && !words[i].op {
                let takes_value = value_flags.contains(&words[i].text.as_str());
                i += if takes_value { 2 } else { 1 };
            }
            continue;
        }
        break;
    }
    i
}

/// The basename of a command word: `/opt/homebrew/bin/gh` → `gh`.
fn basename(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

const OUTWARD_TOOLS: &[&str] = &["gh", "curl"];
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];
const MAX_NESTING: usize = 4;

/// One simple command found in a command line. `opaque` marks a shell that
/// runs a script bot-hq cannot read (a computed `-c` string, or stdin piped
/// from something other than a literal `echo`/`printf`/heredoc) — it might
/// publish anything, so it counts as outward.
#[derive(Debug, Clone)]
struct Cmd {
    tool: String,
    args: Vec<Word>,
    opaque: bool,
    /// The segment's words before this command's own (assignments like
    /// `PGHOST=…`, wrappers) — where a data-read entry's host may sit.
    prefix: Vec<String>,
}

/// Every simple command `command` runs, as `(tool, args)`, looking through:
/// command substitutions in ANY word (`URL=$(gh pr create …)`); grouping and
/// keywords; every word naming an outward tool or a shell at ANY position —
/// so `timeout 30 gh …`, `xargs -I{} gh …`, `find … -exec gh … \;` and
/// unknown wrappers are caught without a wrapper table (a command's args end
/// where the next such word starts); `sh -c "…"`, `eval …` and
/// `ssh host '…'` strings; and a shell's stdin script — its own heredoc or
/// herestring, or a literal `cat <<EOF` / `echo` / `printf` piped into it.
fn simple_commands(command: &str, depth: usize) -> Vec<Cmd> {
    let segs = segments(command);
    let mut out = Vec::new();
    for (idx, seg) in segs.iter().enumerate() {
        let words = &seg.words;
        if words.is_empty() {
            continue;
        }
        // Substitutions run first, wherever they sit — even in an assignment.
        if depth < MAX_NESTING {
            for w in words.iter().filter(|w| w.dynamic) {
                for inner in substitution_bodies(&w.text) {
                    out.extend(simple_commands(&inner, depth + 1));
                }
            }
        }
        // Where commands start: the primary command, plus every later word
        // that names an outward tool, a shell, `eval` or `ssh`.
        let primary = skips_to_command(words);
        let mut starts: Vec<usize> = Vec::new();
        if primary < words.len() && !words[primary].op {
            starts.push(primary);
        }
        // Every word, not only those after `primary`: a mis-read wrapper flag
        // must not hide the command word it swallowed (EYES 7cc26d55).
        for (k, w) in words.iter().enumerate() {
            if w.op || k == primary || (k > 0 && words[k - 1].op) {
                continue;
            }
            let b = basename(&w.text);
            if OUTWARD_TOOLS.contains(&b) || SHELLS.contains(&b) || matches!(b, "eval" | "ssh") {
                starts.push(k);
            }
        }
        starts.sort_unstable();
        starts.dedup();
        for (n, &k) in starts.iter().enumerate() {
            let end = starts.get(n + 1).copied().unwrap_or(words.len());
            let tool = basename(&words[k].text).to_string();
            let args: Vec<Word> = words[k + 1..end].to_vec();
            let mut opaque = false;
            if depth < MAX_NESTING {
                match tool.as_str() {
                    t if SHELLS.contains(&t) => {
                        match shell_script(&args, seg, idx, &segs) {
                            ShellScript::Text(script) => out.extend(simple_commands(&script, depth + 1)),
                            ShellScript::Opaque => opaque = true,
                            ShellScript::None => {}
                        }
                    }
                    "eval" => {
                        if args.iter().any(|w| w.dynamic) {
                            opaque = true;
                        } else {
                            let s = args.iter().filter(|w| !w.op).map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                            out.extend(simple_commands(&s, depth + 1));
                        }
                    }
                    "ssh" => {
                        // `ssh [opts] host [command…]` — the remote command.
                        let rest: Vec<&Word> = args.iter().filter(|w| !w.op && !w.text.starts_with('-')).skip(1).collect();
                        if rest.iter().any(|w| w.dynamic) {
                            opaque = true;
                        } else if !rest.is_empty() {
                            let s = rest.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                            out.extend(simple_commands(&s, depth + 1));
                        }
                    }
                    _ => {}
                }
            }
            let prefix = words[..k].iter().filter(|w| !w.op).map(|w| w.text.clone()).collect();
            out.push(Cmd { tool, args, opaque, prefix });
        }
    }
    out
}

enum ShellScript {
    /// The script is readable: parse it as commands.
    Text(String),
    /// The shell runs a script bot-hq cannot read.
    Opaque,
    /// No script from the command line (a script FILE, or an interactive shell).
    None,
}

/// What script a shell invocation runs, from its args and stdin: `-c STRING`;
/// else, with no script-file operand, stdin — a herestring, its own heredoc,
/// or a literal producer piped into it.
fn shell_script(args: &[Word], seg: &Segment, idx: usize, segs: &[Segment]) -> ShellScript {
    let mut i = 0;
    let mut script_file = false;
    let mut herestring: Option<&Word> = None;
    while i < args.len() {
        let w = &args[i];
        if w.op {
            if w.text == "<<<" {
                herestring = args.get(i + 1);
            }
            i += 2;
            continue;
        }
        let t = w.text.as_str();
        if t.starts_with('-') && !t.starts_with("--") && t.len() > 1 && t.contains('c') {
            return match args.get(i + 1) {
                Some(s) if s.dynamic => ShellScript::Opaque,
                Some(s) => ShellScript::Text(s.text.clone()),
                None => ShellScript::None,
            };
        }
        // Options that take a VALUE: `-o pipefail`, `-O extglob`, `+o …`,
        // `-eo pipefail` (a cluster ending in o/O), `--rcfile f` — the value
        // is not a script file (EYES 7cc26d55).
        let cluster_takes_value = (t.starts_with('-') || t.starts_with('+'))
            && !t.starts_with("--")
            && t.len() > 1
            && (t.ends_with('o') || t.ends_with('O'));
        if cluster_takes_value || matches!(t, "--rcfile" | "--init-file") {
            i += 2;
            continue;
        }
        if !t.starts_with('-') && !t.starts_with('+') {
            script_file = true;
            break;
        }
        i += 1;
    }
    if script_file {
        return ShellScript::None;
    }
    if let Some(h) = herestring {
        return if h.dynamic { ShellScript::Opaque } else { ShellScript::Text(h.text.clone()) };
    }
    if !seg.heredocs.is_empty() {
        return ShellScript::Text(seg.heredocs.join("\n"));
    }
    if seg.piped_from_prev {
        let Some(prev) = idx.checked_sub(1).and_then(|p| segs.get(p)) else {
            return ShellScript::Opaque;
        };
        let start = skips_to_command(&prev.words);
        let producer = prev.words.get(start).map(|w| basename(&w.text)).unwrap_or("");
        let rest: Vec<&Word> = prev.words.iter().skip(start + 1).filter(|w| !w.op).collect();
        return match producer {
            "cat" if !prev.heredocs.is_empty() => ShellScript::Text(prev.heredocs.join("\n")),
            "echo" | "printf" if rest.iter().all(|w| !w.dynamic) => ShellScript::Text(
                rest.iter()
                    .filter(|w| !(producer == "echo" && w.text.starts_with('-')))
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => ShellScript::Opaque,
        };
    }
    ShellScript::None
}

/// OUTWARD classifier: does the command line run `gh` or `curl` anywhere —
/// or a shell whose script cannot be read (see [`Cmd::opaque`])? `git push`
/// is deliberately absent: the pre-push hook owns it end to end.
pub(crate) fn is_outward(command: &str) -> bool {
    simple_commands(command, 0)
        .iter()
        .any(|c| c.opaque || OUTWARD_TOOLS.contains(&c.tool.as_str()))
}

/// The content an outward command publishes: body FILES (paths as written,
/// resolved by the caller) and INLINE strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OutwardBodies {
    pub files: Vec<String>,
    pub inline: Vec<String>,
}

impl OutwardBodies {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty() && self.inline.is_empty()
    }
}

/// How one VALUE-TAKING flag's value is published. A flag absent from a
/// table is a boolean: it never consumes the next word (gh's `-c` takes a
/// value on `issue close` and none on `pr review` — one shared table would
/// swallow the `-b` that follows it).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Carries {
    /// The value is a path whose CONTENT publishes (`--body-file`).
    File,
    /// The value itself publishes (`--body`, `--title`).
    Inline,
    /// `gh api -F k=v`: `v` inline, or `@path` → a file.
    FieldTyped,
    /// `gh api -f k=v`: `v` inline, literally (no `@` expansion).
    FieldRaw,
    /// `curl -d v`: `v` inline, or `@path` → a file.
    CurlData,
    /// `curl --data-raw v`: `v` inline, literally.
    CurlRaw,
    /// `curl --data-urlencode v`: `content`, `=content`, `name=content`
    /// inline; `@file` / `name@file` → a file.
    CurlUrlencode,
    /// `curl -F k=v`: `v` inline, or `@path` / `<path` → a file.
    CurlForm,
    /// Takes a value that is not published (`--repo`, `-H`, `--jq`).
    Skip,
}

type FlagTable = (Vec<(&'static str, Carries)>, Vec<(&'static str, &'static str)>);

const FILL: &str = "publishes commit messages the reviewer has not read — pass --title and --body-file instead";
const TEMPLATE: &str = "publishes a template body the reviewer has not read — pass --body-file instead";

/// The value-taking flags of one `gh <group> <sub>` / `curl` invocation, and
/// the flags whose published content cannot be read up front (refused, with
/// the reason). Checked against gh 2.x's flag sets per subcommand.
fn flag_table(tool: &str, group: &str, sub: &str) -> FlagTable {
    use Carries::*;
    let repo = [("--repo", Skip), ("-R", Skip)];
    let body = [("--body", Inline), ("-b", Inline), ("--body-file", File), ("-F", File)];
    let mut flags: Vec<(&'static str, Carries)> = repo.to_vec();
    let mut refused: Vec<(&'static str, &'static str)> = Vec::new();
    match (tool, group, sub) {
        ("gh", "issue", "create") | ("gh", "pr", "create") => {
            flags.extend(body);
            flags.extend([
                ("--title", Inline), ("-t", Inline), ("--recover", File),
                ("--assignee", Skip), ("-a", Skip), ("--label", Skip), ("-l", Skip),
                ("--milestone", Skip), ("-m", Skip), ("--project", Skip), ("-p", Skip),
                ("--base", Skip), ("-B", Skip), ("--head", Skip), ("-H", Skip),
                ("--reviewer", Skip), ("-r", Skip),
            ]);
            refused.extend([("--template", TEMPLATE), ("-T", TEMPLATE)]);
            if group == "pr" {
                refused.extend([("--fill", FILL), ("--fill-first", FILL), ("--fill-verbose", FILL), ("-f", FILL)]);
            }
        }
        ("gh", "issue" | "pr", "edit") => {
            flags.extend(body);
            flags.extend([
                ("--title", Inline), ("-t", Inline), ("--milestone", Skip), ("-m", Skip),
                ("--base", Skip), ("-B", Skip),
                ("--add-assignee", Skip), ("--remove-assignee", Skip), ("--add-label", Skip),
                ("--remove-label", Skip), ("--add-project", Skip), ("--remove-project", Skip),
                ("--add-reviewer", Skip), ("--remove-reviewer", Skip),
            ]);
        }
        ("gh", "issue" | "pr", "comment") | ("gh", "pr", "review") => flags.extend(body),
        ("gh", "pr", "merge") => {
            flags.extend(body);
            flags.extend([
                ("--subject", Inline), ("-t", Inline), ("--author-email", Skip), ("-A", Skip),
                ("--match-head-commit", Skip),
            ]);
        }
        ("gh", "issue" | "pr", "close" | "reopen") => {
            flags.extend([("--comment", Inline), ("-c", Inline), ("--reason", Skip), ("-r", Skip)]);
        }
        ("gh", "release", "create" | "edit") => {
            flags.extend([
                ("--notes", Inline), ("-n", Inline), ("--notes-file", File), ("-F", File),
                ("--title", Inline), ("-t", Inline), ("--target", Skip), ("--tag", Skip),
                ("--discussion-category", Skip), ("--notes-start-tag", Skip),
            ]);
            refused.push(("--notes-from-tag", "publishes a tag message the reviewer has not read — pass --notes-file instead"));
        }
        ("gh", "api", _) => {
            flags.extend([
                ("--input", File), ("-F", FieldTyped), ("--field", FieldTyped),
                ("-f", FieldRaw), ("--raw-field", FieldRaw),
                ("-X", Skip), ("--method", Skip), ("-H", Skip), ("--header", Skip),
                ("-q", Skip), ("--jq", Skip), ("-t", Skip), ("--template", Skip),
                ("-p", Skip), ("--preview", Skip), ("--hostname", Skip), ("--cache", Skip),
            ]);
        }
        ("gh", "gist", _) => {
            flags.extend([
                ("--desc", Inline), ("-d", Inline), ("--add", File), ("-a", File),
                ("--filename", Skip), ("-f", Skip), ("--remove", Skip), ("-r", Skip),
            ]);
        }
        ("curl", _, _) => {
            flags = vec![
                ("-d", CurlData), ("--data", CurlData), ("--data-ascii", CurlData),
                ("--data-binary", CurlData), ("--data-urlencode", CurlUrlencode), ("--json", CurlData),
                ("--data-raw", CurlRaw), ("--form-string", CurlRaw),
                ("-F", CurlForm), ("--form", CurlForm), ("-T", File), ("--upload-file", File),
                ("-b", Skip), ("--cookie", Skip), ("-c", Skip), ("--cookie-jar", Skip),
                ("-H", Skip), ("--header", Skip), ("-X", Skip), ("--request", Skip),
                ("-u", Skip), ("--user", Skip), ("-o", Skip), ("--output", Skip),
                ("-A", Skip), ("--user-agent", Skip), ("-e", Skip), ("--referer", Skip),
                ("-m", Skip), ("--max-time", Skip), ("-w", Skip), ("--write-out", Skip),
                ("-K", Skip), ("--config", Skip), ("-x", Skip), ("--proxy", Skip),
            ];
        }
        _ => {}
    }
    (flags, refused)
}

/// A value the reviewer cannot read ahead: stdin.
fn is_stdin(path: &str) -> bool {
    matches!(path, "-" | "/dev/stdin" | "/dev/fd/0" | "/proc/self/fd/0")
}

/// Tools that only READ the files they name — a body file one of them
/// mentions in the same command line is not being rewritten under review.
const READ_ONLY_TOOLS: &[&str] = &[
    "cat", "wc", "head", "tail", "grep", "rg", "ls", "stat", "md5", "md5sum", "shasum",
    "sha256sum", "diff", "test", "[", "file", "echo", "printf", "true", "less", "bat",
];

/// The content every outward simple command in `command` publishes, or the
/// reason it cannot be known before the run (the caller refuses the park).
pub(crate) fn extract(command: &str) -> Result<OutwardBodies, String> {
    let commands = simple_commands(command, 0);
    let mut out = OutwardBodies::default();
    // Paths a NON-outward simple command may write: every redirection target,
    // and every argument of a tool not known to be read-only. Shell wrappers
    // (`bash -c "…"`, `eval`) are skipped — their inner commands are analysed
    // on their own.
    let mut written: Vec<String> = Vec::new();
    for Cmd { tool, args, .. } in &commands {
        let mut i = 0;
        while i < args.len() {
            if args[i].op && args[i].text.starts_with('>') {
                if let Some(target) = args.get(i + 1) {
                    written.push(target.text.clone());
                }
            }
            i += 1;
        }
        if matches!(tool.as_str(), "gh" | "curl") {
            extract_one(tool, args, &mut out)?;
            continue;
        }
        if SHELLS.contains(&tool.as_str())
            || matches!(tool.as_str(), "eval" | "ssh")
            || READ_ONLY_TOOLS.contains(&tool.as_str())
        {
            continue;
        }
        written.extend(args.iter().filter(|w| !w.op).map(|w| w.text.clone()));
    }
    // A body file this same command line writes (`cat > b.md <<EOF … && gh
    // … --body-file b.md`): the file on disk NOW is not what publishes.
    for file in &out.files {
        let name = basename(file);
        let same = |w: &String| {
            w == file || w == name || (!name.is_empty() && w.ends_with(&format!("/{name}")))
        };
        if written.iter().any(same) {
            return Err(format!(
                "the body file `{file}` is written by this same command, so what publishes is \
                 not what is on disk now — write the file in one call, then publish in another"
            ));
        }
    }
    Ok(out)
}

fn extract_one(tool: &str, args: &[Word], out: &mut OutwardBodies) -> Result<(), String> {
    // `gh <group> <sub>`: the first two positionals.
    let (group, sub) = if tool == "gh" {
        let mut pos = args.iter().filter(|w| !w.op && !w.text.starts_with('-'));
        (
            pos.next().map(|w| w.text.as_str()).unwrap_or(""),
            pos.next().map(|w| w.text.as_str()).unwrap_or(""),
        )
    } else {
        ("", "")
    };
    let (table, refused) = flag_table(tool, group, sub);
    let gist_create = tool == "gh" && group == "gist" && sub == "create";
    let takes_value = |flag: &str| table.iter().find(|(f, _)| *f == flag).map(|(_, c)| *c);

    let mut i = 0;
    let mut positional_index = 0usize;
    let mut only_positionals = false;
    while i < args.len() {
        let w = &args[i];
        if w.op {
            i += 2; // a redirection and its target
            continue;
        }
        let text = w.text.as_str();
        if only_positionals || !text.starts_with('-') || text == "-" {
            // For `gh gist create`, every positional after the subcommand is a
            // FILE that publishes.
            if gist_create && positional_index >= 2 {
                if w.dynamic {
                    return Err(dynamic_refusal("the gist file"));
                }
                if is_stdin(text) {
                    return Err(stdin_refusal("gh gist create -"));
                }
                out.files.push(text.to_string());
            }
            positional_index += 1;
            i += 1;
            continue;
        }
        if text == "--" {
            only_positionals = true;
            i += 1;
            continue;
        }
        // Resolve the word to (flag, attached value) — `--flag=value`, a long
        // flag alone, or a short-flag CLUSTER (`-sSd @f`, `-d@f`, `-XPOST`):
        // booleans in a cluster pass, and the first value-taking flag takes
        // the rest of the cluster or, when nothing is left, the next word.
        let resolved: Option<(String, Option<String>)> = if let Some(long) = text.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((n, v)) => (format!("--{n}"), Some(v.to_string())),
                None => (text.to_string(), None),
            };
            if let Some((_, why)) = refused.iter().find(|(f, _)| *f == name) {
                return Err(format!("`{name}` {why}"));
            }
            if takes_value(&name).is_some() {
                Some((name, attached))
            } else if name.ends_with("-file") || name == "--input" {
                return Err(format!(
                    "`{name}` carries a file this check does not know how to read — publish \
                     with --body-file / --notes-file so the reviewer can read it first"
                ));
            } else {
                None // a boolean long flag
            }
        } else {
            let cluster: Vec<char> = text[1..].chars().collect();
            let mut hit = None;
            for (k, c) in cluster.iter().enumerate() {
                let flag = format!("-{c}");
                if let Some((_, why)) = refused.iter().find(|(f, _)| *f == flag) {
                    return Err(format!("`{flag}` {why}"));
                }
                if takes_value(&flag).is_some() {
                    let rest: String = cluster[k + 1..].iter().collect();
                    hit = Some((flag, (!rest.is_empty()).then_some(rest)));
                    break;
                }
            }
            hit
        };
        let Some((flag, attached)) = resolved else {
            i += 1;
            continue;
        };
        let carries = takes_value(&flag).unwrap_or(Carries::Skip);
        let (value, dynamic, consumed) = match attached {
            Some(v) => (v, w.dynamic, 1),
            None => match args.get(i + 1) {
                Some(next) if !next.op => (next.text.clone(), next.dynamic, 2),
                _ => (String::new(), false, 1),
            },
        };
        i += consumed;
        if carries == Carries::Skip {
            continue;
        }
        if dynamic {
            return Err(dynamic_refusal(&format!("`{flag}`")));
        }
        publish_value(&flag, carries, value, out)?;
    }
    Ok(())
}

fn dynamic_refusal(what: &str) -> String {
    format!(
        "the value of {what} is computed by the shell at run time ($VAR / $(…) / backticks), \
         so the reviewer cannot read it first — write the content to a file and pass it with a \
         file flag"
    )
}

fn stdin_refusal(what: &str) -> String {
    format!(
        "`{what}` reads the body from stdin, which the reviewer cannot read first — write it to \
         a file and pass the path"
    )
}

/// Record what one content-carrying flag value publishes.
fn publish_value(flag: &str, carries: Carries, value: String, out: &mut OutwardBodies) -> Result<(), String> {
    let field_value = |v: String| v.split_once('=').map(|(_, v)| v.to_string()).unwrap_or(v);
    match carries {
        Carries::File => {
            if is_stdin(&value) {
                return Err(stdin_refusal(&format!("{flag} {value}")));
            }
            out.files.push(value);
        }
        Carries::Inline | Carries::CurlRaw => out.inline.push(value),
        Carries::FieldRaw => out.inline.push(field_value(value)),
        Carries::FieldTyped => {
            let v = field_value(value);
            match v.strip_prefix('@') {
                Some(path) if is_stdin(path) => return Err(stdin_refusal(&format!("{flag} …=@{path}"))),
                Some(path) => out.files.push(path.to_string()),
                None => out.inline.push(v),
            }
        }
        Carries::CurlData => match value.strip_prefix('@') {
            Some(path) if is_stdin(path) => return Err(stdin_refusal(&format!("{flag} @{path}"))),
            Some(path) => out.files.push(path.to_string()),
            None => out.inline.push(value),
        },
        Carries::CurlUrlencode => {
            // curl's rule: an `@` before any `=` means `[name]@filename`.
            let at = value.find('@');
            let eq = value.find('=');
            match at {
                Some(a) if eq.is_none_or(|e| a < e) => {
                    let path = &value[a + 1..];
                    if is_stdin(path) {
                        return Err(stdin_refusal(&format!("{flag} @{path}")));
                    }
                    out.files.push(path.to_string());
                }
                _ => out.inline.push(match eq {
                    Some(e) => value[e + 1..].to_string(),
                    None => value,
                }),
            }
        }
        Carries::CurlForm => {
            let v = field_value(value);
            match v.strip_prefix('@').or_else(|| v.strip_prefix('<')) {
                Some(p) => {
                    // `@file;type=…` — the path ends at the first `;`.
                    let p = p.split(';').next().unwrap_or(p);
                    if is_stdin(p) {
                        return Err(stdin_refusal(&format!("{flag} …=@{p}")));
                    }
                    out.files.push(p.to_string());
                }
                None => out.inline.push(v),
            }
        }
        Carries::Skip => {}
    }
    Ok(())
}

/// One command a line runs, as the shell would run it: the tool's basename,
/// its argument words, and the segment's words before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunCommand {
    pub tool: String,
    pub args: Vec<String>,
    pub prefix: Vec<String>,
}

/// Every command `command` runs — through quotes, wrappers, `sh -c` and
/// `$(…)`, as [`simple_commands`] finds them — for the project's data-read
/// lists (`policy::data_reads`, group K): matched where a command RUNS,
/// never on text inside a quoted argument.
pub(crate) fn commands_run(command: &str) -> Vec<RunCommand> {
    simple_commands(command, 0)
        .into_iter()
        .map(|c| RunCommand {
            tool: c.tool,
            args: c.args.iter().filter(|w| !w.op).map(|w| w.text.clone()).collect(),
            prefix: c.prefix,
        })
        .collect()
}

/// The filters `read_gate` lets a read pipe into: none of them writes a file
/// or runs a command. `sort -o FILE` and `uniq IN OUT` write, so neither is
/// here (EYES, s-3158eb35).
const READ_FILTERS: &[&str] = &["head", "tail", "jq", "grep", "wc", "cut"];

/// Whether `command` is ONE simple command, optionally piped into
/// [`READ_FILTERS`] — the only shape the reviewer's `read_gate` takes (group
/// K, EYES: the lists find a listed command INSIDE a longer line, so
/// `gcloud logging read x; rm -rf ~/d` would otherwise reach the user's
/// Approve as "the reviewer's read"). No `;`, `&&`, `||`, `&`, redirection,
/// heredoc, value the shell computes, wrapper or shell. `Err` says which.
pub(crate) fn single_read_command(command: &str) -> Result<(), String> {
    if command.contains('\n') {
        return Err("more than one line".to_string());
    }
    let segs = segments(command);
    let Some(first) = segs.first() else {
        return Err("no command".to_string());
    };
    for (i, seg) in segs.iter().enumerate() {
        let Some(head) = seg.words.first() else {
            return Err("more than one command (`;`, `&&`, `||` or `&`)".to_string());
        };
        if !seg.heredocs.is_empty() {
            return Err("a heredoc".to_string());
        }
        if seg.words.iter().any(|w| w.op) {
            return Err("a redirection".to_string());
        }
        if seg.words.iter().any(|w| w.dynamic) {
            return Err("a value the shell computes (`$VAR`, `$(…)`, backticks)".to_string());
        }
        if i > 0 {
            if !seg.piped_from_prev {
                return Err("more than one command (`;`, `&&`, `||` or `&`)".to_string());
            }
            let tool = basename(&head.text);
            if !READ_FILTERS.contains(&tool) {
                return Err(format!(
                    "a pipe into `{tool}`, which is not one of the read-only filters (head, tail, \
                     jq, grep, wc, cut)"
                ));
            }
        }
    }
    let lead = &first.words[0].text;
    let tool = basename(lead);
    if lead.contains('=')
        || SHELLS.contains(&tool)
        || matches!(
            tool,
            "eval" | "ssh" | "sudo" | "env" | "command" | "exec" | "builtin" | "xargs" | "time"
                | "nohup" | "timeout" | "nice" | "watch"
        )
    {
        return Err("a wrapper, an assignment or a shell in front of the command".to_string());
    }
    Ok(())
}

/// Verbs that make a command a WRITE, for the reviewer's `read_gate` (EYES:
/// one list both gates the executor and limits the reviewer, and a broad
/// entry like `gcloud run` or `bq` also covers `gcloud run deploy` and
/// `bq rm`). Not exhaustive — cheap.
const WRITE_VERBS: &[&str] = &[
    "delete", "deploy", "rm", "remove", "update", "create", "set", "insert", "drop", "truncate",
    "alter", "apply", "patch", "put", "write", "cp", "mv", "add", "destroy", "import", "replace",
    "cancel", "execute", "restart", "stop", "start", "kill", "enable", "disable", "grant", "revoke",
    "upload", "rollback", "migrate", "scale", "resize", "reset", "undelete", "purge", "prune",
    // EYES `a2c4c8df`: `gcloud scheduler jobs pause`, `bq load`/`mk`/`extract`,
    // `gcloud storage objects compose`. A bare `run` cannot be one: `gcloud
    // run` is a command group.
    "pause", "resume", "load", "mk", "extract", "compose", "trigger", "invoke", "publish", "submit",
    "send",
];

/// SQL that writes, as the first keyword of an argument that is a statement.
const SQL_WRITES: &[&str] = &[
    "insert", "update", "delete", "drop", "alter", "truncate", "create", "grant", "revoke", "merge",
    "copy", "call", "vacuum", "reindex", "cluster", "refresh",
];

/// The first write word in `command`'s first command, if any: a plain
/// argument word holding one of [`WRITE_VERBS`] (`gcloud run jobs delete x`,
/// `bq rm t`), or an argument that is a SQL statement starting with a write
/// (`bq query "DELETE FROM t"`). Flags are skipped, and a filter expression
/// is not a statement (`severity>=ERROR AND textPayload:"failed to create"`
/// starts with `severity`).
pub(crate) fn write_word(command: &str) -> Option<String> {
    let first = simple_commands(command, 0).into_iter().next()?;
    if let Some(word) = bq_query_write(&first) {
        return Some(word);
    }
    for word in first.args.iter().filter(|w| !w.op && !w.text.starts_with('-')) {
        let text = word.text.trim();
        if text.chars().any(char::is_whitespace) {
            let lead = text
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .find(|t| !t.is_empty())
                .map(|t| t.to_ascii_lowercase())?;
            if SQL_WRITES.contains(&lead.as_str()) {
                return Some(lead);
            }
            continue;
        }
        for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
            let token = token.to_ascii_lowercase();
            if WRITE_VERBS.contains(&token.as_str()) {
                return Some(token);
            }
        }
    }
    None
}

/// For `bq query`, what could make it write (EYES `a2c4c8df`): a flag that
/// stores or schedules the result (`--destination_table`, `--append_table`,
/// `--replace`, `--schedule`), a `;` (a script of several statements), a
/// comment (where a statement's first word could hide), or SQL whose first
/// word is not SELECT or WITH.
fn bq_query_write(cmd: &Cmd) -> Option<String> {
    if cmd.tool != "bq" {
        return None;
    }
    let words: Vec<&str> = cmd.args.iter().filter(|w| !w.op).map(|w| w.text.as_str()).collect();
    let at = words.iter().position(|w| *w == "query")?;
    for flag in words.iter().filter(|w| w.starts_with("--")) {
        let name = flag.trim_start_matches('-').split('=').next().unwrap_or("");
        if name.starts_with("destination") || matches!(name, "append_table" | "replace" | "schedule") {
            return Some(format!("--{name}"));
        }
    }
    for word in &words[at + 1..] {
        if let Some(marker) = [";", "--", "/*", "#"].into_iter().find(|m| word.contains(m) && !word.starts_with("--")) {
            return Some(marker.to_string());
        }
        if word.starts_with('-') || !word.chars().any(char::is_whitespace) {
            continue;
        }
        let lead = word
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .find(|t| !t.is_empty())
            .map(|t| t.to_ascii_lowercase())?;
        if lead != "select" && lead != "with" {
            return Some(lead);
        }
    }
    None
}

/// Database clients `read_gate` does not take: each runs its own command
/// language, where a write hides from [`write_word`] — `psql -c 'select 1;
/// delete …'`, `\!`, `-f x.sql` (EYES `a2c4c8df`). A query on production
/// Postgres goes through `prod_read`.
const DATABASE_CLIENTS: &[&str] =
    &["psql", "mysql", "mariadb", "sqlite3", "mongosh", "mongo", "redis-cli", "sqlcmd"];

/// The database client `command`'s first command runs, if it is one of
/// [`DATABASE_CLIENTS`].
pub(crate) fn database_client(command: &str) -> Option<String> {
    let first = simple_commands(command, 0).into_iter().next()?;
    DATABASE_CLIENTS.contains(&first.tool.as_str()).then_some(first.tool)
}

// ---------------------------------------------------------------------------
// The publish an approved command makes — what it is read back against
// ---------------------------------------------------------------------------

/// Which GitHub object a single `gh` publish writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GhPublishKind {
    IssueCreate,
    PrCreate,
    IssueEdit,
    PrEdit,
    IssueComment,
    PrComment,
}

impl GhPublishKind {
    /// It REPLACES a body that exists (`edit`, or a comment's
    /// `--edit-last`) rather than creating one — what the read-back's retry
    /// and the reviewer's live-body diff are for.
    pub(crate) fn replaces(self, edit_last: bool) -> bool {
        matches!(self, Self::IssueEdit | Self::PrEdit)
            || (edit_last && matches!(self, Self::IssueComment | Self::PrComment))
    }

    /// A pull request's own body, whose closing references GitHub reports.
    pub(crate) fn is_pr_body(self) -> bool {
        matches!(self, Self::PrCreate | Self::PrEdit)
    }
}

/// The body a publish sends: a file's content, or text on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PublishedBody {
    File(String),
    Inline(String),
}

/// One `gh issue|pr create|edit|comment`, as an approved command makes it
/// (feedback #57 #60 #97): what to read back, and what to compare it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GhPublish {
    pub kind: GhPublishKind,
    /// The issue or PR the command names (`5`, a URL, a branch); `None` for
    /// a create.
    pub target: Option<String>,
    /// `--repo` / `-R`.
    pub repo: Option<String>,
    pub body: PublishedBody,
    /// `gh … comment --edit-last`.
    pub edit_last: bool,
}

/// The publish `command` makes, when bot-hq can read it back — or the reason
/// it cannot, which the gate's result says in one line (EYES, s-3158eb35: a
/// publish with no read-back must not look like one that matched).
///
/// Only ONE simple command, and only `gh issue|pr create|edit|comment` with
/// one body: with a second command in the line (`gh … ; echo <url>`) the
/// output could name the wrong object, and `gh issue edit 1 2 3` prints its
/// URLs in no fixed order (gh 2.81 runs the edits in parallel). A command
/// that sets its own gh environment (`GH_TOKEN=…`, `GH_HOST=…`) publishes as
/// another identity or host, which a read as the default one may not see.
pub(crate) fn gh_publish(command: &str) -> Result<GhPublish, String> {
    const ONLY: &str = "only a single `gh issue|pr create|edit|comment` is read back";
    let commands = simple_commands(command, 0);
    let [cmd] = commands.as_slice() else {
        return Err(if commands.len() > 1 {
            "the approved command runs more than one command, so its output may name another \
             object"
                .to_string()
        } else {
            ONLY.to_string()
        });
    };
    if cmd.opaque || cmd.tool != "gh" {
        return Err(ONLY.to_string());
    }
    // Only the words BEFORE `gh` set its environment (`GH_TOKEN=… gh`, `env
    // GH_HOST=… gh`): a body that mentions `GH_TOKEN=` is not one.
    let sets_gh_env = segments(command).iter().any(|s| {
        s.words
            .iter()
            .take_while(|w| w.op || basename(&w.text) != "gh")
            .any(|w| {
                !w.op
                    && w.text.split_once('=').is_some_and(|(name, _)| {
                        name.starts_with("GH_") || name.starts_with("GITHUB_")
                    })
            })
    });
    if sets_gh_env {
        return Err(
            "the command sets its own gh environment (`GH_…=` / `GITHUB_…=`), so a read as the \
             default identity and host may not see what it published"
                .to_string(),
        );
    }
    let args = &cmd.args;
    let mut positional = args.iter().filter(|w| !w.op && !w.text.starts_with('-'));
    let (group, sub) = (
        positional.next().map(|w| w.text.as_str()).unwrap_or(""),
        positional.next().map(|w| w.text.as_str()).unwrap_or(""),
    );
    let kind = match (group, sub) {
        ("issue", "create") => GhPublishKind::IssueCreate,
        ("pr", "create") => GhPublishKind::PrCreate,
        ("issue", "edit") => GhPublishKind::IssueEdit,
        ("pr", "edit") => GhPublishKind::PrEdit,
        ("issue", "comment") => GhPublishKind::IssueComment,
        ("pr", "comment") => GhPublishKind::PrComment,
        _ => return Err(ONLY.to_string()),
    };
    let (table, _) = flag_table("gh", group, sub);
    let takes_value = |flag: &str| table.iter().any(|(f, _)| *f == flag);
    let mut operands: Vec<String> = Vec::new();
    let mut bodies: Vec<PublishedBody> = Vec::new();
    let mut repo = None;
    let mut edit_last = false;
    let mut only_operands = false;
    let mut i = 0;
    while i < args.len() {
        let w = &args[i];
        if w.op {
            i += 2;
            continue;
        }
        let text = w.text.as_str();
        if only_operands || !text.starts_with('-') || text == "-" {
            operands.push(w.text.clone());
            i += 1;
            continue;
        }
        if text == "--" {
            only_operands = true;
            i += 1;
            continue;
        }
        // `--flag=value`, `--flag value`, `-Xvalue`, `-X value`.
        let (flag, attached) = match text.strip_prefix("--") {
            Some(long) => match long.split_once('=') {
                Some((name, value)) => (format!("--{name}"), Some(value.to_string())),
                None => (text.to_string(), None),
            },
            None => {
                let mut chars = text.chars();
                chars.next();
                let first = chars.next().map(|c| format!("-{c}")).unwrap_or_default();
                let rest: String = chars.collect();
                (first, (!rest.is_empty()).then_some(rest))
            }
        };
        if flag == "--edit-last" {
            edit_last = true;
            i += 1;
            continue;
        }
        if !takes_value(&flag) {
            i += 1; // a boolean (or a cluster of them)
            continue;
        }
        let (value, consumed) = match attached {
            Some(v) => (v, 1),
            None => match args.get(i + 1) {
                Some(next) if !next.op => (next.text.clone(), 2),
                _ => (String::new(), 1),
            },
        };
        i += consumed;
        match flag.as_str() {
            "--body" | "-b" => bodies.push(PublishedBody::Inline(value)),
            "--body-file" | "-F" => bodies.push(PublishedBody::File(value)),
            "--repo" | "-R" => repo = Some(value),
            _ => {}
        }
    }
    // The first two operands are the group and the subcommand.
    let targets = operands.get(2..).unwrap_or_default();
    if targets.len() > 1 {
        return Err(
            "the command names several issues or pull requests, whose URLs gh prints in no fixed \
             order"
                .to_string(),
        );
    }
    let body = match bodies.len() {
        1 => bodies.remove(0),
        0 => return Err("the command publishes no body to compare".to_string()),
        _ => return Err("the command passes more than one body".to_string()),
    };
    Ok(GhPublish { kind, target: targets.first().cloned(), repo, body, edit_last })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(command: &str) -> Vec<Vec<String>> {
        segments(command)
            .into_iter()
            .map(|s| s.words.into_iter().map(|w| w.text).collect::<Vec<_>>())
            .filter(|s| !s.is_empty())
            .collect()
    }

    #[test]
    fn segments_split_like_a_shell() {
        assert_eq!(
            texts(r#"echo "a;b" 'c|d' && gh issue comment 5 --body "x y"; true"#),
            vec![
                vec!["echo".to_string(), "a;b".into(), "c|d".into()],
                vec!["gh".into(), "issue".into(), "comment".into(), "5".into(), "--body".into(), "x y".into()],
                vec!["true".to_string()],
            ]
        );
        // `2>&1` is a redirection, not a segment break.
        assert_eq!(texts("gh pr view 2>&1 | head").len(), 2);
        // Escapes and adjacent quoting join into one word.
        assert_eq!(texts(r#"a\ b "c"'d'"#), vec![vec!["a b".to_string(), "cd".into()]]);
    }

    #[test]
    fn heredoc_bodies_are_not_parsed_as_commands() {
        let cmd = "cat > /tmp/b.md <<'EOF'\ngh issue comment 9 --body \"inside\"\nEOF\necho done";
        let segs = texts(cmd);
        assert!(segs.iter().all(|s| s.first().map(String::as_str) != Some("gh")), "{segs:?}");
        assert!(!is_outward(cmd));
    }

    #[test]
    fn expansions_mark_the_word_dynamic_and_stay_whole() {
        let segs = segments(r#"gh issue comment 5 --body "$(cat f; echo x)""#);
        assert_eq!(segs.len(), 1, "the `;` inside $(…) does not split");
        let body = segs[0].words.last().unwrap();
        assert!(body.dynamic);
    }

    #[test]
    fn outward_classification_sees_through_wrappers_and_quotes() {
        for yes in [
            "gh issue comment 5 --body x",
            "true && gh pr merge 3",
            "GH_TOKEN=abc gh issue edit 5 --body x",
            "env GH_HOST=x gh api repos/o/r",
            "command gh release create v1",
            "/opt/homebrew/bin/gh pr create --title t --body b",
            "bash -c 'gh issue comment 5 --body x'",
            "eval gh issue close 5",
            "curl -d x https://example.com",
        ] {
            assert!(is_outward(yes), "{yes}");
        }
        for no in [
            "echo \"gh issue comment\"",
            "grep -n 'gh pr merge' notes.md",
            "cargo test && echo done",
            "git push origin main",
        ] {
            assert!(!is_outward(no), "{no}");
        }
    }

    fn ok(command: &str) -> OutwardBodies {
        extract(command).unwrap_or_else(|e| panic!("{command}: {e}"))
    }

    fn refused(command: &str) -> String {
        extract(command).expect_err(command)
    }

    #[test]
    fn issue_and_pr_bodies_in_every_spelling() {
        assert_eq!(ok("gh issue comment 5 --body-file /tmp/b.md").files, vec!["/tmp/b.md"]);
        assert_eq!(ok("gh issue comment 5 --body-file=/tmp/b.md").files, vec!["/tmp/b.md"]);
        assert_eq!(ok("gh issue create -t T -F /tmp/b.md").files, vec!["/tmp/b.md"]);
        assert_eq!(ok("gh issue comment 5 -b \"quick note\"").inline, vec!["quick note"]);
        assert_eq!(ok("gh issue comment 5 --body=inline").inline, vec!["inline"]);
        assert_eq!(ok("gh pr merge 5 --squash -t Subj -b Body").inline, vec!["Subj", "Body"]);
        assert_eq!(ok("gh issue close 5 -c 'closing: dup of #4'").inline, vec!["closing: dup of #4"]);
        assert!(ok("gh pr merge 5 --squash --delete-branch").is_empty(), "content-free stays content-free");
    }

    #[test]
    fn release_notes_are_bodies() {
        assert_eq!(ok("gh release edit v1.0.5 --notes-file /tmp/n.md").files, vec!["/tmp/n.md"]);
        assert_eq!(ok("gh release create v2 -F notes.md").files, vec!["notes.md"]);
        assert_eq!(ok("gh release create v2 --notes 'x y' --title T").inline, vec!["x y", "T"]);
        assert_eq!(ok("gh release create v2 -n short").inline, vec!["short"]);
        assert!(ok("gh release create v2 --generate-notes").is_empty());
        assert!(refused("gh release create v2 --notes-from-tag").contains("tag message"));
    }

    #[test]
    fn api_fields_follow_gh_semantics() {
        let b = ok("gh api repos/o/r/issues/5/comments -F body=@/tmp/c.md -f title=@literal");
        assert_eq!(b.files, vec!["/tmp/c.md"], "-F reads @file");
        assert_eq!(b.inline, vec!["@literal"], "-f never reads a file");
        assert_eq!(ok("gh api graphql --input /tmp/q.json").files, vec!["/tmp/q.json"]);
        assert!(ok("gh api repos/o/r/issues/5 --jq .body").is_empty(), "a read carries no body");
        assert!(refused("gh api x -F body=@-").contains("stdin"));
        assert!(refused("gh api x --input -").contains("stdin"));
    }

    #[test]
    fn gist_files_are_the_body() {
        let b = ok("gh gist create --desc D notes.md /tmp/two.txt");
        assert_eq!(b.files, vec!["notes.md", "/tmp/two.txt"]);
        assert_eq!(b.inline, vec!["D"]);
        assert!(refused("gh gist create -").contains("stdin"));
    }

    #[test]
    fn curl_data_forms_and_the_cookie_flag() {
        assert_eq!(ok("curl -d @/tmp/p.json https://x").files, vec!["/tmp/p.json"]);
        assert_eq!(ok("curl -d@/tmp/p.json https://x").files, vec!["/tmp/p.json"]);
        assert_eq!(ok("curl --data-raw @literal https://x").inline, vec!["@literal"]);
        assert_eq!(ok("curl -F 'file=@/tmp/a.txt;type=text/plain' https://x").files, vec!["/tmp/a.txt"]);
        assert_eq!(ok("curl -T up.bin https://x").files, vec!["up.bin"]);
        // `-b` is curl's --cookie, not a body: no false refusal.
        assert!(ok("curl -b session=1 https://x").is_empty());
        assert!(refused("curl --data-binary @- https://x").contains("stdin"));
    }

    #[test]
    fn shell_computed_stdin_and_unknown_file_bodies_refuse() {
        assert!(refused("gh issue comment 5 --body \"$(cat f.md)\"").contains("computed by the shell"));
        assert!(refused("gh issue comment 5 --body \"$BODY\"").contains("computed by the shell"));
        assert!(refused("gh issue comment 5 --body-file $F").contains("computed by the shell"));
        assert!(refused("gh issue comment 5 --body-file -").contains("stdin"));
        assert!(refused("gh issue comment 5 -F /dev/stdin").contains("stdin"));
        assert!(refused("gh pr create --fill").contains("commit messages"));
        assert!(refused("gh pr create -f").contains("commit messages"));
        assert!(refused("gh issue create --template-file x.md").contains("does not know"));
    }

    #[test]
    fn a_body_file_written_by_the_same_command_refuses() {
        let cmd = "cat > /tmp/b.md <<'EOF'\nhello\nEOF\ngh issue comment 5 --body-file /tmp/b.md";
        assert!(refused(cmd).contains("written by this same command"));
        assert!(refused("cp draft.md /tmp/b.md && gh issue comment 5 --body-file /tmp/b.md")
            .contains("written by this same command"));
        // Reading an unrelated file in the same line is fine.
        assert_eq!(ok("cat other.md && gh issue comment 5 --body-file /tmp/b.md").files, vec!["/tmp/b.md"]);
    }

    #[test]
    fn nested_shell_strings_are_extracted_too() {
        assert_eq!(ok("bash -c 'gh issue comment 5 --body-file /tmp/b.md'").files, vec!["/tmp/b.md"]);
    }

    /// EYES 7d3d4f34: a publish inside a script a SHELL reads from stdin —
    /// its own heredoc, a herestring, or a literal producer piped into it —
    /// is outward, and its body is extracted. A heredoc that is only DATA
    /// (`cat > f <<EOF`) still is not.
    #[test]
    fn scripts_fed_to_a_shell_are_parsed_as_commands() {
        let heredoc = "bash <<'EOF'\ngh issue comment 5 --body-file b.md\nEOF";
        assert!(is_outward(heredoc));
        assert_eq!(ok(heredoc).files, vec!["b.md"]);
        let piped = "cat <<'EOF' | bash\ngh issue comment 5 --body-file b.md\nEOF";
        assert!(is_outward(piped));
        assert_eq!(ok(piped).files, vec!["b.md"]);
        assert_eq!(ok("bash -s <<< 'gh issue comment 5 -b hi'").inline, vec!["hi"]);
        assert!(is_outward("echo 'gh pr merge 3' | sh"));
        assert!(is_outward("printf 'gh pr merge 3\\n' | zsh"));
        // A script bot-hq cannot read is outward, fail closed.
        assert!(is_outward("./gen-script | bash"));
        assert!(is_outward("sh -c \"$SCRIPT\""));
        assert!(is_outward("eval \"$CMD\""));
        // Data heredocs and script FILES are not scripts on the command line.
        assert!(!is_outward("cat > /tmp/n.md <<'EOF'\ngh issue comment 9\nEOF"));
        assert!(!is_outward("bash ./scripts/build.sh"));
    }

    #[test]
    fn command_substitutions_are_analysed_as_commands() {
        let cmd = "URL=$(gh pr create -t T --body-file b.md)";
        assert!(is_outward(cmd));
        assert_eq!(ok(cmd).files, vec!["b.md"]);
        assert!(is_outward("echo \"$(gh issue comment 5 -b x)\""));
        assert!(is_outward("echo `gh issue close 5`"));
        assert!(is_outward("diff <(gh api repos/o/r) old.json"));
    }

    #[test]
    fn grouping_keywords_and_wrappers_do_not_hide_gh() {
        for cmd in [
            "( gh issue comment 5 -b x )",
            "{ gh issue comment 5 -b x; }",
            "! gh pr merge 1",
            "if true; then gh pr merge 1; fi",
            "for i in 1 2; do gh issue close $i; done",
            "timeout 30 gh pr merge 1",
            "xargs -I{} gh issue close {}",
            "nice -n 5 gh pr merge 1",
            "find . -name x -exec gh issue close 5 \\;",
            "ssh host 'gh issue close 5'",
            "caffeinate -i gh release create v1",
        ] {
            assert!(is_outward(cmd), "{cmd}");
        }
        // The wrapper's args end where the gh command starts: its body is
        // extracted, and nothing counts as written by the wrapper.
        assert_eq!(ok("timeout 30 gh issue comment 5 --body-file b.md").files, vec!["b.md"]);
        assert_eq!(ok("xargs -I{} gh issue comment {} -b hello").inline, vec!["hello"]);
    }

    #[test]
    fn curl_urlencode_reads_a_file_when_curl_does() {
        assert_eq!(ok("curl --data-urlencode name@/tmp/v.txt https://x").files, vec!["/tmp/v.txt"]);
        assert_eq!(ok("curl --data-urlencode @/tmp/v.txt https://x").files, vec!["/tmp/v.txt"]);
        assert_eq!(ok("curl --data-urlencode q=hello https://x").inline, vec!["hello"]);
        assert_eq!(ok("curl --data-urlencode '=a@b' https://x").inline, vec!["a@b"]);
        assert!(refused("curl --data-urlencode body@- https://x").contains("stdin"));
    }

    /// EYES 7cc26d55: wrapper flags are per wrapper, and a shell's option
    /// VALUES are not script files.
    #[test]
    fn wrapper_and_shell_option_values_do_not_hide_the_command() {
        assert!(is_outward("time -p gh pr merge 1"));
        assert!(is_outward("command -p gh issue close 5"));
        assert!(is_outward("exec -a name gh pr merge 1"));
        assert!(is_outward("sudo -u me -p pw gh pr merge 1"));
        assert_eq!(ok("bash -o pipefail -c 'gh issue comment 5 --body-file b.md'").files, vec!["b.md"]);
        assert_eq!(ok("bash -eo pipefail -c 'gh issue comment 5 -b x'").inline, vec!["x"]);
        assert!(is_outward("bash -O extglob -c 'gh pr merge 1'"));
        assert!(is_outward("bash --rcfile r.sh -c 'gh pr merge 1'"));
    }

    // --- gh_publish: what an approved publish is read back against ---------

    fn publish(command: &str) -> GhPublish {
        gh_publish(command).unwrap_or_else(|why| panic!("{command:?}: {why}"))
    }

    #[test]
    fn every_issue_and_pr_publish_names_its_kind_target_and_body() {
        use GhPublishKind::*;
        let cases: &[(&str, GhPublishKind, Option<&str>, PublishedBody)] = &[
            ("gh issue create --title t --body-file b.md", IssueCreate, None, PublishedBody::File("b.md".into())),
            ("gh pr create --base main -F b.md", PrCreate, None, PublishedBody::File("b.md".into())),
            ("gh issue edit 63 --body-file /tmp/x.md", IssueEdit, Some("63"), PublishedBody::File("/tmp/x.md".into())),
            ("gh pr edit 62 -b \"new text\"", PrEdit, Some("62"), PublishedBody::Inline("new text".into())),
            ("gh issue comment 5 --body=inline-words", IssueComment, Some("5"), PublishedBody::Inline("inline-words".into())),
            ("gh pr comment https://github.com/o/r/pull/9 --body-file c.md", PrComment, Some("https://github.com/o/r/pull/9"), PublishedBody::File("c.md".into())),
            ("./gh issue comment 5 -Fc.md", IssueComment, Some("5"), PublishedBody::File("c.md".into())),
        ];
        for (command, kind, target, body) in cases {
            let p = publish(command);
            assert_eq!((p.kind, p.target.as_deref(), &p.body), (*kind, *target, body), "{command}");
        }
        let p = publish("gh issue comment 5 --edit-last --repo o/r --body-file c.md");
        assert!(p.edit_last && p.kind.replaces(p.edit_last));
        assert_eq!(p.repo.as_deref(), Some("o/r"));
        assert!(!publish("gh issue comment 5 -b x").kind.replaces(false));
        assert!(publish("gh pr edit 5 -b x").kind.is_pr_body() && !publish("gh pr comment 5 -b x").kind.is_pr_body());
    }

    /// The forms with no read-back say why (EYES, s-3158eb35): a second
    /// command, an unsupported tool or subcommand, several targets, no body,
    /// or a gh environment of its own.
    #[test]
    fn what_cannot_be_read_back_says_why() {
        for (command, why) in [
            ("gh issue comment 5 --body-file c.md; echo https://github.com/o/r/issues/1", "more than one command"),
            ("gh release create v1 --notes-file n.md", "only a single"),
            ("gh pr merge 5 -b x", "only a single"),
            ("curl -d @b.json https://example.com", "only a single"),
            ("gh issue edit 1 2 3 --body-file b.md", "several issues"),
            ("gh pr edit 5 --title t", "no body"),
            ("gh issue comment 5 -b a --body-file b.md", "more than one body"),
            ("GH_TOKEN=x gh issue comment 5 --body-file c.md", "own gh environment"),
            ("env GH_HOST=ghe.example.com gh pr create -F b.md", "own gh environment"),
        ] {
            let err = gh_publish(command).expect_err(command);
            assert!(err.contains(why), "{command}: {err}");
        }
        // A body that MENTIONS a gh variable is not one.
        assert_eq!(publish("gh issue comment 5 -b \"set GH_TOKEN=… first\"").kind, GhPublishKind::IssueComment);
    }

    // --- read_gate's shape (group K) -----------------------------------------

    #[test]
    fn read_gate_takes_one_command_and_pure_filters_only() {
        for ok in [
            "gcloud logging read 'severity>=ERROR' --limit 5",
            "gcloud logging read x | head -5",
            "bq ls --format=json | jq '.[].id' | grep prod | wc -l",
            "psql -h ep-solitary-field-x -c 'select 1' | cut -d, -f1 | tail -3",
        ] {
            assert_eq!(single_read_command(ok), Ok(()), "{ok}");
        }
        for (bad, why) in [
            ("gcloud logging read x; rm -rf ~/d", "more than one command"),
            ("gcloud logging read x && echo done", "more than one command"),
            ("gcloud logging read x || true", "more than one command"),
            ("gcloud logging read x &", "more than one command"),
            ("gcloud logging read x > ~/.zshrc", "redirection"),
            ("gcloud logging read x | sort -o ~/.zshrc", "`sort`"),
            ("gcloud logging read x | uniq in out", "`uniq`"),
            ("gcloud logging read x | sh", "`sh`"),
            ("gcloud logging read $FILTER", "computes"),
            ("gcloud logging read $(cat f)", "computes"),
            ("sh -c 'gcloud logging read x'", "wrapper"),
            ("env CLOUDSDK_CORE_PROJECT=p gcloud logging read x", "wrapper"),
            ("CLOUDSDK_CORE_PROJECT=p gcloud logging read x", "wrapper"),
            ("psql -h h <<'SQL'\nselect 1\nSQL", "line"),
        ] {
            let err = single_read_command(bad).expect_err(bad);
            assert!(err.contains(why), "{bad}: {err}");
        }
    }

    #[test]
    fn write_words_are_found_in_verbs_and_sql_but_not_in_filters() {
        for (command, word) in [
            ("gcloud run jobs delete export", "delete"),
            ("gcloud run deploy web --image x", "deploy"),
            ("bq rm -t ds.t", "rm"),
            ("gcloud storage cp gs://a/b .", "cp"),
            ("gcloud run jobs execute export", "execute"),
            ("bq query --use_legacy_sql=false 'DELETE FROM ds.t WHERE 1=1'", "delete"),
            ("psql -h h -c 'update t set a = 1'", "update"),
            // EYES `a2c4c8df`.
            ("gcloud scheduler jobs pause export", "pause"),
            ("bq load ds.t gs://b/x.csv", "load"),
            ("bq mk -t ds.t", "mk"),
            ("bq extract ds.t gs://b/x", "extract"),
            ("bq query 'SELECT 1; DELETE FROM ds.t WHERE true'", ";"),
            ("bq query '/* x */ DELETE FROM ds.t WHERE true'", "/*"),
            ("bq query --destination_table=ds.copy 'SELECT * FROM ds.t'", "--destination_table"),
            ("bq --format json query 'DECLARE x INT64 DEFAULT 1'", "declare"),
        ] {
            assert_eq!(write_word(command).as_deref(), Some(word), "{command}");
        }
        for read in [
            "gcloud run jobs describe export",
            "gcloud run jobs executions list --job export",
            "gcloud logging read 'severity>=ERROR AND textPayload:\"failed to create\"' --limit 5",
            "bq query --use_legacy_sql=false 'SELECT updated_at FROM ds.t'",
            "bq query --max_rows 10 'WITH x AS (SELECT 1) SELECT * FROM x'",
            "bq show ds.t",
            "psql -h h -c 'select 1'",
        ] {
            assert_eq!(write_word(read), None, "{read}");
        }
        assert_eq!(database_client("psql -h h -c 'select 1' | head").as_deref(), Some("psql"));
        assert_eq!(database_client("/usr/bin/sqlite3 db.sqlite .dump").as_deref(), Some("sqlite3"));
        assert_eq!(database_client("gcloud logging read x"), None);
    }
}
