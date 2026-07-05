//! Plugin manager — Tauri command surface.
//!
//! Consent-gated install (preview → confirm) / list / enable / disable /
//! uninstall / heartbeat feed, backed by [`Storage`] for persistence and
//! [`PluginRegistry`] (disk [`Loader`](crate::plugins::Loader) +
//! [`Heartbeat`] liveness + the enabled cache the serve/proxy layers
//! read). Each command is a thin shim over an `_inner` helper so the
//! logic is testable without a Tauri `State` wrapper.

use crate::plugins::{Heartbeat, PluginManifest, PluginRegistry, PluginStatus};
use crate::storage::{Plugin, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::sync::Arc;
use tauri::Emitter;

/// What the frontend sees for each installed plugin. Combines DB row state,
/// parsed manifest, and live heartbeat status.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InstalledPluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub status: PluginStatus,
    pub manifest: PluginManifest,
    pub dir_path: String,
    pub installed_at: String,
}

impl InstalledPluginView {
    /// Build the frontend view from a stored plugin row, its parsed
    /// manifest, and the live heartbeat. Status defaults to Healthy when the
    /// plugin isn't registered with the heartbeat (e.g. disabled).
    fn from_row(row: Plugin, manifest: PluginManifest, heartbeat: &Heartbeat) -> Self {
        let status = heartbeat.status_of(&row.id).unwrap_or(PluginStatus::Healthy);
        Self {
            id: row.id,
            name: row.name,
            version: row.version,
            enabled: row.enabled,
            status,
            manifest,
            dir_path: row.dir_path,
            installed_at: row.installed_at,
        }
    }
}

