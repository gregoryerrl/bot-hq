//! What a participant WITHOUT `Capability::EditFiles` may not do — one list,
//! two enforcers.
//!
//! On claude-code the list is enforced by the CLI's own permission layer:
//! `build_command` renders it as `--disallowedTools` under `--permission-mode
//! dontAsk` ([`claude_disallowed_tools`]). On Codex there is no such layer —
//! a participant runs under `danger-full-access` with approvals off — so the
//! PreToolUse hook (`policy-check codex-gate`) is the ONLY enforcer, and it
//! decides from the same constants through [`command_denial`] and
//! [`tool_denial`]. The two renderings are derived from one source so they
//! cannot drift (the "derive, don't restate" rule: a test that restates the
//! list has pinned a falsehood before).
//!
//! The Codex matcher has to do what claude-code's prefix patterns do for free
//! and more: it looks at every command a line RUNS (through `&&`, `;`, `|`,
//! `sh -c '…'`, `env`, `$(…)` — `signaling::commands_run`) and skips git's
//! global options (`git -C . push`, `git -c k=v push`), because a reviewer
//! that can type `cd x && git push` has every chance to.

/// Mutating `git branch` forms. Read forms (bare / `--show-current` / `-a` /
/// `-r` / `--list` / `--contains`) fall through. 2026-06-17: replaced the
/// blanket `Bash(git branch:*)` deny that was false-blocking EYES's read-only
/// listing.
pub const GIT_BRANCH_WRITE_VERBS: &[&str] = &[
    "-d",
    "-D",
    "--delete",
    "-m",
    "-M",
    "--move",
    "-c",
    "-C",
    "--copy",
    "-f",
    "--force",
    "-u",
    "--set-upstream-to",
    "--unset-upstream",
    "--track",
    "--no-track",
    "--edit-description",
];
/// Full-noun git mutations — no read form worth preserving, so the noun is
/// denied. `commit` and `push` first: they are the two every reviewer tries.
pub const GIT_DENIED_NOUNS: &[&str] = &[
    "commit",
    "push",
    "checkout",
    "switch",
    "reset",
    "merge",
    "rebase",
    "add",
    "stash",
    "restore",
    "rm",
    "tag",
    "cherry-pick",
    "apply",
];
/// Mutating `gh pr` verbs (read forms — view/diff/list/status/checks — fall through).
pub const GH_PR_WRITE_VERBS: &[&str] = &[
    "create", "edit", "close", "reopen", "merge", "ready", "review", "comment", "lock", "unlock",
    "delete", "checkout",
];
/// Mutating `gh issue` verbs (read forms — view/list — fall through).
pub const GH_ISSUE_WRITE_VERBS: &[&str] = &[
    "create", "edit", "close", "reopen", "comment", "delete", "transfer", "pin", "unpin", "lock",
    "unlock", "develop",
];
/// Mutating `gh release` verbs (read forms — view/list — fall through).
pub const GH_RELEASE_WRITE_VERBS: &[&str] = &["create", "edit", "delete", "upload", "download"];
/// Mutating `gh repo` verbs (read forms — view — fall through).
pub const GH_REPO_WRITE_VERBS: &[&str] = &[
    "create", "edit", "delete", "fork", "sync", "rename", "archive", "clone",
];
/// The `gh` nouns denied by write verb, in rendering order.
pub const GH_WRITE_VERBS: &[(&str, &[&str])] = &[
    ("pr", GH_PR_WRITE_VERBS),
    ("issue", GH_ISSUE_WRITE_VERBS),
    ("release", GH_RELEASE_WRITE_VERBS),
    ("repo", GH_REPO_WRITE_VERBS),
];
/// Tool names a read-only role may not call: claude-code's file editors and
/// its sub-agent tool, and Codex's file editor (`apply_patch`, which the
/// hook also sees as `Edit`/`Write` per the Codex hooks doc).
pub const DENIED_TOOL_NAMES: &[&str] = &["Edit", "Write", "NotebookEdit", "Task", "apply_patch"];

