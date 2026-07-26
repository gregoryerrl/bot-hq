//! Command execution for native agents — **allow-list, no shell**.
//!
//! ## Why an allow-list rather than a deny-list
//!
//! claude-code's `--disallowedTools` is fail-open: EYES is given `Bash` broadly
//! and specific git/gh mutations are subtracted, so `rm -rf`, `mv`, `chmod`,
//! `npm install`, `curl … | sh` and `psql -c "DELETE …"` all run. Today those are
//! forbidden only by the role prompt, with no mechanism behind it. Porting that
//! shape into code we own would rebuild the hole where enforcement is free.
//!
//! An allow-list is also *easier to get right*: a deny-list must anticipate every
//! mutation verb that will ever exist, while an allow-list enumerates the handful
//! of read commands a reviewer actually needs.
//!
//! ## Why no shell
//!
//! Nothing interprets the string as shell. Tokens are split here and handed to
//! `Command` as argv, so `&&`, `;`, `|`, `$( )`, backticks and `>` cannot chain or
//! redirect. This is what makes the allow-list hold: a deny-list over a shell
//! string is defeated by `git log && git push`, and substring matching is defeated
//! by `git  push` (two spaces) or `git -C . push`.
//!
//! ## Layers
//!
//! 1. Refuse shell metacharacters outright (they'd be meaningless as argv, and
//!    refusing explains why instead of producing a baffling error).
//! 2. Refuse path arguments that escape the root — absolute paths and `..`
//!    components. `git`'s `-C` / `--git-dir` / `--work-tree` are refused for the
//!    same reason: they retarget the repo.
//! 3. Match the program against the policy, including per-program flag rules —
//!    `find`'s action predicates and `git diff --output` write despite living on
//!    an allowed program.

use std::path::Path;
use std::time::Duration;

/// Cap captured output so one command can't blow the context window.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Wall-clock budget for one command.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Shell metacharacters that would need a shell to mean anything.
const SHELL_METACHARS: &[&str] = &[
    "&&", "||", ";", "|", ">", ">>", "<", "`", "$(", "\n", "\r", "&",
];

/// `find` predicates that ACT rather than report. Easy to miss: `find` is a read
/// tool right up until `-delete` or `-exec rm`.
const FIND_WRITE_PREDICATES: &[&str] = &[
    "-delete",
    "-exec",
    "-execdir",
    "-ok",
    "-okdir",
    "-fprint",
    "-fprint0",
    "-fprintf",
    "-fls",
];

/// `git` global flags that retarget which repository is read. A read-scope escape
/// exactly like an absolute path, so refused for the same reason.
const GIT_REPO_RETARGET_FLAGS: &[&str] = &["-C", "--git-dir", "--work-tree", "--exec-path"];

/// `git branch` flags that only REPORT. Allow-listed rather than deny-listing the
/// write verbs: a deny-list inside an allowed subcommand is fail-open again — a
/// verb git adds later would pass. Anything not here is refused, including a bare
/// branch name (`git branch foo` CREATES a branch).
const GIT_BRANCH_READ_FLAGS: &[&str] = &[
    "-a",
    "--all",
    "-r",
    "--remotes",
    "-v",
    "-vv",
    "--verbose",
    "-l",
    "--list",
    "--show-current",
    "--contains",
    "--no-contains",
    "--merged",
    "--no-merged",
    "--points-at",
    "--format",
    "--sort",
    "--color",
    "--no-color",
    "-i",
    "--ignore-case",
];

/// `git branch` read flags that consume the next token as a value.
const GIT_BRANCH_VALUE_FLAGS: &[&str] = &[
    "--contains",
    "--no-contains",
    "--merged",
    "--no-merged",
    "--points-at",
    "--format",
    "--sort",
];

/// What an agent may run.
///
/// A policy, not a property of the loop: the native loop can express any of
/// these, and which one an agent gets is a **role** decision. EYES is
/// [`Self::ReadOnly`] because EYES is a reviewer, not because the loop is
/// limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPolicy {
    /// Read-only investigation. The EYES preset.
    ReadOnly,
    /// No command execution at all.
    None,
}

