//! Built-in tools the native loop implements itself.
//!
//! ## Role filtering
//!
//! Tools are classified by [`Access`] and filtered per agent by
//! [`ToolPolicy`]. EYES gets the read set and nothing else — not by prompt, by
//! construction: a write tool is absent from the advertised `tools` array AND
//! refused at exec time, so there is no path to it even if the model invents the
//! name.
//!
//! This is a **role** decision, not a limit of the loop. The loop can express any
//! policy; which one an agent gets depends on what that agent is for.
//!
//! ## Read scope
//!
//! Every path is resolved beneath a root and refused if it escapes. This is not
//! defence in depth — it is the *only* read gate that has ever existed for these
//! tools. On the claude-code path `Read`/`Grep`/`Glob` are not `Bash`, so they
//! never reach the Tool Gate, and the Tool Gate's PreToolUse hook is injected in
//! the HANDS branch only; the session's `working_repo_path` is a prompt
//! convention with no mechanism behind it (`issues.md` #1). A native agent
//! implements these tools itself, so the check is free here.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::command::{self, CommandPolicy};
use super::wire::{ToolCall, ToolOutcome};

/// Whether a tool reads or mutates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// Which built-ins an agent may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicy {
    /// Read tools only. The EYES preset.
    ReadOnly,
    /// Read + write. No agent uses this yet; it exists so the role filter has a
    /// second value and is therefore actually a filter rather than a constant.
    ReadWrite,
}

impl ToolPolicy {
    /// The policy for `agent_name`, via the one place a name becomes
    /// capabilities ([`AgentRole`](crate::agents::AgentRole)).
    ///
    /// An unrecognised name gets read-only — it has no role, so it gets the
    /// most conservative answer rather than an inherited default.
    pub fn for_agent(agent_name: &str) -> Self {
        crate::agents::AgentRole::for_agent(agent_name)
            .map(|r| r.tool_policy())
            .unwrap_or(Self::ReadOnly)
    }

    fn allows(self, access: Access) -> bool {
        match self {
            Self::ReadOnly => access == Access::Read,
            Self::ReadWrite => true,
        }
    }
}

/// Every built-in, with its access class.
const TOOLS: &[(&str, Access)] = &[
    ("read_file", Access::Read),
    ("list_files", Access::Read),
    ("search_files", Access::Read),
    ("run_command", Access::Read),
    ("write_file", Access::Write),
];

/// Cap on `search_files` hits.
pub const MAX_SEARCH_HITS: usize = 200;
/// Cap on `list_files` results.
pub const MAX_GLOB_HITS: usize = 500;

/// Directory names never descended by the FALLBACK walk (see [`glob_files`] —
/// a git repository is enumerated by git instead, which is both more accurate
/// and the common case).
///
/// Not cosmetic. Enumeration is capped at [`MAX_GLOB_HITS`] and these are where
/// the budget goes: an alphabetical walk of this repo reaches 66,667 entries
/// before the first `src/` path. Without pruning, `search_files` spent its whole
/// budget inside them and then reported a confident "no matches" for content
/// sitting in plain sight.
///
/// A fixed list can only ever be a guess about someone else's repo — measured
/// here, pruning all three still left **48,039** entries ahead of `src/`, from a
/// 2.4 GB `bench/` directory no universal list would name. That is precisely why
/// the git path exists and this is the fallback.
///
/// Only ENUMERATION prunes. Direct-path tools still reach these trees when asked
/// explicitly: `read_file(".git/config")` and `run_command("ls .git")` work
/// unchanged.
const PRUNED_DIRS: &[&str] = &[".git", "node_modules", "target"];

/// Cap tool output so one read cannot blow the context window.
pub const MAX_FILE_BYTES: usize = 256 * 1024;

/// Anthropic `tools` entries for the built-ins, ready to concatenate with the
/// converted MCP tool list.
///
/// **Empty when there is no read root.** A session with no working repo has no
/// directory this agent is entitled to read, so the tool is not offered at all
/// rather than pointed somewhere arbitrary — see [`exec`].
pub fn tool_defs_for(root: Option<&Path>, policy: ToolPolicy) -> Vec<Value> {
    if root.is_none() {
        return Vec::new();
    }
    tool_defs()
        .into_iter()
        .filter(|d| {
            d["name"]
                .as_str()
                .and_then(access_of)
                .is_some_and(|a| policy.allows(a))
        })
        .collect()
}

/// The access class of a built-in, or `None` if it isn't one.
pub fn access_of(name: &str) -> Option<Access> {
    TOOLS.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

/// Every built-in definition, unfiltered. Prefer [`tool_defs_for`].
pub fn tool_defs() -> Vec<Value> {
    vec![
        json!({
            "name": "read_file",
            "description": "Read a UTF-8 text file from the working repository, returned with \
                            line numbers. Paths are relative to the repository root. Use \
                            `offset` + `limit` on large files — reading a whole 2000-line \
                            file to see 30 lines of it wastes most of your context window.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to the repository root, e.g. \"Cargo.toml\"." },
                    "offset": { "type": "integer", "description": "1-based line to start at. Defaults to the first line." },
                    "limit": { "type": "integer", "description": "Maximum number of lines to return. Defaults to the whole file." }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "list_files",
            "description": "List repository files matching a glob. Use this instead of \
                            shelling out to `find` or `ls`. Lists what the repository \
                            itself tracks — gitignored paths, build output and \
                            dependency directories are not included; reach those with \
                            `read_file` or `run_command` if you genuinely need them.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob relative to the repository root, e.g. \"src/**/*.rs\"." }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "search_files",
            "description": "Search file CONTENTS with a regular expression and return \
                            matching lines as `path:line: text`. Use this instead of \
                            shelling out to `grep`. Searches what the repository tracks — \
                            gitignored paths, build output and dependency directories are \
                            not searched. If the output notes the file listing was \
                            capped, the result set is incomplete — re-run with a \
                            narrower `glob`.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Rust regex, e.g. \"fn spawn_\\\\w+\"." },
                    "glob": { "type": "string", "description": "Optional glob limiting which files are searched, e.g. \"src/**/*.rs\"." }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "run_command",
            "description": "Run ONE read-only command in the repository root. No shell: no \
                            pipes, chaining or redirection. Only an allow-listed set of \
                            reporting commands is permitted (git/gh read subcommands, cat, \
                            ls, wc, head, tail, find, which, file, stat, du, npm ls, \
                            composer show, cargo tree). Anything that could change state is \
                            refused — ask your peer to run it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command, e.g. \"git log --oneline -5\"." }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "write_file",
            "description": "Write a UTF-8 text file in the working repository, creating or \
                            replacing it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to the repository root." },
                    "content": { "type": "string", "description": "Full new file contents." }
                },
                "required": ["path", "content"]
            }
        }),
    ]
}

