//! Update-check command — the "check-and-notify" feature.
//!
//! Polls GitHub Releases (via [`crate::core::updates`]) and reports whether a
//! newer bot-hq exists. Thin wrapper: all decision logic lives in the core
//! module. The frontend shows a download banner when `update_available` is
//! true; the install itself is manual (no code-signing / updater plugin yet).

use crate::core::build_info::{self, ExeState};
use crate::core::updates::{self, UpdateInfo};
use crate::core::AppState as CoreAppState;
use crate::tauri_cmd::error::AppError;
use serde::Serialize;
use specta::Type;
use std::sync::Arc;
use std::time::Duration;

/// What build is running (the footer, 2026-09-25): the version, the commit it
/// was built from, and whether its program file changed since launch — which
/// on a source install means the git hooks already run newer code than the
/// app does.
#[derive(Debug, Clone, Serialize, Type)]
pub struct BuildInfo {
    pub version: String,
    /// Seven hex characters (plus `-dirty`), or `null` for a build that did not
    /// stamp one — every debug build, and a release built outside `./start`.
    pub commit: Option<String>,
    /// `release` | `debug`.
    pub profile: String,
    pub exe_path: Option<String>,
    /// The program file's modified time AT LAUNCH (RFC 3339, UTC) — when it
    /// was built or installed.
    pub exe_built_at: Option<String>,
    pub exe_state: ExeState,
    pub data_dir: String,
    /// The highest migration applied to the database.
    pub schema_version: Option<i64>,
}

#[tauri::command]
#[specta::specta]
pub async fn app_build_info(
    app: tauri::AppHandle,
    core: tauri::State<'_, Arc<CoreAppState>>,
) -> Result<BuildInfo, AppError> {
    let launch = build_info::launch_stamp();
    Ok(BuildInfo {
        version: app.package_info().version.to_string(),
        commit: build_info::build_commit(),
        profile: build_info::profile().to_string(),
        exe_path: launch.map(|s| s.path.display().to_string()),
        exe_built_at: launch
            .and_then(|s| s.modified)
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        exe_state: build_info::exe_state(launch),
        data_dir: core.paths.data_dir.display().to_string(),
        schema_version: core.storage.schema_version().await,
    })
}

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
    updates::check_for_update(&client, &updates::release_api_url(), &current)
        .await
        .map_err(|e| AppError::internal(format!("update check failed: {e}")))
}
