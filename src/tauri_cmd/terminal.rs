//! Terminal subtab commands: per-session PTY open / input / resize.
//!
//! Thin wrappers over `core::TerminalRegistry` (`src/core/terminal.rs`).
//! Output does NOT flow through commands — the PTY reader emits coalesced
//! `terminal:output` events (base64 chunks) the frontend subscribes to;
//! `terminal_open` returns the scrollback snapshot for replay-on-mount.

use crate::core::AppState as CoreAppState;
use crate::tauri_cmd::error::AppError;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct TerminalOpenView {
    /// Base64 of the retained scrollback — replayed into xterm on mount so a
    /// re-opened tab (or re-opened session view) shows recent history.
    pub snapshot_b64: String,
    pub cols: u16,
    pub rows: u16,
}

/// Ensure the session's shell is running (spawning it in the session's
/// working repo on first open) and return the replay snapshot + geometry.
#[tauri::command]
#[specta::specta]
pub async fn terminal_open(
    app: tauri::AppHandle,
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<TerminalOpenView, AppError> {
    let cwd = core
        .storage
        .get_session(&session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?
        .working_repo_path
        .map(std::path::PathBuf::from);
    let term = core
        .terminals
        .ensure(&session_id, cwd, Some(app))
        .await?;
    let (snapshot, cols, rows) = term.open_view();
    Ok(TerminalOpenView {
        snapshot_b64: base64::engine::general_purpose::STANDARD.encode(snapshot),
        cols,
        rows,
    })
}

/// Forward user keystrokes (xterm `onData` — UTF-8 text incl. escape
/// sequences) into the PTY. No-op error if the shell isn't running.
#[tauri::command]
#[specta::specta]
pub async fn terminal_input(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    data: String,
) -> Result<(), AppError> {
    let term = core
        .terminals
        .get_live(&session_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no live terminal for {session_id}")))?;
    term.write_input(data.as_bytes())?;
    Ok(())
}

/// Propagate the frontend fit addon's geometry to the PTY.
#[tauri::command]
#[specta::specta]
pub async fn terminal_resize(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), AppError> {
    let term = core
        .terminals
        .get_live(&session_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no live terminal for {session_id}")))?;
    term.resize(cols, rows)?;
    Ok(())
}