/// Is `name` one of the built-ins this module handles?
pub fn handles(name: &str) -> bool {
    access_of(name).is_some()
}

/// Resolve `rel` beneath `root`, refusing anything that escapes.
///
/// `root` MUST already be canonicalized. Canonicalizing the candidate too is
/// what makes this hold against `..` *and* symlinks — a plain string prefix
/// check passes both. Note `Path::join` lets an absolute `rel` replace the base
/// entirely, so `/etc/passwd` lands outside `root` and is caught here.
pub fn resolve_in_root(root: &Path, rel: &str) -> Result<PathBuf, String> {
    // Lexical scope check FIRST, before canonicalize.
    //
    // `canonicalize` fails with ENOENT when the target doesn't exist, so an
    // escaping path used to be reported as "cannot resolve … No such file or
    // directory" — which reads as "the file is missing" and invites the agent to
    // retry a different path, instead of "you may not look there". Same
    // actionability problem as the read_file windowing bug: the refusal was
    // correct but told the model the wrong thing. Checking lexically first means
    // the message describes the real reason whether or not the target exists.
    if Path::new(rel).is_absolute() {
        return Err(format!(
            "{rel:?} is an absolute path, which resolves outside the repository root — \
             refused. Use a path relative to the repository root."
        ));
    }
    if rel.split(['/', '\\']).any(|seg| seg == "..") {
        return Err(format!(
            "{rel:?} walks above the repository root — refused. This agent may only read \
             inside the working repository."
        ));
    }

    // Then canonicalize, which is what holds against SYMLINKS — a lexical check
    // alone passes a link inside the root pointing out of it.
    let canonical = root
        .join(rel)
        .canonicalize()
        .map_err(|e| format!("cannot resolve {rel:?}: {e}"))?;

    if !canonical.starts_with(root) {
        return Err(format!(
            "{rel:?} resolves outside the repository root — refused"
        ));
    }
    Ok(canonical)
}

/// Execute a built-in. Returns an `is_error` outcome rather than failing the
/// turn: errors are inputs to the loop, and the model recovers from them.
///
/// `root` is `None` for a session with no working repo. **There is no fallback
/// root, deliberately.** Defaulting to the process's current directory scopes
/// the agent to wherever bot-hq happens to have been launched from — which in
/// practice is the bot-hq data directory, containing `.local/mcp-token` (a
/// UUID: `0600`, but the agent runs as the same user, and it is valid UTF-8 so
/// it reads cleanly), `.local/bot-hq.db` with every `models.auth_token` in
/// plaintext, and the whole Context Library. A read gate aimed at the secrets
/// is worse than no read gate, because it looks like protection.
pub async fn exec(
    call: &ToolCall,
    root: Option<&Path>,
    policy: ToolPolicy,
    commands: CommandPolicy,
) -> ToolOutcome {
    let outcome = |content: String, is_error: bool| ToolOutcome {
        tool_use_id: call.id.clone(),
        content,
        is_error,
    };

    // Role gate, enforced independently of the advertised tool list. A model that
    // invents a tool name it was never offered still cannot reach a write.
    match access_of(&call.name) {
        None => return outcome(format!("unknown tool {:?}", call.name), true),
        Some(access) if !policy.allows(access) => {
            return outcome(
                format!(
                    "`{}` mutates state and is not available to this agent — that is a role \
                     boundary, not a missing feature. Ask your peer to do it.",
                    call.name
                ),
                true,
            );
        }
        Some(_) => {}
    }

    let Some(root) = root else {
        return outcome(
            "this session has no working repository, so there is no directory this \
             agent may read. Ask your peer to paste the contents you need."
                .to_string(),
            true,
        );
    };

    match run(call, root, commands).await {
        Ok(text) => outcome(text, false),
        Err(msg) => outcome(msg, true),
    }
}

/// An optional positive integer argument. Absent, zero, or a non-number → `None`.
///
/// Lenient about the JSON type because models emit `"80"` as readily as `80`, and
/// silently ignoring a windowing argument is exactly the failure this exists to
/// fix.
fn usize_arg(call: &ToolCall, key: &str) -> Option<usize> {
    let v = call.input.get(key)?;
    let n = v
        .as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))?;
    (n > 0).then_some(n as usize)
}