/// Space-joined `Bash(<tool> <verb>:*)` deny patterns for a deny-by-write-verb
/// tool. Read forms (no listed verb) fall through to allowed `Bash`.
fn deny_write_verbs(tool: &str, verbs: &[&str]) -> String {
    verbs
        .iter()
        .map(|v| format!("Bash({tool} {v}:*)"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// claude-code's `--disallowedTools` value for a read-only role: the static
/// denies (Edit/Write/NotebookEdit/Task + `git commit` / `git push`), the
/// verb-denied `git branch`, the full-noun git mutations, the verb-denied `gh`
/// nouns, and `gh api` — the POST/PATCH/DELETE escape hatch, fully denied.
/// Tools with a read form worth keeping (`git branch`, `gh`) are denied BY
/// WRITE VERB rather than by blanket noun, so the read forms fall through to
/// the allowed `Bash` (deny wins over allow).
pub fn claude_disallowed_tools() -> String {
    let mut parts: Vec<String> = ["Edit", "Write", "NotebookEdit", "Task"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // `commit` and `push` come before the branch verbs, as the list always
    // read; the rest of the nouns follow it.
    parts.push("Bash(git commit:*)".to_string());
    parts.push("Bash(git push:*)".to_string());
    parts.push(deny_write_verbs("git branch", GIT_BRANCH_WRITE_VERBS));
    parts.extend(
        GIT_DENIED_NOUNS
            .iter()
            .filter(|n| !matches!(**n, "commit" | "push"))
            .map(|n| format!("Bash(git {n}:*)")),
    );
    for (noun, verbs) in GH_WRITE_VERBS {
        parts.push(deny_write_verbs(&format!("gh {noun}"), verbs));
    }
    parts.push("Bash(gh api:*)".to_string());
    parts.join(" ")
}

/// A denied tool call, as the Codex hook refuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    /// The command the line runs that was denied (one of several, maybe), or
    /// the tool name.
    pub command: String,
    /// The rule it hit, as the deny list spells it (`git push`, `gh pr merge`).
    pub rule: String,
}

/// Whether a read-only role may call a tool of this name at all.
pub fn tool_denial(tool_name: &str) -> Option<Denial> {
    DENIED_TOOL_NAMES
        .iter()
        .find(|n| **n == tool_name)
        .map(|n| Denial { command: n.to_string(), rule: format!("the {n} tool") })
}

/// The first command a Bash line RUNS that a read-only role may not run —
/// through `&&`, `;`, `|`, `sh -c '…'`, `env`, `$(…)` (`signaling::commands_run`)
/// and past git's and gh's global options — or `None` when every command the
/// line runs is a read.
pub fn command_denial(command: &str) -> Option<Denial> {
    for run in crate::signaling::commands_run(command) {
        if let Some(rule) = rule_for(&run.tool, &run.args, 0) {
            let mut words = run.prefix.clone();
            words.push(run.tool.clone());
            words.extend(run.args.iter().cloned());
            return Some(Denial { command: words.join(" "), rule });
        }
    }
    None
}

/// The rule a `(tool, args)` hits, looking through the wrappers the splitter
/// leaves in place (`xargs git rm`, `timeout 30 git push`, `nohup`, `nice`,
/// `command`, `exec`, `sudo`): the wrapped command is the one examined.
fn rule_for(tool: &str, args: &[String], depth: usize) -> Option<String> {
    match tool {
        "git" => git_rule(args),
        "gh" => gh_rule(args),
        _ if depth < 4 => {
            let (inner, rest) = unwrap_wrapper(tool, args)?;
            rule_for(&inner, rest, depth + 1)
        }
        _ => None,
    }
}

/// Wrapper options that take a value as the next word, per wrapper.
const WRAPPER_VALUE_FLAGS: &[(&str, &[&str])] = &[
    ("xargs", &["-I", "-n", "-P", "-L", "-d", "-s", "-E", "-a", "--max-args", "--max-procs", "--delimiter", "--arg-file", "--replace"]),
    ("nice", &["-n", "--adjustment"]),
    ("timeout", &["-s", "--signal", "-k", "--kill-after"]),
    ("sudo", &["-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-T"]),
    ("doas", &["-u", "-C"]),
    ("watch", &["-n", "--interval", "-d"]),
];

/// `(wrapped tool, its args)` when `tool` is a wrapper that runs its
/// remaining words as a command.
fn unwrap_wrapper<'a>(tool: &str, args: &'a [String]) -> Option<(String, &'a [String])> {
    let known = matches!(
        tool,
        "xargs" | "nohup" | "timeout" | "nice" | "time" | "command" | "exec" | "builtin" | "sudo"
            | "doas" | "watch" | "env"
    );
    if !known {
        return None;
    }
    let value_flags = WRAPPER_VALUE_FLAGS
        .iter()
        .find(|(w, _)| *w == tool)
        .map(|(_, f)| *f)
        .unwrap_or(&[]);
    let mut i = 0;
    // `timeout DURATION cmd`: the duration is a positional word.
    let mut positional_to_skip = usize::from(tool == "timeout");
    while i < args.len() {
        let a = args[i].as_str();
        if value_flags.contains(&a) {
            i += 2;
            continue;
        }
        if a.starts_with('-') || (tool == "env" && a.contains('=')) {
            i += 1;
            continue;
        }
        if positional_to_skip > 0 {
            positional_to_skip -= 1;
            i += 1;
            continue;
        }
        let inner = a.rsplit('/').next().unwrap_or(a).to_string();
        return Some((inner, &args[i + 1..]));
    }
    None
}

/// git's global options that take a value as the NEXT word (`git -C dir push`).
const GIT_GLOBAL_WITH_VALUE: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--exec-path",
    "--super-prefix",
    "--config-env",
    "--list-cmds",
];

fn git_rule(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if GIT_GLOBAL_WITH_VALUE.contains(&a) {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            // `--git-dir=x`, `--no-pager`, `-p`, `-Cdir`: one word, skipped.
            i += 1;
            continue;
        }
        let verb = a;
        if GIT_DENIED_NOUNS.contains(&verb) {
            return Some(format!("git {verb}"));
        }
        if verb == "branch" {
            return args[i + 1..].iter().find_map(|w| {
                GIT_BRANCH_WRITE_VERBS
                    .iter()
                    .find(|v| w == *v || w.starts_with(&format!("{v}=")))
                    .map(|v| format!("git branch {v}"))
            });
        }
        return None;
    }
    None
}

