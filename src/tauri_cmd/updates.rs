//! Update-check command — the "check-and-notify" feature.
//!
//! Polls GitHub Releases (via [`crate::core::updates`]) and reports whether a
//! newer bot-hq exists. Thin wrapper: all decision logic lives in the core
//! module. The frontend shows a download banner when `update_available` is
//! true; the install itself is manual (no code-signing / updater plugin yet).

use crate::core::updates::{self, UpdateInfo};
use crate::tauri_cmd::error::AppError;
use std::time::Duration;

#[tauri::command]
#[specta::specta]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<UpdateInfo, AppError> {
    // The running app's version (from tauri.conf.json via generate_context!),
    // not a hardcoded constant — this is what the release tag is compared to.
    let current = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::internal(format!("could not build HTTP client: {e}")))?;
    updates::check_for_update(&client, updates::LATEST_RELEASE_URL, &current)
        .await
        .map_err(|e| AppError::internal(format!("update check failed: {e}")))
}
