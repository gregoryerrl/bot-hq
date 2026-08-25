//! Git hook installation + CLI handlers.
//!
//! When a session opens against a working repo with an enforced policy,
//! bot-hq installs `.git/hooks/{commit-msg,pre-commit,post-commit,pre-push}`
//! that invoke `bot-hq policy-check ...` as a subprocess. The hook is the
//! MECHANICAL BACKSTOP — it fires unconditionally on every git op,
//! regardless of whether the agent remembered to call the MCP tool.
//!
//! Per DeepSeek-V4-Pro's review: MCP tool calls are a probabilistic primary
//! path (audited via violations.jsonl). Hooks are the deterministic backstop
//! that catches the case where the agent context drifted and "forgot" the
//! policy. Two layers > one layer.
//!
//! ## Hook protocol
//!
//! - **commit-msg**: receives `$1` = path to commit message file. Scans for
//!   forbidden words. Exits 1 if any found, blocking the commit. This is the
//!   reliable point for message scanning — pre-commit fires before git
//!   parses the `-m` argument so the message file may be stale or empty.
//! - **pre-commit**: scans the staged diff (`git diff --cached`) for any
//!   forbidden word that snuck into source code. Exits 1 if found.
//! - **post-commit**: reads `git log -1 HEAD` (message + diff). If a forbidden
//!   word slipped through (e.g., via `git commit --amend` rewriting an
//!   already-committed message, or pre-commit/commit-msg bypass), writes a
//!   `CommitGrep` Denied violation to `violations.jsonl`. Always exits 0
//!   — the commit already happened; the verifier is audit-only.
//! - **pre-push**: if `push_gate == auto`, allows the push (exit 0). When
//!   `push_gate == ask` and the push comes from inside a live session, it POSTs
//!   the running app's `/hooks/pre-push` route to surface a per-push
//!   Approve/Reject prompt and blocks on the user's pick (Approve → exit 0,
//!   Reject → exit 1). Fail-closed (exit 1 + a `PushGate`/Denied violation) when
//!   the app is unreachable; a push with no session context is blocked with
//!   guidance.

use crate::policy::violations::{ViolationKind, ViolationOutcome, ViolationsLog};
use crate::policy::Policy;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Marker block written into each hook so we can recognize + safely
/// re-install / detect manual edits.
const MANAGED_MARKER: &str = "# managed-by: bot-hq policy-check";

/// Session id surfaced by the agent's subprocess env (set by `spawn.rs`).
/// Threaded into `Policy::resolve` so hooks resolve the same session-scoped
/// policy snapshot the agent runs under.
fn hook_session_id() -> Option<String> {
    std::env::var("BOT_HQ_SESSION_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The single-use nonce a push RE-RUN carries (round 12): set by the app on
/// the `git push` it starts for a late approve, redeemed once by the app when
/// this hook presents it. Absent on every push an agent or a human runs.
fn hook_push_nonce() -> Option<String> {
    std::env::var("BOT_HQ_PUSH_NONCE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// CLI entrypoint. Dispatches `bot-hq policy-check <sub> ...`.
/// Returns the desired process exit code.
pub fn run_cli(args: &[String]) -> Result<i32> {
    let Some(sub) = args.first() else {
        return Err(anyhow!(
            "usage: bot-hq policy-check {{commit-msg|pre-commit|post-commit|pre-push|tool-gate}} \
             --data-dir <P> [--project <Q>] [--session <S>] [--msg-file <F>]"
        ));
    };
    let mut data_dir: Option<PathBuf> = None;
    let mut project: Option<String> = None;
    let mut session: Option<String> = None;
    let mut msg_file: Option<PathBuf> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--data-dir needs value"))?;
                data_dir = Some(crate::paths::expand_tilde(v)?);
                i += 2;
            }
            "--project" => {
                project = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--project needs value"))?
                        .clone(),
                );
                i += 2;
            }
            "--session" => {
                session = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--session needs value"))?
                        .clone(),
                );
                i += 2;
            }
            "--msg-file" => {
                msg_file = Some(PathBuf::from(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--msg-file needs value"))?,
                ));
                i += 2;
            }
            unknown if unknown.starts_with("--") => {
                return Err(anyhow!("unknown flag {unknown}"));
            }
            // Positional args (git passes the message file path as $1 to
            // commit-msg). We accept it positionally OR via --msg-file.
            _ => {
                positional.push(args[i].clone());
                i += 1;
            }
        }
    }
    let data_dir = data_dir.ok_or_else(|| anyhow!("--data-dir is required"))?;
    // The session context, resolved ONCE, here: `--session` when the hook was
    // given one, else the `BOT_HQ_SESSION_ID` the agent subprocess and the
    // session PTY carry. Every hook takes it as a parameter from this point —
    // none reads the environment itself — so a test says which context it
    // means. That matters because an agent-run `cargo test` INHERITS a real
    // `BOT_HQ_SESSION_ID` (round 8): a hook test that assumed "the test process
    // has no session id" was taking the other branch whenever an agent ran the
    // suite, and passing anyway.
    let env_sid = hook_session_id();
    let sid = session.as_deref().or(env_sid.as_deref());
    // A push RE-RUN the app started on a late approve carries its nonce here
    // (round 12); read once, beside the session id, and passed down — the
    // hook never reads the environment below this point.
    let push_nonce = hook_push_nonce();
    match sub.as_str() {
        "commit-msg" => {
            let path = msg_file
                .or_else(|| positional.into_iter().next().map(PathBuf::from))
                .ok_or_else(|| {
                    anyhow!("commit-msg needs the message file path (as positional or --msg-file)")
                })?;
            run_commit_msg(&data_dir, project.as_deref(), &path, sid)
        }
        // `"."` — every hook git invokes runs with the repo root as its CWD.
        "pre-commit" => run_pre_commit(&data_dir, project.as_deref(), Path::new("."), sid),
        "post-commit" => run_post_commit(&data_dir, project.as_deref(), sid),
        // git passes the remote NAME as $1 (and its URL as $2, not forwarded);
        // the re-run command is `git push <remote> <oid>:<ref>`, so the name
        // travels to the app with the ref updates.
        "pre-push" => {
            let remote = positional.first().map(String::as_str);
            run_pre_push(&data_dir, project.as_deref(), remote, push_nonce.as_deref(), sid)
        }
        "tool-gate" => run_tool_gate(&data_dir, sid),
        other => Err(anyhow!("unknown subcommand {other}")),
    }
}

/// A ruled "BLOCKED" banner for a hook rejection. Centralizes the rule line
/// and `bot-hq <hook>: BLOCKED` header so the commit-msg / pre-commit /
/// pre-push handlers can't drift. `body` is the hook-specific detail.
fn blocked_banner(hook: &str, body: &str) -> String {
    const RULE: &str = "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━";
    format!("\n{RULE}\nbot-hq {hook}: BLOCKED\n{RULE}\n{body}")
}

/// commit-msg handler. Scans the message file (passed by git as $1) for
/// forbidden words. Exits 1 if any found — blocks the commit reliably,
/// even when `git commit -m "..."` is used.
fn run_commit_msg(
    data_dir: &Path,
    project: Option<&str>,
    msg_path: &Path,
    session_id: Option<&str>,
) -> Result<i32> {
    audit_at_hook(data_dir, project, "commit-msg");
    let policy = Policy::resolve(data_dir, project, session_id)?;
    if policy.forbidden_in_commits.is_empty() {
        return Ok(0);
    }
    let msg = std::fs::read_to_string(msg_path)
        .with_context(|| format!("reading commit message file {}", msg_path.display()))?;
    // Strip comment lines (#) — they don't end up in the final commit.
    let cleaned: String = msg
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    match policy.first_forbidden_word(&cleaned) {
        None => Ok(0),
        Some(word) => {
            eprintln!(
                "{}",
                blocked_banner(
                    "commit-msg",
                    &format!(
                        "Forbidden word in commit message: '{word}'\n\
                         Policy: {project}\n\
                         Message file: {msg}\n\
                         \n\
                         Rewrite the commit message to remove '{word}', then retry.\n\
                         Do NOT bypass with --no-verify.\n",
                        project = project.unwrap_or("<none>"),
                        msg = msg_path.display(),
                    )
                )
            );
            Ok(1)
        }
    }
}

/// pre-commit handler. Scans the staged DIFF only (forbidden words in
/// source code being committed). Commit message scanning lives in
/// commit-msg because pre-commit fires before git parses `-m`.
fn run_pre_commit(
    data_dir: &Path,
    project: Option<&str>,
    repo: &Path,
    session_id: Option<&str>,
) -> Result<i32> {
    audit_at_hook(data_dir, project, "pre-commit");
    // Layer 1 — EYES-sign-off gate. Independent of the forbidden-word policy, so
    // it must run BEFORE the empty-list early return below (a project with no
    // forbidden words still needs review-completion enforced).
    if check_findings_gate(data_dir, "pre-commit", session_id) != 0 {
        return Ok(1);
    }
    // Layer 2 — immutable-artifact guard. Always-on (policy-independent), so it
    // runs before the empty-forbidden-list early return below. Blocks a sweep or
    // refactor from editing a committed append-only file (e.g. an applied sqlx
    // migration whose bytes sqlx checksums) — editing one breaks boot.
    if check_immutable_artifacts(repo) != 0 {
        return Ok(1);
    }
    // Layer 3 — forbidden-word scan.
    let policy = Policy::resolve(data_dir, project, session_id)?;
    if policy.forbidden_in_commits.is_empty() {
        return Ok(0);
    }
    let diff = read_staged_diff(repo).unwrap_or_default();
    let added_only = added_lines_only(&diff);
    match policy.first_forbidden_word(&added_only) {
        None => Ok(0),
        Some(word) => {
            eprintln!(
                "{}",
                blocked_banner(
                    "pre-commit",
                    &format!(
                        "Forbidden word in staged diff: '{word}'\n\
                         Policy: {project}\n\
                         \n\
                         Remove '{word}' from the source content, then retry.\n\
                         Do NOT bypass with --no-verify.\n",
                        project = project.unwrap_or("<none>")
                    )
                )
            );
            Ok(1)
        }
    }
}

/// Extract just the added content from a unified diff. Filters out:
/// - File headers (`+++ b/...`)
/// - Hunk headers (`@@ -... +... @@`)
/// - Context lines (no prefix or starting with ` `)
/// - Deleted lines (starting with `-`)
///
/// This makes the forbidden-word scan reflect the comment's intent ("source
/// code being committed"): legitimate cleanup that removes a forbidden word
/// from a file should pass, even though the deleted line is still in the
/// raw diff.
fn added_lines_only(diff: &str) -> String {
    diff.lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .map(|l| &l[1..])
        .collect::<Vec<_>>()
        .join("\n")
}

/// Files that are append-only once committed — a sweep/refactor must NEVER
/// modify them. Applied sqlx migrations are the canonical case: sqlx checksums
/// each migration file's bytes and refuses to boot if an applied one changed,
/// even a comment ("migration N was previously applied but has been modified").
/// Always-on for every project; extend the list (or add a per-project policy
/// field later) for other content-hashed / immutable-once-shipped artifacts.
const IMMUTABLE_GLOBS: &[&str] = &["migrations/*.sql"];

/// Minimal glob match supporting a single `*` that does NOT cross `/`:
/// `migrations/*.sql` matches `migrations/0021_x.sql` but not
/// `migrations/sub/x.sql` or `src/x.sql`.
fn glob_match(path: &str, pat: &str) -> bool {
    match pat.split_once('*') {
        None => path == pat,
        Some((pre, suf)) => {
            path.len() >= pre.len() + suf.len()
                && path.starts_with(pre)
                && path.ends_with(suf)
                && !path[pre.len()..path.len() - suf.len()].contains('/')
        }
    }
}

/// Parse `git diff --cached --name-status` and return committed immutable files
/// being MODIFIED / DELETED / RENAMED (status M/D/R). Newly-ADDED files (A) — a
/// new migration — are fine; only edits to an already-committed immutable file
/// are violations.
fn immutable_violations(name_status: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for line in name_status.lines() {
        let mut cols = line.split('\t');
        let code = cols.next().and_then(|s| s.chars().next()).unwrap_or(' ');
        // A (added) and C (copied) leave the committed file's bytes intact.
        if !matches!(code, 'M' | 'D' | 'R') {
            continue;
        }
        // M/D: the path is the next column. R: `old<TAB>new` — the OLD path
        // (next column) is the immutable file being moved away.
        let Some(path) = cols.next() else { continue };
        if IMMUTABLE_GLOBS.iter().any(|g| glob_match(path, g)) {
            hits.push(path.to_string());
        }
    }
    hits
}

