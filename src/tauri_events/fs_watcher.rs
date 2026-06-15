//! Filesystem watcher: keeps Context Library / EOD views fresh without polling.
//!
//! `notify` (via `notify-debouncer-mini`) reports changes under the CL dir. For
//! each debounced batch we derive the affected CL *scope* (`projects/<name>/…` →
//! that project; root files / `agents/…` → `_globals`), re-index that scope via
//! the existing [`SignalingBridge::cl_rescan`] (which reconciles disk↔index:
//! add / touch / orphan-delete), THEN emit `cl:changed` so the frontend refetches
//! the now-current index.
//!
//! Re-indexing BEFORE the emit is load-bearing: `cl_index_search` reads the
//! SQLite index, not disk, so emitting without a rescan would just refresh stale
//! rows (and never surface brand-new files).
//!
//! `notify`'s callback is synchronous and runs on its own thread, so it merely
//! forwards changed paths over an mpsc channel to a tokio task that performs the
//! async rescan + emit. The task owns the debouncer, so the watch stays alive for
//! the process lifetime.

use crate::signaling::SignalingBridge;
use crate::storage::Project;
use crate::tauri_events::types::ClChangedEvent;
use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Debounce window for CL filesystem changes. Long enough to coalesce an
/// editor's save burst, short enough to feel immediate.
const CL_DEBOUNCE: Duration = Duration::from_millis(400);

/// Start watching the Context Library dir. Best-effort: returns the `notify`
/// error if the watcher can't be created (the caller logs it and CL views fall
/// back to their existing poll). `emit` is the same `app.emit`-backed closure
/// the bridge subscriber uses.
pub fn spawn_fs_watcher<EB>(
    paths: crate::paths::Paths,
    bridge: Arc<SignalingBridge>,
    emit: EB,
) -> Result<(), notify_debouncer_mini::notify::Error>
where
    EB: Fn(&str, Value) + Send + Sync + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();

    // notify's callback is sync + on its own thread — just forward the paths.
    let mut debouncer = new_debouncer(CL_DEBOUNCE, move |res: DebounceEventResult| {
        if let Ok(events) = res {
            for ev in events {
                let _ = tx.send(ev.path);
            }
        }
    })?;
    debouncer
        .watcher()
        .watch(&paths.cl_dir, RecursiveMode::Recursive)?;

    let cl_dir = paths.cl_dir.clone();
    tokio::spawn(async move {
        // Own the debouncer here so the watch lives as long as this task (the
        // whole app); dropping it would stop the watch.
        let _debouncer = debouncer;
        while let Some(first) = rx.recv().await {
            // Coalesce any paths already queued behind this one into one pass.
            let mut batch = vec![first];
            while let Ok(p) = rx.try_recv() {
                batch.push(p);
            }
            let scopes: BTreeSet<String> = batch
                .iter()
                .filter_map(|p| scope_for_path(p, &cl_dir))
                .collect();
            for scope in scopes {
                // Re-index disk→SQLite for this scope BEFORE telling the UI to
                // refetch, or it would re-read a stale index.
                if let Err(e) = bridge.cl_rescan(&scope).await {
                    tracing::warn!(error = ?e, scope = %scope, "fs watcher: cl_rescan failed");
                    continue;
                }
                let project = if scope == Project::GLOBALS {
                    None
                } else {
                    Some(scope)
                };
                emit(
                    ClChangedEvent::EVENT_NAME,
                    serde_json::to_value(ClChangedEvent { project }).unwrap_or(Value::Null),
                );
            }
        }
    });
    Ok(())
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
            scope_for_path(&cl().join("eod.md"), &cl()),
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
    fn scope_bare_projects_dir_is_none() {
        // A change to `projects/` itself (no project subdir) has no scope.
        assert_eq!(scope_for_path(&cl().join("projects"), &cl()), None);
    }

    #[test]
    fn scope_outside_cl_dir_is_none() {
        assert_eq!(
            scope_for_path(Path::new("/somewhere/else/file.md"), &cl()),
            None
        );
    }
}