impl CommandPolicy {
    /// The policy for `agent_name`. EYES gets read-only; anything else gets
    /// nothing, because no native HANDS exists yet and silently granting an
    /// unknown role a shell is the wrong default.
    pub fn for_agent(agent_name: &str) -> Self {
        match agent_name {
            "rain" => Self::ReadOnly,
            _ => Self::None,
        }
    }
}

/// Validate `command` and return its argv.
///
/// `Err` carries a message written for the model: it says what was refused and
/// why, so the agent can adapt rather than retrying blindly.
pub fn validate(command: &str, policy: CommandPolicy) -> Result<Vec<String>, String> {
    if policy == CommandPolicy::None {
        return Err("this agent may not run commands".into());
    }

    reject_shell_metachars(command)?;
    let argv = tokenize(command)?;
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| "empty command".to_string())?;

    for a in args {
        reject_escaping_path(a)?;
    }

    match program.as_str() {
        // Report-only programs: any arguments, since the path check above
        // already keeps them inside the root.
        //
        // `ps` is deliberately ABSENT. A child process inherits this process's
        // environment, and `ps eww <pid>` prints a process's environment — while
        // `spawn.rs` sets `ANTHROPIC_AUTH_TOKEN` explicitly on every claude child.
        // So `ps` would let a read-only agent lift the gateway credential out of a
        // sibling process. Reviewing code needs no process listing; the cheapest
        // correct answer is not to offer it.
        "cat" | "ls" | "wc" | "head" | "tail" | "which" | "file" | "stat" | "du" => {}

        "find" => {
            if let Some(bad) = args.iter().find(|a| FIND_WRITE_PREDICATES.contains(&a.as_str())) {
                return Err(format!(
                    "`find {bad}` acts on files rather than reporting them — refused. \
                     Use `find` with `-name`/`-type` only, or ask your peer."
                ));
            }
        }

        "git" => validate_git(args)?,
        "gh" => validate_gh(args)?,
        "npm" => require_subcommand(args, &["ls", "list", "view", "outdated"], "npm")?,
        "composer" => require_subcommand(args, &["show", "info"], "composer")?,
        "cargo" => require_subcommand(args, &["tree", "metadata"], "cargo")?,

        other => {
            return Err(format!(
                "`{other}` is not in this agent's allowed command list. Allowed: git, gh, \
                 cat, ls, wc, head, tail, find, which, file, stat, du, npm, composer, \
                 cargo (read subcommands only). Anything that could mutate state is your \
                 peer's to run — ask them."
            ));
        }
    }

    Ok(argv)
}

fn validate_git(args: &[String]) -> Result<(), String> {
    if let Some(bad) = args
        .iter()
        .find(|a| GIT_REPO_RETARGET_FLAGS.iter().any(|f| *f == a.as_str() || a.starts_with(&format!("{f}="))))
    {
        return Err(format!(
            "`git {bad}` points git at a different repository, which escapes this \
             agent's read scope — refused."
        ));
    }
    // `git diff --output=f` writes a file despite `diff` being a read verb.
    if let Some(bad) = args
        .iter()
        .find(|a| *a == "-o" || a.as_str() == "--output" || a.starts_with("--output="))
    {
        return Err(format!("`git {bad}` writes a file — refused; read the output instead."));
    }

    let sub = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "`git` needs a subcommand (log, diff, status, show, rev-list, branch)".to_string())?;

    match sub.as_str() {
        "log" | "diff" | "status" | "show" | "rev-list" | "rev-parse" | "blame" | "shortlog"
        | "describe" | "ls-files" | "ls-tree" | "cat-file" | "grep" | "config" => {
            // `git config --unset`/`--add`/`--replace-all` writes.
            if sub == "config"
                && args.iter().any(|a| {
                    matches!(
                        a.as_str(),
                        "--unset" | "--unset-all" | "--add" | "--replace-all" | "--edit" | "-e"
                    )
                })
            {
                return Err("`git config` may only be used to READ values — refused".into());
            }
            Ok(())
        }
        "branch" => validate_git_branch(args),
        other => Err(format!(
            "`git {other}` is not a read-only subcommand — refused. Allowed: log, diff, \
             status, show, rev-list, rev-parse, blame, shortlog, describe, ls-files, \
             ls-tree, cat-file, grep, branch (report flags only), config (read only)."
        )),
    }
}