// ---- commands -------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn install_plugin(
    storage: tauri::State<'_, Arc<Storage>>,
    registry: tauri::State<'_, Arc<PluginRegistry>>,
    source: String,
) -> Result<InstalledPluginView, AppError> {
    install_plugin_inner(&storage, &registry, &source).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_installed_plugins(
    storage: tauri::State<'_, Arc<Storage>>,
    registry: tauri::State<'_, Arc<PluginRegistry>>,
) -> Result<Vec<InstalledPluginView>, AppError> {
    list_installed_plugins_inner(&storage, &registry).await
}

#[tauri::command]
#[specta::specta]
pub async fn enable_plugin(
    app: tauri::AppHandle,
    storage: tauri::State<'_, Arc<Storage>>,
    registry: tauri::State<'_, Arc<PluginRegistry>>,
    plugin_id: String,
) -> Result<(), AppError> {
    set_enabled_inner(&storage, &registry, &plugin_id, true).await?;
    emit_state_changed(&app, &plugin_id, true);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn disable_plugin(
    app: tauri::AppHandle,
    storage: tauri::State<'_, Arc<Storage>>,
    registry: tauri::State<'_, Arc<PluginRegistry>>,
    plugin_id: String,
) -> Result<(), AppError> {
    set_enabled_inner(&storage, &registry, &plugin_id, false).await?;
    emit_state_changed(&app, &plugin_id, false);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn uninstall_plugin(
    app: tauri::AppHandle,
    storage: tauri::State<'_, Arc<Storage>>,
    registry: tauri::State<'_, Arc<PluginRegistry>>,
    plugin_id: String,
) -> Result<(), AppError> {
    uninstall_plugin_inner(&storage, &registry, &plugin_id).await?;
    if let Err(e) = app.emit(
        crate::tauri_events::types::PLUGIN_UNINSTALLED,
        serde_json::json!({ "plugin_id": plugin_id }),
    ) {
        tracing::warn!(?e, plugin_id = %plugin_id, "emit plugin:uninstalled failed");
    }
    Ok(())
}

/// One consent-screen row: a requested capability + what granting it means,
/// in user terms (from the catalog).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CapabilityDescription {
    pub name: String,
    pub description: String,
}

/// What the install-consent dialog renders before anything lands on disk.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PluginManifestPreview {
    pub manifest: PluginManifest,
    pub capabilities: Vec<CapabilityDescription>,
}

/// Fetch + validate a manifest WITHOUT installing — the consent step.
/// The PluginManager calls this first, shows the user what the plugin
/// requests, and only calls `install_plugin` after an explicit confirm.
#[tauri::command]
#[specta::specta]
pub async fn preview_plugin_manifest(
    source: String,
) -> Result<PluginManifestPreview, AppError> {
    let (manifest, _json) = if is_url(&source) {
        fetch_manifest_from_url(&source).await?
    } else {
        read_manifest_from_dir(Path::new(&source))?
    };
    validate_requested_capabilities(&manifest)?;
    let capabilities = manifest
        .requested_capabilities
        .iter()
        .map(|c| CapabilityDescription {
            name: c.clone(),
            description: crate::plugins::catalog::describe(c)
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    Ok(PluginManifestPreview {
        manifest,
        capabilities,
    })
}

/// Unknown capability names are a preview/install-time error, not a
/// dispatch-time surprise — the consent screen can't describe what the
/// catalog doesn't know. (The LOADER stays tolerant of already-installed
/// plugins so a catalog change can't brick an install; the proxy re-checks
/// at dispatch.)
fn validate_requested_capabilities(manifest: &PluginManifest) -> Result<(), AppError> {
    if let Some(bad) = manifest
        .requested_capabilities
        .iter()
        .find(|c| !crate::plugins::catalog::is_valid(c))
    {
        return Err(AppError::Validation(format!(
            "manifest requests unknown capability {bad:?} (not in the api_version-1 catalog)"
        )));
    }
    Ok(())
}

/// Heartbeat feed, called by the frontend PluginHost's 5s ping loop just
/// before it postMessages `bhq:ping` into the plugin iframe. The backend
/// sweep loop (main.rs) turns unanswered pings into Slow/Crashed.
#[tauri::command]
#[specta::specta]
pub async fn plugin_note_ping(
    registry: tauri::State<'_, Arc<PluginRegistry>>,
    plugin_id: String,
) -> Result<(), AppError> {
    registry.heartbeat.note_ping_sent(&plugin_id);
    Ok(())
}

/// Heartbeat feed, called when the plugin iframe answers with `bhq:pong`
/// (and on clean PluginHost unmount, so a mid-flight ping isn't counted
/// as a miss against a plugin that simply closed with its panel).
#[tauri::command]
#[specta::specta]
pub async fn plugin_note_pong(
    registry: tauri::State<'_, Arc<PluginRegistry>>,
    plugin_id: String,
) -> Result<(), AppError> {
    registry.heartbeat.note_pong_received(&plugin_id);
    Ok(())
}

// ---- inner helpers (testable, no Tauri State wrapper) ---------------------

pub(crate) async fn install_plugin_inner(
    storage: &Storage,
    registry: &PluginRegistry,
    source: &str,
) -> Result<InstalledPluginView, AppError> {
    let (manifest, manifest_json) = if is_url(source) {
        fetch_manifest_from_url(source).await?
    } else {
        read_manifest_from_dir(Path::new(source))?
    };

    validate_requested_capabilities(&manifest)?;

    let plugin_dir = registry.data_dir.join("plugins").join(&manifest.id);
    if plugin_dir.exists() {
        return Err(AppError::Conflict(format!(
            "plugin already installed: {}",
            manifest.id
        )));
    }

    if is_url(source) {
        std::fs::create_dir_all(&plugin_dir).map_err(io_to_app)?;
        std::fs::write(plugin_dir.join("manifest.json"), &manifest_json)
            .map_err(io_to_app)?;
        let entry_url = resolve_entry_url(source, &manifest.entry);
        let body = reqwest::get(&entry_url)
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| AppError::Internal(format!("fetch entry {entry_url}: {e}")))?
            .bytes()
            .await
            .map_err(|e| AppError::Internal(format!("read entry body: {e}")))?;
        std::fs::write(plugin_dir.join(&manifest.entry), &body).map_err(io_to_app)?;
    } else {
        copy_dir_all(Path::new(source), &plugin_dir).map_err(io_to_app)?;
    }

    storage
        .insert_plugin(
            &manifest.id,
            &manifest.name,
            &manifest.version,
            &manifest_json,
            &plugin_dir.display().to_string(),
            None,
        )
        .await
        .map_err(anyhow_to_app)?;

    registry.reload().map_err(anyhow_to_app)?;
    registry.heartbeat.register(&manifest.id);
    registry.set_enabled(&manifest.id, true);

    let row = storage
        .list_plugins()
        .await
        .map_err(anyhow_to_app)?
        .into_iter()
        .find(|r| r.id == manifest.id)
        .ok_or_else(|| AppError::Internal("plugin row vanished after insert".into()))?;
    Ok(InstalledPluginView::from_row(row, manifest, &registry.heartbeat))
}

async fn list_installed_plugins_inner(
    storage: &Storage,
    registry: &PluginRegistry,
) -> Result<Vec<InstalledPluginView>, AppError> {
    let rows = storage.list_plugins().await.map_err(anyhow_to_app)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let manifest = match PluginManifest::parse(&row.manifest_json) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(plugin_id = %row.id, error = ?e, "skipping plugin with invalid stored manifest");
                continue;
            }
        };
        out.push(InstalledPluginView::from_row(row, manifest, &registry.heartbeat));
    }
    Ok(out)
}

async fn set_enabled_inner(
    storage: &Storage,
    registry: &PluginRegistry,
    plugin_id: &str,
    enabled: bool,
) -> Result<(), AppError> {
    storage
        .set_plugin_enabled(plugin_id, enabled)
        .await
        .map_err(anyhow_to_app)?;
    if enabled {
        registry.heartbeat.register(plugin_id);
    } else {
        registry.heartbeat.unregister(plugin_id);
    }
    registry.set_enabled(plugin_id, enabled);
    Ok(())
}

