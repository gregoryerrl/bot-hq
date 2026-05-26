//! Session document (IPAV tabs) commands + Apply-tab git diff.

use crate::core::AppState as CoreAppState;
use crate::signaling::SignalingBridge;
use crate::storage::SessionDocument;
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

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
    bridge
        .session_doc_search(&session_id, query.as_deref(), phase.as_deref())
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn session_doc_read(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    slug: String,
) -> Result<Option<SessionDocumentView>, AppError> {
    bridge
        .session_doc_read(&session_id, &slug)
        .await
        .map(|opt| opt.map(Into::into))
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// One classified line of a unified `git diff`. Ports the Slint-era
/// `DiffLine` struct (deleted with `view_model.rs`). `kind` is one of
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
            let diff = String::from_utf8_lossy(&out.stdout).into_owned();
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
}