fn validate_git_branch(args: &[String]) -> Result<(), String> {
    // Skip the leading global flags and the `branch` token itself.
    let mut it = args.iter().skip_while(|a| a.as_str() != "branch");
    it.next();

    let mut expect_value = false;
    for a in it {
        if expect_value {
            expect_value = false;
            continue;
        }
        let name = a.split('=').next().unwrap_or(a);
        if !a.starts_with('-') {
            return Err(format!(
                "`git branch {a}` would create or modify a branch — refused. Use \
                 `git branch --list` / `--show-current` / `-a` to report."
            ));
        }
        if !GIT_BRANCH_READ_FLAGS.contains(&name) {
            return Err(format!(
                "`git branch {a}` is not a report-only flag — refused."
            ));
        }
        if GIT_BRANCH_VALUE_FLAGS.contains(&name) && !a.contains('=') {
            expect_value = true;
        }
    }
    Ok(())
}

fn validate_gh(args: &[String]) -> Result<(), String> {
    let mut positionals = args.iter().filter(|a| !a.starts_with('-'));
    let noun = positionals
        .next()
        .ok_or_else(|| "`gh` needs a noun (issue, pr, repo, release, run)".to_string())?;
    let verb = positionals.next().map(String::as_str);

    let allowed: &[&str] = match noun.as_str() {
        "issue" => &["view", "list", "status"],
        "pr" => &["view", "list", "diff", "status", "checks"],
        "repo" => &["view", "list"],
        "release" => &["view", "list"],
        "run" => &["view", "list"],
        "api" => {
            return Err("`gh api` is the write escape hatch — refused".into());
        }
        other => {
            return Err(format!(
                "`gh {other}` is not allowed. Allowed nouns: issue, pr, repo, release, run."
            ))
        }
    };

    match verb {
        Some(v) if allowed.contains(&v) => Ok(()),
        Some(v) => Err(format!(
            "`gh {noun} {v}` is not read-only — refused. Allowed: {}.",
            allowed.join(", ")
        )),
        None => Err(format!(
            "`gh {noun}` needs a verb: {}.",
            allowed.join(", ")
        )),
    }
}

fn require_subcommand(args: &[String], allowed: &[&str], program: &str) -> Result<(), String> {
    let sub = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| format!("`{program}` needs a subcommand: {}", allowed.join(", ")))?;
    if allowed.contains(&sub.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "`{program} {sub}` is not read-only — refused. Allowed: {}.",
            allowed.join(", ")
        ))
    }
}

/// Refuse anything that would need a shell.
fn reject_shell_metachars(command: &str) -> Result<(), String> {
    let mut in_single = false;
    let mut in_double = false;
    let bytes: Vec<char> = command.chars().collect();

    for (i, c) in bytes.iter().enumerate() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ if in_single || in_double => {}
            _ => {
                let rest: String = bytes[i..].iter().collect();
                if let Some(meta) = SHELL_METACHARS.iter().find(|m| rest.starts_with(**m)) {
                    return Err(format!(
                        "`{}` needs a shell, and this agent runs commands directly with no \
                         shell — refused. Run one command at a time; no pipes, chaining or \
                         redirection. Ask your peer if you need them.",
                        meta.trim()
                    ));
                }
            }
        }
    }
    if in_single || in_double {
        return Err("unbalanced quotes in the command".into());
    }
    Ok(())
}

