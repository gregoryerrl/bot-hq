//! Filesystem watcher: live CL freshness AND live Apply-tab working-tree diffs.
//!
//! One `notify` debouncer watches the Context Library dir plus every live
//! session's working repo (registered/unregistered via [`WatcherHandle`]). For
//! each debounced batch of changed paths:
//!
//! - paths under the CL dir → derive the CL *scope* (`projects/<name>/…` → that
//!   project; root files / `agents/…` → `_globals`), re-index it via the existing
//!   [`SignalingBridge::cl_rescan`] (disk↔index reconcile), THEN emit `cl:changed`
//!   so the frontend refetches the now-current index. Re-indexing BEFORE the emit
//!   is load-bearing: `cl_index_search` reads the SQLite index, not disk.
//! - paths under a watched session repo → map back to the session and emit
//!   `session:worktree_changed`, so the Apply-tab `git diff` re-runs live. Build /
//!   VCS churn (`target/`, `node_modules/`, `.git/`, …) is filtered out so a
//!   `cargo build` / `npm ci` doesn't spam recomputes.
//!
//! `notify`'s callback is synchronous (its own thread); it just forwards changed
//! paths over an mpsc channel to a tokio task. That task owns the debouncer (so
//! the watch lives for the process lifetime) and also mutates its watch-set as
//! sessions come and go, driven by a second `WatchCmd` channel.

use crate::paths::IGNORED_BUILD_DIRS;
use crate::signaling::SignalingBridge;
use crate::storage::Project;
use crate::tauri_events::types::{ClChangedEvent, PluginAssetsChangedEvent, WorktreeChangedEvent};
use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// Debounce window for filesystem changes. One debouncer covers both the CL dir
/// and every watched session repo, so this is a single compromise window — long
/// enough to coalesce an editor's / a git op's burst, short enough to feel live.
const DEBOUNCE: Duration = Duration::from_millis(500);

// Build / VCS directories whose churn must never trigger an A-tab recompute are
// shared with the CL walker via `crate::paths::IGNORED_BUILD_DIRS`. (Dot-prefixed
// names — `.git`, `.vite`, `.next`, … — are caught by the `.`-prefix rule in
// [`is_ignored_component`], so they're not in that list.)

/// Command into the watcher task. Lets the session spawn/close paths register
/// and unregister working repos for live A-tab diffs, and the plugin
/// lifecycle register served dirs for `plugin:assets_changed`.
enum WatchCmd {
    AddRepo { session_id: String, path: PathBuf },
    RemoveRepo { session_id: String },
    AddPluginDir { plugin_id: String, path: PathBuf },
    RemovePluginDir { plugin_id: String },
}

/// Handle to the running filesystem watcher, stored on `AppState`. Sending a
/// command is non-async and best-effort (a dead task just means no watch).
pub struct WatcherHandle {
    cmd_tx: UnboundedSender<WatchCmd>,
}

impl WatcherHandle {
    /// Start live-watching a session's working repo for A-tab diffs.
    pub fn add_repo(&self, session_id: &str, path: PathBuf) {
        let _ = self.cmd_tx.send(WatchCmd::AddRepo {
            session_id: session_id.to_string(),
            path,
        });
    }

    /// Stop watching a session's working repo (on session close).
    pub fn remove_repo(&self, session_id: &str) {
        let _ = self.cmd_tx.send(WatchCmd::RemoveRepo {
            session_id: session_id.to_string(),
        });
    }

    /// Start live-watching an enabled plugin's served dir (install/enable).
    pub fn add_plugin_dir(&self, plugin_id: &str, path: PathBuf) {
        let _ = self.cmd_tx.send(WatchCmd::AddPluginDir {
            plugin_id: plugin_id.to_string(),
            path,
        });
    }

    /// Stop watching a plugin's served dir (disable/uninstall).
    pub fn remove_plugin_dir(&self, plugin_id: &str) {
        let _ = self.cmd_tx.send(WatchCmd::RemovePluginDir {
            plugin_id: plugin_id.to_string(),
        });
    }
}

