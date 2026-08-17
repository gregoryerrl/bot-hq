//! Session document (IPAV tabs) commands + Apply-tab git diff.

use crate::core::AppState as CoreAppState;
use crate::signaling::SignalingBridge;
use crate::storage::{AgentConfig, SessionDocument, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SessionDocumentView {
    pub id: i64,
    pub session_id: String,
    pub slug: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub phase: Option<String>,
}

impl From<SessionDocument> for SessionDocumentView {
    fn from(d: SessionDocument) -> Self {
        Self {
            id: d.id,
            session_id: d.session_id,
            slug: d.slug,
            body: d.body,
            created_at: d.created_at,
            updated_at: d.updated_at,
            phase: d.phase,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn session_doc_search(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    query: Option<String>,
    phase: Option<String>,
) -> Result<Vec<SessionDocumentView>, AppError> {
    let docs = bridge
        .session_doc_search(&session_id, query.as_deref(), phase.as_deref())
        .await?;
    Ok(docs.into_iter().map(Into::into).collect())
}

/// One classified line of a unified `git diff`. `kind` is one of
/// `"add" | "remove" | "hunk" | "file" | "context"` — order-sensitive
/// classification per [`parse_diff_lines`].
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct DiffLine {
    pub kind: String,
    pub text: String,
}

/// Classify each line of a unified `git diff` for color-coded rendering.
/// Order-sensitive: `--- ` / `+++ ` (file headers, trailing space) must be
/// checked BEFORE the single-char `+` / `-` to avoid misclassifying file
/// markers as add/remove lines.
pub fn parse_diff_lines(diff: &str) -> Vec<DiffLine> {
    diff.lines()
        .map(|line| {
            let kind = if line.starts_with("diff --git ")
                || line.starts_with("index ")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
                || line.starts_with("new file mode ")
                || line.starts_with("deleted file mode ")
                || line.starts_with("similarity index ")
                || line.starts_with("rename from ")
                || line.starts_with("rename to ")
            {
                "file"
            } else if line.starts_with("@@ ") {
                "hunk"
            } else if line.starts_with('+') {
                "add"
            } else if line.starts_with('-') {
                "remove"
            } else {
                "context"
            };
            DiffLine {
                kind: kind.to_string(),
                text: line.to_string(),
            }
        })
        .collect()
}

/// Add-only diff for each untracked, non-ignored file in `repo`, so brand-new
/// files — invisible to plain `git diff`, which is tracked-only — still render
/// in the Apply tab. Side-effect free: the diff is synthesized in-process from
/// the file bytes (no `git add -N`, no index mutation). One `git ls-files`
/// subprocess total — the old shape spawned a `git diff --no-index` PER FILE,
/// on the worktree-change hot path.
fn untracked_diff(repo: &std::path::Path) -> String {
    // NUL-separated list of untracked paths, honoring .gitignore via
    // --exclude-standard (so target/, node_modules/, etc. stay out).
    let listing = match std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return String::new(),
    };
    let mut acc = String::new();
    for raw in listing.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let path = String::from_utf8_lossy(raw);
        acc.push_str(&synthetic_new_file_diff(repo, &path));
    }
    acc
}

/// Unified add-only diff for one untracked file, mirroring the shape
/// `git diff --no-index <null> <file>` used to produce: `new file mode`
/// (100755 for executables), git's binary heuristic (NUL byte in the first
/// 8 KiB), no body for an empty file, and the `\ No newline at end of file`
/// marker. A vanished/unreadable file yields nothing, like the old skip.
fn synthetic_new_file_diff(repo: &std::path::Path, rel: &str) -> String {
    let abs = repo.join(rel);
    let Ok(bytes) = std::fs::read(&abs) else {
        return String::new();
    };
    let mut out = format!(
        "diff --git a/{rel} b/{rel}\nnew file mode {}\n",
        new_file_mode(&abs)
    );
    if bytes[..bytes.len().min(8000)].contains(&0) {
        out.push_str(&format!("Binary files /dev/null and b/{rel} differ\n"));
        return out;
    }
    if bytes.is_empty() {
        // Git shows a 0-byte new file as headers only — no ---/+++, no hunk.
        return out;
    }
    out.push_str(&format!("--- /dev/null\n+++ b/{rel}\n"));
    let text = String::from_utf8_lossy(&bytes);
    let ends_with_newline = text.ends_with('\n');
    // split('\n') (not `lines()`) so a `\r` in CRLF files stays in the line
    // bytes, as git emits it. A trailing newline leaves one empty tail entry.
    let mut lines: Vec<&str> = text.split('\n').collect();
    if ends_with_newline {
        lines.pop();
    }
    if lines.len() == 1 {
        out.push_str("@@ -0,0 +1 @@\n");
    } else {
        out.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
    }
    for line in &lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    if !ends_with_newline {
        out.push_str("\\ No newline at end of file\n");
    }
    out
}

/// Git file mode for a new file: `100755` when any execute bit is set (unix),
/// else `100644`.
#[cfg(unix)]
fn new_file_mode(path: &std::path::Path) -> &'static str {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(m) if m.permissions().mode() & 0o111 != 0 => "100755",
        _ => "100644",
    }
}

