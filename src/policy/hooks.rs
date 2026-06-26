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
    match sub.as_str() {
        "commit-msg" => {
            let path = msg_file
                .or_else(|| positional.into_iter().next().map(PathBuf::from))
                .ok_or_else(|| {
                    anyhow!("commit-msg needs the message file path (as positional or --msg-file)")
                })?;
            run_commit_msg(&data_dir, project.as_deref(), &path)
        }
        "pre-commit" => run_pre_commit(&data_dir, project.as_deref()),
        "post-commit" => run_post_commit(&data_dir, project.as_deref(), session.as_deref()),
        "pre-push" => run_pre_push(&data_dir, project.as_deref()),
        "tool-gate" => run_tool_gate(&data_dir),
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
fn run_commit_msg(data_dir: &Path, project: Option<&str>, msg_path: &Path) -> Result<i32> {
    audit_at_hook(data_dir, project, "commit-msg");
    let policy = Policy::resolve(data_dir, project, hook_session_id().as_deref())?;
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
fn run_pre_commit(data_dir: &Path, project: Option<&str>) -> Result<i32> {
    audit_at_hook(data_dir, project, "pre-commit");
    // Layer 1 — EYES-sign-off gate. Independent of the forbidden-word policy, so
    // it must run BEFORE the empty-list early return below (a project with no
    // forbidden words still needs review-completion enforced).
    if check_findings_gate(data_dir, "pre-commit") != 0 {
        return Ok(1);
    }
    // Layer 2 — immutable-artifact guard. Always-on (policy-independent), so it
    // runs before the empty-forbidden-list early return below. Blocks a sweep or
    // refactor from editing a committed append-only file (e.g. an applied sqlx
    // migration whose bytes sqlx checksums) — editing one breaks boot.
    if check_immutable_artifacts() != 0 {
        return Ok(1);
    }
    // Layer 3 — forbidden-word scan.
    let policy = Policy::resolve(data_dir, project, hook_session_id().as_deref())?;
    if policy.forbidden_in_commits.is_empty() {
        return Ok(0);
    }
    let diff = read_staged_diff().unwrap_or_default();
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
fn check_immutable_artifacts() -> i32 {
    if matches!(
        std::env::var("BOTHQ_ALLOW_IMMUTABLE_EDIT").as_deref(),
        Ok("1")
    ) {
        return 0;
    }
    let Some(status) = git_output(&["diff", "--cached", "--name-status"]) else {
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
fn run_post_commit(data_dir: &Path, project: Option<&str>, session: Option<&str>) -> Result<i32> {
    audit_at_hook(data_dir, project, "post-commit");
    let policy = Policy::resolve(data_dir, project, hook_session_id().as_deref())?;
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
        let log = ViolationsLog::new(data_dir);
        // Best-effort log. Use a tokio runtime since the log API is async.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building runtime for post-commit log")?;
        rt.block_on(async {
            let _ = log
                .record(
                    session.unwrap_or("<post-commit>").to_string(),
                    "git-hook".to_string(),
                    ViolationKind::CommitGrep,
                    format!("git commit (sha={sha_short})"),
                    ViolationOutcome::Denied,
                    Some(format!(
                        "forbidden word '{word}' detected post-commit by hook"
                    )),
                )
                .await;
        });
    }
    Ok(0)
}

/// pre-push handler. Allows the push when `push_gate == auto` (exit 0). When
/// `push_gate == ask` AND the push originates inside a live bot-hq session, it
/// `BOT_HQ_AGENT` env (trimmed, non-empty) or the "brian" default. Shared by the
/// pre-push + findings-gate hooks so the fallback can't drift between them.
fn hook_agent() -> String {
    std::env::var("BOT_HQ_AGENT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "brian".to_string())
}

/// True for git's all-zero object id (the remote side of a ref create / the
/// local side of a delete).
fn is_zero_oid(oid: &str) -> bool {
    !oid.is_empty() && oid.bytes().all(|b| b == b'0')
}

/// Classify one pre-push stdin line — `<local ref> <local oid> <remote ref>
/// <remote oid>` — as a non-fast-forward (force) update, given an ancestry oracle
/// `is_ancestor(remote_oid, local_oid)`. Creates (remote all-zero), deletes
/// (local all-zero), and malformed lines are NOT force updates. Pure, so the
/// classification is unit-testable without a git process.
fn line_is_force(line: &str, is_ancestor: impl Fn(&str, &str) -> bool) -> bool {
    let mut f = line.split_whitespace();
    let _local_ref = f.next();
    let local_oid = f.next().unwrap_or("");
    let _remote_ref = f.next();
    let remote_oid = f.next().unwrap_or("");
    if local_oid.is_empty() || remote_oid.is_empty() {
        return false;
    }
    if is_zero_oid(local_oid) || is_zero_oid(remote_oid) {
        return false;
    }
    // Non-fast-forward = the remote tip is not an ancestor of the local tip.
    !is_ancestor(remote_oid, local_oid)
}

/// Whether the in-flight push rewrites published history — git's pre-push signal
/// for `--force` / `--force-with-lease` (the flag itself is never passed to the
/// hook). Reads the ref updates on stdin and asks git for ancestry. Fail-OPEN
/// (returns false) if stdin can't be read, so a read glitch never blocks a plain
/// fast-forward push. A remote tip missing locally makes `--is-ancestor` error,
/// which is treated as a rewrite (safe direction for a `blocked` policy).
fn pushing_non_fast_forward() -> bool {
    use std::io::Read;
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return false;
    }
    input.lines().any(|line| {
        line_is_force(line, |remote, local| {
            std::process::Command::new("git")
                .args(["merge-base", "--is-ancestor", remote, local])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
    })
}

/// Best-effort fail-closed `ForcePush`/Denied violation for a force-push the hook
/// blocked. Mirrors `log_push_block`.
async fn log_force_push_block(
    data_dir: &Path,
    session_id: &str,
    agent: &str,
    branch: Option<&str>,
) {
    let action = crate::policy::push_gate_action(branch);
    let log = ViolationsLog::new(data_dir);
    let _ = log
        .record(
            session_id.to_string(),
            agent.to_string(),
            ViolationKind::ForcePush,
            action,
            ViolationOutcome::Denied,
            Some("pre-push blocked: force_push policy is 'blocked'".into()),
        )
        .await;
}

/// POSTs the running app's `/hooks/pre-push` route to surface a per-push
/// Approve/Reject prompt (reusing the same `request_approval` machinery as the
/// agent-facing tools), blocking until the user picks: Approve → exit 0,
/// Reject → exit 1. Fail-closed (exit 1 + a `PushGate`/Denied violation) when
/// the app can't be reached. A push with no `BOT_HQ_SESSION_ID` (e.g. a human
/// pushing from a terminal) is blocked with guidance — `ask` only prompts a
/// session's user.
fn run_pre_push(data_dir: &Path, project: Option<&str>) -> Result<i32> {
    audit_at_hook(data_dir, project, "pre-push");
    // EYES-sign-off backstop: a push must not carry unresolved blocking findings
    // (catches a commit created before the finding was filed, an --amend, or a
    // bypassed pre-commit). Independent of push_gate; fail-open on DB errors.
    if check_findings_gate(data_dir, "pre-push") != 0 {
        return Ok(1);
    }
    let session_id = hook_session_id();
    let policy = Policy::resolve(data_dir, project, session_id.as_deref())?;
    use crate::policy::{ForcePushMode, PushGateMode};

    // force_push gate — independent of push_gate and checked FIRST, so a
    // force-push can't ride through on push_gate=auto. A non-fast-forward push is
    // git's pre-push signal for --force / --force-with-lease (the flag is never
    // passed to the hook). Blocked outright when force_push == Blocked.
    if matches!(policy.force_push, ForcePushMode::Blocked) && pushing_non_fast_forward() {
        let branch = current_branch();
        if let Some(sid) = session_id.as_deref() {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(log_force_push_block(data_dir, sid, &hook_agent(), branch.as_deref()));
            }
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

    let branch = current_branch();

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

    // One non-alarming line so the agent doesn't mistake the wait for a block and
    // try to work around it. Silent until the user answers.
    eprintln!(
        "bot-hq pre-push: awaiting user approval for `git push`{} (session {session_id})…",
        branch
            .as_deref()
            .map(|b| format!(" to `{b}`"))
            .unwrap_or_default()
    );

    // The hook is a fresh subprocess that can't reach the running app's bridge
    // directly — POST `/hooks/pre-push` and block on the user's pick. One
    // current-thread runtime drives both the HTTP call and the fail-closed
    // violation log (mirrors run_post_commit).
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
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

    match rt.block_on(decide_push(
        data_dir,
        &session_id,
        &agent,
        branch.as_deref(),
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
            rt.block_on(log_push_block(
                data_dir,
                &session_id,
                &agent,
                branch.as_deref(),
                &reason,
            ));
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
) -> PushDecision {
    let Some(addr) = crate::paths::read_signaling_addr(data_dir) else {
        return PushDecision::Blocked("bot-hq is not running (no signaling address)".into());
    };
    let url = format!("http://{addr}/hooks/pre-push");
    let body = serde_json::json!({
        "session_id": session_id,
        "agent": agent,
        "branch": branch,
    })
    .to_string();

    // Generous timeout — the user may take minutes to decide; a push isn't
    // time-critical.
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()
    {
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

/// Best-effort fail-closed violation record (`PushGate` / Denied) for a push
/// the hook blocked because the prompt couldn't be surfaced.
async fn log_push_block(
    data_dir: &Path,
    session_id: &str,
    agent: &str,
    branch: Option<&str>,
    reason: &str,
) {
    let action = crate::policy::push_gate_action(branch);
    let log = ViolationsLog::new(data_dir);
    let _ = log
        .record(
            session_id.to_string(),
            agent.to_string(),
            ViolationKind::PushGate,
            action,
            ViolationOutcome::Denied,
            Some(format!("pre-push blocked: {reason}")),
        )
        .await;
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
fn run_tool_gate(data_dir: &Path) -> Result<i32> {
    use std::io::Read;
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        return Ok(0); // fail-open: couldn't read the payload
    }
    let Some(command) = parse_pretool_bash_command(&buf) else {
        return Ok(0); // not a Bash tool call (or empty command) → allow
    };
    // Prefer the session's frozen Tool-Gate list from its canonical
    // session-policy snapshot (seeded at spawn, gear-tab-editable). Only fall
    // back to the GLOBAL `tool-gate.json` when there's no session id or no
    // snapshot on disk. Reading the snapshot is fail-open: any read/parse error
    // resolves to None → global list, mirroring the rest of this hook's posture.
    let keywords = match hook_session_id().and_then(|sid| {
        crate::policy::session_policy::read_session_policy(data_dir, &sid)
            .ok()
            .flatten()
    }) {
        Some(sp) => sp.tool_gate,
        None => crate::policy::tool_gate::load(data_dir),
    };
    let (code, message) = tool_gate_exit(&command, &keywords);
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
) -> (i32, Option<String>) {
    use crate::policy::tool_gate::GateMode;
    match crate::policy::tool_gate::match_keyword("Bash", command, keywords) {
        Some(GateMode::Gate) => (
            2,
            Some(format!(
                "BLOCKED by the bot-hq Tool Gate: `{command}`.\n\
                 This command is gated. Do NOT retry it directly — call the \
                 `action_gate` MCP tool with command=\"{command}\". bot-hq will \
                 surface an Approve/Reject prompt to the user and, on approve, run \
                 the command in your working repo and return its output."
            )),
        ),
        // auto_allow or no match → allow the agent's direct Bash call.
        _ => (0, None),
    }
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
    /// commit-msg gets the message file path passed as $1 from git. Others
    /// receive no positional args.
    fn passes_dollar_one(self) -> bool {
        matches!(self, HookKind::CommitMsg)
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
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
fn check_findings_gate(data_dir: &Path, hook: &str) -> i32 {
    let Some(session_id) = hook_session_id() else {
        return 0; // no bot-hq session context (e.g. a human commit) → gate N/A
    };
    let Some(findings) = open_blocking_findings(data_dir, &session_id) else {
        return 0; // fail-open: DB unreadable for any reason
    };
    if findings.is_empty() {
        return 0;
    }
    eprintln!("{}", blocked_banner(hook, &findings_block_body(&findings)));
    log_findings_block(data_dir, hook, &session_id, findings.len());
    1
}

/// Best-effort audit record for a findings-gate block (`Findings` / Denied), so
/// violations.jsonl shows the gate fired — mirrors `run_post_commit`'s logging
/// (own current-thread runtime; the hook is a sync subprocess). Never fails the
/// hook: a logging error is swallowed (the block already landed via stderr).
fn log_findings_block(data_dir: &Path, hook: &str, session_id: &str, n: usize) {
    let agent = hook_agent();
    let action = if hook == "pre-push" { "git push" } else { "git commit" };
    let log = ViolationsLog::new(data_dir);
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    rt.block_on(async {
        let _ = log
            .record(
                session_id.to_string(),
                agent,
                ViolationKind::Findings,
                action.to_string(),
                ViolationOutcome::Denied,
                Some(format!("{n} unresolved EYES blocking finding(s)")),
            )
            .await;
    });
}

/// Read open BLOCKING findings for `session_id` from the DB, read-only. Returns
/// `None` on ANY error (the caller treats None as fail-open → proceed). Builds
/// its own current-thread runtime — the hook runs in a sync context.
fn open_blocking_findings(
    data_dir: &Path,
    session_id: &str,
) -> Option<Vec<(String, String, Option<String>)>> {
    let db_path = crate::paths::Paths::for_data_dir(data_dir.to_path_buf()).db_path;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    match rt.block_on(query_open_blocking(&db_path, session_id)) {
        Ok(rows) => Some(rows),
        Err(e) => {
            eprintln!(
                "bot-hq: EYES-findings gate could not read the DB ({e}); proceeding (fail-open)."
            );
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
    let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT finding_uid, summary, code_ref FROM findings \
         WHERE session_id = ? AND status = 'open' AND severity = 'blocking' \
         ORDER BY id ASC",
    )
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
        "{n} unresolved EYES blocking finding(s) — blocked.\n\n{list}\n\n\
         Resolve each before retrying: call `disposition_finding(finding_id, status, reason)` \
         with status='fixed' (reference the fix) or 'rebutted' (justify why). A rebuttal needs \
         no EYES agreement, so this cannot deadlock. Do NOT bypass with --no-verify.\n",
        n = findings.len(),
    )
}

// ---- git helpers ----

fn read_staged_diff() -> Option<String> {
    git_output(&["diff", "--cached", "--no-color"])
}

fn current_branch() -> Option<String> {
    git_output(&["symbolic-ref", "--short", "HEAD"]).map(|s| s.trim().to_string())
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
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
    fn line_is_force_flags_only_non_fast_forward() {
        let local = "1111111111111111111111111111111111111111";
        let remote = "2222222222222222222222222222222222222222";
        let zero = "0000000000000000000000000000000000000000";
        let r = "refs/heads/main";

        // Fast-forward: remote IS an ancestor of local → not a force.
        assert!(!line_is_force(&format!("{r} {local} {r} {remote}"), |_, _| true));
        // Non-fast-forward: remote is NOT an ancestor of local → force.
        assert!(line_is_force(&format!("{r} {local} {r} {remote}"), |_, _| false));
        // Create (remote all-zero) is never a force, even if the oracle says no.
        assert!(!line_is_force(&format!("{r} {local} {r} {zero}"), |_, _| false));
        // Delete (local all-zero) is never a force.
        assert!(!line_is_force(&format!("{r} {zero} {r} {remote}"), |_, _| false));
        // Malformed lines (missing oids) are never a force.
        assert!(!line_is_force("refs/heads/main", |_, _| false));
        assert!(!line_is_force("", |_, _| false));
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
        let code = run_commit_msg(data.path(), Some("foo"), &msg_file).unwrap();
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
        let code = run_commit_msg(data.path(), Some("foo"), &msg_file).unwrap();
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
        let code = run_commit_msg(data.path(), Some("foo"), &msg_file).unwrap();
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
        let code = run_pre_commit(data.path(), Some("nope")).unwrap();
        assert_eq!(code, 0);
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
                "rain",
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
                "rain",
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
                "rain",
                crate::storage::FindingSeverity::Blocking,
                "fixed one",
                None,
            )
            .await
            .unwrap();
        storage
            .disposition_finding(
                "f3",
                crate::storage::FindingStatus::Fixed,
                Some("done"),
                "brian",
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
        let code = run_pre_push(data.path(), Some("nope")).unwrap();
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
        // The cargo test process has no BOT_HQ_SESSION_ID, so this push has no
        // session context → blocked with guidance (exit 1) before any HTTP call.
        let code = run_pre_push(data.path(), Some("foo")).unwrap();
        assert_eq!(code, 1);
    }

    #[tokio::test]
    async fn decide_push_blocks_when_app_not_running() {
        // No signaling-addr file → the app isn't reachable → fail-closed Blocked,
        // with a reason naming the cause (no network call attempted).
        let data = tempdir().unwrap();
        match decide_push(data.path(), "s1", "brian", Some("main")).await {
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
        assert!(body.contains("/usr/local/bin/bot-hq policy-check pre-commit"));
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
        let (code, msg) = tool_gate_exit("gh issue comment 41 --body x", &kws);
        assert_eq!(code, 2);
        assert!(
            msg.unwrap().contains("action_gate"),
            "gate message must route the agent to action_gate"
        );
        // auto_allow keyword → allow, no message.
        assert_eq!(tool_gate_exit("git commit -m wip", &kws), (0, None));
        // unmatched command → allow.
        assert_eq!(tool_gate_exit("ls -la", &kws).0, 0);
        // empty config → fail-open allow.
        assert_eq!(tool_gate_exit("gh issue comment 1", &[]).0, 0);
    }
}
