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

#[tauri::command]
#[specta::specta]
pub async fn session_doc_read(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    slug: String,
) -> Result<Option<SessionDocumentView>, AppError> {
    let doc = bridge.session_doc_read(&session_id, &slug).await?;
    Ok(doc.map(Into::into))
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
/// in the Apply tab. Side-effect free: compares each file against the null
/// device via `git diff --no-index` rather than `git add -N`, so the agent's
/// index is never mutated.
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
        // `git diff --no-index` exits 1 when the files differ — the normal case
        // for a new file, NOT an error — so we read stdout regardless of status.
        // Diff against the platform null device so the file renders as all-adds.
        let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
        if let Ok(out) = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["diff", "--no-index", "--no-color", "--"])
            .arg(null_device)
            .arg(path.as_ref())
            .output()
        {
            acc.push_str(&String::from_utf8_lossy(&out.stdout));
        }
    }
    acc
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
/// resolved from `default_model_id` (app settings), falling back to the
/// session's Brian model, then Brian's agent config (same chain the live agents
/// use, via [`resolve_spawn_config`]). Bounded by a 60s timeout; the child is
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

    // App-wide default model wins; else the session's chosen Brian model; else
    // resolve_spawn_config falls through to Brian's agent config / hardcoded.
    let default_model_id = storage
        .get_setting("default_model_id")
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    let session_brian_model = storage
        .get_session(&session_id)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.brian_model_id);
    let model_id = default_model_id.or(session_brian_model);
    let cfg =
        crate::core::session::resolve_spawn_config(&storage, "brian", model_id.as_deref()).await;

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
    let cfg = crate::core::session::resolve_spawn_config(&storage, "brian", Some(&model_id)).await;
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
}
