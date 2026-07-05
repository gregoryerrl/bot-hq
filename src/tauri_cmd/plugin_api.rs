//! `plugin_invoke_proxy` — the single enforcement + dispatch point for
//! plugin-iframe RPC (plugin runtime v1).
//!
//! Plugin iframes postMessage the host shell (`frontend/src/lib/
//! pluginBridge.ts`), which forwards every call HERE. The frontend's
//! origin/nonce checks are transport hygiene only — this command re-checks
//! (enabled ∧ granted ∧ catalog-listed) in Rust and dispatches through an
//! explicit match, so a compromised or buggy shell listener never exposes
//! the broader Tauri command surface.
//!
//! The args/return contract is JSON-in-a-String on both sides. That keeps
//! the specta/TS surface trivially stable (`(pluginId, command, argsJson?)
//! -> string`) while each catalog arm owns its own arg schema — versioned
//! by `api_version` in the manifest, documented in docs/PLUGINS.md.

use crate::core::AppState as CoreAppState;
use crate::plugins::{catalog, PluginRegistry};
use crate::signaling::SignalingBridge;
use crate::storage::Storage;
use crate::tauri_cmd::error::AppError;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// Caps: plugin args are untrusted input arriving through the shell.
const MAX_ARGS_BYTES: usize = 65_536; // 64 KB of JSON args
const MAX_KV_KEY_BYTES: usize = 256;
const MAX_KV_VALUE_BYTES: usize = 262_144; // 256 KB per value
/// `cl_retrieve` budget: same default as the MCP tool, hard-capped so a
/// plugin can't vacuum the whole atom store in one call.
const DEFAULT_RETRIEVE_BUDGET: i64 = 3_000;
const MAX_RETRIEVE_BUDGET: i64 = 20_000;

#[tauri::command]
#[specta::specta]
pub async fn plugin_invoke_proxy(
    storage: tauri::State<'_, Arc<Storage>>,
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    core: tauri::State<'_, Arc<CoreAppState>>,
    registry: tauri::State<'_, Arc<PluginRegistry>>,
    plugin_id: String,
    command: String,
    args_json: Option<String>,
) -> Result<String, AppError> {
    check_plugin_grant(&registry, &plugin_id, &command)?;
    let args = parse_args(args_json.as_deref())?;
    tracing::debug!(plugin_id = %plugin_id, command = %command, "plugin_invoke_proxy");
    dispatch(&storage, &bridge, Some(&core), &plugin_id, &command, &args).await
}

/// The gate: catalog membership, enabled cache, and the CONSENT-FROZEN
/// grant cache (seeded from the DB-stored manifest the user approved at
/// install/re-approve — never a manifest on disk, so editing a linked
/// repo's manifest.json cannot widen grants without the re-approval flow).
fn check_plugin_grant(
    registry: &PluginRegistry,
    plugin_id: &str,
    command: &str,
) -> Result<(), AppError> {
    if !catalog::is_valid(command) {
        return Err(AppError::Validation(format!(
            "unknown plugin command {command:?} (not in the api_version-1 catalog)"
        )));
    }
    if !registry.is_enabled(plugin_id) {
        return Err(AppError::Validation(format!(
            "plugin {plugin_id:?} is not installed+enabled"
        )));
    }
    match registry.granted_caps_for(plugin_id) {
        Some(caps) if caps.iter().any(|c| c == command) => Ok(()),
        Some(_) => Err(AppError::Validation(format!(
            "capability {command:?} was not granted to plugin {plugin_id:?}"
        ))),
        None => Err(AppError::Validation(format!(
            "plugin {plugin_id:?} has no recorded capability grants"
        ))),
    }
}

/// Parse the JSON args envelope. Absent/empty = `{}`; anything else must be
/// a JSON object within the size cap.
fn parse_args(args_json: Option<&str>) -> Result<Value, AppError> {
    let Some(raw) = args_json else {
        return Ok(Value::Object(Default::default()));
    };
    if raw.len() > MAX_ARGS_BYTES {
        return Err(AppError::Validation(format!(
            "args too large ({} bytes, max {MAX_ARGS_BYTES})",
            raw.len()
        )));
    }
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| AppError::Validation(format!("args must be JSON: {e}")))?;
    match v {
        Value::Object(_) | Value::Null => Ok(v),
        _ => Err(AppError::Validation("args must be a JSON object".into())),
    }
}