/// gh's global options that take a value as the next word.
const GH_GLOBAL_WITH_VALUE: &[&str] = &["-R", "--repo", "--hostname"];

fn gh_rule(args: &[String]) -> Option<String> {
    let mut words = args.iter().map(String::as_str);
    let mut noun = None;
    while let Some(a) = words.next() {
        if GH_GLOBAL_WITH_VALUE.contains(&a) {
            words.next();
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        noun = Some(a);
        break;
    }
    let noun = noun?;
    if noun == "api" {
        return Some("gh api".to_string());
    }
    let (_, verbs) = GH_WRITE_VERBS.iter().find(|(n, _)| *n == noun)?;
    let verb = words.find(|a| !a.starts_with('-'))?;
    verbs.contains(&verb).then(|| format!("gh {noun} {verb}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claude rendering is byte-for-byte what `build_command` shipped
    /// before the list moved here (`eyes_denies_*` in `agents::spawn` iterate
    /// the same consts): static denies, the verb-denied `git branch`, the
    /// full-noun git mutations, the verb-denied `gh` nouns, and `gh api`.
    #[test]
    fn the_claude_rendering_is_the_shipped_disallowed_list() {
        let list = claude_disallowed_tools();
        let padded = format!(" {list} ");
        for fixed in ["Edit", "Write", "NotebookEdit", "Task", "Bash(git commit:*)", "Bash(git push:*)", "Bash(gh api:*)"] {
            assert!(padded.contains(&format!(" {fixed} ")), "{fixed} missing from {list}");
        }
        for verb in GIT_BRANCH_WRITE_VERBS {
            assert!(list.contains(&format!("Bash(git branch {verb}:*)")), "{verb}");
        }
        for noun in GIT_DENIED_NOUNS {
            assert!(list.contains(&format!("Bash(git {noun}:*)")), "{noun}");
        }
        for (noun, verbs) in GH_WRITE_VERBS {
            for verb in *verbs {
                assert!(list.contains(&format!("Bash(gh {noun} {verb}:*)")), "gh {noun} {verb}");
            }
        }
        // Read forms are NOT denied: a blanket noun would kill them.
        assert!(!list.contains("Bash(git branch:*)"));
        assert!(!list.contains("Bash(gh pr:*)"));
        assert!(!list.contains("Bash(git status"));
    }

    /// Claude's own tool names and Codex's: a file edit is denied whatever
    /// the connector calls it.
    #[test]
    fn a_read_only_role_may_not_edit_through_any_tool_name() {
        for name in ["Edit", "Write", "NotebookEdit", "Task", "apply_patch"] {
            assert!(tool_denial(name).is_some(), "{name} must be denied");
        }
        for name in ["Read", "Grep", "Bash", "mcp__bot-hq-signaling__session_doc_write", "exec_command"] {
            assert_eq!(tool_denial(name), None, "{name} is not a file edit");
        }
    }

    /// The forms EYES listed (plan-eyes P5): every command the line RUNS is
    /// examined, so a shell, a wrapper, a `cd` or git's global options in
    /// front of the verb change nothing.
    #[test]
    fn a_denied_git_verb_is_found_wherever_the_line_runs_it() {
        let denied = [
            "git push",
            "git push origin main",
            "cd /tmp/x && git push",
            "echo a; git push",
            "sh -c 'git push'",
            "bash -c \"git commit -m x\"",
            "env git push",
            "env GIT_TRACE=1 git push",
            "git -C . push",
            "git -c core.editor=true commit -m x",
            "git --no-pager -C /tmp/x --git-dir=/tmp/x/.git push",
            "git branch -D old",
            "git branch --set-upstream-to=origin/main",
            "git checkout -b feature",
            "git stash",
            "git tag v1",
            "git cherry-pick abc",
            "git apply p.diff",
            // The full-noun denies take the noun's read forms with them, on
            // claude-code (`Bash(git stash:*)`) and so here: parity, not taste.
            "git stash list",
            "true && (git reset --hard HEAD)",
            "ls | xargs git rm",
            "$(git push)",
        ];
        for cmd in denied {
            let d = command_denial(cmd);
            assert!(d.is_some(), "{cmd:?} must be denied");
        }
        let allowed = [
            "git status",
            "git log --oneline -5",
            "git diff HEAD~1",
            "git show HEAD:README.md",
            "git branch",
            "git branch --show-current",
            "git branch -a",
            "git branch --list 'codex*'",
            "git branch --contains abc",
            "git -C /tmp/x status",
            "git fetch origin",
            "git rev-parse HEAD",
            "echo 'git push' > notes.txt",
            "grep -n 'git push' src/*.rs",
            "cat git-push.log",
            "ls",
        ];
        for cmd in allowed {
            assert_eq!(command_denial(cmd), None, "{cmd:?} is a read");
        }
    }

    /// `gh` by write verb: the reads fall through, every write verb and `gh api`
    /// (the POST/PATCH escape hatch) are denied, global flags before the noun
    /// skipped.
    #[test]
    fn a_denied_gh_verb_is_found_and_the_reads_fall_through() {
        let denied = [
            "gh pr merge 12",
            "gh pr create --title x",
            "gh -R owner/repo pr comment 12 --body hi",
            "gh --repo owner/repo issue close 3",
            "gh release create v1",
            "gh repo delete x",
            "gh api repos/o/r/issues -f title=x",
            "gh api -X PATCH repos/o/r",
            "cd x && gh pr merge 1",
            "sh -c 'gh issue create -t x'",
        ];
        for cmd in denied {
            assert!(command_denial(cmd).is_some(), "{cmd:?} must be denied");
        }
        let allowed = [
            "gh pr view 12",
            "gh pr list",
            "gh pr diff 12",
            "gh pr checks 12",
            "gh issue view 3",
            "gh issue list --state open",
            "gh release view v1",
            "gh repo view",
            "gh run list",
            "gh auth status",
        ];
        for cmd in allowed {
            assert_eq!(command_denial(cmd), None, "{cmd:?} is a read");
        }
    }

    /// The refusal names the command it found and the rule, so the reviewer
    /// learns what was denied rather than guessing from a bare exit code.
    #[test]
    fn a_denial_names_the_offending_command() {
        let d = command_denial("cd x && git -C . push origin main").expect("denied");
        assert_eq!(d.command, "git -C . push origin main");
        assert!(d.rule.contains("git push"), "{}", d.rule);
        let d = command_denial("gh pr merge 1").expect("denied");
        assert!(d.rule.contains("gh pr merge"), "{}", d.rule);
        let d = command_denial("gh api -X POST x").expect("denied");
        assert!(d.rule.contains("gh api"), "{}", d.rule);
    }
}