/// Pre-commit layer: block staged edits to committed append-only artifacts.
/// Returns 1 (block) on violation, else 0. Fail-open if the index can't be read
/// (e.g. not a git repo). Bypass a genuinely-intentional edit with
/// `BOTHQ_ALLOW_IMMUTABLE_EDIT=1` (since `--no-verify` is forbidden).
fn check_immutable_artifacts(repo: &Path) -> i32 {
    if matches!(
        std::env::var("BOTHQ_ALLOW_IMMUTABLE_EDIT").as_deref(),
        Ok("1")
    ) {
        return 0;
    }
    let Some(status) = git_output_in(repo, &["diff", "--cached", "--name-status"]) else {
        return 0;
    };
    let hits = immutable_violations(&status);
    if hits.is_empty() {
        return 0;
    }
    eprintln!(
        "{}",
        blocked_banner(
            "pre-commit",
            &format!(
                "Edit to a committed append-only artifact ({}). Migrations are immutable once committed: sqlx checksums each migration file, so editing a committed migration (even a comment) breaks boot with 'migration N was previously applied but has been modified'. Add a NEW migration instead. If this is genuinely intentional, re-run with BOTHQ_ALLOW_IMMUTABLE_EDIT=1 (do NOT use --no-verify).",
                hits.join(", ")
            )
        )
    );
    1
}

/// post-commit verifier. Writes a violation if a forbidden word made it
/// through pre-commit (e.g., via --amend, or pre-commit was bypassed).
/// Always exits 0; the commit already happened.
fn run_post_commit(
    data_dir: &Path,
    project: Option<&str>,
    session_id: Option<&str>,
) -> Result<i32> {
    audit_at_hook(data_dir, project, "post-commit");
    let policy = Policy::resolve(data_dir, project, session_id)?;
    if policy.forbidden_in_commits.is_empty() {
        return Ok(0);
    }
    let msg = git_output(&["log", "-1", "--pretty=%B", "HEAD"]).unwrap_or_default();
    let diff = git_output(&["show", "--no-color", "HEAD"]).unwrap_or_default();
    let sha = git_output(&["rev-parse", "HEAD"]).unwrap_or_default();
    let sha_short = sha.trim().chars().take(8).collect::<String>();
    // Mirror pre-commit's added-only filter — otherwise removing a forbidden
    // word from a file logs a spurious violation against the very commit that
    // cleaned it up. The commit message stays in the scan as-is.
    let combined = format!("{msg}\n{}", added_lines_only(&diff));
    if let Some(word) = policy.first_forbidden_word(&combined) {
        eprintln!(
            "bot-hq post-commit: forbidden word '{word}' slipped through \
             (sha={sha_short}). Logging violation."
        );
        // Best-effort log. The append is a synchronous file write — no runtime
        // needed for it (round 9: three hook sites built a tokio runtime solely
        // to `block_on` a `record` whose body was `append_blocking`).
        let _ = ViolationsLog::new(data_dir).record_blocking(
            session_id.unwrap_or("<post-commit>").to_string(),
            "git-hook".to_string(),
            ViolationKind::CommitGrep,
            format!("git commit (sha={sha_short})"),
            ViolationOutcome::Denied,
            Some(format!(
                "forbidden word '{word}' detected post-commit by hook"
            )),
        );
    }
    Ok(0)
}

/// The pushing/committing participant for hook attribution: the `BOT_HQ_AGENT`
/// env (trimmed, non-empty), else a neutral LABEL. Shared by the pre-push +
/// findings-gate hooks so the fallback can't drift between them.
///
/// The fallback names a tray entry; it is not a roster lookup, and under rc3
/// D10's role-derived slugs no fixed name could be one.
fn hook_agent() -> String {
    std::env::var("BOT_HQ_AGENT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "an agent".to_string())
}

/// Build a current-thread runtime to drive async calls from a sync git-hook
/// subprocess (hooks run outside any runtime). Returns the builder's
/// `io::Result` so each caller keeps its own failure policy — the post-commit,
/// pre-push, and findings-gate hooks variously skip / propagate / fail-closed.
/// Centralizes the `new_current_thread().enable_all().build()` boilerplate.
fn hook_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// True for git's all-zero object id (the remote side of a ref create / the
/// local side of a delete). One definition, shared with the app's re-run
/// builder (round 12).
use crate::policy::is_zero_oid;

/// Classify one parsed pre-push update as a non-fast-forward (force) update,
/// given an ancestry oracle `is_ancestor(remote_oid, local_oid)`. Creates
/// (remote all-zero) and deletes (local all-zero) are NOT force updates;
/// malformed lines never become a `PushUpdate` at all (`parse_push_updates`).
/// Pure, so the classification is unit-testable without a git process. Took a
/// raw line until round 11, and the hook — which parses stdin once — rendered
/// each update back into a line for it to re-split.
fn update_is_force(u: &PushUpdate, is_ancestor: impl Fn(&str, &str) -> bool) -> bool {
    if is_zero_oid(&u.local_oid) || is_zero_oid(&u.remote_oid) {
        return false;
    }
    // Non-fast-forward = the remote tip is not an ancestor of the local tip.
    !is_ancestor(&u.remote_oid, &u.local_oid)
}

/// One ref update git hands the pre-push hook on stdin: `<local ref> <local
/// oid> <remote ref> <remote oid>`. Parsed ONCE per hook run (round 10, B3) and
/// shared by the force check and the approval prompt — stdin can only be read
/// once, and until this the prompt never read it at all: it named the
/// checked-out branch (`symbolic-ref HEAD`), so pushing `526-…` from a checkout
/// of `527-…` asked the user to "Allow `git push` to `527-…`" (`s-766f4ab9`,
/// trays b8725c80 / 66a2b6e2 — the user approved a push labelled with the
/// wrong branch, and the violations log carries the same wrong action).
/// The same shape the hook POSTs to `/hooks/pre-push` (round 12): the app
/// rebuilds the push from it for a late approve.
type PushUpdate = crate::policy::PushRef;

/// Parse git's pre-push stdin lines. Malformed lines are dropped rather than
/// failing the whole read — the force check and the naming both degrade to
/// "no updates" for them, which is the pre-existing fail-open posture.
fn parse_push_updates(input: &str) -> Vec<PushUpdate> {
    input
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (local_ref, local_oid, remote_ref, remote_oid) =
                (f.next()?, f.next()?, f.next()?, f.next()?);
            Some(PushUpdate {
                local_ref: local_ref.to_string(),
                local_oid: local_oid.to_string(),
                remote_ref: remote_ref.to_string(),
                remote_oid: remote_oid.to_string(),
            })
        })
        .collect()
}

/// Read the pre-push ref updates off stdin — ONLY when stdin is not a
/// terminal. Under git the hook's stdin is a pipe; from `cargo test` or a
/// hand-run hook it is the terminal, and `read_to_string` on a terminal blocks
/// until EOF (the reason this used to be lazy and force-only). Fail-open:
/// unreadable stdin reads as no updates.
fn read_push_updates() -> Vec<PushUpdate> {
    use std::io::{IsTerminal, Read};
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Vec::new();
    }
    let mut input = String::new();
    if stdin.lock().read_to_string(&mut input).is_err() {
        return Vec::new();
    }
    parse_push_updates(&input)
}

/// The refs a push touches, as the user should read them: `refs/heads/x` →
/// `x`, `refs/tags/v1` → `v1`, a delete (local oid all-zero) as `:x`; anything
/// else verbatim. Empty when git handed the hook no updates (a push of nothing,
/// or a hand-run hook), which callers turn into the HEAD fallback.
fn pushed_ref_names(updates: &[PushUpdate]) -> Vec<String> {
    updates
        .iter()
        .map(|u| {
            let name = u
                .remote_ref
                .strip_prefix("refs/heads/")
                .or_else(|| u.remote_ref.strip_prefix("refs/tags/"))
                .unwrap_or(&u.remote_ref);
            if is_zero_oid(&u.local_oid) {
                format!(":{name}")
            } else {
                name.to_string()
            }
        })
        .collect()
}

/// What the approval prompt / violations action name for this push: the pushed
/// refs when git said which, else the checked-out branch (the pre-round-10
/// behaviour, and the only answer when stdin carried nothing). `head` is a
/// thunk (round 11): it is a `git symbolic-ref` subprocess, and it used to be
/// spawned eagerly as an argument even when the refs were known and it was
/// never read.
fn push_target_label(names: &[String], head: impl FnOnce() -> Option<String>) -> Option<String> {
    if names.is_empty() {
        head()
    } else {
        // Plain, comma-joined: the prompt and the action wrap it in their own
        // backticks ("Allow `git push` to `a, b` …").
        Some(names.join(", "))
    }
}