/// Refuse a path argument that leaves the root.
///
/// `..` is checked per path COMPONENT, not as a substring: git revision ranges
/// like `HEAD..main` are legitimate and contain `..`, while `../etc` must be
/// refused.
fn reject_escaping_path(arg: &str) -> Result<(), String> {
    // Also check the VALUE half of `--flag=value`. Without this, a bare token
    // check passes `--exclude-from=/etc/shadow` on an allowed program, because the
    // argument as a whole neither starts with `/` nor contains a `..` component.
    let candidates: [&str; 2] = match arg.split_once('=') {
        Some((_, value)) if arg.starts_with('-') => [arg, value],
        _ => [arg, arg],
    };

    for c in candidates {
        if c.starts_with('/') {
            return Err(format!(
                "`{arg}` names an absolute path, which escapes this agent's read scope — \
                 refused. Use a path relative to the repository root."
            ));
        }
        if c.split('/').any(|seg| seg == "..") {
            return Err(format!("`{arg}` walks above the repository root — refused."));
        }
    }
    Ok(())
}

/// Split into argv, honouring quotes. No expansion, no operators.
fn tokenize(command: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut in_single = false;
    let mut in_double = false;

    for c in command.chars() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                started = true;
            }
            '"' if !in_single => {
                in_double = !in_double;
                started = true;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    if out.is_empty() {
        return Err("empty command".into());
    }
    Ok(out)
}

