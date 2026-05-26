//! Capture the bot-hq main window and share the PNG as a chat message.
//!
//! Wraps macOS `screencapture -R` with the window's logical geometry from
//! Tauri (physical pixels divided by scale factor — Retina-safe). The PNG
//! lands under `<data_dir>/screenshots/<timestamp>.png`. The path is
//! broadcast into the target session via `CoreAppState::broadcast` so
//! Brian + Rain see the message and can Read the file as an image.

use crate::core::AppState as CoreAppState;
use crate::tauri_cmd::error::AppError;
use anyhow::Context;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tauri::Manager;

#[tauri::command]
#[specta::specta]
pub async fn capture_window_screenshot(
    app_handle: tauri::AppHandle,
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<String, AppError> {
    let data_dir = core.paths.data_dir.clone();
    let path = capture_main_window(&app_handle, &data_dir)
        .map_err(|e| AppError::Internal(format!("screenshot capture: {e}")))?;

    let text = format!("[Screenshot from user]\n{}", path.display());
    core.broadcast(&session_id, &text)
        .await
        .map_err(|e| AppError::Internal(format!("broadcast: {e}")))?;

    Ok(path.display().to_string())
}

/// Capture the main bot-hq window to a PNG under `<data_dir>/screenshots/`.
/// Shared by the Tauri command (user-triggered, also broadcasts a message)
/// and the external MCP tool `webview_screenshot` (agent-triggered, just
/// returns the path).
pub(crate) fn capture_main_window(
    app_handle: &tauri::AppHandle,
    data_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let window = app_handle
        .get_webview_window("main")
        .context("main window not found")?;

    // `screencapture -R` reads whatever pixels are at the given screen
    // coordinates — including any window stacked on top of bot-hq. Raise
    // bot-hq to the front first so we capture its actual contents instead
    // of whatever overlay (terminal, devtools, another app) happens to be
    // covering it. Brief sleep lets the compositor redraw before the snap.
    let _ = window.set_focus();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let pos = window.outer_position().context("outer_position")?;
    let size = window.outer_size().context("outer_size")?;
    let scale = window.scale_factor().context("scale_factor")?;

    // screencapture -R takes points (logical screen coordinates).
    // Tauri's outer_position / outer_size return physical pixels.
    let logical_x = (pos.x as f64 / scale).round() as i64;
    let logical_y = (pos.y as f64 / scale).round() as i64;
    let logical_w = (size.width as f64 / scale).round() as u64;
    let logical_h = (size.height as f64 / scale).round() as u64;

    let dir = data_dir.join("screenshots");
    std::fs::create_dir_all(&dir).context("mkdir screenshots dir")?;

    let ts = Utc::now().format("%Y%m%dT%H%M%S%3f").to_string();
    let path = dir.join(format!("{ts}.png"));

    let region = format!("{logical_x},{logical_y},{logical_w},{logical_h}");
    let output = Command::new("/usr/sbin/screencapture")
        .args(["-R", &region, "-x", "-t", "png"])
        .arg(&path)
        .output()
        .context("screencapture spawn")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // macOS Screen Recording permission missing → "could not create
        // image from display/rect". Translate into something actionable so
        // the user knows to flip the toggle in System Settings.
        if stderr.contains("could not create image") {
            anyhow::bail!(
                "Screen Recording permission required. Open System Settings → \
                 Privacy & Security → Screen Recording, enable the entry for \
                 bot-hq (or your terminal if launched via `cargo run`), then \
                 try again. Raw output: {}",
                stderr.trim()
            );
        }
        anyhow::bail!("screencapture failed: {}", stderr.trim());
    }
    Ok(path)
}
