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
/// parsed manifest, live heartbeat status, and (for linked installs) the
/// manifest-drift verdict.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InstalledPluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub status: PluginStatus,
    pub manifest: PluginManifest,
    pub dir_path: String,
    /// Dev-mode install serving straight from `dir_path` (the user's repo).
    pub linked: bool,
    /// Linked only: the source `manifest.json` no longer byte-matches the
    /// consented (stored) manifest — grants stay FROZEN until the user
    /// re-approves. Missing/unreadable source manifest also reports true.
    pub manifest_drifted: bool,
    pub installed_at: String,
}

impl InstalledPluginView {
    /// Build the frontend view from a stored plugin row, its parsed
    /// manifest, and the live heartbeat. Status defaults to Healthy when the
    /// plugin isn't registered with the heartbeat (e.g. disabled). For
    /// linked rows this reads the source manifest.json to compute drift —
    /// small file, list-time only.
    fn from_row(row: Plugin, manifest: PluginManifest, heartbeat: &Heartbeat) -> Self {
        let status = heartbeat.status_of(&row.id).unwrap_or(PluginStatus::Healthy);
        let manifest_drifted = row.linked
            && std::fs::read_to_string(Path::new(&row.dir_path).join("manifest.json"))
                .map(|live| live != row.manifest_json)
                .unwrap_or(true);
        Self {
            id: row.id,
            name: row.name,
            version: row.version,
            enabled: row.enabled,
            status,
            manifest,
            dir_path: row.dir_path,
            linked: row.linked,
            manifest_drifted,
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
    linked: bool,
) -> Result<InstalledPluginView, AppError> {
    install_plugin_inner(&storage, &registry, &source, linked).await
}

/// Re-consent a LINKED plugin whose source manifest drifted from the stored
/// (consented) one: re-validate the live manifest, REPLACE the row (KV rows
/// survive — unlike uninstall+reinstall), and re-seed the consent-frozen
/// registry caches. The frontend drives the consent dialog BEFORE calling
/// this (preview → confirm → reapprove), same trust model as install.
#[tauri::command]
#[specta::specta]
pub async fn reapprove_linked_plugin(
    app: tauri::AppHandle,
    storage: tauri::State<'_, Arc<Storage>>,
    registry: tauri::State<'_, Arc<PluginRegistry>>,
    plugin_id: String,
) -> Result<InstalledPluginView, AppError> {
    let view = reapprove_linked_plugin_inner(&storage, &registry, &plugin_id).await?;
    emit_state_changed(&app, &plugin_id, true);
    Ok(view)
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
    let (manifest, raw_json) = if is_url(&source) {
        fetch_manifest_from_url(&source).await?
    } else {
        read_manifest_from_dir(Path::new(&source))?
    };
    validate_requested_capabilities(&manifest)?;
    validate_csp_origins(&raw_json)?;
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

/// CSP extra-origin content rules run against the RAW manifest JSON (the
/// struct parse tolerates unknown directive keys so old installs keep
/// loading; preview/install must reject them). Preview/install-time only —
/// the same consent-screen-can't-describe-it logic as unknown capabilities.
fn validate_csp_origins(raw_manifest_json: &str) -> Result<(), AppError> {
    crate::plugins::manifest::validate_csp_extra_origins(raw_manifest_json)
        .map_err(|e| AppError::Validation(e.to_string()))
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
    linked: bool,
) -> Result<InstalledPluginView, AppError> {
    if linked && is_url(source) {
        return Err(AppError::Validation(
            "linked installs serve a LOCAL directory — URL sources can't be linked".into(),
        ));
    }
    let (manifest, manifest_json) = if is_url(source) {
        fetch_manifest_from_url(source).await?
    } else {
        read_manifest_from_dir(Path::new(source))?
    };

    validate_requested_capabilities(&manifest)?;
    validate_csp_origins(&manifest_json)?;

    // The consent-frozen CSP grant: canonical serialization of the APPROVED
    // field (None when absent or all-empty). Serving reads this column via
    // the registry cache — never the manifest on disk.
    let granted_csp = manifest.csp_extra_origins.as_ref().filter(|c| !c.is_empty());
    let csp_json = granted_csp
        .map(|c| {
            serde_json::to_string(c)
                .map_err(|e| AppError::Internal(format!("serializing csp grant: {e}")))
        })
        .transpose()?;

    let plugin_dir = registry.data_dir.join("plugins").join(&manifest.id);
    if plugin_dir.exists() {
        return Err(AppError::Conflict(format!(
            "plugin already installed: {}",
            manifest.id
        )));
    }
    if linked
        && storage
            .list_plugins()
            .await
            .map_err(anyhow_to_app)?
            .iter()
            .any(|r| r.id == manifest.id)
    {
        return Err(AppError::Conflict(format!(
            "plugin already installed: {} (uninstall first — that's the normal↔linked migration path)",
            manifest.id
        )));
    }

    // Resolve the serve root. Linked: the user's source directory, taken
    // absolute + canonicalized (also proves it exists); nothing is copied.
    // Normal: copy the bundle into data_dir as before.
    let serve_root = if linked {
        let src = Path::new(source);
        if !src.is_absolute() {
            return Err(AppError::Validation(format!(
                "linked install path must be absolute (got {source:?})"
            )));
        }
        src.canonicalize()
            .map_err(|e| AppError::Validation(format!("linked install path {source:?}: {e}")))?
    } else if is_url(source) {
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
        plugin_dir.clone()
    } else {
        copy_dir_all(Path::new(source), &plugin_dir).map_err(io_to_app)?;
        plugin_dir.clone()
    };

    storage
        .insert_plugin(
            &manifest.id,
            &manifest.name,
            &manifest.version,
            &manifest_json,
            &serve_root.display().to_string(),
            csp_json.as_deref(),
            linked,
        )
        .await
        .map_err(anyhow_to_app)?;

    registry.reload().map_err(anyhow_to_app)?;
    registry.heartbeat.register(&manifest.id);
    registry.set_enabled(&manifest.id, true);
    registry.set_serve_root(&manifest.id, Some(serve_root));
    registry.set_granted_caps(&manifest.id, Some(manifest.requested_capabilities.clone()));
    registry.set_csp_header(
        &manifest.id,
        granted_csp.map(|c| crate::plugins::serve::build_plugin_csp(Some(c))),
    );

    let row = storage
        .list_plugins()
        .await
        .map_err(anyhow_to_app)?
        .into_iter()
        .find(|r| r.id == manifest.id)
        .ok_or_else(|| AppError::Internal("plugin row vanished after insert".into()))?;
    Ok(InstalledPluginView::from_row(row, manifest, &registry.heartbeat))
}

/// Body of [`reapprove_linked_plugin`]. Grants change ONLY here (or at
/// install) — the consented manifest is re-read from the LIVE source and
/// replaces the stored one after passing the same validations as install.
pub(crate) async fn reapprove_linked_plugin_inner(
    storage: &Storage,
    registry: &PluginRegistry,
    plugin_id: &str,
) -> Result<InstalledPluginView, AppError> {
    let row = storage
        .list_plugins()
        .await
        .map_err(anyhow_to_app)?
        .into_iter()
        .find(|r| r.id == plugin_id)
        .ok_or_else(|| AppError::NotFound(format!("plugin {plugin_id}")))?;
    if !row.linked {
        return Err(AppError::Validation(format!(
            "plugin {plugin_id:?} is not a linked install — re-install to change a copied bundle"
        )));
    }

    let (manifest, manifest_json) = read_manifest_from_dir(Path::new(&row.dir_path))?;
    if manifest.id != plugin_id {
        return Err(AppError::Validation(format!(
            "source manifest id changed ({:?} → {:?}) — that's a different plugin; uninstall and install it fresh",
            plugin_id, manifest.id
        )));
    }
    validate_requested_capabilities(&manifest)?;
    validate_csp_origins(&manifest_json)?;

    let granted_csp = manifest.csp_extra_origins.as_ref().filter(|c| !c.is_empty());
    let csp_json = granted_csp
        .map(|c| {
            serde_json::to_string(c)
                .map_err(|e| AppError::Internal(format!("serializing csp grant: {e}")))
        })
        .transpose()?;

    // UPDATE in place — INSERT OR REPLACE would cascade-delete the
    // plugin's KV rows (REPLACE = DELETE+INSERT and plugin_kv has ON
    // DELETE CASCADE). Re-approving a manifest must not destroy state.
    storage
        .update_plugin_consent(
            plugin_id,
            &manifest.name,
            &manifest.version,
            &manifest_json,
            csp_json.as_deref(),
        )
        .await
        .map_err(anyhow_to_app)?;

    registry.heartbeat.register(plugin_id);
    registry.set_enabled(plugin_id, true);
    registry.set_granted_caps(plugin_id, Some(manifest.requested_capabilities.clone()));
    registry.set_csp_header(
        plugin_id,
        granted_csp.map(|c| crate::plugins::serve::build_plugin_csp(Some(c))),
    );

    let row = storage
        .list_plugins()
        .await
        .map_err(anyhow_to_app)?
        .into_iter()
        .find(|r| r.id == plugin_id)
        .ok_or_else(|| AppError::Internal("plugin row vanished after reapprove".into()))?;
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

    if row.linked {
        // dir_path is the USER'S source repo — bot-hq never deletes or
        // modifies it. Only the registry row + KV rows go.
        tracing::info!(plugin_id = %plugin_id, dir = %row.dir_path, "linked uninstall leaves source directory untouched");
    } else if let Err(e) = std::fs::remove_dir_all(&row.dir_path) {
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
    registry.set_csp_header(plugin_id, None);
    registry.set_serve_root(plugin_id, None);
    registry.set_granted_caps(plugin_id, None);
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
            install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
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

        install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
            .await
            .unwrap();
        let err = install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn list_after_install_returns_installed_view() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "notes", "0.1.0");
        install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
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
        install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
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
        install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
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
        install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
            .await
            .unwrap();
        set_enabled_inner(&storage, &registry, "notes", false)
            .await
            .unwrap();
        // Even when disabled, the dir is still on disk, so re-install conflicts.
        let err = install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
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
        let err = install_plugin_inner(&storage, &registry, &dir.display().to_string(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
        assert!(storage.list_plugins().await.unwrap().is_empty());
    }

    fn write_plugin_source_with_csp(root: &Path, id: &str, csp_block: &str) -> PathBuf {
        let dir = root.join(format!("src-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = format!(
            r#"{{
                "id": "{id}",
                "name": "Plugin {id}",
                "version": "0.1.0",
                "entry": "index.html",
                "requested_capabilities": [],
                "csp_extra_origins": {csp_block}
            }}"#
        );
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        std::fs::write(dir.join("index.html"), b"<!doctype html><h1>hi</h1>").unwrap();
        dir
    }

    #[tokio::test]
    async fn install_with_csp_grant_freezes_column_and_caches_header() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source_with_csp(
            tmp.path(),
            "cdn",
            r#"{ "script-src": ["https://cdn.jsdelivr.net"], "font-src": ["https://fonts.gstatic.com"] }"#,
        );

        install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
            .await
            .unwrap();

        // Grant frozen into the column, canonically serialized.
        let row = storage
            .list_plugins()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.id == "cdn")
            .unwrap();
        let stored: crate::plugins::CspExtraOrigins =
            serde_json::from_str(row.csp_json.as_deref().unwrap()).unwrap();
        assert_eq!(stored.script_src, vec!["https://cdn.jsdelivr.net"]);
        assert_eq!(stored.font_src, vec!["https://fonts.gstatic.com"]);

        // Header cache prebuilt: defaults intact + granted origins present.
        let header = registry.csp_header_for("cdn").unwrap();
        assert_eq!(
            header,
            crate::plugins::serve::build_plugin_csp(Some(&stored))
        );
        assert!(header.contains("script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net"));

        // Uninstall clears the cache entry.
        uninstall_plugin_inner(&storage, &registry, "cdn").await.unwrap();
        assert_eq!(registry.csp_header_for("cdn"), None);
    }

    #[tokio::test]
    async fn install_without_csp_field_leaves_default_header_path() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "plain", "0.1.0");
        install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
            .await
            .unwrap();
        let row = storage.list_plugins().await.unwrap().pop().unwrap();
        assert_eq!(row.csp_json, None);
        assert_eq!(registry.csp_header_for("plain"), None);
    }

    #[tokio::test]
    async fn install_and_preview_reject_forbidden_csp_origins() {
        let tmp = TempDir::new().unwrap();
        for (name, bad_block) in [
            ("wild", r#"{ "script-src": ["https://*.example.com"] }"#),
            ("scheme", r#"{ "script-src": ["https:"] }"#),
            ("keyword", r#"{ "script-src": ["'unsafe-eval'"] }"#),
            ("datauri", r#"{ "img-src": ["data:"] }"#),
            ("http", r#"{ "script-src": ["http://x.com"] }"#),
            ("directive", r#"{ "connect-src": ["https://x.com"] }"#),
        ] {
            let src = write_plugin_source_with_csp(tmp.path(), name, bad_block);

            let err = preview_plugin_manifest(src.display().to_string())
                .await
                .unwrap_err();
            assert!(matches!(err, AppError::Validation(_)), "{name}: got {err:?}");

            let (storage, registry) = fresh(&tmp).await;
            let err = install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
                .await
                .unwrap_err();
            assert!(matches!(err, AppError::Validation(_)), "{name}: got {err:?}");
            assert!(storage.list_plugins().await.unwrap().is_empty());
            assert_eq!(registry.csp_header_for(name), None);
            assert!(!registry.data_dir.join("plugins").join(name).exists());
        }
    }

    #[tokio::test]
    async fn preview_carries_csp_origins_for_consent_screen() {
        let tmp = TempDir::new().unwrap();
        let src = write_plugin_source_with_csp(
            tmp.path(),
            "cdn",
            r#"{ "script-src": ["https://cdn.jsdelivr.net", "https://unpkg.com"] }"#,
        );
        let preview = preview_plugin_manifest(src.display().to_string())
            .await
            .unwrap();
        let csp = preview.manifest.csp_extra_origins.unwrap();
        assert_eq!(
            csp.script_src,
            vec!["https://cdn.jsdelivr.net", "https://unpkg.com"]
        );
        // Preview is still read-only.
        assert!(!tmp.path().join("plugins").exists());
    }

    #[tokio::test]
    async fn linked_install_serves_from_source_without_copy() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "dev", "0.1.0");

        let view = install_plugin_inner(&storage, &registry, &src.display().to_string(), true)
            .await
            .unwrap();
        assert!(view.linked);
        assert!(!view.manifest_drifted, "fresh linked install can't drift");

        // Nothing copied into data_dir.
        assert!(!registry.data_dir.join("plugins").join("dev").exists());

        // Serve root IS the (canonicalized) source; assets resolve there.
        let root = registry.serve_root_for("dev").unwrap();
        assert_eq!(root, src.canonicalize().unwrap());
        let (path, mime) = crate::plugins::serve::resolve_with_root(
            Some(&root),
            registry.is_enabled("dev"),
            "dev",
            "index.html",
        )
        .unwrap();
        assert!(path.is_file());
        assert_eq!(mime, "text/html; charset=utf-8");

        // An asset edit is visible on next resolve (same root, no copy).
        std::fs::write(src.join("new.txt"), "live").unwrap();
        assert!(crate::plugins::serve::resolve_with_root(
            Some(&root),
            true,
            "dev",
            "new.txt"
        )
        .is_ok());
    }

    #[tokio::test]
    async fn linked_install_rejects_bad_sources() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;

        // URL sources can't be linked.
        let err = install_plugin_inner(&storage, &registry, "https://x.com/manifest.json", true)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        // Relative path refused.
        let err = install_plugin_inner(&storage, &registry, "./relative", true)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        // Missing directory / missing manifest.json refused.
        let err = install_plugin_inner(
            &storage,
            &registry,
            &tmp.path().join("nope").display().to_string(),
            true,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        assert!(storage.list_plugins().await.unwrap().is_empty());
    }

    /// THE consent rule: editing a linked manifest.json must never widen
    /// grants. Drift is surfaced on the view; enforcement keeps reading the
    /// frozen grant until an explicit re-approve.
    #[tokio::test]
    async fn linked_manifest_edit_never_widens_grants_until_reapprove() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        // Source manifest granting ONLY cl_index_search.
        let dir = tmp.path().join("src-dev");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest_v1 = r#"{
                "id": "dev",
                "name": "Dev",
                "version": "0.1.0",
                "entry": "index.html",
                "requested_capabilities": ["cl_index_search"]
            }"#;
        std::fs::write(dir.join("manifest.json"), manifest_v1).unwrap();
        std::fs::write(dir.join("index.html"), b"<h1>hi</h1>").unwrap();

        install_plugin_inner(&storage, &registry, &dir.display().to_string(), true)
            .await
            .unwrap();
        assert_eq!(
            registry.granted_caps_for("dev"),
            Some(vec!["cl_index_search".to_string()])
        );

        // Attacker/dev edits the LINKED manifest to request more.
        let manifest_v2 = r#"{
                "id": "dev",
                "name": "Dev",
                "version": "0.2.0",
                "entry": "index.html",
                "requested_capabilities": ["cl_index_search", "list_sessions"]
            }"#;
        std::fs::write(dir.join("manifest.json"), manifest_v2).unwrap();

        // Grants unchanged; drift surfaced.
        assert_eq!(
            registry.granted_caps_for("dev"),
            Some(vec!["cl_index_search".to_string()]),
            "editing the linked manifest must not change enforced grants"
        );
        let views = list_installed_plugins_inner(&storage, &registry).await.unwrap();
        assert!(views[0].manifest_drifted, "drift must be surfaced");

        // Consented re-approve applies the new manifest.
        let view = reapprove_linked_plugin_inner(&storage, &registry, "dev")
            .await
            .unwrap();
        assert!(!view.manifest_drifted, "re-approve clears drift");
        assert_eq!(view.version, "0.2.0");
        let caps = registry.granted_caps_for("dev").unwrap();
        assert!(caps.contains(&"list_sessions".to_string()));
    }

    #[tokio::test]
    async fn reapprove_preserves_kv_and_rejects_id_change_or_nonlinked() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let dir = write_plugin_source(tmp.path(), "dev", "0.1.0");
        install_plugin_inner(&storage, &registry, &dir.display().to_string(), true)
            .await
            .unwrap();
        storage.plugin_kv_set("dev", "state", "keepme").await.unwrap();

        // Re-approve keeps KV (REPLACE, not uninstall+reinstall).
        reapprove_linked_plugin_inner(&storage, &registry, "dev").await.unwrap();
        assert_eq!(
            storage.plugin_kv_get("dev", "state").await.unwrap(),
            Some("keepme".to_string())
        );

        // Id change in the source = a different plugin → refused.
        let swapped = std::fs::read_to_string(dir.join("manifest.json"))
            .unwrap()
            .replace("\"dev\"", "\"other\"");
        std::fs::write(dir.join("manifest.json"), swapped).unwrap();
        let err = reapprove_linked_plugin_inner(&storage, &registry, "dev")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        // Non-linked plugins can't use the reapprove path.
        let src2 = write_plugin_source(tmp.path(), "copymode", "0.1.0");
        install_plugin_inner(&storage, &registry, &src2.display().to_string(), false)
            .await
            .unwrap();
        let err = reapprove_linked_plugin_inner(&storage, &registry, "copymode")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn uninstall_leaves_linked_source_untouched() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let dir = write_plugin_source(tmp.path(), "dev", "0.1.0");
        install_plugin_inner(&storage, &registry, &dir.display().to_string(), true)
            .await
            .unwrap();
        storage.plugin_kv_set("dev", "k", "v").await.unwrap();

        uninstall_plugin_inner(&storage, &registry, "dev").await.unwrap();

        // Registry row, caches, KV: gone.
        assert!(storage.list_plugins().await.unwrap().is_empty());
        assert_eq!(registry.serve_root_for("dev"), None);
        assert_eq!(registry.granted_caps_for("dev"), None);
        assert_eq!(storage.plugin_kv_get("dev", "k").await.unwrap(), None);

        // The user's source directory: UNTOUCHED.
        assert!(dir.join("manifest.json").is_file());
        assert!(dir.join("index.html").is_file());
    }

    #[tokio::test]
    async fn linked_install_conflicts_with_existing_id() {
        let tmp = TempDir::new().unwrap();
        let (storage, registry) = fresh(&tmp).await;
        let src = write_plugin_source(tmp.path(), "dev", "0.1.0");
        install_plugin_inner(&storage, &registry, &src.display().to_string(), false)
            .await
            .unwrap();
        // Same id as a normal install → Conflict (migration = uninstall first).
        let err = install_plugin_inner(&storage, &registry, &src.display().to_string(), true)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {err:?}");
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