async fn uninstall_plugin_inner(
    storage: &Storage,
    registry: &PluginRegistry,
    plugin_id: &str,
) -> Result<(), AppError> {
    let row = storage
        .list_plugins()
        .await
        .map_err(anyhow_to_app)?
        .into_iter()
        .find(|r| r.id == plugin_id)
        .ok_or_else(|| AppError::NotFound(format!("plugin {plugin_id}")))?;

    storage.delete_plugin(plugin_id).await.map_err(anyhow_to_app)?;

    if let Err(e) = std::fs::remove_dir_all(&row.dir_path) {
        // Don't fail uninstall over a missing directory — DB is authoritative.
        tracing::warn!(
            ?e,
            plugin_id = %plugin_id,
            dir = %row.dir_path,
            "remove_dir_all failed; continuing"
        );
    }

    registry.heartbeat.unregister(plugin_id);
    registry.set_enabled(plugin_id, false);
    registry.reload().map_err(anyhow_to_app)?;
    Ok(())
}

// ---- small helpers --------------------------------------------------------

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Resolve the entry asset URL relative to the manifest URL. Takes everything
/// up to the last `/` of the manifest URL and appends the entry filename.
fn resolve_entry_url(manifest_url: &str, entry: &str) -> String {
    match manifest_url.rfind('/') {
        Some(i) => format!("{}{}", &manifest_url[..=i], entry),
        None => entry.to_string(),
    }
}

async fn fetch_manifest_from_url(url: &str) -> Result<(PluginManifest, String), AppError> {
    let body = reqwest::get(url)
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| AppError::Internal(format!("fetch manifest {url}: {e}")))?
        .text()
        .await
        .map_err(|e| AppError::Internal(format!("read manifest body: {e}")))?;
    let manifest = PluginManifest::parse(&body)
        .map_err(|e| AppError::Validation(format!("manifest at {url}: {e}")))?;
    Ok((manifest, body))
}