/// Start watching the Context Library dir. Returns a [`WatcherHandle`] for
/// registering session repos later, or the `notify` error if the watcher can't
/// be created (the caller logs it; views fall back to their existing poll).
/// `emit` is the same `app.emit`-backed closure the bridge subscriber uses.
pub fn spawn_fs_watcher<EB>(
    paths: crate::paths::Paths,
    bridge: Arc<SignalingBridge>,
    emit: EB,
) -> Result<WatcherHandle, notify_debouncer_mini::notify::Error>
where
    EB: Fn(&str, Value) + Send + Sync + 'static,
{
    let (path_tx, mut path_rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<WatchCmd>();

    // notify's callback is sync + on its own thread — just forward the paths.
    let mut debouncer = new_debouncer(DEBOUNCE, move |res: DebounceEventResult| {
        if let Ok(events) = res {
            for ev in events {
                let _ = path_tx.send(ev.path);
            }
        }
    })?;
    debouncer
        .watcher()
        .watch(&paths.cl_dir, RecursiveMode::Recursive)?;

    let cl_dir = paths.cl_dir.clone();
    tokio::spawn(async move {
        // Own the debouncer so the watch lives as long as this task (the whole
        // app); we also mutate its watch-set as sessions register/unregister.
        let mut debouncer = debouncer;
        // Watched session repos: repo root → session_id.
        let mut repos: HashMap<PathBuf, String> = HashMap::new();
        // Watched plugin served dirs: dir root → plugin_id.
        let mut plugin_dirs: HashMap<PathBuf, String> = HashMap::new();
        loop {
            tokio::select! {
                Some(cmd) = cmd_rx.recv() => match cmd {
                    WatchCmd::AddRepo { session_id, path } => {
                        match debouncer.watcher().watch(&path, RecursiveMode::Recursive) {
                            Ok(()) => {
                                repos.insert(path, session_id);
                            }
                            Err(e) => {
                                tracing::warn!(error = ?e, ?path, "fs watcher: failed to watch session repo");
                            }
                        }
                    }
                    WatchCmd::RemoveRepo { session_id } => {
                        let gone: Vec<PathBuf> = repos
                            .iter()
                            .filter(|(_, sid)| **sid == session_id)
                            .map(|(p, _)| p.clone())
                            .collect();
                        for p in gone {
                            let _ = debouncer.watcher().unwatch(&p);
                            repos.remove(&p);
                        }
                    }
                    WatchCmd::AddPluginDir { plugin_id, path } => {
                        match debouncer.watcher().watch(&path, RecursiveMode::Recursive) {
                            Ok(()) => {
                                plugin_dirs.insert(path, plugin_id);
                            }
                            Err(e) => {
                                tracing::warn!(error = ?e, ?path, "fs watcher: failed to watch plugin dir");
                            }
                        }
                    }
                    WatchCmd::RemovePluginDir { plugin_id } => {
                        let gone: Vec<PathBuf> = plugin_dirs
                            .iter()
                            .filter(|(_, pid)| **pid == plugin_id)
                            .map(|(p, _)| p.clone())
                            .collect();
                        for p in gone {
                            let _ = debouncer.watcher().unwatch(&p);
                            plugin_dirs.remove(&p);
                        }
                    }
                },
                Some(first) = path_rx.recv() => {
                    let mut batch = vec![first];
                    while let Ok(p) = path_rx.try_recv() {
                        batch.push(p);
                    }
                    // CL files → re-index the affected scope, then emit cl:changed.
                    let cl_scopes: BTreeSet<String> =
                        batch.iter().filter_map(|p| scope_for_path(p, &cl_dir)).collect();
                    for scope in cl_scopes {
                        // Re-index disk→SQLite BEFORE telling the UI to refetch,
                        // or it would re-read a stale index.
                        if let Err(e) = bridge.cl_rescan(&scope).await {
                            tracing::warn!(error = ?e, scope = %scope, "fs watcher: cl_rescan failed");
                            continue;
                        }
                        let project = if scope == Project::GLOBALS { None } else { Some(scope) };
                        emit(
                            ClChangedEvent::EVENT_NAME,
                            serde_json::to_value(ClChangedEvent { project }).unwrap_or(Value::Null),
                        );
                    }
                    // Working-repo files → the session's A-tab diff is now stale.
                    let sessions: BTreeSet<String> =
                        batch.iter().filter_map(|p| session_for_path(p, &repos)).collect();
                    for session_id in sessions {
                        emit(
                            WorktreeChangedEvent::EVENT_NAME,
                            serde_json::to_value(WorktreeChangedEvent { session_id })
                                .unwrap_or(Value::Null),
                        );
                    }
                    // Plugin served dirs → tell the mounted panel its own
                    // content changed (same churn filter as session repos —
                    // linked repos see cargo/npm build noise).
                    let changed_plugins: BTreeSet<String> = batch
                        .iter()
                        .filter_map(|p| plugin_for_path(p, &plugin_dirs))
                        .collect();
                    for plugin_id in changed_plugins {
                        emit(
                            PluginAssetsChangedEvent::EVENT_NAME,
                            serde_json::to_value(PluginAssetsChangedEvent { plugin_id })
                                .unwrap_or(Value::Null),
                        );
                    }
                },
                else => break,
            }
        }
    });
    Ok(WatcherHandle { cmd_tx })
}

/// Map a changed path to its CL scope, relative to the CL dir.
/// `projects/<name>/…` → `Some(name)`; root files + `agents/…` (anything else
/// directly under the CL dir) → `Some("_globals")`; a hidden component (editor
/// swap files, `.DS_Store`, `.git`) or the CL dir itself → `None`.
fn scope_for_path(path: &Path, cl_dir: &Path) -> Option<String> {
    let rel = path.strip_prefix(cl_dir).ok()?;
    // Collect normal components; bail on any hidden one (editor swap/temp churn
    // shouldn't trigger a rescan).
    let mut names: Vec<&str> = Vec::new();
    for comp in rel.components() {
        if let std::path::Component::Normal(n) = comp {
            let s = n.to_str()?;
            if s.starts_with('.') {
                return None;
            }
            names.push(s);
        }
    }
    match names.split_first() {
        Some((first, rest)) if *first == "projects" => rest.first().map(|name| name.to_string()),
        Some(_) => Some(Project::GLOBALS.to_string()),
        None => None,
    }
}

/// Map a changed path under a watched session repo back to its session_id.
/// `None` if the path is under no watched repo, or if it lives in a build / VCS
/// dir whose churn shouldn't trigger an A-tab recompute.
fn session_for_path(path: &Path, repos: &HashMap<PathBuf, String>) -> Option<String> {
    for (root, session_id) in repos {
        if let Ok(rel) = path.strip_prefix(root) {
            if rel
                .components()
                .any(|c| matches!(c, std::path::Component::Normal(n) if is_ignored_component(n)))
            {
                return None;
            }
            return Some(session_id.clone());
        }
    }
    None
}

/// Map a changed path under a watched plugin served dir back to its
/// plugin_id, with the same build/VCS churn filter as session repos (linked
/// plugin dirs are user repos — `cargo build` noise must not spam reloads).
fn plugin_for_path(path: &Path, plugin_dirs: &HashMap<PathBuf, String>) -> Option<String> {
    for (root, plugin_id) in plugin_dirs {
        if let Ok(rel) = path.strip_prefix(root) {
            if rel
                .components()
                .any(|c| matches!(c, std::path::Component::Normal(n) if is_ignored_component(n)))
            {
                return None;
            }
            return Some(plugin_id.clone());
        }
    }
    None
}

/// A path component to ignore: any hidden (`.`-prefixed) name — covers `.git`,
/// `.vite`, `.next`, `.idea`, `.turbo`, editor temp dirs — or a known build dir.
fn is_ignored_component(name: &OsStr) -> bool {
    match name.to_str() {
        Some(s) => s.starts_with('.') || IGNORED_BUILD_DIRS.contains(&s),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cl() -> PathBuf {
        PathBuf::from("/data/library")
    }

    #[test]
    fn scope_project_file_maps_to_project() {
        assert_eq!(
            scope_for_path(&cl().join("projects/bot-hq/notes.md"), &cl()),
            Some("bot-hq".to_string())
        );
    }

    #[test]
    fn scope_root_file_is_globals() {
        assert_eq!(
            scope_for_path(&cl().join("scratch.md"), &cl()),
            Some(Project::GLOBALS.to_string())
        );
    }

    #[test]
    fn scope_agents_file_is_globals() {
        assert_eq!(
            scope_for_path(&cl().join("agents/brian/custom-instruction.md"), &cl()),
            Some(Project::GLOBALS.to_string())
        );
    }

    #[test]
    fn scope_hidden_component_is_skipped() {
        assert_eq!(
            scope_for_path(&cl().join("projects/bot-hq/.notes.md.swp"), &cl()),
            None
        );
        assert_eq!(scope_for_path(&cl().join(".DS_Store"), &cl()), None);
    }

    #[test]
    fn plugin_path_maps_to_plugin_and_filters_churn() {
        let mut dirs = HashMap::new();
        dirs.insert(PathBuf::from("/home/me/cognotify"), "cognotify".to_string());
        dirs.insert(PathBuf::from("/data/plugins/hello"), "hello".to_string());

        // Files inside a watched dir map to their plugin.
        assert_eq!(
            plugin_for_path(Path::new("/home/me/cognotify/materials/m1.html"), &dirs),
            Some("cognotify".to_string())
        );
        assert_eq!(
            plugin_for_path(Path::new("/data/plugins/hello/index.html"), &dirs),
            Some("hello".to_string())
        );
        // Build/VCS churn in a LINKED repo is filtered.
        assert_eq!(
            plugin_for_path(Path::new("/home/me/cognotify/target/debug/x"), &dirs),
            None
        );
        assert_eq!(
            plugin_for_path(Path::new("/home/me/cognotify/.git/index"), &dirs),
            None
        );
        assert_eq!(
            plugin_for_path(Path::new("/home/me/cognotify/node_modules/x/y.js"), &dirs),
            None
        );
        // Unwatched paths map to nothing.
        assert_eq!(plugin_for_path(Path::new("/somewhere/else.html"), &dirs), None);
    }

    #[test]
    fn scope_bare_projects_dir_is_none() {
        assert_eq!(scope_for_path(&cl().join("projects"), &cl()), None);
    }

    #[test]
    fn scope_outside_cl_dir_is_none() {
        assert_eq!(
            scope_for_path(Path::new("/somewhere/else/file.md"), &cl()),
            None
        );
    }

    fn repos_with(root: &str, sid: &str) -> HashMap<PathBuf, String> {
        let mut m = HashMap::new();
        m.insert(PathBuf::from(root), sid.to_string());
        m
    }

    #[test]
    fn session_for_source_file_maps_to_session() {
        let repos = repos_with("/repo", "s1");
        assert_eq!(
            session_for_path(Path::new("/repo/src/main.rs"), &repos),
            Some("s1".to_string())
        );
    }

    #[test]
    fn session_ignores_build_and_vcs_churn() {
        let repos = repos_with("/repo", "s1");
        assert_eq!(session_for_path(Path::new("/repo/target/debug/x"), &repos), None);
        assert_eq!(session_for_path(Path::new("/repo/.git/index"), &repos), None);
        assert_eq!(
            session_for_path(Path::new("/repo/node_modules/a/b.js"), &repos),
            None
        );
        assert_eq!(session_for_path(Path::new("/repo/.vite/dep.js"), &repos), None);
        // Shared IGNORED_BUILD_DIRS adds vendor/ + coverage/ (previously only the
        // CL walker filtered these — the watcher copy had drifted).
        assert_eq!(session_for_path(Path::new("/repo/vendor/x/y.php"), &repos), None);
        assert_eq!(
            session_for_path(Path::new("/repo/coverage/lcov.info"), &repos),
            None
        );
    }

    #[test]
    fn session_for_path_outside_all_repos_is_none() {
        let repos = repos_with("/repo", "s1");
        assert_eq!(session_for_path(Path::new("/elsewhere/file"), &repos), None);
    }
}