#[cfg(not(unix))]
fn new_file_mode(_path: &std::path::Path) -> &'static str {
    "100644"
}

/// Result of `compute_apply_diff`: the classified diff lines plus an
/// optional human-readable note (e.g., the session-start anchor was lost
/// and we fell back to `git diff HEAD`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ComputeApplyDiffResult {
    pub lines: Vec<DiffLine>,
    pub note: Option<String>,
}

/// Run `git diff --no-color <session_start_sha>` (falling back to
/// `git diff HEAD` if the anchor was never captured) inside the session's
/// `working_repo_path`, then classify each line via [`parse_diff_lines`].
///
/// Returns an empty `lines` Vec with a note if the diff is empty, the
/// session isn't running, or the session has no working repo. Errors only
/// when the git invocation itself can't be spawned.
#[tauri::command]
#[specta::specta]
pub async fn compute_apply_diff(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<ComputeApplyDiffResult, AppError> {
    compute_apply_diff_inner(&core, session_id).await
}

/// Shared logic behind the `compute_apply_diff` command and the plugin
/// proxy's catalog arm (tauri_cmd/plugin_api.rs).
pub(crate) async fn compute_apply_diff_inner(
    core: &CoreAppState,
    session_id: String,
) -> Result<ComputeApplyDiffResult, AppError> {
    let Some(repo) = core.working_repo_path(&session_id).await else {
        return Ok(ComputeApplyDiffResult {
            lines: Vec::new(),
            note: Some("(no working repo for this session)".to_string()),
        });
    };
    let start_sha = core.session_start_sha(&session_id).await;

    let result = tokio::task::spawn_blocking(
        move || -> std::io::Result<(String, Option<String>)> {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-C").arg(&repo).arg("diff").arg("--no-color");
            let note = if let Some(ref sha) = start_sha {
                cmd.arg(sha);
                None
            } else {
                cmd.arg("HEAD");
                Some(
                    "(session-start anchor lost \u{2014} showing working-tree diff only)"
                        .to_string(),
                )
            };
            let out = cmd.output()?;
            if !out.status.success() {
                return Ok((String::new(), Some("git diff failed".to_string())));
            }
            let mut diff = String::from_utf8_lossy(&out.stdout).into_owned();
            // Append add-only diffs for untracked/new files — the `git diff`
            // above is tracked-only, so without this a brand-new file (`??` in
            // `git status`) is invisible in the Apply tab.
            diff.push_str(&untracked_diff(&repo));
            Ok((diff, note))
        },
    )
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking join: {e}")))?
    .map_err(|e| AppError::Internal(format!("git diff io: {e}")))?;

    let (diff, note) = result;
    if diff.trim().is_empty() {
        return Ok(ComputeApplyDiffResult {
            lines: Vec::new(),
            note: note.or_else(|| Some("(no changes)".to_string())),
        });
    }
    Ok(ComputeApplyDiffResult {
        lines: parse_diff_lines(&diff),
        note,
    })
}

/// Summarize a session document via a one-shot, headless `claude -p` call —
/// a TL;DR for users who don't want to read the full I/P/A/V doc. The model is
/// the session's slot-0 participant's, then that agent's config (same chain the
/// live agents use, via [`resolve_spawn_config`]). Bounded by a 60s timeout; the child is
/// killed on drop. Runs `--max-turns 1 --strict-mcp-config` so it cannot loop,
/// use tools, or touch MCP — a pure text response.
#[tauri::command]
#[specta::specta]
pub async fn summarize_session_doc(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
    slug: String,
) -> Result<String, AppError> {
    let Some(doc) = bridge.session_doc_read(&session_id, &slug).await? else {
        return Err(AppError::Internal(format!(
            "no document '{slug}' in this session"
        )));
    };

    // The model the session's FIRST participant runs on; else
    // `resolve_spawn_config` falls through to the hardcoded default.
    //
    // rc3 D10: this read `sessions.slot0_model_id`, which the create paths no
    // longer write — it would have silently degraded to the app default on every
    // session. The roster is where a participant's model lives now, and slot 0
    // is the same participant the old column named. Round 9: it also read the
    // `default_model_id` app setting first — a key nothing has WRITTEN since
    // rc3 D8 retired the Agents tab (a role owns its default model), so that
    // branch was dead; dropped rather than kept as a promise.
    let session_model = storage
        .participants_for_session(&session_id)
        .await
        .ok()
        .and_then(|roster| roster.into_iter().next())
        .and_then(|p| p.model_id)
        .filter(|m| !m.is_empty());
    let model_id = session_model;
    // The summarizer is a one-off subprocess, not a session participant; the
    // name is only the label `resolve_spawn_config` puts on the config it
    // returns and the `agent_configs` key it may fall back to.
    let cfg =
        crate::core::session::resolve_spawn_config(&storage, "doc-summarizer", model_id.as_deref())
            .await;

    let prompt = format!(
        "Summarize the document below in 3-5 concise, plain-English bullet points \
         (a TL;DR). Output only the bullet points, nothing else.\n\n---\n{}\n---",
        doc.body
    );

    tokio::time::timeout(Duration::from_secs(60), run_summarizer(cfg, prompt))
        .await
        .map_err(|_| AppError::Internal("summary timed out after 60s".into()))?
}

/// Build a one-shot, tool-free headless `claude -p` command carrying the
/// resolved model's env (model id + token + gateway via the normalizing proxy,
/// exactly as live-agent spawn does — see `agents::spawn::build_command`).
/// Shared by the doc summarizer and the model pre-flight probe (B5).
fn headless_claude_cmd(cfg: &AgentConfig, prompt: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("claude");
    cmd.arg("-p")
        .arg(prompt)
        .args(["--output-format", "text"])
        .args(["--max-turns", "1"])
        .arg("--strict-mcp-config")
        .env("ANTHROPIC_MODEL", &cfg.model_name)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(token) = cfg.auth_token.as_deref().filter(|t| !t.is_empty()) {
        cmd.env("ANTHROPIC_AUTH_TOKEN", token);
    }
    if let Some(base) = crate::agents::llm_proxy::resolve_anthropic_base_url(
        cfg.base_url.as_deref(),
        crate::agents::llm_proxy::proxy_addr(),
    ) {
        cmd.env("ANTHROPIC_BASE_URL", base);
    }
    cmd
}

/// Spawn the one-shot summarizer subprocess with the resolved model's env and
/// return its trimmed stdout. Separated so the timeout wrapper above stays terse.
async fn run_summarizer(cfg: AgentConfig, prompt: String) -> Result<String, AppError> {
    crate::agents::spawn::ensure_claude_runnable("claude")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let out = headless_claude_cmd(&cfg, &prompt)
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("failed to spawn claude: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(AppError::Internal(format!(
            "summarizer exited with {}: {}",
            out.status,
            stderr.trim()
        )));
    }
    let summary = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if summary.is_empty() {
        return Err(AppError::Internal(
            "summarizer returned no output".into(),
        ));
    }
    Ok(summary)
}