fn read_manifest_from_dir(dir: &Path) -> Result<(PluginManifest, String), AppError> {
    let manifest_path = dir.join("manifest.json");
    let body = std::fs::read_to_string(&manifest_path).map_err(|e| {
        AppError::Validation(format!(
            "reading {}: {e}",
            manifest_path.display()
        ))
    })?;
    let manifest = PluginManifest::parse(&body)
        .map_err(|e| AppError::Validation(format!("manifest at {}: {e}", manifest_path.display())))?;
    Ok((manifest, body))
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let src_path = entry.path();
        let name = match src_path.file_name() {
            Some(n) => n.to_os_string(),
            None => continue,
        };
        let dst_path = dst.join(&name);
        if src_path.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn emit_state_changed(app: &tauri::AppHandle, plugin_id: &str, enabled: bool) {
    if let Err(e) = app.emit(
        crate::tauri_events::types::PLUGIN_STATE_CHANGED,
        serde_json::json!({ "plugin_id": plugin_id, "enabled": enabled }),
    ) {
        tracing::warn!(?e, plugin_id = %plugin_id, "emit plugin:state-changed failed");
    }
}

fn anyhow_to_app(e: anyhow::Error) -> AppError {
    AppError::Internal(e.to_string())
}

fn io_to_app(e: std::io::Error) -> AppError {
    AppError::Internal(e.to_string())
}

// ---- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    async fn fresh(tmp: &TempDir) -> (Arc<Storage>, Arc<PluginRegistry>) {
        let data_dir = tmp.path().to_path_buf();
        let registry = Arc::new(PluginRegistry::new(data_dir).unwrap());
        let storage = Storage::memory().await.unwrap();
        (Arc::new(storage), registry)
    }

    fn write_plugin_source(root: &Path, id: &str, version: &str) -> PathBuf {
        let dir = root.join(format!("src-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = format!(
            r#"{{
                "id": "{id}",
                "name": "Plugin {id}",
                "version": "{version}",
                "entry": "index.html",
                "requested_capabilities": ["cl_index_search"]
            }}"#
        );
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        std::fs::write(dir.join("index.html"), b"<!doctype html><h1>hi</h1>").unwrap();
        dir
    }

    #[tokio::test]
    async fn install_local_dir_populates_db_and_disk() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "notes", "0.1.0");

        let view =
            install_plugin_inner(&storage, &registry, &src.display().to_string())
                .await
                .unwrap();
        assert_eq!(view.id, "notes");
        assert_eq!(view.version, "0.1.0");
        assert!(view.enabled);
        assert_eq!(view.status, PluginStatus::Healthy);

        let plugin_dir = registry.data_dir.join("plugins").join("notes");
        assert!(plugin_dir.join("manifest.json").exists());
        assert!(plugin_dir.join("index.html").exists());

        let rows = storage.list_plugins().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "notes");

        // Install flips the runtime gates the serve/proxy layers read.
        assert!(registry.is_enabled("notes"));
        assert!(registry.heartbeat.status_of("notes").is_some());
    }

    #[tokio::test]
    async fn install_rejects_duplicate_id() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "notes", "0.1.0");

        install_plugin_inner(&storage, &registry, &src.display().to_string())
            .await
            .unwrap();
        let err = install_plugin_inner(&storage, &registry, &src.display().to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn list_after_install_returns_installed_view() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "notes", "0.1.0");
        install_plugin_inner(&storage, &registry, &src.display().to_string())
            .await
            .unwrap();

        let views = list_installed_plugins_inner(&storage, &registry).await.unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].id, "notes");
        assert_eq!(views[0].manifest.entry, "index.html");
    }

    #[tokio::test]
    async fn enable_disable_toggles_db_and_heartbeat() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "notes", "0.1.0");
        install_plugin_inner(&storage, &registry, &src.display().to_string())
            .await
            .unwrap();

        set_enabled_inner(&storage, &registry, "notes", false)
            .await
            .unwrap();
        assert!(!storage.list_plugins().await.unwrap()[0].enabled);
        assert!(registry.heartbeat.status_of("notes").is_none());

        set_enabled_inner(&storage, &registry, "notes", true)
            .await
            .unwrap();
        assert!(storage.list_plugins().await.unwrap()[0].enabled);
        assert!(registry.heartbeat.status_of("notes").is_some());
    }

    #[tokio::test]
    async fn uninstall_removes_row_and_dir() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "notes", "0.1.0");
        install_plugin_inner(&storage, &registry, &src.display().to_string())
            .await
            .unwrap();
        let plugin_dir = registry.data_dir.join("plugins").join("notes");
        assert!(plugin_dir.exists());

        uninstall_plugin_inner(&storage, &registry, "notes")
            .await
            .unwrap();
        assert!(storage.list_plugins().await.unwrap().is_empty());
        assert!(!plugin_dir.exists());
        assert!(registry.heartbeat.status_of("notes").is_none());
    }

    #[tokio::test]
    async fn uninstall_missing_plugin_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let err = uninstall_plugin_inner(&storage, &registry, "ghost")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn disable_then_install_repeated_install_still_conflicts_on_disk() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "notes", "0.1.0");
        install_plugin_inner(&storage, &registry, &src.display().to_string())
            .await
            .unwrap();
        set_enabled_inner(&storage, &registry, "notes", false)
            .await
            .unwrap();
        // Even when disabled, the dir is still on disk, so re-install conflicts.
        let err = install_plugin_inner(&storage, &registry, &src.display().to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn preview_describes_capabilities_without_installing() {
        let tmp = TempDir::new().unwrap();
        let src = write_plugin_source(tmp.path(), "notes", "0.1.0");

        let preview = preview_plugin_manifest(src.display().to_string())
            .await
            .unwrap();
        assert_eq!(preview.manifest.id, "notes");
        assert_eq!(preview.capabilities.len(), 1);
        assert_eq!(preview.capabilities[0].name, "cl_index_search");
        assert!(
            !preview.capabilities[0].description.is_empty(),
            "consent copy comes from the catalog"
        );
        // Nothing landed anywhere — preview is read-only.
        assert!(!tmp.path().join("plugins").exists());
    }

    #[tokio::test]
    async fn preview_and_install_reject_unknown_capability() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("src-shady");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{
                "id": "shady",
                "name": "Shady",
                "version": "0.1.0",
                "entry": "index.html",
                "requested_capabilities": ["broadcast_message"]
            }"#,
        )
        .unwrap();
        std::fs::write(dir.join("index.html"), b"<h1>hi</h1>").unwrap();

        let err = preview_plugin_manifest(dir.display().to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        let (storage, registry) = fresh(&tmp).await;
        let err = install_plugin_inner(&storage, &registry, &dir.display().to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
        assert!(storage.list_plugins().await.unwrap().is_empty());
    }

    #[test]
    fn url_detection_recognizes_http_and_https() {
        assert!(is_url("http://example.com/x"));
        assert!(is_url("https://example.com/x"));
        assert!(!is_url("/tmp/foo"));
        assert!(!is_url("./relative/path"));
    }

    #[test]
    fn resolve_entry_url_uses_manifest_parent() {
        assert_eq!(
            resolve_entry_url("https://x.com/plugins/notes/manifest.json", "index.html"),
            "https://x.com/plugins/notes/index.html"
        );
        assert_eq!(
            resolve_entry_url("https://x.com/manifest.json", "main.html"),
            "https://x.com/main.html"
        );
    }
}
