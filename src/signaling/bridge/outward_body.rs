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
            out.push(Cmd { tool, args, opaque });
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
}
