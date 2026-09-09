//! Diagnostics (telemetry) commands — the Settings → Diagnostics panel's
//! surface. Thin wrappers over `core::telemetry` + the `app_settings` KV.
//!
//! The install id's lifecycle is the privacy contract: minted when the user
//! enables, DELETED when they disable, re-minted on re-enable — so "stable
//! while enabled" is true and "off means unlinkable" is too. PRIVACY.md
//! states it; this file enforces it.

use crate::core::AppState as CoreAppState;
use crate::core::telemetry::{
    self, KEY_ASKED, KEY_ENABLED, KEY_ENDPOINT, KEY_INSTALL_ID, TELEMETRY_ENABLED,
};
use crate::storage::Storage;
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TelemetryStatus {
    pub enabled: bool,
    pub asked: bool,
    pub install_id: Option<String>,
    pub endpoint: String,
    pub queued_bytes: u32,
}

async fn status_of(storage: &Storage, core: &CoreAppState) -> Result<TelemetryStatus, AppError> {
    let enabled = matches!(storage.get_setting(KEY_ENABLED).await, Ok(Some(v)) if v == "1");
    let asked = matches!(storage.get_setting(KEY_ASKED).await, Ok(Some(v)) if v == "1");
    let install_id = storage.get_setting(KEY_INSTALL_ID).await.ok().flatten();
    let endpoint = storage
        .get_setting(KEY_ENDPOINT)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let queued_bytes = std::fs::metadata(telemetry::queue_path(&core.paths.local_dir))
        .map(|m| m.len() as u32)
        .unwrap_or(0);
    Ok(TelemetryStatus {
        enabled,
        asked,
        install_id,
        endpoint,
        queued_bytes,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn get_telemetry_status(
    storage: tauri::State<'_, Arc<Storage>>,
    core: tauri::State<'_, Arc<CoreAppState>>,
) -> Result<TelemetryStatus, AppError> {
    status_of(&storage, &core).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_telemetry_enabled(
    storage: tauri::State<'_, Arc<Storage>>,
    core: tauri::State<'_, Arc<CoreAppState>>,
    enabled: bool,
) -> Result<TelemetryStatus, AppError> {
    if enabled {
        storage.set_setting(KEY_ENABLED, "1").await?;
        let has_id =
            matches!(storage.get_setting(KEY_INSTALL_ID).await, Ok(Some(v)) if !v.is_empty());
        if !has_id {
            storage
                .set_setting(KEY_INSTALL_ID, &uuid::Uuid::new_v4().to_string())
                .await?;
        }
        // The boot path only enqueues app_launch when ALREADY enabled, so a
        // fresh install that opts in here would send nothing until its next
        // launch — the first real Windows user hit exactly this. Enqueue the
        // launch event at the opt-in moment; the running flusher ships it.
        let _ = telemetry::enqueue(
            &telemetry::queue_path(&core.paths.local_dir),
            &telemetry::app_launch_event(),
        );
    } else {
        storage.set_setting(KEY_ENABLED, "0").await?;
        // Off means unlinkable: the id dies with the opt-out, and so does
        // anything queued but unsent.
        storage.delete_setting(KEY_INSTALL_ID).await?;
        let _ = std::fs::remove_file(telemetry::queue_path(&core.paths.local_dir));
    }
    TELEMETRY_ENABLED.store(enabled, Ordering::Relaxed);
    status_of(&storage, &core).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_telemetry_endpoint(
    storage: tauri::State<'_, Arc<Storage>>,
    core: tauri::State<'_, Arc<CoreAppState>>,
    endpoint: String,
) -> Result<TelemetryStatus, AppError> {
    let trimmed = endpoint.trim();
    if !trimmed.is_empty() && !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(AppError::Internal(
            "endpoint must be an http(s) URL (the worker's workers.dev address)".into(),
        ));
    }
    storage.set_setting(KEY_ENDPOINT, trimmed).await?;
    status_of(&storage, &core).await
}

#[tauri::command]
#[specta::specta]
pub async fn mark_telemetry_asked(
    storage: tauri::State<'_, Arc<Storage>>,
    core: tauri::State<'_, Arc<CoreAppState>>,
) -> Result<TelemetryStatus, AppError> {
    storage.set_setting(KEY_ASKED, "1").await?;
    status_of(&storage, &core).await
}