/// Outcome of a pre-flight model probe (B5).
#[derive(Debug, Clone, Serialize, Type)]
pub struct ValidateResult {
    pub ok: bool,
    pub message: String,
}

/// Pre-flight check a saved model before a session uses it (B5): a one-shot,
/// headless `claude -p` ping through the model's resolved env (token + gateway,
/// via the same normalizing proxy live agents use). Surfaces a bad token, wrong
/// model id, or unreachable gateway at configure-time instead of as a silent
/// mid-session API error. Bounded by a 30s timeout; the child is killed on drop.
#[tauri::command]
#[specta::specta]
pub async fn validate_model(
    storage: tauri::State<'_, Arc<Storage>>,
    model_id: String,
) -> Result<ValidateResult, AppError> {
    // A one-shot probe of a MODEL, not of any participant — an explicit
    // `model_id` is always supplied, so the name is only the config's label.
    let cfg =
        crate::core::session::resolve_spawn_config(&storage, "model-probe", Some(&model_id)).await;
    Ok(
        match tokio::time::timeout(Duration::from_secs(30), probe_model(cfg)).await {
            Ok(result) => result,
            Err(_) => ValidateResult {
                ok: false,
                message: "Timed out after 30s — the gateway is unreachable or too slow.".into(),
            },
        },
    )
}