fn need_str(args: &Value, key: &str) -> Result<String, AppError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Validation(format!("missing required string arg {key:?}")))
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn opt_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

fn opt_str_vec(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key).and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    })
}

fn to_json<T: Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|e| AppError::Internal(format!("serializing plugin result: {e}")))
}

/// What plugins see for a retrieved CL atom. Local (not the storage
/// `RetrievedAtom`) so the plugin contract can't drift when storage grows
/// fields. `stale` is reserved: the storage path always reports `false`
/// (drift-recompute lives in the bridge MCP wrapper, deliberately not
/// invoked here so plugin reads don't pollute agent retrieval telemetry).
#[derive(Debug, Serialize)]
struct PluginAtomView {
    file_path: String,
    heading_path: String,
    body: String,
    stale: bool,
}

/// Agent-aligned CL index row for plugins — the SAME trimmed shape the
/// agent MCP tool returns (signaling/jsonrpc.rs `cl_index_search`), incl.
/// the `project_id`→`project` rename, so plugin authors and agent docs
/// describe one contract. Deliberately NOT the UI's `ClIndexEntryView`
/// (which carries id/project_id/created_at and may grow fields freely).
#[derive(Debug, Serialize)]
struct PluginClIndexEntryView {
    project: String,
    file_path: String,
    description: String,
    tags: Option<String>,
    updated_at: String,
}

/// Agent-aligned CL folder row for plugins — mirrors the agent MCP
/// `cl_folder_search` trim, same rationale as [`PluginClIndexEntryView`].
#[derive(Debug, Serialize)]
struct PluginClFolderView {
    project: String,
    folder_path: String,
    description: String,
    tags: Option<String>,
    updated_at: String,
}

