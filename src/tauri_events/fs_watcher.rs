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

    // notify's callback is sync + on its own thread. Drop build-dir churn here
    // (target/, node_modules/, …) BEFORE it enters the channel: a `cargo build`
    // otherwise floods the mpsc with thousands of paths per window that only
    // wake the task to be discarded by the downstream filter. Build-dir NAMES
    // only, NOT the `.`-prefix rule — this sees the ABSOLUTE path and the CL dir
    // lives under `~/.bot-hq/`, so the dot-rule would match `.bot-hq` and drop
    // every CL event. Dot-prefixed churn (`.git`, …) stays filtered downstream,
    // where the path is first made relative to its watched root.
    let mut debouncer = new_debouncer(DEBOUNCE, move |res: DebounceEventResult| {
        if let Ok(events) = res {
            for ev in events {
                if has_ignored_build_dir(&ev.path) {
                    continue;
                }
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
        // Watched session repos, keyed by session with the notify watch shared
        // per root; watched plugin served dirs, keyed by plugin, same shape.
        let mut repos = WatchSet::default();
        let mut plugin_dirs = WatchSet::default();
        loop {
            tokio::select! {
                Some(cmd) = cmd_rx.recv() => match cmd {
                    WatchCmd::AddRepo { session_id, path } => {
                        if let Some(stale) = repos.remove(&session_id) {
                            let _ = debouncer.watcher().unwatch(&stale);
                        }
                        if repos.is_watched(&path)
                            || debouncer.watcher().watch(&path, RecursiveMode::Recursive).is_ok()
                        {
                            repos.add(session_id, path);
                        } else {
                            tracing::warn!(?path, "fs watcher: failed to watch session repo");
                        }
                    }
                    WatchCmd::RemoveRepo { session_id } => {
                        if let Some(p) = repos.remove(&session_id) {
                            let _ = debouncer.watcher().unwatch(&p);
                        }
                    }
                    WatchCmd::AddPluginDir { plugin_id, path } => {
                        if let Some(stale) = plugin_dirs.remove(&plugin_id) {
                            let _ = debouncer.watcher().unwatch(&stale);
                        }
                        if plugin_dirs.is_watched(&path)
                            || debouncer.watcher().watch(&path, RecursiveMode::Recursive).is_ok()
                        {
                            plugin_dirs.add(plugin_id, path);
                        } else {
                            tracing::warn!(?path, "fs watcher: failed to watch plugin dir");
                        }
                    }
                    WatchCmd::RemovePluginDir { plugin_id } => {
                        if let Some(p) = plugin_dirs.remove(&plugin_id) {
                            let _ = debouncer.watcher().unwatch(&p);
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
                    // Working-repo files → EVERY session on that repo has a
                    // stale A-tab diff.
                    let sessions: BTreeSet<String> =
                        batch.iter().flat_map(|p| repos.owners_for_path(p)).collect();
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
                    for ev in plugin_events_for_batch(&batch, &plugin_dirs) {
                        emit(
                            PluginAssetsChangedEvent::EVENT_NAME,
                            serde_json::to_value(ev).unwrap_or(Value::Null),
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

/// Watched roots keyed by OWNER (a session id, a plugin id), the notify watch
/// shared per PATH and reference-counted.
///
/// Round 11: both registries were `HashMap<PathBuf, owner>`, so two owners on
/// one root — two sessions on the same working repo (worktrees off, or the
/// direct fallback), two linked plugins on one dir — overwrote each other: only
/// the last registrant was ever notified, and closing either one UNWATCHED the
/// path the other still needed. Here a root is watched once, released when its
/// last owner leaves, and a change under it names every owner.
#[derive(Default)]
struct WatchSet {
    by_owner: HashMap<String, PathBuf>,
    refs: HashMap<PathBuf, usize>,
}

impl WatchSet {
    /// Is `path` already carrying a notify watch (some owner registered it)?
    fn is_watched(&self, path: &Path) -> bool {
        self.refs.get(path).is_some_and(|n| *n > 0)
    }

    /// Register `owner` at `path` (the caller has made sure the path is
    /// watched). An owner registers once; re-registering replaces its root —
    /// call [`remove`](Self::remove) first to release the old one.
    fn add(&mut self, owner: String, path: PathBuf) {
        if let Some(prev) = self.by_owner.insert(owner, path.clone()) {
            if prev != path {
                self.release(&prev);
            }
        }
        *self.refs.entry(path).or_insert(0) += 1;
    }

    /// Unregister `owner`. Returns the root to UNWATCH when this was its last
    /// owner — `None` while another owner still needs it, or when the owner
    /// was not registered.
    fn remove(&mut self, owner: &str) -> Option<PathBuf> {
        let path = self.by_owner.remove(owner)?;
        self.release(&path)
    }

    fn release(&mut self, path: &Path) -> Option<PathBuf> {
        let n = self.refs.get_mut(path)?;
        *n = n.saturating_sub(1);
        if *n == 0 {
            self.refs.remove(path);
            Some(path.to_path_buf())
        } else {
            None
        }
    }

    /// Every owner whose root contains `path` — deterministic order (sorted) —
    /// or none if the path lives in a build / VCS dir whose churn shouldn't
    /// trigger anything.
    fn owners_for_path(&self, path: &Path) -> Vec<String> {
        let mut out: Vec<String> = self
            .by_owner
            .iter()
            .filter(|(_, root)| under_root_and_not_churn(path, root))
            .map(|(owner, _)| owner.clone())
            .collect();
        out.sort();
        out
    }
}

/// `path` is under `root` and no component between them is hidden or a known
/// build dir.
fn under_root_and_not_churn(path: &Path, root: &Path) -> bool {
    match path.strip_prefix(root) {
        Ok(rel) => !rel
            .components()
            .any(|c| matches!(c, std::path::Component::Normal(n) if is_ignored_component(n))),
        Err(_) => false,
    }
}

/// One `plugin:assets_changed` event per plugin whose served dir the batch
/// touched — deduped (BTreeSet, so deterministic order), churn-filtered,
/// and scoped strictly to the owning plugin(s): a path under A's root can
/// never yield B's id unless B is registered on that same root. This is the
/// whole emit-mapping the watcher loop runs; extracted so the scoping
/// contract is testable without notify/debounce timing.
fn plugin_events_for_batch(batch: &[PathBuf], plugin_dirs: &WatchSet) -> Vec<PluginAssetsChangedEvent> {
    let changed: BTreeSet<String> = batch
        .iter()
        .flat_map(|p| plugin_dirs.owners_for_path(p))
        .collect();
    changed
        .into_iter()
        .map(|plugin_id| PluginAssetsChangedEvent { plugin_id })
        .collect()
}

/// A path component to ignore: any hidden (`.`-prefixed) name — covers `.git`,
/// `.vite`, `.next`, `.idea`, `.turbo`, editor temp dirs — or a known build dir.
fn is_ignored_component(name: &OsStr) -> bool {
    match name.to_str() {
        Some(s) => s.starts_with('.') || IGNORED_BUILD_DIRS.contains(&s),
        None => false,
    }
}

/// True if any component of `path` is a known build directory (`target`,
/// `node_modules`, …). The notify callback uses this to drop the high-volume
/// churn a `cargo build` / `npm ci` produces on the watcher thread, before it
/// enters the channel.
///
/// Deliberately NOT the full [`is_ignored_component`] dot-rule: the callback
/// sees the ABSOLUTE path, and the CL dir lives under `~/.bot-hq/`, so a
/// `.`-prefix check would match `.bot-hq` and silently drop every CL event.
/// Dot-prefixed churn (`.git`, `.vite`, …) stays filtered downstream, where the
/// path has first been made relative to its watched root.
fn has_ignored_build_dir(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(c, std::path::Component::Normal(n)
            if n.to_str().is_some_and(|s| IGNORED_BUILD_DIRS.contains(&s)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cl() -> PathBuf {
        PathBuf::from("/data/library")
    }

    #[test]
    fn build_dir_churn_is_filtered_before_the_channel() {
        // The dominant churn a `cargo build` / `npm ci` emits → dropped early.
        assert!(has_ignored_build_dir(Path::new(
            "/home/me/repo/target/debug/x.rlib"
        )));
        assert!(has_ignored_build_dir(Path::new(
            "/home/me/repo/node_modules/pkg/index.js"
        )));
        // Real source edits → kept.
        assert!(!has_ignored_build_dir(Path::new("/home/me/repo/src/main.rs")));
        // CL events live under `~/.bot-hq/`; the `.bot-hq` dot-component must NOT
        // be treated as build churn here, or the callback would drop every CL
        // change (the dot-rule is applied downstream on the repo-relative path).
        assert!(!has_ignored_build_dir(Path::new(
            "/Users/me/.bot-hq/library/projects/bot-hq/notes.md"
        )));
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

    /// A registry from `(root, owner)` pairs, as the loop builds one.
    fn watch_set(pairs: &[(&str, &str)]) -> WatchSet {
        let mut set = WatchSet::default();
        for (root, owner) in pairs {
            set.add(owner.to_string(), PathBuf::from(root));
        }
        set
    }

    fn owners(set: &WatchSet, path: &str) -> Vec<String> {
        set.owners_for_path(Path::new(path))
    }

    #[test]
    fn plugin_path_maps_to_plugin_and_filters_churn() {
        let dirs = watch_set(&[
            ("/home/me/cognotify", "cognotify"),
            ("/data/plugins/hello", "hello"),
        ]);

        // Files inside a watched dir map to their plugin.
        assert_eq!(owners(&dirs, "/home/me/cognotify/materials/m1.html"), vec!["cognotify"]);
        assert_eq!(owners(&dirs, "/data/plugins/hello/index.html"), vec!["hello"]);
        // Build/VCS churn in a LINKED repo is filtered.
        assert!(owners(&dirs, "/home/me/cognotify/target/debug/x").is_empty());
        assert!(owners(&dirs, "/home/me/cognotify/.git/index").is_empty());
        assert!(owners(&dirs, "/home/me/cognotify/node_modules/x/y.js").is_empty());
        // Unwatched paths map to nothing.
        assert!(owners(&dirs, "/somewhere/else.html").is_empty());
    }

    /// **Two owners on one root both hear about it, and the watch outlives the
    /// first to leave** (round 11). The registries were path→owner maps, so the
    /// second session on a shared working repo (worktrees off) silently
    /// replaced the first, and closing either one unwatched the root the other
    /// still needed. Now: one watch, reference-counted; a change names every
    /// owner; only the last release asks for an unwatch.
    #[test]
    fn two_owners_share_one_watch_and_the_last_one_out_releases_it() {
        let mut set = WatchSet::default();
        set.add("s1".into(), PathBuf::from("/repo"));
        set.add("s2".into(), PathBuf::from("/repo"));
        assert!(set.is_watched(Path::new("/repo")));
        assert_eq!(owners(&set, "/repo/src/main.rs"), vec!["s1", "s2"], "both are told");
        // s1 closes: the root is still watched for s2, nothing to unwatch yet.
        assert_eq!(set.remove("s1"), None);
        assert!(set.is_watched(Path::new("/repo")));
        assert_eq!(owners(&set, "/repo/src/main.rs"), vec!["s2"]);
        // s2 closes: the last owner out releases the watch.
        assert_eq!(set.remove("s2"), Some(PathBuf::from("/repo")));
        assert!(!set.is_watched(Path::new("/repo")));
        assert!(owners(&set, "/repo/src/main.rs").is_empty());
        // Removing a stranger is a no-op.
        assert_eq!(set.remove("nobody"), None);
    }

    /// An owner that re-registers on a different root releases the old one
    /// (or keeps it if somebody else still holds it) and counts once on the new.
    #[test]
    fn re_registering_an_owner_moves_it_and_keeps_the_counts_honest() {
        let mut set = WatchSet::default();
        set.add("s1".into(), PathBuf::from("/a"));
        set.add("s2".into(), PathBuf::from("/a"));
        set.add("s1".into(), PathBuf::from("/b"));
        assert!(set.is_watched(Path::new("/a")), "s2 still holds /a");
        assert!(set.is_watched(Path::new("/b")));
        assert_eq!(owners(&set, "/a/f"), vec!["s2"]);
        assert_eq!(owners(&set, "/b/f"), vec!["s1"]);
        assert_eq!(set.remove("s2"), Some(PathBuf::from("/a")));
        assert_eq!(set.remove("s1"), Some(PathBuf::from("/b")));
    }

    /// The two-plugin scoping contract (PLUGINS.md: "a file in YOUR served
    /// directory changed"): touching A's dir yields exactly A's event —
    /// never B's, never a duplicate.
    #[test]
    fn assets_events_scope_to_the_owning_plugin_only() {
        let dirs = watch_set(&[("/plugins/a", "plugin-a"), ("/plugins/b", "plugin-b")]);

        // Batch touching only A → exactly one event, tagged A.
        let evs =
            plugin_events_for_batch(&[PathBuf::from("/plugins/a/index.html")], &dirs);
        assert_eq!(
            evs,
            vec![PluginAssetsChangedEvent {
                plugin_id: "plugin-a".to_string()
            }]
        );

        // Several A-paths dedupe to ONE A event; B still silent.
        let evs = plugin_events_for_batch(
            &[
                PathBuf::from("/plugins/a/x.js"),
                PathBuf::from("/plugins/a/sub/y.css"),
            ],
            &dirs,
        );
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].plugin_id, "plugin-a");

        // Both touched → one each (BTreeSet order: a then b).
        let evs = plugin_events_for_batch(
            &[
                PathBuf::from("/plugins/b/z.html"),
                PathBuf::from("/plugins/a/x.js"),
            ],
            &dirs,
        );
        let ids: Vec<&str> = evs.iter().map(|e| e.plugin_id.as_str()).collect();
        assert_eq!(ids, vec!["plugin-a", "plugin-b"]);

        // Churn inside a watched dir + paths outside any dir → nothing.
        let evs = plugin_events_for_batch(
            &[
                PathBuf::from("/plugins/a/node_modules/x.js"),
                PathBuf::from("/plugins/a/.git/index"),
                PathBuf::from("/elsewhere/f.txt"),
            ],
            &dirs,
        );
        assert!(evs.is_empty());
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

    #[test]
    fn session_for_source_file_maps_to_session() {
        let repos = watch_set(&[("/repo", "s1")]);
        assert_eq!(owners(&repos, "/repo/src/main.rs"), vec!["s1"]);
    }

    #[test]
    fn session_ignores_build_and_vcs_churn() {
        let repos = watch_set(&[("/repo", "s1")]);
        assert!(owners(&repos, "/repo/target/debug/x").is_empty());
        assert!(owners(&repos, "/repo/.git/index").is_empty());
        assert!(owners(&repos, "/repo/node_modules/a/b.js").is_empty());
        assert!(owners(&repos, "/repo/.vite/dep.js").is_empty());
        // Shared IGNORED_BUILD_DIRS adds vendor/ + coverage/ (previously only the
        // CL walker filtered these — the watcher copy had drifted).
        assert!(owners(&repos, "/repo/vendor/x/y.php").is_empty());
        assert!(owners(&repos, "/repo/coverage/lcov.info").is_empty());
    }

    #[test]
    fn session_for_path_outside_all_repos_is_none() {
        let repos = watch_set(&[("/repo", "s1")]);
        assert!(owners(&repos, "/elsewhere/file").is_empty());
    }
}