/// Render `text` with 1-based line numbers, optionally windowed.
///
/// Line numbers are always included: they make `search_files` hits directly
/// referenceable, and they tell the agent where a windowed read actually landed.
///
/// The window exists because agents ask for one. The first live run of these tools
/// showed EYES calling `read_file` with `offset`/`limit` on six of twelve calls —
/// claude-code's `Read` accepts them — while this tool declared neither and
/// silently returned the whole file: 85,381 bytes of `spawn.rs` for a request of 80
/// lines, roughly 20K tokens spent to deliver about 1K.
fn slice_with_line_numbers(
    text: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let start = offset.unwrap_or(1).saturating_sub(1);

    if start >= total && total > 0 {
        return Err(format!(
            "offset {} is past the end of the file ({total} lines)",
            start + 1
        ));
    }

    let end = match limit {
        Some(n) => (start + n).min(total),
        None => total,
    };

    let mut out = String::new();
    let mut bytes = 0usize;
    let mut last = start;
    for (i, line) in lines[start..end].iter().enumerate() {
        let n = start + i + 1;
        let rendered = format!("{n:6}\t{line}\n");
        // Byte cap still applies: a windowless read of a huge file must not blow
        // the context window just because no limit was given.
        if bytes + rendered.len() > MAX_FILE_BYTES {
            out.push_str(&format!(
                "… truncated at {MAX_FILE_BYTES} bytes (line {n} of {total}); \
                 re-read with offset/limit\n"
            ));
            return Ok(out);
        }
        bytes += rendered.len();
        out.push_str(&rendered);
        last = n;
    }

    if end < total || start > 0 {
        out.push_str(&format!(
            "… showing lines {}-{last} of {total}\n",
            start + 1
        ));
    }
    if out.is_empty() {
        out.push_str("(empty file)\n");
    }
    Ok(out)
}

fn str_arg<'a>(call: &'a ToolCall, key: &str) -> Result<&'a str, String> {
    call.input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string field {key:?}"))
}

async fn run(call: &ToolCall, root: &Path, commands: CommandPolicy) -> Result<String, String> {
    match call.name.as_str() {
        // The three read tools are synchronous filesystem work — a full-tree
        // walk plus up to MAX_GLOB_HITS whole-file reads for `search_files` —
        // so they run on the blocking pool rather than stalling the async
        // worker that drives every other agent's IO. Same reasoning, and same
        // precedent, as `persist()` in `agent.rs`, which does this for the far
        // smaller history write. `run_command` already awaits async process IO
        // and `write_file` is a single small write; both stay inline.
        "read_file" | "list_files" | "search_files" => {
            let call = call.clone();
            let root = root.to_path_buf();
            tokio::task::spawn_blocking(move || run_read_tool(&call, &root))
                .await
                .map_err(|e| format!("tool task failed: {e}"))?
        }

        // The agent's OWN command policy, not one derived from its tool policy.
        // The derivation had two identical arms, so an agent `CommandPolicy`
        // said must get no shell (an unrecognised role) was handed a read-only
        // one — and `CommandPolicy::for_agent`'s tests asserted the opposite of
        // what shipped.
        "run_command" => command::run(str_arg(call, "command")?, root, commands).await,

        "write_file" => {
            let target = root.join(str_arg(call, "path")?);
            // Re-check the parent: `resolve_in_root` needs an existing path, and a
            // new file has none yet.
            let parent = target
                .parent()
                .ok_or_else(|| "path has no parent directory".to_string())?;
            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| format!("cannot resolve parent of the target: {e}"))?;
            if !canonical_parent.starts_with(root) {
                return Err("target resolves outside the repository root — refused".into());
            }
            // The parent check alone is not enough when the TARGET already
            // exists: `fs::write` follows a symlink, so a link inside the root
            // pointing outside it would land the write outside — the exact
            // escape `resolve_in_root` closes on the read path by canonicalizing
            // the target. A dangling link is refused too (`canonicalize` fails):
            // writing through it would CREATE the outside file.
            if target.symlink_metadata().is_ok() {
                let resolved = target
                    .canonicalize()
                    .map_err(|e| format!("cannot resolve the write target: {e}"))?;
                if !resolved.starts_with(root) {
                    return Err(
                        "target resolves outside the repository root — refused".into()
                    );
                }
            }
            std::fs::write(&target, str_arg(call, "content")?)
                .map_err(|e| format!("write failed: {e}"))?;
            Ok(format!("wrote {}", target.display()))
        }

        other => Err(format!("unknown tool {other:?}")),
    }
}

/// The synchronous bodies of the three read tools, split out so [`run`] can
/// move them onto the blocking pool as one unit.
fn run_read_tool(call: &ToolCall, root: &Path) -> Result<String, String> {
    match call.name.as_str() {
        "read_file" => {
            let target = resolve_in_root(root, str_arg(call, "path")?)?;
            let bytes = std::fs::read(&target).map_err(|e| format!("read failed: {e}"))?;
            let text =
                String::from_utf8(bytes).map_err(|_| "file is not valid UTF-8".to_string())?;
            let offset = usize_arg(call, "offset");
            let limit = usize_arg(call, "limit");
            slice_with_line_numbers(&text, offset, limit)
        }

        "list_files" => list_files(root, str_arg(call, "pattern")?),

        "search_files" => search_files(
            root,
            str_arg(call, "pattern")?,
            call.input.get("glob").and_then(Value::as_str),
        ),

        // `run` only routes the three names above here.
        other => Err(format!("unknown tool {other:?}")),
    }
}

/// Structured enumeration result, so `search_files` consumes DATA rather than
/// parsing `list_files`' rendered output. The string-parsing version treated the
/// "… capped" sentinel line as a file path and silently dropped it — which made
/// the cap invisible exactly where it mattered.
struct GlobHits {
    /// Relative paths, sorted.
    files: Vec<String>,
    /// True when enumeration stopped at [`MAX_GLOB_HITS`]. Every consumer must
    /// SAY so — a capped listing that reads as complete is a wrong answer.
    capped: bool,
}