/// One-shot model ping. Non-empty output on a clean exit ⇒ reachable; a non-zero
/// exit or empty output ⇒ failure with the captured detail (the API error
/// usually lands on stderr — e.g. a 401 for a bad token).
async fn probe_model(cfg: AgentConfig) -> ValidateResult {
    // One connector, so one probe (rc3 D9). This used to fork on the model's
    // `native` flag and ping the gateway over HTTP instead — the fork existed
    // because a native model never spawned claude-code, so `claude -p` tested
    // something unrelated. With the native loop deleted, `claude -p` through the
    // model's own env IS the runtime that will spawn, which is the only property
    // a pre-flight has to have. A gateway the CLI cannot talk to fails here,
    // named, at configure time.
    if let Err(e) = crate::agents::spawn::ensure_claude_runnable("claude") {
        return ValidateResult {
            ok: false,
            message: e.to_string(),
        };
    }
    let out = match headless_claude_cmd(&cfg, "Reply with exactly the word: ok")
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return ValidateResult {
                ok: false,
                message: format!("Couldn't launch claude: {e}"),
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() && !stdout.trim().is_empty() {
        return ValidateResult {
            ok: true,
            message: "Connected — the model responded.".into(),
        };
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "no output"
    };
    let detail: String = detail.chars().take(300).collect();
    ValidateResult {
        ok: false,
        message: format!("Check failed: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_doc_search_empty_when_storage_absent() {
        let bridge = SignalingBridge::new();
        let res = bridge.session_doc_search("sx", None, None).await.unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn parse_diff_lines_classifies_each_kind() {
        let diff = "\
diff --git a/foo b/foo
index 0000..1111 100644
--- a/foo
+++ b/foo
@@ -1,2 +1,2 @@
 context line
-removed line
+added line";
        let lines = parse_diff_lines(diff);
        let kinds: Vec<&str> = lines.iter().map(|l| l.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["file", "file", "file", "file", "hunk", "context", "remove", "add"]
        );
    }

    #[test]
    fn parse_diff_lines_handles_file_rename_markers() {
        let diff = "\
diff --git a/old b/new
similarity index 100%
rename from old
rename to new";
        let lines = parse_diff_lines(diff);
        assert!(lines.iter().all(|l| l.kind == "file"));
    }

    #[test]
    fn parse_diff_lines_empty_input_returns_empty() {
        assert!(parse_diff_lines("").is_empty());
    }

    /// **The pre-flight has to carry the model's OWN gateway and credential**,
    /// or it tests something other than the model under test.
    ///
    /// This became load-bearing with rc3 D9. `probe_model` used to fork: a
    /// `native` model was pinged over HTTP through the loop's own machinery, and
    /// only a CLI model reached `claude -p`. The fork is gone, so this one
    /// command is now the only runtime check any saved model gets — including
    /// the two rows that carried `native = 1` because of their gateway, which
    /// nothing distinguishes any more. If the env did not follow the config,
    /// every model would test green against whatever ambient auth the host has
    /// and fail at spawn instead, which is the exact failure a pre-flight exists
    /// to prevent.
    ///
    /// Asserts on the built `Command` rather than running it: the subprocess is
    /// the part that needs a real `claude` and a real network, and the part
    /// worth pinning is what it would be handed.
    #[test]
    fn the_preflight_command_carries_the_models_own_gateway_and_credential() {
        let cfg = AgentConfig {
            agent_name: "hands".into(),
            provider: "deepseek".into(),
            model_name: "deepseek-v4-pro".into(),
            base_url: Some("https://api.deepseek.com/anthropic".into()),
            auth_token: Some("ds-token".into()),
            updated_at: String::new(),
            context_window: None,
        };
        let cmd = headless_claude_cmd(&cfg, "ping");
        let env: std::collections::HashMap<String, String> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| {
                Some((k.to_string_lossy().into_owned(), v?.to_string_lossy().into_owned()))
            })
            .collect();

        assert_eq!(env.get("ANTHROPIC_MODEL").map(String::as_str), Some("deepseek-v4-pro"));
        assert_eq!(env.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str), Some("ds-token"));
        // Routed through the same `resolve_anthropic_base_url` the live spawn
        // uses, so the probe meets the gateway the same way the agent will. The
        // proxy is not running under test, which is exactly the fall-through
        // that function documents — so the raw base_url is what lands.
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("https://api.deepseek.com/anthropic")
        );
    }

    /// The other half: a model with no explicit credential must NOT have one
    /// invented for it. `ANTHROPIC_AUTH_TOKEN` unset is what lets a first-party
    /// Anthropic model fall through to claude-code's ambient auth — the case
    /// every seeded model row is in.
    #[test]
    fn the_preflight_sets_no_token_when_the_model_carries_none() {
        // A whitespace-only token is deliberately NOT in this list: the filter is
        // `!is_empty()`, so `"   "` IS passed and the gateway answers 401 — a
        // legible failure, and not one this test claims otherwise about.
        for token in [None, Some("")] {
            let cfg = AgentConfig {
                agent_name: "hands".into(),
                provider: "anthropic".into(),
                model_name: "claude-opus-5".into(),
                base_url: None,
                auth_token: token.map(str::to_string),
                updated_at: String::new(),
                context_window: None,
            };
            let cmd = headless_claude_cmd(&cfg, "ping");
            let names: Vec<String> = cmd
                .as_std()
                .get_envs()
                .filter(|(_, v)| v.is_some())
                .map(|(k, _)| k.to_string_lossy().into_owned())
                .collect();
            assert!(
                !names.iter().any(|n| n == "ANTHROPIC_AUTH_TOKEN"),
                "an empty token was passed through as a credential: {names:?}"
            );
            assert!(
                !names.iter().any(|n| n == "ANTHROPIC_BASE_URL"),
                "a model with no gateway was given one: {names:?}"
            );
        }
    }

    #[test]
    fn parse_diff_lines_distinguishes_minus_minus_minus_from_remove() {
        // `--- a/file` is a file header, not a remove line.
        let diff = "--- a/file\n-removed";
        let lines = parse_diff_lines(diff);
        assert_eq!(lines[0].kind, "file");
        assert_eq!(lines[1].kind, "remove");
    }

    #[test]
    fn untracked_diff_includes_new_files_and_respects_gitignore() {
        use std::fs;
        use std::process::Command;
        let dir = std::env::temp_dir()
            .join(format!("bothq_untracked_diff_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .expect("git available")
        };
        assert!(git(&["init"]).status.success());
        // `.gitignore` is read from the working tree — no commit needed.
        fs::write(dir.join(".gitignore"), "secret.txt\n").unwrap();
        fs::write(dir.join("new.txt"), "BRAND_NEW_LINE\n").unwrap();
        fs::write(dir.join("secret.txt"), "SHOULD_NOT_APPEAR\n").unwrap();

        let out = untracked_diff(&dir);
        let _ = fs::remove_dir_all(&dir);

        assert!(out.contains("new.txt"), "new file path missing:\n{out}");
        assert!(
            out.contains("BRAND_NEW_LINE"),
            "new file content missing:\n{out}"
        );
        assert!(
            !out.contains("SHOULD_NOT_APPEAR"),
            "gitignored file leaked into the diff:\n{out}"
        );
    }

    /// Scratch dir for the synthetic-diff tests — no git repo needed, the
    /// synthesizer reads the filesystem directly.
    fn synth_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bothq_synth_diff_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn synthetic_diff_text_file_matches_git_shape() {
        let dir = synth_dir("text");
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let out = synthetic_new_file_diff(&dir, "a.txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            out,
            "diff --git a/a.txt b/a.txt\nnew file mode 100644\n\
             --- /dev/null\n+++ b/a.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n"
        );
    }

    #[test]
    fn synthetic_diff_single_line_hunk_omits_count() {
        let dir = synth_dir("single");
        std::fs::write(dir.join("a.txt"), "only\n").unwrap();
        let out = synthetic_new_file_diff(&dir, "a.txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.contains("@@ -0,0 +1 @@\n+only\n"), "got:\n{out}");
    }

    #[test]
    fn synthetic_diff_marks_missing_trailing_newline() {
        let dir = synth_dir("nonl");
        std::fs::write(dir.join("a.txt"), "no newline").unwrap();
        let out = synthetic_new_file_diff(&dir, "a.txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            out.ends_with("+no newline\n\\ No newline at end of file\n"),
            "got:\n{out}"
        );
    }

    #[test]
    fn synthetic_diff_binary_file_has_no_content() {
        let dir = synth_dir("bin");
        std::fs::write(dir.join("b.bin"), b"\x00\x01\x02SECRET").unwrap();
        let out = synthetic_new_file_diff(&dir, "b.bin");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            out.contains("Binary files /dev/null and b/b.bin differ"),
            "got:\n{out}"
        );
        assert!(!out.contains("SECRET"), "binary bytes leaked:\n{out}");
        assert!(!out.contains("@@"), "binary diff must have no hunk:\n{out}");
    }

    #[test]
    fn synthetic_diff_empty_file_is_headers_only() {
        let dir = synth_dir("empty");
        std::fs::write(dir.join("e.txt"), "").unwrap();
        let out = synthetic_new_file_diff(&dir, "e.txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            out,
            "diff --git a/e.txt b/e.txt\nnew file mode 100644\n"
        );
    }

    #[test]
    fn synthetic_diff_missing_file_yields_nothing() {
        let dir = synth_dir("missing");
        let out = synthetic_new_file_diff(&dir, "ghost.txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn synthetic_diff_executable_gets_755_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = synth_dir("exec");
        let p = dir.join("run.sh");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = synthetic_new_file_diff(&dir, "run.sh");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.contains("new file mode 100755"), "got:\n{out}");
    }
}