/// Catalog dispatch. `core` is `Option` ONLY so unit tests can exercise the
/// storage/bridge arms without booting a full `CoreAppState` (which needs a
/// live signaling server); the production shim always passes `Some`.
pub(crate) async fn dispatch(
    storage: &Storage,
    bridge: &SignalingBridge,
    core: Option<&CoreAppState>,
    plugin_id: &str,
    command: &str,
    args: &Value,
) -> Result<String, AppError> {
    match command {
        "list_sessions" => {
            let rows = storage
                .list_active_sessions_with_preview()
                .await
                .map_err(|e| AppError::DbError(e.to_string()))?;
            let out: Vec<crate::tauri_cmd::sessions::SessionInfo> =
                rows.into_iter().map(Into::into).collect();
            to_json(&out)
        }
        "get_session" => {
            let session_id = need_str(args, "session_id")?;
            let out: Option<crate::tauri_cmd::sessions::SessionInfo> = storage
                .get_session(&session_id)
                .await
                .map_err(|e| AppError::DbError(e.to_string()))?
                .map(Into::into);
            to_json(&out)
        }
        "list_messages" => {
            let session_id = need_str(args, "session_id")?;
            let since_id = opt_i64(args, "since_id");
            let msgs = storage
                .messages_for_session(&session_id, since_id)
                .await
                .map_err(|e| AppError::DbError(e.to_string()))?;
            let out: Vec<crate::tauri_events::types::AgentMessage> =
                msgs.into_iter().map(Into::into).collect();
            to_json(&out)
        }
        "session_doc_search" => {
            let session_id = need_str(args, "session_id")?;
            let query = opt_str(args, "query");
            let phase = opt_str(args, "phase");
            let docs = bridge
                .session_doc_search(&session_id, query.as_deref(), phase.as_deref())
                .await?;
            let out: Vec<crate::tauri_cmd::docs::SessionDocumentView> =
                docs.into_iter().map(Into::into).collect();
            to_json(&out)
        }
        "cl_index_search" => {
            let project = opt_str(args, "project");
            let query = opt_str(args, "query");
            let rows = bridge
                .cl_index_search(project.as_deref(), query.as_deref())
                .await?;
            let out: Vec<PluginClIndexEntryView> = rows
                .into_iter()
                .map(|r| PluginClIndexEntryView {
                    project: r.project_id,
                    file_path: r.file_path,
                    description: r.description,
                    tags: r.tags,
                    updated_at: r.updated_at,
                })
                .collect();
            to_json(&out)
        }
        "cl_folder_search" => {
            let project = opt_str(args, "project");
            let query = opt_str(args, "query");
            let rows = bridge
                .cl_folder_search(project.as_deref(), query.as_deref())
                .await?;
            let out: Vec<PluginClFolderView> = rows
                .into_iter()
                .map(|r| PluginClFolderView {
                    project: r.project_id,
                    folder_path: r.folder_path,
                    description: r.description,
                    tags: r.tags,
                    updated_at: r.updated_at,
                })
                .collect();
            to_json(&out)
        }
        "cl_retrieve" => {
            let project = need_str(args, "project")?;
            let query = need_str(args, "query")?;
            let paths = opt_str_vec(args, "paths");
            let budget = opt_i64(args, "budget_tokens")
                .unwrap_or(DEFAULT_RETRIEVE_BUDGET)
                .clamp(1, MAX_RETRIEVE_BUDGET);
            let atoms = storage
                .cl_retrieve(&project, &query, paths.as_deref(), budget)
                .await
                .map_err(|e| AppError::DbError(e.to_string()))?;
            let out: Vec<PluginAtomView> = atoms
                .into_iter()
                .map(|a| PluginAtomView {
                    file_path: a.file_path,
                    heading_path: a.heading_path,
                    body: a.body,
                    stale: a.stale,
                })
                .collect();
            to_json(&out)
        }
        "cl_read_file" => {
            let project = need_str(args, "project")?;
            let file_path = need_str(args, "file_path")?;
            let out = crate::tauri_cmd::cl::cl_read_file_inner(bridge, project, file_path).await?;
            to_json(&out)
        }
        "list_projects" => {
            let rows = storage.list_projects().await?;
            let out: Vec<crate::tauri_cmd::cl::ProjectView> =
                rows.into_iter().map(Into::into).collect();
            to_json(&out)
        }
        "compute_apply_diff" => {
            let session_id = need_str(args, "session_id")?;
            let core = core.ok_or_else(|| {
                AppError::Internal("core state unavailable for compute_apply_diff".into())
            })?;
            let out = crate::tauri_cmd::docs::compute_apply_diff_inner(core, session_id).await?;
            to_json(&out)
        }
        "spawn_session" => {
            // Session CREATION only — double-consented (install-time grant,
            // checked by check_plugin_grant like every command, PLUS the
            // shell's per-spawn confirm dialog before this call is even
            // made). The arm mints a fresh id; there is no path from here
            // to any existing session.
            let prompt = need_str(args, "prompt")?;
            if prompt.trim().is_empty() {
                return Err(AppError::Validation(
                    "spawn_session: prompt must not be empty".into(),
                ));
            }
            let project = opt_str(args, "project");
            if let Some(p) = &project {
                if storage.get_project(p).await?.is_none() {
                    return Err(AppError::Validation(format!(
                        "spawn_session: unknown project {p:?}"
                    )));
                }
            }
            let title = opt_str(args, "title")
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| format!("{plugin_id} session"));
            let core = core.ok_or_else(|| {
                AppError::Internal("core state unavailable for spawn_session".into())
            })?;
            // Same id shape the host UI mints (s-<uuid4 first 8>).
            let id = format!("s-{}", &uuid::Uuid::new_v4().to_string()[..8]);
            let info = crate::tauri_cmd::sessions::dispatch_session_inner(
                core, storage, bridge, id, title, project, None, prompt,
            )
            .await?;
            // Narrow contract: the id and nothing else (the session-read
            // commands are separate grants).
            to_json(&serde_json::json!({ "session_id": info.id }))
        }
        "plugin_kv_get" => {
            let key = need_str(args, "key")?;
            if key.len() > MAX_KV_KEY_BYTES {
                return Err(AppError::Validation(format!(
                    "kv key too long (max {MAX_KV_KEY_BYTES} bytes)"
                )));
            }
            let out = storage
                .plugin_kv_get(plugin_id, &key)
                .await
                .map_err(|e| AppError::DbError(e.to_string()))?;
            to_json(&out)
        }
        "plugin_kv_set" => {
            let key = need_str(args, "key")?;
            let value = need_str(args, "value")?;
            if key.len() > MAX_KV_KEY_BYTES {
                return Err(AppError::Validation(format!(
                    "kv key too long (max {MAX_KV_KEY_BYTES} bytes)"
                )));
            }
            if value.len() > MAX_KV_VALUE_BYTES {
                return Err(AppError::Validation(format!(
                    "kv value too large (max {MAX_KV_VALUE_BYTES} bytes)"
                )));
            }
            storage
                .plugin_kv_set(plugin_id, &key, &value)
                .await
                .map_err(|e| AppError::DbError(e.to_string()))?;
            to_json(&true)
        }
        // check_plugin_grant runs first, so reaching here means the catalog
        // lists a command this match doesn't — a bug, not plugin input.
        other => Err(AppError::Internal(format!(
            "catalog command {other:?} has no dispatch arm"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginRegistry;
    use tempfile::TempDir;

    /// Registry with one plugin granting `caps` (seeded into the
    /// consent-frozen grant cache, the way boot/install do), enabled per
    /// flag.
    fn registry_with(tmp: &TempDir, id: &str, caps: &[&str], enabled: bool) -> PluginRegistry {
        let reg = PluginRegistry::new(tmp.path().to_path_buf()).unwrap();
        reg.set_granted_caps(id, Some(caps.iter().map(|c| c.to_string()).collect()));
        reg.set_enabled(id, enabled);
        reg
    }

    async fn test_bridge(tmp: &TempDir, storage: &Storage) -> Arc<SignalingBridge> {
        let bridge = SignalingBridge::with_policy(
            crate::policy::ViolationsLog::new(tmp.path()),
            tmp.path().to_path_buf(),
        );
        bridge.set_storage(storage.clone()).await;
        bridge
    }

    #[test]
    fn grant_check_rejects_unknown_command_disabled_and_ungranted() {
        let tmp = TempDir::new().unwrap();
        let reg = registry_with(&tmp, "deck", &["list_sessions"], true);

        // Catalog miss — even for an enabled plugin that requested it.
        assert!(check_plugin_grant(&reg, "deck", "create_session").is_err());
        // Granted + enabled + in catalog = ok.
        assert!(check_plugin_grant(&reg, "deck", "list_sessions").is_ok());
        // In catalog but not granted.
        assert!(check_plugin_grant(&reg, "deck", "cl_index_search").is_err());
        // Unknown plugin id.
        assert!(check_plugin_grant(&reg, "ghost", "list_sessions").is_err());

        // Disabled plugin: same manifest, cache off.
        reg.set_enabled("deck", false);
        assert!(check_plugin_grant(&reg, "deck", "list_sessions").is_err());
    }

    /// spawn_session sits behind the SAME Rust gate as every command: in the
    /// catalog, grantable, and refused for plugins that didn't request it or
    /// are disabled — all BEFORE any dialog or dispatch.
    #[test]
    fn grant_check_gates_spawn_session_like_any_command() {
        let tmp = TempDir::new().unwrap();
        let reg = registry_with(&tmp, "deck", &["spawn_session"], true);
        assert!(check_plugin_grant(&reg, "deck", "spawn_session").is_ok());

        let tmp2 = TempDir::new().unwrap();
        let ungranted = registry_with(&tmp2, "deck", &["list_sessions"], true);
        assert!(check_plugin_grant(&ungranted, "deck", "spawn_session").is_err());

        reg.set_enabled("deck", false);
        assert!(check_plugin_grant(&reg, "deck", "spawn_session").is_err());
    }

    #[tokio::test]
    async fn dispatch_spawn_session_validates_args_before_touching_core() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::memory().await.unwrap();
        let bridge = test_bridge(&tmp, &storage).await;

        // Missing prompt.
        let err = dispatch(
            &storage,
            &bridge,
            None,
            "deck",
            "spawn_session",
            &Value::Object(Default::default()),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        // Whitespace-only prompt.
        let args = serde_json::json!({ "prompt": "   " });
        let err = dispatch(&storage, &bridge, None, "deck", "spawn_session", &args)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        // Unknown project.
        let args = serde_json::json!({ "prompt": "do things", "project": "ghost" });
        let err = dispatch(&storage, &bridge, None, "deck", "spawn_session", &args)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        // Valid args but no core (unit-test path) → Internal, proving arg
        // validation happens first and nothing was created.
        let args = serde_json::json!({ "prompt": "do things" });
        let err = dispatch(&storage, &bridge, None, "deck", "spawn_session", &args)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Internal(_)), "got {err:?}");
        assert!(storage.list_active_sessions_with_preview().await.unwrap().is_empty());
    }

    #[test]
    fn parse_args_accepts_object_rejects_rest() {
        assert!(parse_args(None).unwrap().is_object());
        assert!(parse_args(Some(r#"{"a":1}"#)).unwrap().is_object());
        assert!(parse_args(Some("null")).unwrap().is_null());
        assert!(parse_args(Some("[1,2]")).is_err());
        assert!(parse_args(Some("\"str\"")).is_err());
        assert!(parse_args(Some("not json")).is_err());
        let big = format!(r#"{{"k":"{}"}}"#, "x".repeat(MAX_ARGS_BYTES));
        assert!(parse_args(Some(&big)).is_err());
    }

    #[tokio::test]
    async fn dispatch_list_sessions_returns_json_array() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::memory().await.unwrap();
        let bridge = test_bridge(&tmp, &storage).await;
        let out = dispatch(
            &storage,
            &bridge,
            None,
            "deck",
            "list_sessions",
            &Value::Object(Default::default()),
        )
        .await
        .unwrap();
        assert_eq!(out, "[]");
    }

    #[tokio::test]
    async fn dispatch_kv_roundtrip_is_plugin_namespaced() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::memory().await.unwrap();
        storage.insert_plugin("deck", "D", "0.1", "{}", "/x", None, false).await.unwrap();
        storage.insert_plugin("other", "O", "0.1", "{}", "/y", None, false).await.unwrap();
        let bridge = test_bridge(&tmp, &storage).await;

        let set_args: Value = serde_json::from_str(r#"{"key":"lens","value":"graph"}"#).unwrap();
        dispatch(&storage, &bridge, None, "deck", "plugin_kv_set", &set_args)
            .await
            .unwrap();

        let get_args: Value = serde_json::from_str(r#"{"key":"lens"}"#).unwrap();
        let got = dispatch(&storage, &bridge, None, "deck", "plugin_kv_get", &get_args)
            .await
            .unwrap();
        assert_eq!(got, "\"graph\"");
        // The OTHER plugin sees nothing under the same key — plugin_id is
        // stamped by the proxy, not passed by the plugin.
        let other = dispatch(&storage, &bridge, None, "other", "plugin_kv_get", &get_args)
            .await
            .unwrap();
        assert_eq!(other, "null");
    }

    #[tokio::test]
    async fn dispatch_kv_set_enforces_size_caps() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::memory().await.unwrap();
        storage.insert_plugin("deck", "D", "0.1", "{}", "/x", None, false).await.unwrap();
        let bridge = test_bridge(&tmp, &storage).await;

        let long_key = "k".repeat(MAX_KV_KEY_BYTES + 1);
        let args = serde_json::json!({ "key": long_key, "value": "v" });
        assert!(dispatch(&storage, &bridge, None, "deck", "plugin_kv_set", &args)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn dispatch_missing_required_arg_is_validation_error() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::memory().await.unwrap();
        let bridge = test_bridge(&tmp, &storage).await;
        let err = dispatch(
            &storage,
            &bridge,
            None,
            "deck",
            "get_session",
            &Value::Object(Default::default()),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn dispatch_cl_retrieve_empty_store_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::memory().await.unwrap();
        let bridge = test_bridge(&tmp, &storage).await;
        let args = serde_json::json!({ "project": "p", "query": "anything" });
        let out = dispatch(&storage, &bridge, None, "deck", "cl_retrieve", &args)
            .await
            .unwrap();
        assert_eq!(out, "[]");
    }

    /// End to end over the REAL example plugin (`examples/hello-plugin`,
    /// which doubles as the fixture): install through the real install
    /// path, then exercise the gate, the dispatch arms it was granted, and
    /// the serve-layer resolution of its actual bundle files.
    #[tokio::test]
    async fn full_flow_example_plugin_install_then_proxy() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::memory().await.unwrap();
        let registry = PluginRegistry::new(tmp.path().to_path_buf()).unwrap();
        let bridge = test_bridge(&tmp, &storage).await;

        let example = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/hello-plugin");
        crate::tauri_cmd::plugins::install_plugin_inner(&storage, &registry, example, false)
            .await
            .unwrap();

        // Granted command flows install → gate → dispatch.
        check_plugin_grant(&registry, "hello-plugin", "list_sessions").unwrap();
        let out = dispatch(
            &storage,
            &bridge,
            None,
            "hello-plugin",
            "list_sessions",
            &Value::Object(Default::default()),
        )
        .await
        .unwrap();
        assert_eq!(out, "[]");

        // KV roundtrip under the real granted set.
        let set = serde_json::json!({ "key": "opens", "value": "1" });
        dispatch(&storage, &bridge, None, "hello-plugin", "plugin_kv_set", &set)
            .await
            .unwrap();
        let get = serde_json::json!({ "key": "opens" });
        assert_eq!(
            dispatch(&storage, &bridge, None, "hello-plugin", "plugin_kv_get", &get)
                .await
                .unwrap(),
            "\"1\""
        );

        // Catalog command the manifest did NOT request is refused.
        assert!(check_plugin_grant(&registry, "hello-plugin", "compute_apply_diff").is_err());

        // The serve layer resolves the example's real bundle files through
        // the PRODUCTION path: install seeded the serve-root cache, the
        // handler resolves against it.
        let root = registry.serve_root_for("hello-plugin");
        assert_eq!(
            root.as_deref(),
            Some(registry.data_dir.join("plugins").join("hello-plugin").as_path())
        );
        let (path, mime) = crate::plugins::serve::resolve_with_root(
            root.as_deref(),
            registry.is_enabled("hello-plugin"),
            "hello-plugin",
            "index.html",
        )
        .unwrap();
        assert!(path.is_file());
        assert_eq!(mime, "text/html; charset=utf-8");
        let (_, sdk_mime) = crate::plugins::serve::resolve_with_root(
            root.as_deref(),
            registry.is_enabled("hello-plugin"),
            "hello-plugin",
            "bhq-sdk.js",
        )
        .unwrap();
        assert_eq!(sdk_mime, "text/javascript; charset=utf-8");

        // Install seeded the consent-frozen grant cache too.
        assert!(registry
            .granted_caps_for("hello-plugin")
            .unwrap()
            .contains(&"list_sessions".to_string()));
    }

    /// The plugin CL views must keep the AGENT field names — `project`, not
    /// the storage/UI `project_id`, and no id/created_at. This is the
    /// contract test for the 2026-07-05 breaking alignment (PLUGINS.md).
    #[test]
    fn plugin_cl_views_use_agent_aligned_field_names() {
        let entry = serde_json::to_value(PluginClIndexEntryView {
            project: "p".into(),
            file_path: "notes.md".into(),
            description: "d".into(),
            tags: None,
            updated_at: "t".into(),
        })
        .unwrap();
        // serde_json's default Map is a BTreeMap → keys arrive sorted.
        let keys: Vec<&str> = entry.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, ["description", "file_path", "project", "tags", "updated_at"]);
        assert!(entry.get("project_id").is_none());
        assert!(entry.get("id").is_none());
        assert!(entry.get("created_at").is_none());

        let folder = serde_json::to_value(PluginClFolderView {
            project: "p".into(),
            folder_path: "plans".into(),
            description: "d".into(),
            tags: None,
            updated_at: "t".into(),
        })
        .unwrap();
        assert!(folder.get("project").is_some());
        assert!(folder.get("folder_path").is_some());
        assert!(folder.get("project_id").is_none());
        assert!(folder.get("id").is_none());
    }

    #[tokio::test]
    async fn dispatch_cl_index_search_via_bridge_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::memory().await.unwrap();
        let bridge = test_bridge(&tmp, &storage).await;
        let out = dispatch(
            &storage,
            &bridge,
            None,
            "deck",
            "cl_index_search",
            &Value::Object(Default::default()),
        )
        .await
        .unwrap();
        assert_eq!(out, "[]");
    }
}