/// Enumerate paths under `root` matching `pattern`.
///
/// **The candidate set comes from git when `root` is a repository**, and only
/// from a filesystem walk otherwise. That ordering is the fix for the bug this
/// module's cap kept producing: `.git`/`node_modules`/`target` are the obvious
/// budget sinks, but pruning them by name still left 48,039 entries ahead of
/// `src/` in this repo — a 2.4 GB `bench/` tree that no hardcoded list would
/// ever name. A fixed exclusion list is a guess about someone else's repo.
///
/// The read root IS a working repository, and the repository already publishes
/// which files matter: `git ls-files -c -o --exclude-standard` is tracked files
/// plus untracked-and-not-ignored ones, honouring every `.gitignore`,
/// `.git/info/exclude` and the global excludes file. Here that is 354 entries
/// rather than 480,000 — comfortably inside the cap, and exactly the set a
/// developer means by "the repo". It is also what `rg` and `fd` do.
///
/// Falls back to the pruned walk when `root` is not a repo, git is missing, or
/// the command fails — the tool must not stop working just because a session
/// points somewhere unversioned.
///
/// Differences from the `glob::glob` walker this replaced, all deliberate:
///
/// - [`PRUNED_DIRS`] are never entered (fallback path only);
/// - symlinked directories are never entered (glob followed them — a link back
///   into the tree loops, one pointing out escapes; file hits are still
///   individually canonicalize-checked);
/// - `*` does not cross `/` (`require_literal_separator`), matching
///   filesystem-glob expectations — `**` is the recursive form;
/// - directories are no longer emitted as hits. The tool is `list_files`, and
///   `search_files` skipped non-files anyway.
fn glob_files(root: &Path, pattern: &str) -> Result<GlobHits, String> {
    if pattern.starts_with('/') || pattern.split('/').any(|s| s == "..") {
        return Err(format!(
            "{pattern:?} points outside the repository root — refused"
        ));
    }
    let matcher =
        glob::Pattern::new(pattern).map_err(|e| format!("bad glob {pattern:?}: {e}"))?;
    let opts = glob::MatchOptions {
        require_literal_separator: true,
        ..glob::MatchOptions::new()
    };

    let mut files = Vec::new();
    let mut capped = false;
    match git_listed_files(root) {
        Some(candidates) => {
            for rel in candidates {
                if !matcher.matches_with(&rel, opts) {
                    continue;
                }
                // The PATTERN is untrusted and a tracked path can be a symlink
                // pointing anywhere — same re-check the walk does.
                if root.join(&rel).canonicalize().is_ok_and(|c| c.starts_with(root)) {
                    files.push(rel);
                    if files.len() >= MAX_GLOB_HITS {
                        capped = true;
                        break;
                    }
                }
            }
        }
        None => walk_dir(root, root, &matcher, opts, &mut files, &mut capped),
    }
    files.sort();
    Ok(GlobHits { files, capped })
}

/// The repository's own view of which files exist, or `None` when `root` is not
/// a git repo (or git is unavailable).
///
/// `-c -o --exclude-standard` = tracked + untracked-not-ignored. `-z` because
/// git quotes unusual filenames otherwise, and a quoted path would fail to
/// resolve. Run with `-C root`, which reports paths relative to `root` — so a
/// session scoped to a SUBDIRECTORY of a repo gets that subdirectory's files,
/// with paths already relative to its own read root.
fn git_listed_files(root: &Path) -> Option<Vec<String>> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-c", "-o", "--exclude-standard", "-z"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // not a repo, or git refused — fall back to the walk
    }
    let listed: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    // An empty repo is a legitimate answer, but so is a git that printed
    // nothing for a reason we did not anticipate. Only trust a non-empty list;
    // the walk is a correct (if noisier) answer either way.
    (!listed.is_empty()).then_some(listed)
}

/// Depth-first, per-directory alphabetical — deterministic, so the capped
/// prefix is stable across runs.
fn walk_dir(
    root: &Path,
    dir: &Path,
    matcher: &glob::Pattern,
    opts: glob::MatchOptions,
    out: &mut Vec<String>,
    capped: &mut bool,
) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return; // unreadable directory: skip, like the glob walker did
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        if *capped {
            return;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().into_owned();
        let is_symlink = entry
            .file_type()
            .is_ok_and(|t| t.is_symlink());

        if matcher.matches_with(&rel, opts) {
            // The PATTERN is untrusted and symlinked FILES can point anywhere —
            // each hit is re-checked, exactly as the glob-based version did.
            if path.canonicalize().is_ok_and(|c| c.starts_with(root)) {
                out.push(rel);
                if out.len() >= MAX_GLOB_HITS {
                    *capped = true;
                    return;
                }
            }
        }

        if path.is_dir() && !is_symlink {
            if let Some(name) = entry.file_name().to_str() {
                if PRUNED_DIRS.contains(&name) {
                    continue;
                }
            }
            walk_dir(root, &path, matcher, opts, out, capped);
        }
    }
}

fn list_files(root: &Path, pattern: &str) -> Result<String, String> {
    let hits = glob_files(root, pattern)?;
    if hits.files.is_empty() {
        return Ok(format!("no files match {pattern:?}"));
    }
    let mut out = hits.files.join("\n");
    if hits.capped {
        out.push_str(&format!(
            "\n… capped at {MAX_GLOB_HITS} results — pass a narrower glob"
        ));
    }
    Ok(out)
}