/// Validate then run, with `root` as the working directory.
///
/// A non-zero exit is a normal result, not an error — the model reads the output
/// and decides. Only a refusal or a spawn failure is `Err`.
pub async fn run(command: &str, root: &Path, policy: CommandPolicy) -> Result<String, String> {
    let argv = validate(command, policy)?;
    let (program, args) = argv.split_first().expect("validate rejects empty argv");

    let fut = tokio::process::Command::new(program)
        .args(args)
        .current_dir(root)
        .kill_on_drop(true)
        .output();

    let out = match tokio::time::timeout(COMMAND_TIMEOUT, fut).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("could not run `{program}`: {e}")),
        Err(_) => {
            return Err(format!(
                "`{command}` exceeded {}s and was killed",
                COMMAND_TIMEOUT.as_secs()
            ))
        }
    };

    let mut body = String::new();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stdout.trim().is_empty() {
        body.push_str(&stdout);
    }
    if !stderr.trim().is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str("--- stderr ---\n");
        body.push_str(&stderr);
    }
    if !out.status.success() {
        body.push_str(&format!("\n--- exit status: {} ---", out.status));
    }
    if body.len() > MAX_OUTPUT_BYTES {
        let cut = body
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|i| *i <= MAX_OUTPUT_BYTES)
            .last()
            .unwrap_or(0);
        body.truncate(cut);
        body.push_str("\n--- output truncated ---");
    }
    if body.trim().is_empty() {
        body.push_str("(no output)");
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(cmd: &str) {
        assert!(
            validate(cmd, CommandPolicy::ReadOnly).is_ok(),
            "should be allowed: {cmd}"
        );
    }
    fn refused(cmd: &str) -> String {
        match validate(cmd, CommandPolicy::ReadOnly) {
            Ok(argv) => panic!("should have been refused: {cmd} -> {argv:?}"),
            Err(e) => e,
        }
    }

    // ---- the read set Rain's prompt already promises ---------------------

    #[test]
    fn allows_the_read_only_investigation_set() {
        for cmd in [
            "git log --oneline -5",
            "git diff HEAD~1",
            "git diff main..HEAD",
            "git status --short",
            "git show HEAD",
            "git rev-list --count HEAD",
            "git branch --show-current",
            "git branch -a",
            "git branch --contains HEAD",
            "git branch --format=%(refname)",
            "gh pr view 12",
            "gh pr diff 12",
            "gh issue list",
            "gh repo view",
            "cat Cargo.toml",
            "ls -la src",
            "wc -l src/main.rs",
            "head -20 README.md",
            "find . -name '*.rs'",
            "npm ls",
            "cargo tree",
        ] {
            ok(cmd);
        }
    }

    // ---- mutations ------------------------------------------------------

    #[test]
    fn refuses_git_and_gh_mutations() {
        for cmd in [
            "git commit -m x",
            "git push",
            "git checkout main",
            "git reset --hard",
            "git add .",
            "git rebase main",
            "git stash",
            "git tag v1",
            "git cherry-pick abc",
            "git apply p.patch",
            "gh pr create",
            "gh pr merge 1",
            "gh issue close 1",
            "gh api /repos/x",
        ] {
            refused(cmd);
        }
    }

    #[test]
    fn refuses_everything_outside_the_allow_list() {
        // The whole class the CLI deny-list lets through today.
        for cmd in [
            "rm -rf src",
            "mv a b",
            "chmod 777 x",
            "npm install",
            "composer require foo/bar",
            "psql -c 'DELETE FROM users'",
            "curl https://example.com",
            "python -c 'print(1)'",
            "sh -c 'echo hi'",
            "bash script.sh",
            "make install",
            "docker run x",
        ] {
            refused(cmd);
        }
    }

    // ---- evasion --------------------------------------------------------

    #[test]
    fn refuses_chaining_and_redirection() {
        // Each of these defeats a substring deny-list over a shell string.
        for cmd in [
            "git log && git push",
            "git log; git push",
            "git log || git push",
            "git log | git push",
            "git log > out.txt",
            "git log >> out.txt",
            "cat < in.txt",
            "echo `git push`",
            "echo $(git push)",
            "git log\ngit push",
            "git log & git push",
        ] {
            let e = refused(cmd);
            assert!(e.contains("shell"), "{cmd} -> {e}");
        }
    }

    #[test]
    fn double_spacing_does_not_slip_past() {
        // Defeats a naive `contains("git push")` check.
        refused("git  push");
    }

    #[test]
    fn refuses_repo_retargeting() {
        // `git -C /other log` reads a DIFFERENT repo — a read-scope escape.
        for cmd in [
            "git -C /other log",
            "git -C ../other log",
            "git --git-dir=/other/.git log",
            "git --work-tree=/other status",
        ] {
            refused(cmd);
        }
    }

    #[test]
    fn ps_is_not_allowed_because_it_leaks_sibling_process_environments() {
        // `spawn.rs` sets ANTHROPIC_AUTH_TOKEN on every claude child, and
        // `ps eww <pid>` prints a process's environment. A read-only agent must not
        // have a path to a sibling's credentials.
        for cmd in ["ps", "ps eww", "ps aux", "ps -ef"] {
            refused(cmd);
        }
    }

    #[test]
    fn refuses_an_absolute_path_hidden_in_a_flag_value() {
        // The whole argument neither starts with `/` nor holds a `..` component, so
        // only checking the token would let these through.
        for cmd in [
            "du --exclude-from=/etc/shadow",
            "wc --files0-from=/etc/passwd",
            "tail --follow=../../secrets",
        ] {
            refused(cmd);
        }
    }

    #[test]
    fn an_equals_sign_in_a_non_path_flag_still_works() {
        // The flag-value check must not break ordinary `--flag=value` usage.
        ok("git log --pretty=format:%h");
        ok("git log --grep=fixup");
        ok("git branch --format=%(refname)");
        ok("find . -name '*.rs'");
    }

    #[test]
    fn refuses_paths_that_leave_the_root() {
        for cmd in [
            "cat /etc/passwd",
            "cat ../../etc/passwd",
            "ls /",
            "find /etc -name passwd",
            "head ../secrets.txt",
        ] {
            refused(cmd);
        }
    }

    #[test]
    fn a_git_revision_range_is_not_a_path_escape() {
        // `..` in `HEAD..main` is a rev range, not a parent directory. A blanket
        // substring check on ".." would break legitimate diffs.
        ok("git diff HEAD..main");
        ok("git log origin/main..HEAD");
        ok("git rev-list HEAD~3..HEAD");
    }

    // ---- write vectors on allowed programs ------------------------------

    #[test]
    fn refuses_find_action_predicates() {
        for cmd in [
            "find . -name '*.rs' -delete",
            "find . -exec rm {} ;",
            "find . -execdir rm {} ;",
            "find . -fprint out.txt",
        ] {
            let e = refused(cmd);
            assert!(!e.is_empty(), "{cmd}");
        }
        ok("find . -name '*.rs' -type f");
    }

    #[test]
    fn refuses_git_diff_output_redirection_flag() {
        for cmd in ["git diff --output=f", "git diff --output f", "git diff -o f"] {
            refused(cmd);
        }
    }

    #[test]
    fn refuses_git_config_writes_but_allows_reads() {
        ok("git config --get user.name");
        ok("git config --list");
        for cmd in [
            "git config --unset user.name",
            "git config --add user.name x",
            "git config --replace-all user.name x",
            "git config --edit",
        ] {
            refused(cmd);
        }
    }

    #[test]
    fn refuses_branch_creation_and_write_verbs() {
        for cmd in [
            "git branch newthing",
            "git branch -d old",
            "git branch -D old",
            "git branch -m a b",
            "git branch -f main HEAD",
            "git branch --set-upstream-to=origin/main",
            "git branch -u origin/main",
            "git branch --edit-description",
        ] {
            refused(cmd);
        }
    }

    #[test]
    fn a_branch_read_flag_value_is_not_mistaken_for_a_branch_name() {
        // `--contains HEAD` — HEAD is the flag's value, not a new branch.
        ok("git branch --contains HEAD");
        ok("git branch --merged main");
        ok("git branch --points-at HEAD -a");
    }

    // ---- policy ---------------------------------------------------------

    #[test]
    fn the_none_policy_refuses_even_allowed_commands() {
        assert!(validate("git log", CommandPolicy::None).is_err());
    }

    #[test]
    fn eyes_gets_read_only_and_unknown_roles_get_nothing() {
        assert_eq!(CommandPolicy::for_agent("rain"), CommandPolicy::ReadOnly);
        // No native HANDS exists yet; granting an unknown role a shell by
        // default would be the wrong direction.
        assert_eq!(CommandPolicy::for_agent("brian"), CommandPolicy::None);
        assert_eq!(CommandPolicy::for_agent("someone-new"), CommandPolicy::None);
    }

    // ---- tokenizer ------------------------------------------------------

    #[test]
    fn tokenizes_quoted_arguments_as_one_token() {
        assert_eq!(
            tokenize(r#"grep "two words" file"#).unwrap(),
            vec!["grep", "two words", "file"]
        );
        assert_eq!(
            tokenize("find . -name '*.rs'").unwrap(),
            vec!["find", ".", "-name", "*.rs"]
        );
    }

    #[test]
    fn unbalanced_quotes_are_refused() {
        assert!(validate("cat \"unclosed", CommandPolicy::ReadOnly).is_err());
    }

    #[test]
    fn an_empty_command_is_refused() {
        assert!(validate("", CommandPolicy::ReadOnly).is_err());
        assert!(validate("   ", CommandPolicy::ReadOnly).is_err());
    }

    #[test]
    fn a_metachar_inside_quotes_is_an_argument_not_an_operator() {
        // `grep` isn't allow-listed, but the point is the metachar check passes.
        assert!(reject_shell_metachars("git log --grep='a && b'").is_ok());
        ok("git log --grep='a && b'");
    }

    // ---- execution ------------------------------------------------------

    #[tokio::test]
    async fn runs_an_allowed_command_in_the_root() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = run("cat a.txt", &root, CommandPolicy::ReadOnly).await.unwrap();
        assert!(out.contains("hello"));
    }

    #[tokio::test]
    async fn a_refusal_never_reaches_the_process() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let marker = root.join("pwned");

        let e = run(
            &format!("rm {}", marker.display()),
            &root,
            CommandPolicy::ReadOnly,
        )
        .await
        .unwrap_err();
        assert!(!e.is_empty());
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn a_nonzero_exit_is_a_result_not_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // Reading a missing file exits non-zero; the model should see that.
        let out = run("cat nope.txt", &root, CommandPolicy::ReadOnly)
            .await
            .unwrap();
        assert!(out.contains("exit status") || out.contains("stderr"));
    }
}