/// Whether the in-flight push rewrites published history — git's pre-push signal
/// for `--force` / `--force-with-lease` (the flag itself is never passed to the
/// hook). Asks git for ancestry over the parsed stdin updates. A remote tip
/// missing locally makes `--is-ancestor` error, which is treated as a rewrite
/// (safe direction for a `blocked` policy).
fn pushing_non_fast_forward(updates: &[PushUpdate]) -> bool {
    updates.iter().any(|u| {
        update_is_force(u, |remote, local| {
            std::process::Command::new("git")
                .args(["merge-base", "--is-ancestor", remote, local])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
    })
}

/// Best-effort fail-closed Denied violation for a push the hook blocked —
/// `ForcePush` (policy is 'blocked') or `PushGate` (the prompt couldn't be
/// surfaced). One function for both (round 9: two near-identical async fns
/// differed only in the kind and the detail string, and each needed a tokio
/// runtime built around it to `block_on` a synchronous file append).
fn log_push_denial(
    data_dir: &Path,
    session_id: &str,
    agent: &str,
    branch: Option<&str>,
    kind: ViolationKind,
    reason: &str,
) {
    let action = crate::policy::push_gate_action(branch);
    let _ = ViolationsLog::new(data_dir).record_blocking(
        session_id.to_string(),
        agent.to_string(),
        kind,
        action,
        ViolationOutcome::Denied,
        Some(format!("pre-push blocked: {reason}")),
    );
}

/// POSTs the running app's `/hooks/pre-push` route to surface a per-push
/// Approve/Reject prompt (reusing the same `request_approval` machinery as the
/// agent-facing tools), blocking until the user picks: Approve → exit 0,
/// Reject → exit 1. Fail-closed (exit 1 + a `PushGate`/Denied violation) when
/// the app can't be reached. A push with no `BOT_HQ_SESSION_ID` (e.g. a human
/// pushing from a terminal) is blocked with guidance — `ask` only prompts a
/// session's user.
fn run_pre_push(
    data_dir: &Path,
    project: Option<&str>,
    remote: Option<&str>,
    push_nonce: Option<&str>,
    session_id: Option<&str>,
) -> Result<i32> {
    audit_at_hook(data_dir, project, "pre-push");
    // EYES-sign-off backstop: a push must not carry unresolved blocking findings
    // (catches a commit created before the finding was filed, an --amend, or a
    // bypassed pre-commit). Independent of push_gate; fail-open on DB errors.
    if check_findings_gate(data_dir, "pre-push", session_id) != 0 {
        return Ok(1);
    }
    let session_id = session_id.map(str::to_string);
    // **Fail CLOSED here, and only here** (E1). Every other `?` in this file
    // returns an error that `run_policy_check_cli` maps to exit 0 — soft-fail,
    // so an internal bug cannot break the user's git workflow. That is right for
    // the advisory hooks and wrong for this one: a malformed `policy.yaml` made
    // `push_gate: ask` and `force_push: blocked` silently evaporate, which is
    // the exact opposite of what this module's own doc promises. A gate that
    // cannot read its policy does not know that the push is allowed.
    let policy = match Policy::resolve(data_dir, project, session_id.as_deref()) {
        Ok(policy) => policy,
        Err(e) => {
            eprintln!(
                "{}",
                blocked_banner(
                    "pre-push",
                    &format!(
                        "Push BLOCKED: the policy could not be read ({e}).\n\
                         \n\
                         This gate fails closed: with no readable policy it cannot tell \
                         `push_gate: auto` from `push_gate: ask`, or whether force-push \
                         is blocked. Fix the policy file (usually a YAML syntax error in \
                         the project's `policy.yaml` or the session snapshot under \
                         `.local/session-policies/`) and push again."
                    ),
                ),
            );
            return Ok(1);
        }
    };
    use crate::policy::{ForcePushMode, PushGateMode};

    // The ref updates, read ONCE and LAZILY (round 10, B3): both the force
    // check and the approval prompt read them, stdin has one read in it, and
    // it is read only on a path that needs it — a `push_gate: auto` push and a
    // session-less one return before ever touching stdin, so a hand-run hook
    // or a test binary whose stdin is an open pipe never blocks on EOF.
    let updates: std::cell::OnceCell<Vec<PushUpdate>> = std::cell::OnceCell::new();
    // What the user is asked about: the pushed refs, else HEAD.
    let label = |updates: &[PushUpdate]| {
        push_target_label(&pushed_ref_names(updates), || current_branch())
    };

    // force_push gate — independent of push_gate and checked FIRST, so a
    // force-push can't ride through on push_gate=auto. A non-fast-forward push is
    // git's pre-push signal for --force / --force-with-lease (the flag is never
    // passed to the hook). Blocked outright when force_push == Blocked.
    if matches!(policy.force_push, ForcePushMode::Blocked)
        && pushing_non_fast_forward(updates.get_or_init(read_push_updates))
    {
        let branch = label(updates.get_or_init(read_push_updates));
        if let Some(sid) = session_id.as_deref() {
            log_push_denial(
                data_dir,
                sid,
                &hook_agent(),
                branch.as_deref(),
                ViolationKind::ForcePush,
                "force_push policy is 'blocked'",
            );
        }
        eprintln!(
            "{}",
            blocked_banner(
                "pre-push",
                "Force-push BLOCKED: this push rewrites published history \
                 (non-fast-forward) and the force_push policy is 'blocked'.\n\
                 \n\
                 Do not retry with --force / --force-with-lease. If a history rewrite is \
                 genuinely required, ask the user to set force_push to 'allowed' in Session \
                 Settings (per-action authorized), then push again.\n"
            )
        );
        return Ok(1);
    }

    if matches!(policy.push_gate, PushGateMode::Auto) {
        return Ok(0);
    }

    // No session id → not an agent push inside a live session (e.g. a human at a
    // terminal). `ask` can only prompt a session's user, so block with guidance
    // rather than allowing — allowing here would let an agent bypass via
    // `env -u BOT_HQ_SESSION_ID git push`.
    let Some(session_id) = session_id else {
        eprintln!(
            "{}",
            blocked_banner(
                "pre-push",
                "Push blocked: push gate is 'ask' but this push has no bot-hq session \
                 context (BOT_HQ_SESSION_ID unset).\n\
                 \n\
                 push_gate='ask' surfaces a per-push Approve/Reject prompt only inside a \
                 live bot-hq session. To push from outside a session, flip the push toggle \
                 to 'auto' in Session Settings, or push from within a session.\n"
            )
        );
        return Ok(1);
    };

    let agent = hook_agent();
    let branch = label(updates.get_or_init(read_push_updates));

    // The hook is a fresh subprocess that can't reach the running app's bridge
    // directly — POST `/hooks/pre-push` and block on the user's pick. One
    // current-thread runtime drives the HTTP call (the fail-closed violation
    // log is a synchronous append and needs none).
    let rt = match hook_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!(
                "{}",
                blocked_banner(
                    "pre-push",
                    &format!("Push blocked: could not start the approval client ({e}).\n")
                )
            );
            return Ok(1);
        }
    };

    // **A re-run the app started presents its nonce and asks nothing** (round
    // 12, EYES F9): nonce present ⇒ redeem-or-exit-1, never the approval
    // prompt — a re-run whose nonce the app refuses must not park a SECOND
    // gate behind a push nobody is waiting on. Everything above still ran
    // (findings gate, policy, the force check), so a pre-approved re-run
    // cannot smuggle what the first push could not.
    if let Some(nonce) = push_nonce {
        return Ok(match rt.block_on(redeem_push_nonce(
            data_dir,
            &session_id,
            &agent,
            nonce,
            updates.get_or_init(read_push_updates),
        )) {
            PushDecision::Approved => {
                eprintln!("bot-hq pre-push: pre-approved re-run (gate redeemed once); pushing.");
                0
            }
            PushDecision::Rejected | PushDecision::Blocked(_) => {
                eprintln!(
                    "{}",
                    blocked_banner(
                        "pre-push",
                        "Push blocked: this re-run's approval could not be redeemed (already \
                         used, another session's, or the refs differ from what was approved). \
                         Nothing was pushed; the user can approve a fresh gate.\n"
                    )
                );
                1
            }
        });
    }

    // One non-alarming line so the agent doesn't mistake the wait for a block and
    // try to work around it. Silent until the user answers.
    eprintln!(
        "bot-hq pre-push: awaiting user approval for `git push`{} (session {session_id})… \
         (if this process is killed before the answer, the approval still runs the push — \
         check `gate_status`)",
        branch
            .as_deref()
            .map(|b| format!(" to `{b}`"))
            .unwrap_or_default()
    );

    match rt.block_on(decide_push(
        data_dir,
        &session_id,
        &agent,
        branch.as_deref(),
        remote,
        updates.get_or_init(read_push_updates),
    )) {
        PushDecision::Approved => Ok(0),
        PushDecision::Rejected => {
            eprintln!(
                "{}",
                blocked_banner(
                    "pre-push",
                    "Push rejected by the user.\n\
                     \n\
                     The user declined this `git push`. Do not retry it — ask the user what \
                     they'd like to do instead.\n"
                )
            );
            Ok(1)
        }
        PushDecision::Blocked(reason) => {
            // Fail-closed: the prompt couldn't be surfaced. The happy path's
            // violation is written by the bridge's resolve_choice; this records
            // our own so a blocked push still leaves an audit trail.
            log_push_denial(
                data_dir,
                &session_id,
                &agent,
                branch.as_deref(),
                ViolationKind::PushGate,
                &reason,
            );
            eprintln!(
                "{}",
                blocked_banner(
                    "pre-push",
                    &format!(
                        "Push blocked: {reason}.\n\
                         \n\
                         push_gate='ask' needs the bot-hq app running to surface the approval \
                         prompt. Make sure bot-hq is running, or ask the user to flip the push \
                         toggle to 'auto' in Session Settings.\n"
                    )
                )
            );
            Ok(1)
        }
    }
}

/// Outcome of asking the running app to approve a push.
#[derive(Debug, PartialEq)]
enum PushDecision {
    Approved,
    Rejected,
    /// The prompt couldn't be surfaced (app down / network / bad response). The
    /// `String` is a human-readable reason for the audit trail + banner.
    Blocked(String),
}

/// POST `{session_id, agent, branch}` to the running app's `/hooks/pre-push`
/// route and block until the user picks (or a transport failure). Distinct
/// `Blocked` reasons so the audit trail separates "app down" from "timeout"
/// from "bad response". reqwest here lacks the `json` feature, so the body is
/// sent raw and the response parsed from text.
async fn decide_push(
    data_dir: &Path,
    session_id: &str,
    agent: &str,
    branch: Option<&str>,
    remote: Option<&str>,
    updates: &[PushUpdate],
) -> PushDecision {
    let body = pre_push_request_body(session_id, agent, branch, remote, updates, None);
    post_pre_push(data_dir, body, std::time::Duration::from_secs(1800)).await
}

/// A re-run's hook presents its nonce (round 12): same route, same answer
/// shape, no wait — the app redeems at once or refuses. 30 s is plenty for a
/// localhost round trip; a re-run must never sit on a prompt.
async fn redeem_push_nonce(
    data_dir: &Path,
    session_id: &str,
    agent: &str,
    nonce: &str,
    updates: &[PushUpdate],
) -> PushDecision {
    let body = pre_push_request_body(session_id, agent, None, None, updates, Some(nonce));
    post_pre_push(data_dir, body, std::time::Duration::from_secs(30)).await
}

/// The JSON the hook POSTs to `/hooks/pre-push`: the session + agent + prompt
/// label as before, plus (round 12) the remote name and the ref updates the
/// app needs to rebuild the push for a late approve, and — on a re-run — the
/// nonce it redeems. Pure, so the shape is unit-testable.
fn pre_push_request_body(
    session_id: &str,
    agent: &str,
    branch: Option<&str>,
    remote: Option<&str>,
    updates: &[PushUpdate],
    nonce: Option<&str>,
) -> String {
    serde_json::json!({
        "session_id": session_id,
        "agent": agent,
        "branch": branch,
        "remote": remote,
        "updates": updates,
        "nonce": nonce,
    })
    .to_string()
}

/// POST `body` to the running app's `/hooks/pre-push` route and map the reply
/// (or the transport failure) to a decision. Distinct `Blocked` reasons so the
/// audit trail separates "app down" from "timeout" from "bad response".
/// reqwest here lacks the `json` feature, so the body is sent raw and the
/// response parsed from text.
async fn post_pre_push(
    data_dir: &Path,
    body: String,
    timeout: std::time::Duration,
) -> PushDecision {
    let Some(addr) = crate::paths::read_signaling_addr(data_dir) else {
        return PushDecision::Blocked("bot-hq is not running (no signaling address)".into());
    };
    let url = format!("http://{addr}/hooks/pre-push");

    // Generous timeout — the user may take minutes to decide; a push isn't
    // time-critical.
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(e) => return PushDecision::Blocked(format!("approval client init failed: {e}")),
    };

    let resp = match client
        .post(&url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return PushDecision::Blocked("approval timed out (no answer)".into())
        }
        Err(e) if e.is_connect() => {
            return PushDecision::Blocked("could not connect to bot-hq".into())
        }
        Err(e) => return PushDecision::Blocked(format!("request to bot-hq failed: {e}")),
    };

    let status = resp.status();
    let txt = match resp.text().await {
        Ok(t) => t,
        Err(e) => return PushDecision::Blocked(format!("could not read bot-hq response: {e}")),
    };
    classify_push_response(status, &txt)
}

/// Map a `(status, body)` from the app's `/hooks/pre-push` route to a decision.
/// Pure + fail-CLOSED: a non-success status, an unparseable body, or a missing
/// `approved` field all Block — only an explicit `{"approved": true|false}` on a
/// 2xx yields Approved/Rejected. Extracted from `decide_push` so the safety
/// mapping is unit-testable without a live HTTP round-trip.
fn classify_push_response(status: reqwest::StatusCode, body: &str) -> PushDecision {
    if !status.is_success() {
        return PushDecision::Blocked(format!("bot-hq returned HTTP {}", status.as_u16()));
    }
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return PushDecision::Blocked(format!("malformed bot-hq response: {e}")),
    };
    match v.get("approved").and_then(|b| b.as_bool()) {
        Some(true) => PushDecision::Approved,
        Some(false) => PushDecision::Rejected,
        None => PushDecision::Blocked("bot-hq response missing 'approved'".into()),
    }
}

/// PreToolUse hook handler — the **Tool Gate** tripwire, injected into
/// HANDS at spawn via `--settings`. Reads the claude-code PreToolUse JSON
/// payload on stdin and matches the Bash command against the GLOBAL keyword
/// config (`<data_dir>/config/tool-gate.json`, NOT per-project `policy.yaml`). A
/// `gate` keyword BLOCKS the direct call (exit 2) and routes the agent to the
/// `action_gate` MCP tool (which surfaces Approve/Reject and executes on
/// approve); an `auto_allow`/unmatched command runs normally (exit 0). The
/// config is global + bot-hq-side, so nothing is written into a working repo.
/// This replaces the per-project
/// `tool_blocklist` role (post-2026-05-29 fabricated-comment incident) with a
/// single user-configurable gate that can also EXECUTE the command on approval.
///
/// IMPORTANT (verified empirically 2026-05-29): under
/// `--dangerously-skip-permissions` (HANDS's mode) claude-code SILENTLY
/// IGNORES a JSON `{"decision":"deny"}` PreToolUse result — that is a
/// permission-layer decision and bypass skips the permission layer. Exit code 2
/// ("blocking error") IS honored under bypass because it fires before the
/// permission layer; stderr is fed back to the agent. So this hook blocks via
/// exit 2, NOT JSON.
/// FAIL-OPEN (exit 0) on any parse/IO error or empty keyword list: a hook bug
/// must never brick every Bash call; the prompt rules remain as the other layer.
fn run_tool_gate(data_dir: &Path, session_id: Option<&str>) -> Result<i32> {
    use std::io::Read;
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        return Ok(0); // fail-open: couldn't read the payload
    }
    let Some(command) = parse_pretool_bash_command(&buf) else {
        return Ok(0); // not a Bash tool call (or empty command) → allow
    };
    // Session-snapshot-first, global-fallback — the shared two-tier resolve
    // (`tool_gate::resolve_keywords`), same list `action_gate` and
    // `terminal_exec` enforce. Fail-open on snapshot read errors, mirroring
    // the rest of this hook's posture.
    let sid = session_id.map(str::to_string);
    let keywords = crate::policy::tool_gate::resolve_keywords(data_dir, sid.as_deref());
    // Auto-park (issues.md #29): when the command is gated AND we know the
    // session, park the approval here so the refusal IS the approval request —
    // the agent doesn't have to convert it into an `action_gate` call (which
    // cost a ToolSearch round-trip on every observed conversion) and has
    // nothing to gain by rewording around the keyword (which is what 2 of 5
    // measured refusals did instead of converting). Best-effort: any failure
    // leaves `parked` None and the refusal falls back to the call-action_gate
    // wording. The command stays blocked either way.
    let parked = match (
        crate::policy::tool_gate::match_keyword("Bash", &command, &keywords),
        sid.as_deref(),
    ) {
        (Some(crate::policy::tool_gate::GateMode::Gate), Some(session_id)) => match hook_runtime() {
            Ok(rt) => rt.block_on(park_gate(data_dir, session_id, &hook_agent(), &command)),
            Err(e) => {
                tracing::warn!(%e, "tool-gate auto-park: could not start the client");
                None
            }
        },
        _ => None,
    };
    let (code, message) = tool_gate_exit(&command, &keywords, parked.as_ref());
    if let Some(m) = message {
        // Exit 2 = claude-code "blocking error": stops the tool call and feeds
        // stderr to the agent. The ONLY block form honored under bypass.
        eprintln!("{m}");
    }
    Ok(code)
}