/// Regex over file contents beneath the root.
///
/// Skips anything that isn't valid UTF-8 — a binary "match" is noise, and
/// decoding lossily would report line numbers that don't exist. (`.git` and
/// friends never reach here: they are pruned at enumeration, see [`PRUNED_DIRS`].)
fn search_files(root: &Path, pattern: &str, file_glob: Option<&str>) -> Result<String, String> {
    let re = regex::Regex::new(pattern).map_err(|e| format!("bad regex {pattern:?}: {e}"))?;
    let listing = glob_files(root, file_glob.unwrap_or("**/*"))?;
    // The candidate list being truncated means absence of a hit proves nothing.
    // Say so on EVERY outcome, including — especially — "no matches": that is
    // the one the model acts on most confidently.
    let cap_note = if listing.capped {
        format!(
            "\n… file listing capped at {MAX_GLOB_HITS} — results may be incomplete; \
             pass a narrower glob"
        )
    } else {
        String::new()
    };

    let mut hits = Vec::new();
    for rel in &listing.files {
        let path = root.join(rel);
        if !path.is_file() {
            continue;
        }
        let Ok(body) = std::fs::read(&path) else {
            continue;
        };
        let Ok(text) = String::from_utf8(body) else {
            continue; // binary
        };
        for (n, line) in text.lines().enumerate() {
            if re.is_match(line) {
                let trimmed: String = line.chars().take(300).collect();
                hits.push(format!("{rel}:{}: {}", n + 1, trimmed));
                if hits.len() >= MAX_SEARCH_HITS {
                    hits.push(format!("… capped at {MAX_SEARCH_HITS} matches"));
                    return Ok(hits.join("\n") + &cap_note);
                }
            }
        }
    }
    if hits.is_empty() {
        return Ok(format!("no matches for {pattern:?}{cap_note}"));
    }
    Ok(hits.join("\n") + &cap_note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn call(name: &str, path: &str) -> ToolCall {
        ToolCall {
            id: "tu_1".into(),
            name: name.into(),
            input: json!({ "path": path }),
        }
    }

    fn root_with_file(name: &str, body: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(name), body).unwrap();
        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    }

    #[tokio::test]
    async     fn reads_a_file_inside_the_root() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "a.txt"), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(!out.is_error);
        assert!(out.content.contains("hello"));
        assert_eq!(out.tool_use_id, "tu_1");
    }

    fn windowed(path: &str, offset: Option<u64>, limit: Option<u64>) -> ToolCall {
        let mut input = json!({ "path": path });
        if let Some(o) = offset {
            input["offset"] = json!(o);
        }
        if let Some(l) = limit {
            input["limit"] = json!(l);
        }
        ToolCall { id: "tu_1".into(), name: "read_file".into(), input }
    }

    fn numbered_root() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let body: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        fs::write(dir.path().join("big.txt"), body).unwrap();
        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    }

    #[tokio::test]
    async fn read_file_honours_offset_and_limit() {
        // The live-run defect: EYES sent offset/limit (claude-code's `Read` takes
        // them) and got the WHOLE file back — 85,381 bytes of spawn.rs for an
        // 80-line request, ~20K tokens to deliver about 1K.
        let (_d, root) = numbered_root();
        let out = exec(&windowed("big.txt", Some(20), Some(3)), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;

        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("line 20"), "{}", out.content);
        assert!(out.content.contains("line 22"));
        assert!(!out.content.contains("line 19"));
        assert!(!out.content.contains("line 23"));
        assert!(out.content.contains("showing lines 20-22 of 100"));
    }

    #[tokio::test]
    async fn read_file_numbers_lines_so_search_hits_are_referenceable() {
        let (_d, root) = numbered_root();
        let out = exec(&windowed("big.txt", Some(7), Some(1)), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.content.contains("7\tline 7"), "{}", out.content);
    }

    #[tokio::test]
    async fn read_file_accepts_a_stringified_number() {
        // Models emit "20" as readily as 20; silently ignoring it is the bug.
        let (_d, root) = numbered_root();
        let mut c = windowed("big.txt", None, None);
        c.input["offset"] = json!("20");
        c.input["limit"] = json!("2");
        let out = exec(&c, Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.content.contains("line 20"));
        assert!(!out.content.contains("line 22"));
    }

    #[tokio::test]
    async fn read_file_limit_past_the_end_is_not_an_error() {
        let (_d, root) = numbered_root();
        let out = exec(&windowed("big.txt", Some(99), Some(500)), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("line 100"));
    }

    #[tokio::test]
    async fn read_file_offset_past_the_end_says_so() {
        let (_d, root) = numbered_root();
        let out = exec(&windowed("big.txt", Some(500), None), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.is_error);
        assert!(out.content.contains("past the end"));
    }

    #[tokio::test]
    async fn read_file_without_a_window_still_returns_everything() {
        let (_d, root) = numbered_root();
        let out = exec(&windowed("big.txt", None, None), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.content.contains("line 1\n"));
        assert!(out.content.contains("line 100"));
    }

    #[test]
    fn the_read_file_schema_declares_the_window_params() {
        // An undeclared param is what the model silently loses.
        let def = tool_defs()
            .into_iter()
            .find(|d| d["name"] == "read_file")
            .unwrap();
        let props = &def["input_schema"]["properties"];
        assert!(props["offset"].is_object(), "offset undeclared");
        assert!(props["limit"].is_object(), "limit undeclared");
    }

    #[tokio::test]
    async     fn refuses_a_dotdot_escape() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "../../etc/hosts"), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.is_error, "`..` must not escape the root");
        // The message must name the SCOPE, not the filesystem. "No such file or
        // directory" invites a retry with another path; "above the repository root"
        // tells the agent the boundary exists.
        assert!(
            out.content.contains("above the repository root"),
            "misleading refusal: {}",
            out.content
        );
    }

    #[test]
    fn joining_then_starts_with_does_not_catch_a_dotdot_escape() {
        // Documents why the pre-check is lexical-per-component rather than the
        // obvious `root.join(rel).starts_with(root)`. `Path::starts_with` compares
        // COMPONENTS without normalising, so `/root/../foo` really does start with
        // `/root` — the naive check returns true and lets the escape through.
        let root = Path::new("/root");
        assert!(
            root.join("../foo").starts_with(root),
            "if this ever becomes false, the naive pre-check would be viable"
        );
        // The component scan is what actually catches it.
        assert!("../foo".split('/').any(|s| s == ".."));
    }

    #[tokio::test]
    async fn an_existing_escaping_path_is_still_refused_with_a_scope_message() {
        // The original bug only surfaced because `../Cargo.toml` happened not to
        // exist. Point at something that definitely does.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "../"), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.is_error);
        assert!(out.content.contains("above the repository root"), "{}", out.content);
    }

    #[tokio::test]
    async     fn refuses_an_absolute_path() {
        // `Path::join` lets an absolute component replace the base outright —
        // the canonicalized-prefix check is what catches it.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "/etc/hosts"), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.is_error, "an absolute path must not replace the root");
        assert!(out.content.contains("outside the repository root"), "{}", out.content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_a_symlink_pointing_outside_the_root() {
        // A string prefix check passes this; canonicalizing the candidate is
        // what makes the gate real.
        let (dir, root) = root_with_file("a.txt", "hello");
        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "classified").unwrap();
        std::os::unix::fs::symlink(&secret, dir.path().join("link.txt")).unwrap();

        let out = exec(&call("read_file", "link.txt"), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.is_error, "a symlink out of the root must be refused");
        assert!(!out.content.contains("classified"));
    }

    #[tokio::test]
    async     fn missing_path_argument_is_a_readable_error_not_a_panic() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "read_file".into(),
                input: json!({}),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;
        assert!(out.is_error);
        assert!(out.content.contains("path"));
    }

    #[tokio::test]
    async     fn no_root_refuses_every_read_rather_than_falling_back() {
        // The B4 defect: a repo-less session used to fall back to the process
        // cwd, which is bot-hq's own data dir — `.local/mcp-token`,
        // `bot-hq.db` with every auth token, and the whole Context Library.
        let out = exec(&call("read_file", "Cargo.toml"), None, ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.is_error);
        assert!(out.content.contains("no working repository"));
    }

    #[test]
    fn no_root_means_the_tool_is_not_even_offered() {
        // Belt and braces: refusing at exec time is the guarantee, but the model
        // should not be told the tool exists in the first place.
        assert!(tool_defs_for(None, ToolPolicy::ReadOnly).is_empty());
        let dir = TempDir::new().unwrap();
        assert!(!tool_defs_for(Some(dir.path()), ToolPolicy::ReadOnly).is_empty());
    }

    #[tokio::test]
    async fn an_oversized_file_is_truncated_visibly_and_says_how_to_continue() {
        // Behaviour changed with offset/limit. Refusing an oversized read outright
        // used to be a dead end — there was no way to ask for less. Now it stops at
        // the cap and names the remedy. The property that matters is unchanged:
        // truncation must never be SILENT, or the model reasons confidently about a
        // file it only partly saw.
        let dir = TempDir::new().unwrap();
        let body: String = (1..=40_000)
            .map(|n| format!("line {n} padding padding padding\n"))
            .collect();
        assert!(body.len() > MAX_FILE_BYTES);
        fs::write(dir.path().join("big.txt"), body).unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(&call("read_file", "big.txt"), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("truncated"), "truncation must be visible");
        assert!(
            out.content.contains("offset/limit"),
            "truncation must name the way forward"
        );
        assert!(out.content.len() < MAX_FILE_BYTES + 4096);
    }

    #[tokio::test]
    async     fn non_utf8_is_reported_not_lossily_decoded() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bin"), [0xff, 0xfe, 0x00]).unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(&call("read_file", "bin"), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.is_error);
        assert!(out.content.contains("UTF-8"));
    }

    #[tokio::test]
    async     fn unknown_tool_is_an_error_outcome() {
        let (_d, root) = root_with_file("a.txt", "hello");
        // A name that is not a built-in at all. `write_file` IS one — it's refused
        // by role, which is a different message and a different test.
        let out = exec(&call("nuke_everything", "a.txt"), Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        assert!(out.is_error);
        assert!(out.content.contains("unknown tool"));
    }

    // ---- role filtering -------------------------------------------------

    #[test]
    fn eyes_is_never_offered_a_write_tool() {
        let dir = TempDir::new().unwrap();
        let offered: Vec<String> = tool_defs_for(Some(dir.path()), ToolPolicy::ReadOnly)
            .iter()
            .map(|d| d["name"].as_str().unwrap().to_string())
            .collect();

        assert!(offered.contains(&"read_file".to_string()));
        assert!(offered.contains(&"search_files".to_string()));
        assert!(offered.contains(&"list_files".to_string()));
        assert!(offered.contains(&"run_command".to_string()));
        assert!(
            !offered.contains(&"write_file".to_string()),
            "a write tool was advertised to EYES: {offered:?}"
        );

        // Every write tool must be absent, not just the ones named above.
        for (name, access) in TOOLS {
            if *access == Access::Write {
                assert!(!offered.contains(&name.to_string()), "{name} leaked");
            }
        }
    }

    #[tokio::test]
    async fn a_write_tool_is_refused_even_when_invented_by_name() {
        // Filtering the advertised list is not the guarantee — the exec-time gate
        // is. A model that names a tool it was never offered must still be refused.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "write_file".into(),
                input: json!({ "path": "a.txt", "content": "overwritten" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(out.is_error);
        assert!(out.content.contains("role boundary"));
        // And the file is untouched.
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "hello");
    }

    #[tokio::test]
    async fn read_write_policy_can_actually_write() {
        // Proves the filter is a filter, not a constant.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "write_file".into(),
                input: json!({ "path": "a.txt", "content": "new" }),
            },
            Some(&root),
            ToolPolicy::ReadWrite,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(!out.is_error, "{}", out.content);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "new");
    }

    #[tokio::test]
    async fn a_write_cannot_escape_the_root_even_under_read_write() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "write_file".into(),
                input: json!({ "path": "../escaped.txt", "content": "x" }),
            },
            Some(&root),
            ToolPolicy::ReadWrite,
            CommandPolicy::ReadOnly,
        )
        .await;
        assert!(out.is_error);
        assert!(!root.parent().unwrap().join("escaped.txt").exists());
    }

    #[test]
    fn every_agent_currently_resolves_to_read_only() {
        for agent in ["rain", "brian", "unknown"] {
            assert_eq!(ToolPolicy::for_agent(agent), ToolPolicy::ReadOnly);
        }
    }

    #[tokio::test]
    async fn run_command_obeys_the_agents_own_command_policy() {
        // The contradiction finding 11 named: `CommandPolicy::for_agent` said an
        // agent without a role gets NO shell and its tests asserted that, while
        // production derived the command policy from `ToolPolicy` through two
        // identical arms and handed that agent a read-only one. `exec` now takes
        // the command policy directly, so `None` actually means none.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "run_command".into(),
                input: json!({ "command": "cat a.txt" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::None,
        )
        .await;

        assert!(out.is_error, "CommandPolicy::None must refuse: {}", out.content);
        assert!(!out.content.contains("hello"), "the command ran anyway");

        // …and the same call under the EYES policy still works, so this is a
        // gate rather than a blanket refusal.
        let ok = exec(
            &ToolCall {
                id: "t".into(),
                name: "run_command".into(),
                input: json!({ "command": "cat a.txt" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;
        assert!(!ok.is_error, "{}", ok.content);
        assert!(ok.content.contains("hello"));
    }

    #[test]
    fn the_two_policy_sources_now_agree() {
        // They disagreed for every agent without a role: `CommandPolicy` said
        // None, the shipped derivation said ReadOnly.
        for agent in ["rain", "brian", "nobody"] {
            let via_role = crate::agents::AgentRole::for_agent(agent)
                .map(|r| r.command_policy())
                .unwrap_or(CommandPolicy::None);
            assert_eq!(CommandPolicy::for_agent(agent), via_role, "{agent}");
        }
        assert_eq!(CommandPolicy::for_agent("nobody"), CommandPolicy::None);
    }

    // ---- search_files / list_files ---------------------------------------

    /// A real git repo with an ignored mass — the shape that defeated the
    /// hardcoded prune list. Returns (dir, canonical root).
    fn git_root_with_ignored_mass() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .expect("git must be available for this test");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "t"]);

        // A 2.4 GB `bench/` is what actually broke this repo — no universal
        // prune list would name it, but the repo's own .gitignore does.
        fs::write(dir.path().join(".gitignore"), "bench/\n").unwrap();
        fs::create_dir_all(dir.path().join("bench")).unwrap();
        for i in 0..(MAX_GLOB_HITS + 200) {
            fs::write(dir.path().join("bench").join(format!("b{i:04}.rs")), "junk").unwrap();
        }
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/agent.rs"), "fn spawn_native_agent() {}\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "init"]);

        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    }

    #[tokio::test]
    async fn enumeration_uses_the_repository_not_a_hardcoded_prune_list() {
        // The regression that verification caught: pruning `.git`/`node_modules`/
        // `target` by name STILL left 48,039 entries ahead of `src/` in this repo,
        // because the mass was a gitignored `bench/` no fixed list would name.
        // The repo already publishes which files matter — use that.
        let (_d, root) = git_root_with_ignored_mass();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "search_files".into(),
                input: json!({ "pattern": "spawn_native_agent" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("src/agent.rs:1:"),
            "gitignored mass defeated enumeration again: {}",
            out.content
        );
        assert!(
            !out.content.contains("capped"),
            "the ignored files should never have been candidates: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_globless_search_is_not_defeated_by_junk_directories() {
        // The live failure: `.git` + `node_modules` + `target` alphabetically
        // precede `src`, so enumeration spent its whole 500-entry budget inside
        // them and `search_files` with no glob answered "no matches" for content
        // sitting in plain sight (measured on this repo: 66,667 entries walked
        // before the first `src/` path). Pruning at enumeration is the fix —
        // this tree reproduces the failure shape in miniature.
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        for i in 0..10 {
            fs::write(dir.path().join(".git").join(format!("g{i}")), "x").unwrap();
        }
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        for i in 0..(MAX_GLOB_HITS + 100) {
            fs::write(
                dir.path().join("node_modules").join(format!("m{i:04}.js")),
                "junk",
            )
            .unwrap();
        }
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/agent.rs"), "fn spawn_native_agent() {}\n").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "search_files".into(),
                input: json!({ "pattern": "spawn_native_agent" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("src/agent.rs:1:"),
            "the match must be found despite the junk mass: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_capped_listing_is_visible_from_search_files_not_a_bare_no_matches() {
        // When the candidate list is truncated, absence of a hit proves nothing —
        // the old version swallowed the cap sentinel as a bogus path and reported
        // a confident "no matches".
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("aaa")).unwrap();
        for i in 0..(MAX_GLOB_HITS + 50) {
            fs::write(dir.path().join("aaa").join(format!("f{i:04}.txt")), "hay").unwrap();
        }
        let root = dir.path().canonicalize().unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "search_files".into(),
                input: json!({ "pattern": "needle-not-present" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("no matches"), "{}", out.content);
        assert!(
            out.content.contains("capped") && out.content.contains("narrower glob"),
            "a truncated candidate list must be visible, or 'no matches' is a lie: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn list_files_prunes_vcs_and_dependency_directories() {
        let dir = TempDir::new().unwrap();
        for junk in [".git", "node_modules", "target"] {
            fs::create_dir_all(dir.path().join(junk)).unwrap();
            fs::write(dir.path().join(junk).join("inside.rs"), "").unwrap();
        }
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a.rs"), "").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "list_files".into(),
                input: json!({ "pattern": "**/*.rs" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("src/a.rs"));
        for junk in [".git", "node_modules", "target"] {
            assert!(
                !out.content.contains(junk),
                "{junk} must be pruned from enumeration: {}",
                out.content
            );
        }
    }

    #[tokio::test]
    async fn a_star_does_not_cross_directory_separators() {
        // Filesystem-glob semantics: `*` stays within one component; `**` is the
        // recursive form. `glob::Pattern`'s default would let `*` match `/`,
        // silently turning every shallow glob into a recursive one.
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("top.rs"), "").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/deep.rs"), "").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "list_files".into(),
                input: json!({ "pattern": "*.rs" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(out.content.contains("top.rs"), "{}", out.content);
        assert!(
            !out.content.contains("deep.rs"),
            "`*` crossed a directory separator: {}",
            out.content
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn enumeration_does_not_descend_a_symlinked_directory() {
        // glob's walker followed directory symlinks — a link out of the root
        // walked OUTSIDE trees into the listing, and a link back in loops.
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.txt"), "classified").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
        fs::write(dir.path().join("here.txt"), "").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "list_files".into(),
                input: json!({ "pattern": "**/*" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(out.content.contains("here.txt"));
        assert!(
            !out.content.contains("secret.txt"),
            "a symlinked directory was descended: {}",
            out.content
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_write_through_a_symlink_cannot_escape_the_root() {
        // `fs::write` follows symlinks. The parent-only check passed a link
        // INSIDE the root pointing OUTSIDE it, so the write landed outside —
        // the read path already canonicalizes the target; now the write path
        // does too when the target exists.
        let (dir, root) = root_with_file("a.txt", "hello");
        let outside = TempDir::new().unwrap();
        let victim = outside.path().join("victim.txt");
        fs::write(&victim, "original").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join("link.txt")).unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "write_file".into(),
                input: json!({ "path": "link.txt", "content": "PWNED" }),
            },
            Some(&root),
            ToolPolicy::ReadWrite,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(out.is_error, "a symlink out of the root must refuse the write");
        assert_eq!(
            fs::read_to_string(&victim).unwrap(),
            "original",
            "the outside file was modified"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_write_through_a_dangling_symlink_cannot_create_an_outside_file() {
        // A dangling link inside the root pointing at a nonexistent OUTSIDE path:
        // writing through it would CREATE the outside file.
        let (dir, root) = root_with_file("a.txt", "hello");
        let outside = TempDir::new().unwrap();
        let target = outside.path().join("created.txt");
        std::os::unix::fs::symlink(&target, dir.path().join("dangling.txt")).unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "write_file".into(),
                input: json!({ "path": "dangling.txt", "content": "PWNED" }),
            },
            Some(&root),
            ToolPolicy::ReadWrite,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(out.is_error);
        assert!(!target.exists(), "the write escaped through a dangling link");
    }

    #[tokio::test]
    async fn search_files_returns_path_line_and_text() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "fn one() {}\nfn two() {}\n").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "search_files".into(),
                input: json!({ "pattern": r"fn two" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;

        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("a.rs:2:"), "{}", out.content);
        assert!(!out.content.contains("fn one"));
    }

    #[tokio::test]
    async fn search_files_reports_a_bad_regex_instead_of_panicking() {
        let (_d, root) = root_with_file("a.txt", "x");
        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "search_files".into(),
                input: json!({ "pattern": "(unclosed" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;
        assert!(out.is_error);
        assert!(out.content.contains("bad regex"));
    }

    #[tokio::test]
    async fn list_files_matches_a_glob_and_refuses_an_escaping_one() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "").unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let hit = exec(
            &ToolCall {
                id: "t".into(),
                name: "list_files".into(),
                input: json!({ "pattern": "src/**/*.rs" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;
        assert!(!hit.is_error, "{}", hit.content);
        assert!(hit.content.contains("src/main.rs"));
        assert!(hit.content.contains("src/lib.rs"));

        for bad in ["../**/*", "/etc/*"] {
            let out = exec(
                &ToolCall {
                    id: "t".into(),
                    name: "list_files".into(),
                    input: json!({ "pattern": bad }),
                },
                Some(&root),
                ToolPolicy::ReadOnly,
                CommandPolicy::ReadOnly,
            )
            .await;
            assert!(out.is_error, "glob {bad} should be refused");
        }
    }

    #[tokio::test]
    async fn run_command_refuses_a_mutation_through_the_tool_layer() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "run_command".into(),
                input: json!({ "command": "rm a.txt" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
            CommandPolicy::ReadOnly,
        )
        .await;
        assert!(out.is_error);
        assert!(root.join("a.txt").exists(), "the file was deleted");
    }

    #[test]
    fn handles_reports_only_what_exec_implements() {
        assert!(handles("read_file"));
        assert!(!handles("Bash"));
        assert!(handles("write_file"));
        assert!(!handles("cl_index_search"));
    }

    #[test]
    fn tool_defs_use_the_messages_api_schema_key() {
        for def in tool_defs() {
            assert!(def["name"].is_string());
            assert!(def["input_schema"].is_object(), "{}", def["name"]);
            assert!(def.get("inputSchema").is_none());
        }
    }
}