/// Pure decision for a parsed Bash `command` against the global keyword list.
/// `gate` → `(2, Some(routing message))`; `auto_allow`/no-match → `(0, None)`.
/// Split from stdin handling so the gate decision is unit-testable.
fn tool_gate_exit(
    command: &str,
    keywords: &[crate::policy::tool_gate::GatedKeyword],
    parked: Option<&ParkedGate>,
) -> (i32, Option<String>) {
    use crate::policy::tool_gate::GateMode;
    match crate::policy::tool_gate::match_keyword("Bash", command, keywords) {
        Some(GateMode::Gate) => (2, Some(gate_refusal_text(command, parked))),
        // auto_allow or no match → allow the agent's direct Bash call.
        _ => (0, None),
    }
}

/// The refusal an agent reads. Two shapes, one shared spine.
///
/// When the hook managed to auto-park, the approval is ALREADY queued, so the
/// text's job is to stop the agent doing anything at all — no `action_gate`
/// call (that would be a second, redundant park), no retry, no reword. When
/// parking failed (app not running, no session, a non-2xx), it degrades to the
/// original wording: convert it yourself.
///
/// Both shapes carry the command verbatim and the anti-rewording clause, which
/// exists because 2 of the 5 measured Aug 4–5 refusals were answered by
/// rephrasing around the keyword rather than by routing the command.
fn gate_refusal_text(command: &str, parked: Option<&ParkedGate>) -> String {
    let no_dodging = "Do NOT rewrite the command to get around the gated keyword — \
         splitting it up, swapping the gated form for an equivalent one (e.g. \
         `rm -rf` → `rm -f` + `rmdir`), or moving it into a script or a \
         here-doc. The gate IS the user's decision point; routing around it \
         silently is the failure this message exists to prevent, and it has \
         happened. If the gate is wrong for this command, say so and ask — \
         don't dodge it.";
    match parked {
        Some(gate) => {
            let lead = if gate.existing {
                "an identical command was ALREADY awaiting the user's approval"
            } else {
                "bot-hq has PARKED it for the user's approval"
            };
            format!(
                "PARKED for the user's approval (a Tool Gate stop, NOT an error): `{command}`.\n\
                 You do not need to do anything to queue it — {lead} \
                 (gate_id: {}).\n\
                 Do NOT call `action_gate` for this command; it is already \
                 queued. On approve, bot-hq runs it in your working repo and the \
                 output arrives as an out-of-band message; on reject you get a \
                 rejection notice. If you need the current state before \
                 continuing, call gate_status(\"{}\") — never re-issue the \
                 command or assume it ran.\n\
                 {no_dodging}",
                gate.gate_id, gate.gate_id
            )
        }
        None => format!(
            "STOPPED by the bot-hq Tool Gate (normal control flow, NOT a fault): `{command}`.\n\
             This command needs the USER'S APPROVAL — not a different command. \
             Call the `action_gate` MCP tool with command=\"{command}\". bot-hq \
             will surface an Approve/Reject prompt to the user and, on approve, \
             run the command in your working repo and return its output \
             out-of-band.\n\
             {no_dodging}"
        ),
    }
}

/// A gate the PreToolUse hook parked on the agent's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParkedGate {
    gate_id: String,
    /// The command was already pending approval — the app deduped rather than
    /// stacking a second card.
    existing: bool,
}

/// POST `/hooks/tool-gate` on the running app to park the blocked command.
/// `None` on every failure (app not running, connect/timeout, non-2xx,
/// unparseable body) — the caller then falls back to the call-`action_gate`
/// wording, so a miss costs convenience and never safety.
///
/// Short timeout, unlike `decide_push`'s 1800s: parking returns immediately
/// (the user's pick arrives out-of-band later), so a wedged app must not stall
/// every gated Bash call the agent makes.
async fn park_gate(
    data_dir: &Path,
    session_id: &str,
    agent: &str,
    command: &str,
) -> Option<ParkedGate> {
    let addr = crate::paths::read_signaling_addr(data_dir)?;
    let body = serde_json::json!({
        "session_id": session_id,
        "agent": agent,
        "command": command,
    })
    .to_string();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = match client
        .post(format!("http://{addr}/hooks/tool-gate"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(%e, "tool-gate auto-park request failed");
            return None;
        }
    };
    let status = resp.status();
    let txt = resp.text().await.ok()?;
    classify_park_response(status, &txt)
}

/// Map a `(status, body)` from `/hooks/tool-gate` onto a parked gate. Pure, so
/// the mapping is testable without HTTP (the `classify_push_response` pattern).
/// `Some` ONLY for a 2xx carrying a non-empty `gate_id` — anything else means
/// we cannot promise the agent a gate exists, and promising one that doesn't
/// would strand the command with nobody asked.
fn classify_park_response(status: reqwest::StatusCode, body: &str) -> Option<ParkedGate> {
    if !status.is_success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let gate_id = v.get("gate_id")?.as_str()?.trim().to_string();
    if gate_id.is_empty() {
        return None;
    }
    Some(ParkedGate {
        gate_id,
        existing: v.get("existing").and_then(|e| e.as_bool()).unwrap_or(false),
    })
}

/// Extract the Bash command from a claude-code PreToolUse payload. None for
/// non-Bash tools or a missing/empty command.
fn parse_pretool_bash_command(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v.get("tool_name").and_then(|t| t.as_str()) != Some("Bash") {
        return None;
    }
    let cmd = v.get("tool_input")?.get("command")?.as_str()?.trim();
    if cmd.is_empty() {
        None
    } else {
        Some(cmd.to_string())
    }
}

/// Install bot-hq hooks into `<working_repo>/.git/hooks/`. Idempotent.
///
/// - If a hook file doesn't exist, write a fresh one.
/// - If a hook exists and contains [`MANAGED_MARKER`], rewrite (we own it).
/// - If a hook exists WITHOUT the marker, leave it untouched and write a
///   side-by-side `<hook>.bot-hq` file so the user/admin can wire it in
///   manually. (We don't clobber husky/pre-commit-framework setups.)
pub fn install_hooks(
    working_repo: &Path,
    data_dir: &Path,
    project: Option<&str>,
) -> Result<HookInstallReport> {
    let git_marker = working_repo.join(".git");
    let hooks_dir = if git_marker.is_dir() {
        git_marker.join("hooks")
    } else if git_marker.is_file() {
        // Linked worktree: `.git` is a FILE pointing at the common git dir,
        // and git reads hooks from the SHARED common hooks dir (or
        // core.hooksPath). Resolve through git so the write lands where git
        // will actually look — a `.git/hooks` join here would silently
        // install nothing enforcement-wise.
        match resolve_hooks_dir(working_repo) {
            Some(d) => d,
            None => return Ok(HookInstallReport::not_a_git_repo()),
        }
    } else {
        return Ok(HookInstallReport::not_a_git_repo());
    };
    std::fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("creating hooks dir at {}", hooks_dir.display()))?;

    let bot_hq_bin = std::env::current_exe()
        .context("locating current bot-hq binary")?
        .display()
        .to_string();

    let mut report = HookInstallReport::default();
    for kind in [
        HookKind::CommitMsg,
        HookKind::PreCommit,
        HookKind::PostCommit,
        HookKind::PrePush,
    ] {
        let body = render_hook_body(kind, &bot_hq_bin, data_dir, project);
        let outcome = write_hook(&hooks_dir, kind, &body)?;
        match outcome {
            WriteOutcome::Installed => report.installed.push(kind.filename().into()),
            WriteOutcome::Updated => report.updated.push(kind.filename().into()),
            WriteOutcome::Sidecar => report.sidecar.push(kind.filename().into()),
            WriteOutcome::Unchanged => report.unchanged.push(kind.filename().into()),
        }
    }
    Ok(report)
}

/// Where git actually reads hooks for this checkout — `git rev-parse
/// --git-path hooks` honors linked worktrees (shared common dir) AND
/// `core.hooksPath`. Relative output is anchored at the repo. None when git
/// is missing or the dir isn't a repo.
fn resolve_hooks_dir(working_repo: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(working_repo)
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8(out.stdout).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let p = PathBuf::from(raw);
    Some(if p.is_absolute() {
        p
    } else {
        working_repo.join(p)
    })
}

#[derive(Debug, Default, Clone)]
pub struct HookInstallReport {
    pub installed: Vec<String>,
    pub updated: Vec<String>,
    pub sidecar: Vec<String>,
    pub unchanged: Vec<String>,
    pub not_a_git_repo: bool,
}

impl HookInstallReport {
    fn not_a_git_repo() -> Self {
        Self {
            not_a_git_repo: true,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HookKind {
    CommitMsg,
    PreCommit,
    PostCommit,
    PrePush,
}

impl HookKind {
    /// The git hook filename. By design this doubles as the `policy-check`
    /// subcommand name the hook body invokes (`bot-hq policy-check <name>`),
    /// so there's one canonical string per kind.
    fn filename(self) -> &'static str {
        match self {
            HookKind::CommitMsg => "commit-msg",
            HookKind::PreCommit => "pre-commit",
            HookKind::PostCommit => "post-commit",
            HookKind::PrePush => "pre-push",
        }
    }
    /// commit-msg gets the message file path passed as $1 from git; pre-push
    /// gets the remote NAME as $1 (its URL as $2 is not forwarded) — the app
    /// rebuilds a late-approved push over that name (round 12). Others receive
    /// no positional args.
    fn passes_dollar_one(self) -> bool {
        matches!(self, HookKind::CommitMsg | HookKind::PrePush)
    }
}

#[derive(Debug, Clone, Copy)]
enum WriteOutcome {
    Installed, // file didn't exist; we wrote a fresh hook
    Updated,   // file existed AND was ours (marker present); rewrote
    Sidecar,   // file existed WITHOUT marker; we wrote <name>.bot-hq instead
    Unchanged, // file content was identical to what we'd write
}

fn write_hook(hooks_dir: &Path, kind: HookKind, body: &str) -> Result<WriteOutcome> {
    let path = hooks_dir.join(kind.filename());
    if !path.exists() {
        write_executable(&path, body)?;
        return Ok(WriteOutcome::Installed);
    }
    let existing = std::fs::read_to_string(&path)
        .with_context(|| format!("reading existing hook {}", path.display()))?;
    if existing.contains(MANAGED_MARKER) {
        if existing == body {
            // Unchanged CONTENT is not an unchanged hook: git runs a hook only
            // when it is executable and says nothing otherwise, and
            // `write_executable` writes then chmods in two steps, so a hook
            // that lost its bit stayed silently off while every later install
            // reported it unchanged (round 11). Re-assert the mode.
            ensure_executable(&path)?;
            return Ok(WriteOutcome::Unchanged);
        }
        write_executable(&path, body)?;
        return Ok(WriteOutcome::Updated);
    }
    // Foreign hook present — don't clobber. Drop a sidecar.
    let sidecar = hooks_dir.join(format!("{}.bot-hq", kind.filename()));
    write_executable(&sidecar, body)?;
    Ok(WriteOutcome::Sidecar)
}

fn write_executable(path: &Path, body: &str) -> Result<()> {
    std::fs::write(path, body).with_context(|| format!("writing hook {}", path.display()))?;
    ensure_executable(path)
}

/// `chmod 0755` unless the file already carries an executable bit. Split out
/// of `write_executable` so an unchanged hook can be re-armed without a rewrite.
fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .with_context(|| format!("reading hook mode {}", path.display()))?
            .permissions();
        if perms.mode() & 0o111 == 0 {
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms)
                .with_context(|| format!("marking hook executable {}", path.display()))?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn render_hook_body(
    kind: HookKind,
    bot_hq_bin: &str,
    data_dir: &Path,
    project: Option<&str>,
) -> String {
    // The hook runs through `sh` — on Windows that's Git-for-Windows' bundled
    // MSYS2 shell, which execs a native path written with forward slashes
    // (backslashes are escapes); double-quote it for spaces. Unix is unchanged
    // (byte-identical passthrough). The `--data-dir` arg stays single-quoted with
    // its native separators — it's passed literally to bot-hq, which parses
    // Windows paths fine.
    let bin_for_sh = if cfg!(windows) {
        format!("\"{}\"", bot_hq_bin.replace('\\', "/"))
    } else {
        bot_hq_bin.to_string()
    };
    let mut cmd = format!(
        "{bin_for_sh} policy-check {sub} --data-dir {dd}",
        sub = kind.filename(),
        dd = shell_quote(&data_dir.display().to_string())
    );
    if let Some(p) = project {
        cmd.push_str(&format!(" --project {}", shell_quote(p)));
    }
    // commit-msg gets $1 = path to message file. Forward it.
    let tail = if kind.passes_dollar_one() {
        " \"$1\""
    } else {
        ""
    };
    format!(
        "#!/bin/sh\n\
         {marker}\n\
         # Do NOT edit by hand — bot-hq rewrites this file when policy changes.\n\
         # Generated for: {project}\n\
         exec {cmd}{tail}\n",
        marker = MANAGED_MARKER,
        project = project.unwrap_or("<none>"),
    )
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_.-:~@".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Run the policy-mutation audit. Best-effort: any error is logged but
/// never aborts the hook (we'd rather block on policy than block on
/// audit). The hook still proceeds to enforce the (potentially mutated)
/// policy; the audit just records the change for human review.
fn audit_at_hook(data_dir: &Path, project: Option<&str>, hook_name: &str) {
    let log = ViolationsLog::new(data_dir);
    if let Err(err) = crate::policy::audit_policy_files(
        data_dir,
        project,
        Some(&log),
        &format!("<hook:{hook_name}>"),
        "git-hook",
    ) {
        eprintln!("bot-hq {hook_name}: policy audit failed: {err}");
    }
}

// ---- EYES-sign-off gate (findings) ----
//
// The mechanical backstop for the EYES-sign-off gate: a `blocking` finding that
// EYES filed (via `eyes_flag`) and HANDS hasn't dispositioned blocks `git commit`
// (and, as a re-check, `git push`). The agent-facing MCP `check_open_findings`
// tool is the prompted primary; this hook fires regardless of whether the agent
// remembered to call it — the same two-layer model as the commit-message gate.
//
// Findings live in the SQLite DB, so (unlike the YAML-only forbidden-word scan)
// the hook reads the DB directly, READ-ONLY. It is FAIL-OPEN on every DB error
// (missing/locked/corrupt DB, an un-migrated DB without the `findings` table,
// SQLITE_BUSY mid-write): a DB hiccup must NEVER block a human's commit. A push/
// commit with no `BOT_HQ_SESSION_ID` (a human at a terminal) skips the gate
// entirely — findings are session-scoped, so there's nothing to enforce.
//
// Audit-logging of a hook block (a `Findings` ViolationKind) is intentionally
// deferred — the block + banner are the enforcement; the audit row is additive.

/// Gate decision for `hook` (commit/push). Returns 1 to BLOCK, 0 to proceed.
/// 0 covers all the proceed cases: no session context, fail-open DB error, and
/// no open blocking findings. On a block it prints the actionable banner.
fn check_findings_gate(data_dir: &Path, hook: &str, session_id: Option<&str>) -> i32 {
    // The session id is a PARAMETER, resolved once in `run_cli` — the gate
    // never reads the environment itself, so a test can drive the block arm
    // below without setting a process-global env var (tests run in parallel;
    // a `set_var` in one would leak into every other hook test). Until round 8
    // no test reached that arm at all: every hook test took the `None` return,
    // so a hook that never blocked would have stayed green.
    let Some(session_id) = session_id else {
        return 0; // no bot-hq session context (e.g. a human commit) → gate N/A
    };
    let Some(findings) = open_blocking_findings(data_dir, session_id) else {
        return 0; // fail-open: DB unreadable for any reason
    };
    if findings.is_empty() {
        return 0;
    }
    eprintln!("{}", blocked_banner(hook, &findings_block_body(&findings)));
    log_findings_block(data_dir, hook, session_id, findings.len());
    1
}

/// Best-effort audit record for a findings-gate block (`Findings` / Denied), so
/// violations.jsonl shows the gate fired — mirrors `run_post_commit`'s logging
/// (a synchronous append; the hook is a sync subprocess). Never fails the
/// hook: a logging error is swallowed (the block already landed via stderr).
fn log_findings_block(data_dir: &Path, hook: &str, session_id: &str, n: usize) {
    let agent = hook_agent();
    let action = if hook == "pre-push" { "git push" } else { "git commit" };
    let _ = ViolationsLog::new(data_dir).record_blocking(
        session_id.to_string(),
        agent,
        ViolationKind::Findings,
        action.to_string(),
        ViolationOutcome::Denied,
        Some(format!("{n} unresolved reviewer blocking finding(s)")),
    );
}

/// Read open BLOCKING findings for `session_id` from the DB, read-only. Returns
/// `None` on ANY error (the caller treats None as fail-open → proceed). Builds
/// its own current-thread runtime — the hook runs in a sync context.
fn open_blocking_findings(
    data_dir: &Path,
    session_id: &str,
) -> Option<Vec<(String, String, Option<String>)>> {
    let db_path = crate::paths::Paths::for_data_dir(data_dir.to_path_buf()).db_path;
    let rt = hook_runtime().ok()?;
    match rt.block_on(query_open_blocking(&db_path, session_id)) {
        Ok(rows) => Some(rows),
        Err(e) => {
            // Fail-open silently, like the other DB/git reads in this file. A
            // host-side warn rather than agent-facing stderr so a transient
            // SQLITE_BUSY doesn't surface a scary "could not read the DB" line in
            // the HANDS transcript on every blocked-from-reading commit. (No-op in
            // the subscriber-less hook subprocess; captured if ever app-hosted.)
            tracing::warn!(?e, "EYES-findings gate could not read the DB; proceeding (fail-open)");
            None
        }
    }
}

/// Async core of [`open_blocking_findings`] — split out so tests can drive it
/// under `#[tokio::test]` without nesting a runtime. Opens `db_path` READ-ONLY.
async fn query_open_blocking(
    db_path: &Path,
    session_id: &str,
) -> Result<Vec<(String, String, Option<String>)>> {
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::ConnectOptions;
    let mut conn = SqliteConnectOptions::new()
        .filename(db_path)
        .read_only(true)
        .connect()
        .await
        .with_context(|| format!("opening {} read-only", db_path.display()))?;
    // The predicate is storage's own (`OPEN_BLOCKING_FOR_SESSION`), so the
    // hook and the MCP `check_open_findings` cannot disagree on what gates.
    let rows = sqlx::query_as::<_, (String, String, Option<String>)>(&format!(
        "SELECT finding_uid, summary, code_ref FROM findings {}",
        crate::storage::OPEN_BLOCKING_FOR_SESSION
    ))
    .bind(session_id)
    .fetch_all(&mut conn)
    .await
    .context("querying open blocking findings")?;
    Ok(rows)
}

/// Body of the block banner — lists each open blocking finding + how to clear it.
fn findings_block_body(findings: &[(String, String, Option<String>)]) -> String {
    let list = findings
        .iter()
        .map(|(uid, summary, code_ref)| {
            let r = code_ref
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            format!("  - [{uid}] {summary}{r}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{n} unresolved reviewer blocking finding(s) — blocked.\n\n{list}\n\n\
         Resolve each before retrying: call `disposition_finding(finding_id, status, reason)` \
         with status='fixed' (reference the fix) or 'rebutted' (justify why). A rebuttal needs \
         no reviewer agreement, so this cannot deadlock. Do NOT bypass with --no-verify.\n",
        n = findings.len(),
    )
}

// ---- git helpers ----

fn read_staged_diff(repo: &Path) -> Option<String> {
    git_output_in(repo, &["diff", "--cached", "--no-color"])
}

fn current_branch() -> Option<String> {
    git_output(&["symbolic-ref", "--short", "HEAD"]).map(|s| s.trim().to_string())
}

/// Git stdout → text the scanners can read. **Lossy on purpose.**
///
/// Git calls a file binary only when it finds a NUL in the first 8 KB, so any
/// NUL-free file in a non-UTF-8 encoding (a latin-1 source file, a fixture with
/// one stray high byte) has its RAW BYTES emitted into `git diff` / `git show`.
/// A strict `String::from_utf8` fails on that, and four of `git_output`'s
/// callers read `None` as "nothing to scan": `run_pre_commit`'s forbidden-word
/// layer (`:203`), `check_immutable_artifacts` (`:302`), and BOTH halves of the
/// post-commit verifier (`:331`, `:332`). The last one is why this had to be
/// fixed here rather than at one call site — the post-commit backstop exists to
/// catch what pre-commit missed, and it read the same helper, so it went blind
/// on exactly the input that defeated pre-commit. Neither layer was independent.
///
/// U+FFFD replaces only the invalid bytes, so a forbidden word on an untouched
/// line still matches — which is why lossy beats blocking: a block would refuse
/// the commit without naming the word that tripped it.
///
/// After this, `None` from `git_output` means git actually FAILED (spawn error
/// or non-zero exit), never "the output was not UTF-8".
fn decode_git_stdout(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Run git in `repo` and decode its stdout.
///
/// The explicit directory is what makes the gates testable. Every hook runs
/// with the repo root as its CWD, so production passes `"."` and nothing about
/// the behaviour changes — but a test can point a gate at a tempdir instead of
/// calling `set_current_dir`, which is process-global and races a parallel
/// suite. Without this the only reachable seam was the decode helper, and a
/// test on the decode alone leaves the line the audit actually found (the
/// strict `from_utf8` here) revertible with the suite green.
fn git_output_in(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(decode_git_stdout(out.stdout))
}

fn git_output(args: &[&str]) -> Option<String> {
    git_output_in(Path::new("."), args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_repo(dir: &Path) {
        Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(dir)
            .status()
            .unwrap();
        // Disable signing so test commits don't need a GPG key.
        Command::new("git")
            .args(["config", "commit.gpgsign", "false"])
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test"])
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(dir)
            .status()
            .unwrap();
    }

    #[test]
    fn is_zero_oid_only_matches_all_zeros() {
        assert!(is_zero_oid("0000000000000000000000000000000000000000"));
        assert!(!is_zero_oid("0000000000000000000000000000000000000001"));
        assert!(!is_zero_oid("deadbeef"));
        assert!(!is_zero_oid("")); // empty is not the zero oid
    }

    #[test]
    fn update_is_force_flags_only_non_fast_forward() {
        let local = "1111111111111111111111111111111111111111";
        let remote = "2222222222222222222222222222222222222222";
        let zero = "0000000000000000000000000000000000000000";
        let r = "refs/heads/main";
        let force = |line: String, oracle: fn(&str, &str) -> bool| {
            parse_push_updates(&line)
                .first()
                .is_some_and(|u| update_is_force(u, oracle))
        };

        // Fast-forward: remote IS an ancestor of local → not a force.
        assert!(!force(format!("{r} {local} {r} {remote}"), |_, _| true));
        // Non-fast-forward: remote is NOT an ancestor of local → force.
        assert!(force(format!("{r} {local} {r} {remote}"), |_, _| false));
        // Create (remote all-zero) is never a force, even if the oracle says no.
        assert!(!force(format!("{r} {local} {r} {zero}"), |_, _| false));
        // Delete (local all-zero) is never a force.
        assert!(!force(format!("{r} {zero} {r} {remote}"), |_, _| false));
        // Malformed lines (missing oids) never parse into an update at all.
        assert!(!force("refs/heads/main".to_string(), |_, _| false));
        assert!(!force(String::new(), |_, _| false));
    }

    /// **The push prompt names the refs being pushed, not the checked-out
    /// branch (round 10, B3).** `s-766f4ab9`: HANDS pushed `526-…` from a
    /// checkout of `527-…` and the user approved "Allow `git push` to `527-…`".
    /// The label comes from git's stdin lines; HEAD is only the fallback.
    #[test]
    fn the_push_prompt_names_the_pushed_refs_and_falls_back_to_head() {
        let local = "1111111111111111111111111111111111111111";
        let remote = "2222222222222222222222222222222222222222";
        let zero = "0000000000000000000000000000000000000000";
        let input = format!(
            "refs/heads/526-nanoid-advisory-still-open {local} refs/heads/526-nanoid-advisory-still-open {zero}\n\
             refs/tags/v1.2.0 {local} refs/tags/v1.2.0 {zero}\n\
             refs/heads/old-branch {zero} refs/heads/old-branch {remote}\n\
             refs/heads/short 3333333333333333333333333333333333333333 refs/heads/short\n"
        );
        let updates = parse_push_updates(&input);
        assert_eq!(updates.len(), 3, "a three-field line is dropped, not fatal");
        assert_eq!(updates[0].remote_ref, "refs/heads/526-nanoid-advisory-still-open");
        let names = pushed_ref_names(&updates);
        assert_eq!(
            names,
            vec![
                "526-nanoid-advisory-still-open".to_string(),
                "v1.2.0".to_string(),
                ":old-branch".to_string(),
            ],
            "heads and tags lose their prefix; a delete is spelled `:name`"
        );
        // The label the prompt/action carry: the refs, never HEAD, when git
        // said which refs move…
        assert_eq!(
            push_target_label(&names, || Some("527-reconcile-test-timezone".to_string()))
                .as_deref(),
            Some("526-nanoid-advisory-still-open, v1.2.0, :old-branch")
        );
        // …and HEAD only when stdin carried nothing (a hand-run hook, a push
        // of nothing) — the fallback is a thunk, not read when the refs are
        // known (round 11).
        assert_eq!(
            push_target_label(&[], || Some("527-reconcile-test-timezone".to_string()))
                .as_deref(),
            Some("527-reconcile-test-timezone")
        );
        assert_eq!(push_target_label(&[], || None), None);
        assert_eq!(
            push_target_label(&names, || panic!("HEAD must not be read when the refs are known"))
                .as_deref(),
            Some("526-nanoid-advisory-still-open, v1.2.0, :old-branch")
        );
        // The force check reads the same parsed updates: the delete and the
        // create are never a force; the oracle decides the rest.
        assert!(!pushing_non_fast_forward(&parse_push_updates(&format!(
            "refs/heads/x {zero} refs/heads/x {remote}\n"
        ))));
        // And the hook WIRES it: run_pre_push reads the updates once and labels
        // the prompt from them — the label reaches `decide_push`, not
        // `current_branch()` alone (which is what shipped the wrong branch).
        let src = include_str!("hooks.rs");
        let body = src
            .split("fn run_pre_push(")
            .nth(1)
            .expect("run_pre_push exists")
            .split("\nfn ")
            .next()
            .expect("a split always yields a first part");
        let label_def = body
            .find("push_target_label(&pushed_ref_names(updates)")
            .expect("the prompt label comes from the pushed refs");
        let ask_label = body
            .rfind("let branch = label(updates.get_or_init(read_push_updates));")
            .expect("the ask path labels from the (lazily read) updates");
        let decide = body.find("decide_push(").expect("the hook still asks the app");
        assert!(label_def < ask_label && ask_label < decide, "label → ask, in that order");
        assert_eq!(
            body.matches("current_branch()").count(),
            1,
            "HEAD is read once, as the fallback inside the label — not as the prompt"
        );
        // Lazy: nothing reads stdin unconditionally — an `auto` push and a
        // session-less push return before the read (a hand-run hook or a test
        // binary with an open pipe on stdin must never block on EOF).
        assert!(
            !body.contains("let updates = read_push_updates();"),
            "stdin is read through the OnceCell on the paths that need it, never eagerly"
        );
    }

    #[test]
    fn glob_match_single_star_does_not_cross_slash() {
        assert!(glob_match("migrations/0021_findings.sql", "migrations/*.sql"));
        assert!(glob_match("migrations/0001_init.sql", "migrations/*.sql"));
        assert!(!glob_match("migrations/sub/x.sql", "migrations/*.sql")); // * stops at /
        assert!(!glob_match("src/policy/hooks.rs", "migrations/*.sql"));
        assert!(!glob_match("migrations/notes.txt", "migrations/*.sql")); // wrong suffix
        assert!(glob_match("exact.txt", "exact.txt")); // no star = literal
    }

    #[test]
    fn immutable_violations_blocks_edits_allows_new() {
        // Modified / deleted / renamed committed migration -> violation.
        assert_eq!(
            immutable_violations("M\tmigrations/0021_findings.sql"),
            vec!["migrations/0021_findings.sql".to_string()]
        );
        assert_eq!(
            immutable_violations("D\tmigrations/0021_findings.sql"),
            vec!["migrations/0021_findings.sql".to_string()]
        );
        assert_eq!(
            immutable_violations(
                "R100\tmigrations/0021_findings.sql\tmigrations/0099_renamed.sql"
            ),
            vec!["migrations/0021_findings.sql".to_string()]
        );
        // Newly-added migration is fine (append-only); non-migration edits too.
        assert!(immutable_violations("A\tmigrations/0023_new.sql").is_empty());
        assert!(immutable_violations("M\tsrc/policy/hooks.rs").is_empty());
        // Mixed staging: only the modified committed migration trips it.
        assert_eq!(
            immutable_violations(
                "A\tmigrations/0023_new.sql\nM\tsrc/main.rs\nM\tmigrations/0021_findings.sql"
            ),
            vec!["migrations/0021_findings.sql".to_string()]
        );
    }

    #[test]
    fn install_hooks_into_fresh_repo() {
        let repo = tempdir().unwrap();
        let data = tempdir().unwrap();
        init_repo(repo.path());
        let rep = install_hooks(repo.path(), data.path(), Some("foo")).unwrap();
        assert_eq!(rep.installed.len(), 4);
        assert!(rep.unchanged.is_empty());
        assert!(rep.sidecar.is_empty());
        for name in ["commit-msg", "pre-commit", "post-commit", "pre-push"] {
            let p = repo.path().join(".git/hooks").join(name);
            assert!(p.exists(), "{name} should exist");
            let body = std::fs::read_to_string(&p).unwrap();
            assert!(body.contains(MANAGED_MARKER));
            assert!(body.contains("policy-check"));
        }
        // commit-msg must forward $1
        let cm = std::fs::read_to_string(repo.path().join(".git/hooks/commit-msg")).unwrap();
        assert!(cm.contains("\"$1\""), "commit-msg should forward $1: {cm}");
        // pre-commit must NOT
        let pc = std::fs::read_to_string(repo.path().join(".git/hooks/pre-commit")).unwrap();
        assert!(!pc.contains("\"$1\""));
    }

    #[test]
    fn install_hooks_idempotent() {
        let repo = tempdir().unwrap();
        let data = tempdir().unwrap();
        init_repo(repo.path());
        install_hooks(repo.path(), data.path(), Some("foo")).unwrap();
        let rep = install_hooks(repo.path(), data.path(), Some("foo")).unwrap();
        assert_eq!(rep.unchanged.len(), 4, "second run should change nothing");
        assert!(rep.installed.is_empty());
    }

    /// **An unchanged managed hook is re-made executable** (round 11). git
    /// runs a hook only if it is executable and says nothing when it is not;
    /// `write_executable` writes then chmods in two steps, so a crash between
    /// them — or anything else that dropped the bit — left a byte-identical,
    /// non-executable hook that every later install reported `unchanged` and
    /// never touched: every git-side gate silently off while the report said
    /// success.
    #[cfg(unix)]
    #[test]
    fn install_hooks_restores_the_executable_bit_on_an_unchanged_hook() {
        use std::os::unix::fs::PermissionsExt;
        let repo = tempdir().unwrap();
        let data = tempdir().unwrap();
        init_repo(repo.path());
        install_hooks(repo.path(), data.path(), Some("foo")).unwrap();
        let hook = repo.path().join(".git/hooks/pre-commit");
        let mut perms = std::fs::metadata(&hook).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&hook, perms).unwrap();
        assert_eq!(std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111, 0);

        let rep = install_hooks(repo.path(), data.path(), Some("foo")).unwrap();
        assert!(rep.unchanged.contains(&"pre-commit".to_string()), "content is unchanged");
        assert_ne!(
            std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111,
            0,
            "the unchanged hook must be executable again, or git never runs it"
        );
    }

    #[test]
    fn install_hooks_from_linked_worktree_lands_in_common_hooks_dir() {
        // A linked worktree's `.git` is a FILE; hooks live in the base repo's
        // shared `.git/hooks`. Installing "into the worktree" must write
        // there — the old `.git/hooks` join skipped install entirely
        // (not_a_git_repo), silently dropping the enforcement backstop.
        let base = tempdir().unwrap();
        let data = tempdir().unwrap();
        init_repo(base.path());
        // init_repo leaves an empty repo — the worktree needs a commit.
        std::fs::write(base.path().join("f"), "x").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(base.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-qm", "init"])
            .current_dir(base.path())
            .status()
            .unwrap();
        let wt_holder = tempdir().unwrap();
        let wt = wt_holder.path().join("wt");
        let ok = Command::new("git")
            .args(["worktree", "add", wt.to_str().unwrap(), "-b", "wt-branch"])
            .current_dir(base.path())
            .status()
            .unwrap();
        assert!(ok.success());

        let rep = install_hooks(&wt, data.path(), Some("foo")).unwrap();
        assert!(!rep.not_a_git_repo, "worktree must not read as non-repo");
        assert_eq!(rep.installed.len(), 4);
        for name in ["commit-msg", "pre-commit", "post-commit", "pre-push"] {
            let p = base.path().join(".git/hooks").join(name);
            assert!(p.exists(), "{name} must land in the COMMON hooks dir");
        }
        // Idempotent from the base repo too — same target dir.
        let rep2 = install_hooks(base.path(), data.path(), Some("foo")).unwrap();
        assert_eq!(rep2.unchanged.len(), 4);
    }

    #[test]
    fn added_lines_only_strips_deletions_and_headers() {
        // Uses a fixture word that's NOT in the real forbidden list so the
        // test source itself doesn't trip the pre-commit hook scanning this
        // very file.
        let diff = "diff --git a/x b/x\n\
                    index abc..def 100644\n\
                    --- a/x\n\
                    +++ b/x\n\
                    @@ -1,3 +1,3 @@\n\
                     context line\n\
                    -old line with FORBID\n\
                    +new line lowercase forbid\n";
        let added = added_lines_only(diff);
        assert!(
            !added.contains("FORBID"),
            "deletion must not be scanned: {added:?}"
        );
        assert!(added.contains("new line lowercase forbid"));
        assert!(
            !added.contains("+++"),
            "+++ header must not appear: {added:?}"
        );
        assert!(
            !added.contains("context line"),
            "context must not appear: {added:?}"
        );
    }

    #[test]
    fn commit_msg_blocks_forbidden_word() {
        let data = tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("library/projects/foo")).unwrap();
        std::fs::write(
            data.path().join("library/projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - Acme\n  - Foo-Bar-Baz\n",
        )
        .unwrap();
        // Simulate git writing the commit message file.
        let msg_file = data.path().join("MSG");
        std::fs::write(&msg_file, "feat: helped by Acme\n").unwrap();
        let code = run_commit_msg(data.path(), Some("foo"), &msg_file, None).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn commit_msg_passes_clean_message() {
        let data = tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("library/projects/foo")).unwrap();
        std::fs::write(
            data.path().join("library/projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - Acme\n",
        )
        .unwrap();
        let msg_file = data.path().join("MSG");
        std::fs::write(&msg_file, "feat: clean message\n").unwrap();
        let code = run_commit_msg(data.path(), Some("foo"), &msg_file, None).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn commit_msg_ignores_comment_lines() {
        // Git includes commented-out instruction lines in the message file
        // that don't end up in the actual commit — don't flag them.
        let data = tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("library/projects/foo")).unwrap();
        std::fs::write(
            data.path().join("library/projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - Acme\n",
        )
        .unwrap();
        let msg_file = data.path().join("MSG");
        std::fs::write(
            &msg_file,
            "feat: clean\n# Please enter the commit message — Acme can help\n",
        )
        .unwrap();
        let code = run_commit_msg(data.path(), Some("foo"), &msg_file, None).unwrap();
        assert_eq!(code, 0, "comment lines should not trigger");
    }

    #[test]
    fn install_hooks_writes_sidecar_when_foreign_hook_present() {
        let repo = tempdir().unwrap();
        let data = tempdir().unwrap();
        init_repo(repo.path());
        let hooks_dir = repo.path().join(".git/hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        std::fs::write(
            hooks_dir.join("pre-commit"),
            "#!/bin/sh\necho husky says hi\n",
        )
        .unwrap();
        let rep = install_hooks(repo.path(), data.path(), Some("foo")).unwrap();
        assert!(rep.sidecar.contains(&"pre-commit".to_string()));
        // husky hook untouched
        let body = std::fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
        assert!(body.contains("husky says hi"));
        // sidecar present
        assert!(hooks_dir.join("pre-commit.bot-hq").exists());
    }

    #[test]
    fn install_hooks_no_git_repo() {
        let dir = tempdir().unwrap();
        let data = tempdir().unwrap();
        let rep = install_hooks(dir.path(), data.path(), Some("foo")).unwrap();
        assert!(rep.not_a_git_repo);
    }

    #[test]
    fn run_pre_commit_exits_zero_with_empty_policy() {
        let data = tempdir().unwrap();
        let code = run_pre_commit(data.path(), Some("nope"), Path::new("."), None).unwrap();
        assert_eq!(code, 0);
    }

    /// **A staged non-UTF-8 file must not switch the forbidden-word scan off.**
    ///
    /// Measured before the fix (round-2 audit H1): staging one NUL-free latin-1
    /// file made `git diff --cached` invalid UTF-8, the strict `from_utf8` in
    /// `git_output` returned `None`, `run_pre_commit:203`'s `unwrap_or_default()`
    /// turned that into an empty diff, and the commit passed the gate with the
    /// forbidden word in it — exit 0, no warning.
    ///
    /// Driven by REAL git bytes rather than a hand-written `Vec<u8>`, because
    /// the premise being pinned is git's own behaviour (it emits raw bytes for
    /// any file without a NUL in the first 8 KB). The `is_err()` assertion below
    /// is load-bearing: without it a fixture that quietly became valid UTF-8
    /// would leave this test green while proving nothing.
    ///
    /// **It drives `run_pre_commit`, not the decode helper.** The first version
    /// of this test called `decode_git_stdout` directly and passed with the bug
    /// fully restored — the reviewer measured it — because the line the audit
    /// found is the strict `from_utf8` in `git_output`, and nothing exercised
    /// the join. That is the defect this whole round is about, committed inside
    /// its own fix. `git_output_in`'s explicit repo path is what made the real
    /// wire reachable without `set_current_dir`.
    #[test]
    fn a_staged_non_utf8_file_cannot_hide_a_forbidden_word() {
        let repo = tempdir().unwrap();
        let data = tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("library/projects/p")).unwrap();
        std::fs::write(
            data.path().join("library/projects/p/policy.yaml"),
            "forbidden_in_commits:\n  - Acme-Trailer\n",
        )
        .unwrap();
        init_repo(repo.path());
        // A stand-in term, NOT this repo's real forbidden word: the fixture is
        // itself a staged diff, so spelling the real one here would trip our own
        // pre-commit hook on every commit that touches this file. The policy
        // below is local to the test, so the word is arbitrary — the hyphen is
        // kept because it exercises `contains_word`'s non-word boundary.
        // 0xE9 is latin-1 'é' — invalid UTF-8, and no NUL, so git calls it text.
        std::fs::write(
            repo.path().join("note.txt"),
            b"Acme-Trailer: someone\ncaf\xe9 latin1\n",
        )
        .unwrap();
        Command::new("git")
            .args(["add", "note.txt"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        let out = Command::new("git")
            .args(["diff", "--cached", "--no-color"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(
            String::from_utf8(out.stdout).is_err(),
            "fixture is no longer invalid UTF-8 — this test would prove nothing"
        );

        assert_eq!(
            run_pre_commit(data.path(), Some("p"), repo.path(), None).unwrap(),
            1,
            "the forbidden-word gate went blind on a diff it could not decode"
        );
    }

    /// **The block arm runs** (round 8, M1). Every earlier hook test took
    /// `check_findings_gate`'s "no session id → 0" return, so a gate that
    /// never blocked would have stayed green. With the session id passed in
    /// and one open blocking finding in the DB the pre-commit gate returns 1;
    /// another session's id, or none, returns 0. Plain `#[test]`: the gate
    /// builds its own current-thread runtime, so it must not run inside one.
    #[test]
    fn the_findings_gate_blocks_a_session_commit_and_skips_a_human_one() {
        let data = tempdir().unwrap();
        let db_path = crate::paths::Paths::for_data_dir(data.path().to_path_buf()).db_path;
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let storage = crate::storage::Storage::open(&db_path).await.unwrap();
            storage.create_session("s1", "t", None).await.unwrap();
            storage
                .insert_finding(
                    "s1",
                    "f1",
                    "eyes",
                    crate::storage::FindingSeverity::Blocking,
                    "real bug",
                    Some("a.rs:1"),
                )
                .await
                .unwrap();
        });
        assert_eq!(
            check_findings_gate(data.path(), "pre-commit", Some("s1")),
            1,
            "an open blocking finding must block the session's commit"
        );
        assert_eq!(
            check_findings_gate(data.path(), "pre-commit", Some("s-other")),
            0,
            "findings are session-scoped"
        );
        assert_eq!(
            check_findings_gate(data.path(), "pre-commit", None),
            0,
            "no session context → the gate is N/A (a human commit)"
        );
    }

    /// **The session context is read from the environment in exactly one place
    /// and handed to every hook** (round 8). The join a test cannot exercise is
    /// the one that got cut silently before: `check_findings_gate` reading
    /// `hook_session_id()` itself meant replacing that read with `None` kept
    /// the whole suite green while the gate was off in production. Now the
    /// only reader is `run_cli`, and each dispatch arm passes `sid` — pinned
    /// here in the source. Kill-tested: drop `sid` from any arm → red.
    #[test]
    fn run_cli_is_the_only_reader_of_the_session_env_and_hands_it_to_every_hook() {
        let code = include_str!("hooks.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let prod = &code[..code.find("\n#[cfg(test)]").expect("test module")];
        let calls = prod.matches("hook_session_id()").count()
            - prod.matches("fn hook_session_id()").count();
        assert_eq!(
            calls, 1,
            "hook_session_id() must be called exactly once in production (run_cli)"
        );
        let cli_at = prod.find("pub fn run_cli(").expect("run_cli exists");
        let cli = &prod[cli_at..];
        let cli = &cli[..cli.find("\nfn ").unwrap_or(cli.len())];
        assert!(cli.contains("hook_session_id()"), "run_cli reads the env");
        for hook in [
            "run_commit_msg(",
            "run_pre_commit(",
            "run_post_commit(",
            "run_pre_push(",
            "run_tool_gate(",
        ] {
            let at = cli.find(hook).unwrap_or_else(|| panic!("run_cli dispatches {hook}"));
            // The argument list, to its MATCHING close paren (arguments carry
            // their own parens: `project.as_deref()`).
            let rest = &cli[at + hook.len()..];
            let mut depth = 1usize;
            let mut end = rest.len();
            for (i, ch) in rest.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let args = rest[..end].trim();
            assert!(
                args.ends_with(", sid"),
                "{hook} must receive the session context as its last argument: ({args})"
            );
        }
    }

    #[test]
    fn findings_gate_fail_open_when_db_absent() {
        // No DB at the data_dir → open_blocking_findings returns None (fail-open).
        // A DB hiccup (missing/locked/corrupt) must NEVER block a commit.
        let data = tempdir().unwrap();
        assert_eq!(open_blocking_findings(data.path(), "s1"), None);
    }

    #[tokio::test]
    async fn query_open_blocking_filters_to_open_blocking() {
        // Only OPEN + BLOCKING findings count: advisory and already-disposed are
        // excluded; the scan is scoped to the session.
        let data = tempdir().unwrap();
        let db_path = crate::paths::Paths::for_data_dir(data.path().to_path_buf()).db_path;
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let storage = crate::storage::Storage::open(&db_path).await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        storage
            .insert_finding(
                "s1",
                "f1",
                "eyes",
                crate::storage::FindingSeverity::Blocking,
                "real bug",
                Some("a.rs:1"),
            )
            .await
            .unwrap();
        storage
            .insert_finding(
                "s1",
                "f2",
                "eyes",
                crate::storage::FindingSeverity::Advisory,
                "nit",
                None,
            )
            .await
            .unwrap();
        storage
            .insert_finding(
                "s1",
                "f3",
                "eyes",
                crate::storage::FindingSeverity::Blocking,
                "fixed one",
                None,
            )
            .await
            .unwrap();
        storage
            .disposition_finding(
                "s1",
                "f3",
                crate::storage::FindingStatus::Fixed,
                Some("done"),
                "hands",
            )
            .await
            .unwrap();

        let rows = query_open_blocking(&db_path, "s1").await.unwrap();
        assert_eq!(rows.len(), 1, "only the open blocking finding is returned");
        assert_eq!(rows[0].0, "f1");
        assert_eq!(rows[0].1, "real bug");
        assert_eq!(rows[0].2.as_deref(), Some("a.rs:1"));

        // Unknown session → nothing to gate.
        assert!(query_open_blocking(&db_path, "other")
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn findings_block_body_lists_each_finding() {
        let findings = vec![
            (
                "uid1".to_string(),
                "bug one".to_string(),
                Some("x.rs:1".to_string()),
            ),
            ("uid2".to_string(), "bug two".to_string(), None),
        ];
        let body = findings_block_body(&findings);
        assert!(body.contains("2 unresolved"));
        assert!(body.contains("uid1") && body.contains("bug one") && body.contains("(x.rs:1)"));
        assert!(body.contains("uid2") && body.contains("bug two"));
        assert!(
            body.contains("disposition_finding"),
            "banner must tell the agent how to clear it"
        );
    }

    #[test]
    fn run_pre_push_exits_zero_when_mode_auto() {
        let data = tempdir().unwrap();
        // No policy file → default policy → mode=auto → exit 0
        let code = run_pre_push(data.path(), Some("nope"), None, None, None).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn run_pre_push_blocks_ask_without_session() {
        let data = tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("library/projects/foo")).unwrap();
        std::fs::write(
            data.path().join("library/projects/foo/policy.yaml"),
            "push_gate: ask\n",
        )
        .unwrap();
        // No session context (passed explicitly — the test process itself DOES
        // carry a real BOT_HQ_SESSION_ID whenever an agent runs the suite) →
        // blocked with guidance (exit 1) before any HTTP call.
        let code = run_pre_push(data.path(), Some("foo"), None, None, None).unwrap();
        assert_eq!(code, 1);
    }

    /// **A push gate that cannot read its policy BLOCKS** (E1).
    ///
    /// Every `?` in this file is mapped to exit 0 by `run_policy_check_cli` —
    /// soft-fail, so an internal bug cannot break the user's git workflow. Right
    /// for the advisory hooks, wrong for this one: a malformed `policy.yaml`
    /// made `push_gate: ask` and `force_push: blocked` evaporate silently, which
    /// is the opposite of what this module's doc promises. A gate that cannot
    /// read its policy does not know the push is allowed.
    #[test]
    fn run_pre_push_blocks_when_the_policy_cannot_be_read() {
        let data = tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("library/projects/foo")).unwrap();
        // Valid YAML, wrong SHAPE — a mapping key holding a mapping where a
        // string belongs. This is what a hand-edited policy file fails as.
        std::fs::write(
            data.path().join("library/projects/foo/policy.yaml"),
            "push_gate:\n  ask: yes\n",
        )
        .unwrap();
        assert_eq!(
            run_pre_push(data.path(), Some("foo"), None, None, None).unwrap(),
            1,
            "an unreadable policy let the push through — `push_gate` and \
             `force_push` both silently stopped applying"
        );
    }

    #[tokio::test]
    async fn decide_push_blocks_when_app_not_running() {
        // No signaling-addr file → the app isn't reachable → fail-closed Blocked,
        // with a reason naming the cause (no network call attempted).
        let data = tempdir().unwrap();
        match decide_push(data.path(), "s1", "hands", Some("main"), Some("origin"), &[]).await {
            PushDecision::Blocked(reason) => {
                assert!(reason.contains("not running"), "reason: {reason}");
            }
            _ => panic!("expected Blocked when no signaling addr is present"),
        }
    }

    #[test]
    fn push_response_approved_true_approves() {
        assert_eq!(
            classify_push_response(reqwest::StatusCode::OK, r#"{"approved": true}"#),
            PushDecision::Approved
        );
    }

    #[test]
    fn push_response_approved_false_rejects() {
        assert_eq!(
            classify_push_response(reqwest::StatusCode::OK, r#"{"approved": false}"#),
            PushDecision::Rejected
        );
    }

    #[test]
    fn push_response_missing_field_blocks() {
        assert!(matches!(
            classify_push_response(reqwest::StatusCode::OK, r#"{"other": 1}"#),
            PushDecision::Blocked(_)
        ));
    }

    #[test]
    fn push_response_non_2xx_blocks_even_if_body_approves() {
        // Status is authoritative: a non-2xx blocks regardless of body content.
        assert!(matches!(
            classify_push_response(
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"approved": true}"#
            ),
            PushDecision::Blocked(_)
        ));
    }

    #[test]
    fn push_response_malformed_json_blocks() {
        assert!(matches!(
            classify_push_response(reqwest::StatusCode::OK, "not json {"),
            PushDecision::Blocked(_)
        ));
    }

    #[test]
    fn reject_never_resolves_to_approved() {
        // The fail-closed safety property: only an explicit {"approved": true} on a
        // 2xx may Approve. Reject / missing / malformed / non-2xx never approve.
        let non_approving = [
            (reqwest::StatusCode::OK, r#"{"approved": false}"#),
            (reqwest::StatusCode::OK, r#"{}"#),
            (reqwest::StatusCode::OK, "garbage"),
            (reqwest::StatusCode::FORBIDDEN, r#"{"approved": true}"#),
            (reqwest::StatusCode::BAD_GATEWAY, r#"{"approved": true}"#),
        ];
        for (status, body) in non_approving {
            assert!(
                !matches!(
                    classify_push_response(status, body),
                    PushDecision::Approved
                ),
                "status={status} body={body} must not approve"
            );
        }
    }

    /// Round 12: the pre-push hook forwards git's `$1` (the remote name) — the
    /// app rebuilds a late-approved push as `git push <remote> <oid>:<ref>` and
    /// needs the name the hook was invoked for; commit-msg keeps forwarding
    /// its message-file path; the other hooks stay positional-free.
    #[test]
    fn pre_push_hook_forwards_the_remote_name() {
        let body = |kind| render_hook_body(kind, "/usr/local/bin/bot-hq", Path::new("/d"), None);
        assert!(body(HookKind::PrePush).contains("policy-check pre-push --data-dir /d \"$1\""));
        assert!(body(HookKind::CommitMsg).contains("policy-check commit-msg --data-dir /d \"$1\""));
        assert!(!body(HookKind::PreCommit).contains("$1"));
        assert!(!body(HookKind::PostCommit).contains("$1"));
    }

    /// Round 12: the body the hook POSTs carries what the app needs to rebuild
    /// the push for a late approve — the remote name and the ref updates as
    /// git reported them — and, on a re-run, the nonce it redeems. The field
    /// names are the route's contract (`server.rs::handle_pre_push`).
    #[test]
    fn pre_push_request_body_carries_remote_updates_and_nonce() {
        let u = PushUpdate {
            local_ref: "refs/heads/x".into(),
            local_oid: "1111aaaa".into(),
            remote_ref: "refs/heads/x".into(),
            remote_oid: "0000000000000000000000000000000000000000".into(),
        };
        let body = pre_push_request_body("s1", "hands", Some("x"), Some("origin"), &[u.clone()], None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["session_id"], "s1");
        assert_eq!(v["agent"], "hands");
        assert_eq!(v["branch"], "x");
        assert_eq!(v["remote"], "origin");
        assert_eq!(v["updates"][0]["local_oid"], "1111aaaa");
        assert_eq!(v["updates"][0]["remote_ref"], "refs/heads/x");
        assert!(v["nonce"].is_null(), "a first-time push presents no nonce");
        // The round trip the app makes: the same struct parses back.
        let back: Vec<PushUpdate> = serde_json::from_value(v["updates"].clone()).unwrap();
        assert_eq!(back, vec![u.clone()]);
        let body = pre_push_request_body("s1", "hands", None, None, &[u], Some("n0nce"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["nonce"], "n0nce");
        assert!(v["remote"].is_null() && v["branch"].is_null());
    }

    #[test]
    fn render_hook_body_includes_marker_and_args() {
        let body = render_hook_body(
            HookKind::PreCommit,
            "/usr/local/bin/bot-hq",
            Path::new("/home/u/.bot-hq-dev"),
            Some("acme-app"),
        );
        assert!(body.starts_with("#!/bin/sh"));
        assert!(body.contains(MANAGED_MARKER));
        // On Windows `render_hook_body` double-quotes and forward-slashes the
        // binary path, so Git-for-Windows' MSYS `sh` execs it as a native path.
        let expect_bin = if cfg!(windows) {
            "\"/usr/local/bin/bot-hq\" policy-check pre-commit"
        } else {
            "/usr/local/bin/bot-hq policy-check pre-commit"
        };
        assert!(body.contains(expect_bin), "body: {body}");
        assert!(body.contains("--data-dir /home/u/.bot-hq-dev"));
        assert!(body.contains("--project acme-app"));
    }

    #[test]
    fn cli_dispatch_pre_commit_with_args() {
        let data = tempdir().unwrap();
        let args = vec![
            "pre-commit".to_string(),
            "--data-dir".to_string(),
            data.path().display().to_string(),
            "--project".to_string(),
            "foo".to_string(),
        ];
        let code = run_cli(&args).unwrap();
        // No policy → exit 0
        assert_eq!(code, 0);
    }

    #[test]
    fn cli_dispatch_rejects_missing_data_dir() {
        let args = vec!["pre-commit".to_string()];
        let err = run_cli(&args).unwrap_err();
        assert!(err.to_string().contains("--data-dir"));
    }

    #[test]
    fn pretool_parses_bash_command() {
        let j = r#"{"tool_name":"Bash","tool_input":{"command":"gh issue comment 41 --body x"}}"#;
        assert_eq!(
            parse_pretool_bash_command(j).as_deref(),
            Some("gh issue comment 41 --body x")
        );
    }

    #[test]
    fn pretool_ignores_non_bash_tools() {
        let j = r#"{"tool_name":"Write","tool_input":{"file_path":"/x","content":"y"}}"#;
        assert_eq!(parse_pretool_bash_command(j), None);
    }

    #[test]
    fn pretool_ignores_empty_or_missing_command() {
        assert_eq!(
            parse_pretool_bash_command(r#"{"tool_name":"Bash","tool_input":{"command":"   "}}"#),
            None
        );
        assert_eq!(
            parse_pretool_bash_command(r#"{"tool_name":"Bash","tool_input":{}}"#),
            None
        );
    }

    #[test]
    fn pretool_malformed_json_is_none() {
        assert_eq!(parse_pretool_bash_command("not json at all"), None);
    }

    #[test]
    fn tool_gate_exit_gates_blocks_and_allows() {
        // The reworked hook reads the GLOBAL keyword config (not policy.yaml):
        // a `gate` keyword → exit 2 + a message routing the agent to
        // `action_gate`; `auto_allow`/no-match → exit 0; empty config fails open.
        use crate::policy::tool_gate::{GateMode, GatedKeyword};
        let kws = vec![
            GatedKeyword {
                keyword: "gh issue".into(),
                mode: GateMode::Gate,
            },
            GatedKeyword {
                keyword: "git commit".into(),
                mode: GateMode::AutoAllow,
            },
        ];
        let (code, msg) = tool_gate_exit("gh issue comment 41 --body x", &kws, None);
        assert_eq!(code, 2);
        assert!(
            msg.unwrap().contains("action_gate"),
            "gate message must route the agent to action_gate"
        );
        // auto_allow keyword → allow, no message.
        assert_eq!(tool_gate_exit("git commit -m wip", &kws, None), (0, None));
        // unmatched command → allow.
        assert_eq!(tool_gate_exit("ls -la", &kws, None).0, 0);
        // empty config → fail-open allow.
        assert_eq!(tool_gate_exit("gh issue comment 1", &[], None).0, 0);
    }

    #[test]
    fn gate_refusal_carries_the_command_and_forbids_rewording() {
        // issues.md #29, corrected by measurement: the Aug 4-5 refusals showed no
        // same-command retry loop — 3/5 converted to action_gate correctly, but
        // 2/5 REWORDED to slip past the keyword (one narrated swapping `rm -rf`
        // for `rm -f` + `rmdir`). Embedding the exact call was already shipped and
        // did not stop that, so the refusal must name evasion as the failure.
        use crate::policy::tool_gate::{GateMode, GatedKeyword};
        let kws = vec![GatedKeyword {
            keyword: "rm -rf".into(),
            mode: GateMode::Gate,
        }];
        let (code, msg) = tool_gate_exit("rm -rf ./scratch", &kws, None);
        assert_eq!(code, 2);
        let msg = msg.expect("a gated command must carry a refusal message");
        // The exact command, twice: once as the block subject, once inside the
        // ready-to-paste action_gate call.
        assert!(
            msg.contains("command=\"rm -rf ./scratch\""),
            "refusal must embed the exact action_gate invocation: {msg}"
        );
        assert!(
            msg.contains("Do NOT rewrite the command"),
            "refusal must forbid rewording around the keyword: {msg}"
        );
        assert!(
            msg.contains("out-of-band"),
            "refusal must state where the approved command's output arrives: {msg}"
        );
    }

    // --- issues.md #29(ii): the refusal parks the gate itself ---------------

    #[test]
    fn parked_refusal_stops_the_agent_instead_of_routing_it() {
        let gate = ParkedGate {
            gate_id: "cid-7".into(),
            existing: false,
        };
        let msg = gate_refusal_text("gh pr create --title x", Some(&gate));
        assert!(msg.contains("gate_id: cid-7"), "got: {msg}");
        assert!(msg.contains("gate_status(\"cid-7\")"), "got: {msg}");
        assert!(
            msg.contains("Do NOT call `action_gate`"),
            "a parked command must not be parked a second time: {msg}"
        );
        // The dedupe case says so, so a retry doesn't read as a fresh ask.
        let existing = ParkedGate {
            gate_id: "cid-7".into(),
            existing: true,
        };
        assert!(
            gate_refusal_text("gh pr create --title x", Some(&existing))
                .contains("ALREADY awaiting"),
            "an already-pending command must be named as such"
        );
        // Both shapes keep the command and the anti-rewording clause.
        for parked in [Some(&gate), None] {
            let m = gate_refusal_text("gh pr create --title x", parked);
            assert!(m.contains("gh pr create --title x"), "got: {m}");
            assert!(m.contains("Do NOT rewrite the command"), "got: {m}");
        }
        // Unparked still routes the agent to action_gate itself.
        assert!(gate_refusal_text("gh pr create --title x", None)
            .contains("Call the `action_gate` MCP tool"));
    }

    #[test]
    fn classify_park_response_promises_a_gate_only_on_a_real_one() {
        use reqwest::StatusCode;
        assert_eq!(
            classify_park_response(StatusCode::OK, r#"{"gate_id":"abc","existing":true}"#),
            Some(ParkedGate {
                gate_id: "abc".into(),
                existing: true
            })
        );
        // `existing` defaults to false when absent.
        assert_eq!(
            classify_park_response(StatusCode::OK, r#"{"gate_id":"abc"}"#),
            Some(ParkedGate {
                gate_id: "abc".into(),
                existing: false
            })
        );
        // Anything that doesn't prove a gate exists → None, so the refusal
        // falls back to telling the agent to call action_gate. Promising a
        // gate that isn't there would strand the command with nobody asked.
        assert_eq!(classify_park_response(StatusCode::OK, r#"{"ok":true}"#), None);
        assert_eq!(classify_park_response(StatusCode::OK, r#"{"gate_id":"  "}"#), None);
        assert_eq!(classify_park_response(StatusCode::OK, "not json"), None);
        assert_eq!(
            classify_park_response(StatusCode::INTERNAL_SERVER_ERROR, r#"{"gate_id":"abc"}"#),
            None
        );
    }

    #[tokio::test]
    async fn park_gate_returns_none_when_the_app_is_not_running() {
        // No signaling-addr file → no network call, no gate promised.
        let data = tempdir().unwrap();
        assert_eq!(
            park_gate(data.path(), "s1", "hands", "rm -rf /tmp/x").await,
            None
        );
    }
}
