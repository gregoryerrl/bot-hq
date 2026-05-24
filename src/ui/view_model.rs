//! Slint <-> core glue.
//!
//! `install_view_model` is the bottleneck called from `main.rs`. It:
//! - subscribes to signaling events and pushes them into Slint models;
//! - wires the Slint global callbacks (`broadcast`, `open-session`, etc.) to
//!   `core::AppState` methods (running on a tokio runtime);
//! - kicks off periodic refresh tasks for chat history.
//!
//! Slint's main-loop is on the OS main thread; tokio runs on its own thread.
//! We use `slint::Weak::upgrade_in_event_loop` to mutate Slint models from tokio.

use crate::core::AppState as CoreAppState;
use crate::signaling::SignalingEvent;
use crate::storage::{AgentConfig as DbAgentConfig, Message};
use crate::{
    AgentConfigRow, AppState as SlintAppState, AppWindow, CLFileEntry, ChatMsg, NewSessionProject,
    PendingQuestion, SessionTile,
};
use once_cell::sync::Lazy;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::warn;

/// Wrap a Slint callback body in `catch_unwind` so a Rust panic logs +
/// surfaces a toast instead of unwinding across the C++ FFI boundary
/// into `abort()`. Slint dispatches the registered `on_*` callbacks
/// from Cocoa/C++ via the event loop; a Rust panic that crosses that
/// barrier is undefined behavior and the runtime aborts the process.
///
/// `weak` is captured by reference (cheap) and used to surface a toast
/// when a panic is caught. The body closure must wrap in
/// `AssertUnwindSafe` because moved-in captures like `Arc<CoreAppState>`
/// and `Weak<AppWindow>` do not auto-derive `UnwindSafe`.
fn ffi_safe<F>(name: &'static str, weak: &Weak<AppWindow>, f: F)
where
    F: FnOnce() + std::panic::UnwindSafe,
{
    if let Err(payload) = std::panic::catch_unwind(f) {
        let msg = panic_payload_string(&*payload);
        tracing::error!(callback = name, panic = %msg, "panic in Slint callback (contained)");
        show_toast(weak, &format!("Internal error in {name}; see logs."));
    }
}

fn panic_payload_string(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic payload>".to_string()
}

/// In-memory accumulator for the questions-tray "Send all" form. Keyed by
/// `choice_id`. The Slint side fires `set-answer(cid, value)` on every
/// change to a card's input; on `submit-questions-batch` we drain this and
/// route each row to its destination (bridge.resolve_choice for choices,
/// storage.answer_question for halts) + broadcast a single combined chat
/// message so the agent sees one turn instead of N.
static ANSWER_ACCUMULATOR: Lazy<std::sync::Mutex<HashMap<String, String>>> =
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

/// Per-session fingerprint of the last-rendered message list. Used to short-
/// circuit `set_session_msgs` when no new messages arrived — replacing a
/// ModelRc with identical content still tears down + rebuilds every row,
/// which clears TextInput selection (you can't highlight text mid-poll).
///
/// Fingerprint = (count, last_msg_id). Cheap to compute; misses an edited
/// middle message (rare and we don't edit messages in place anyway).
static MSG_FINGERPRINTS: Lazy<Mutex<HashMap<String, (usize, i64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Whichever session_id is currently rendered into `session-msgs`. We compare
/// against the refresh target so a session-switch (A→B) force-reloads the
/// model even if B's fingerprint is unchanged since the last visit. Without
/// this, returning to a session you've seen before keeps the previous
/// session's chat visible.
static DISPLAYED_SESSION: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

fn fingerprint(msgs: &[Message]) -> (usize, i64) {
    (msgs.len(), msgs.last().map(|m| m.id).unwrap_or(0))
}

/// Returns true iff the fingerprint changed and the cache was updated.
fn fingerprint_changed(session_id: &str, fp: (usize, i64)) -> bool {
    let mut cache = MSG_FINGERPRINTS.lock().unwrap_or_else(|p| p.into_inner());
    if cache.get(session_id) == Some(&fp) {
        false
    } else {
        cache.insert(session_id.to_string(), fp);
        true
    }
}

/// Returns true iff the refresh target session_id differs from whatever's
/// currently displayed in `session-msgs`. Side-effect: updates the tracker.
/// Use to force a session-switch reload regardless of fingerprint state.
fn displayed_session_changed(session_id: &str) -> bool {
    let mut current = DISPLAYED_SESSION.lock().unwrap_or_else(|p| p.into_inner());
    if *current != session_id {
        *current = session_id.to_string();
        true
    } else {
        false
    }
}

// ---- Send-safe shadow types for cross-thread refreshes ---------------------

#[derive(Clone)]
struct TileData {
    id: String,
    title: String,
    phase: String,
    last_activity: String,
    awaiting: bool,
    /// Live count of pending rows in `session_questions` for this session.
    /// Powers the dashboard card's `[Need User Input · N]` chip.
    pending_input_count: i32,
    quickview: String,
}

#[derive(Clone)]
struct AgentConfigData {
    agent_name: String,
    provider: String,
    model_name: String,
    base_url: String,
    auth_token: String,
}

#[derive(Clone)]
struct CLEntryData {
    relative_path: String,
    display_name: String,
    is_dir: bool,
    depth: i32,
    /// Directories: true when expanded (children listed below it). Files: false.
    expanded: bool,
    /// True when this is a synthetic "currently being typed" placeholder row
    /// for inline new-file/new-folder creation.
    is_ghost: bool,
    /// True when this directory is the on-disk CL root of a registered
    /// project (cl_path override OR default convention). Files always false.
    is_registered_project: bool,
    /// Folder description from `cl_folders`. Empty for files and for folders
    /// without a description on record.
    description: String,
}

#[derive(Clone)]
struct ChatMsgData {
    author: String,
    kind: String,
    content: String,
    created_at: String,
}

fn tile_from_data(d: TileData) -> SessionTile {
    SessionTile {
        id: SharedString::from(d.id),
        title: SharedString::from(d.title),
        phase: SharedString::from(d.phase),
        last_activity: SharedString::from(d.last_activity),
        awaiting: d.awaiting,
        pending_input_count: d.pending_input_count,
        quickview: SharedString::from(d.quickview),
    }
}

fn agent_cfg_from_data(d: AgentConfigData) -> AgentConfigRow {
    AgentConfigRow {
        agent_name: SharedString::from(d.agent_name),
        provider: SharedString::from(d.provider),
        model_name: SharedString::from(d.model_name),
        base_url: SharedString::from(d.base_url),
        auth_token: SharedString::from(d.auth_token),
    }
}

fn cl_entry_from_data(d: CLEntryData) -> CLFileEntry {
    CLFileEntry {
        relative_path: SharedString::from(d.relative_path),
        display_name: SharedString::from(d.display_name),
        is_dir: d.is_dir,
        depth: d.depth,
        expanded: d.expanded,
        is_ghost: d.is_ghost,
        is_registered_project: d.is_registered_project,
        description: SharedString::from(d.description),
    }
}

fn chat_from_data(d: ChatMsgData) -> ChatMsg {
    ChatMsg {
        author: SharedString::from(d.author),
        kind: SharedString::from(d.kind),
        content: SharedString::from(d.content),
        created_at: SharedString::from(d.created_at),
    }
}

/// Install all callbacks + start background refresh tasks. Returns once the
/// initial pull-from-storage finishes — callers can run the Slint event loop
/// afterwards.
pub async fn install_view_model(
    window: &AppWindow,
    core: Arc<CoreAppState>,
    rt: Handle,
) -> anyhow::Result<()> {
    let weak = window.as_weak();

    // ---- Initial state push --------------------------------------------
    refresh_dashboard(&weak, &core).await;
    refresh_agent_configs(&weak, &core).await;
    refresh_new_session_projects(&weak, &core).await;
    // Use the no-hop variant: the Slint event loop is not running yet —
    // window.run() is called by main.rs AFTER install_view_model returns —
    // so anything that awaits an upgrade_in_event_loop hop will deadlock
    // here. TreeState::default() is correct at startup: no folders are
    // collapsed and no inline-edit is in progress.
    refresh_cl_tree_with_state(&weak, &core, &TreeState::default()).await;
    seed_external_mcp_panel(&weak, &core).await;

    // ---- Callbacks -----------------------------------------------------
    let app = window.global::<SlintAppState>();

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_open_session({
            let weak_for_safe = weak.clone();
            move |session_id| {
                ffi_safe("on_open_session", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    let session_id = session_id.to_string();
                    rt.spawn(async move {
                        // Sessions persist in the DB across app restarts but their live
                        // subprocess handles do NOT. If the user clicks a session that
                        // existed before the current process, the `sessions` HashMap
                        // won't have it and broadcasts will fail with "no live session".
                        // Auto-respawn here so the session is live by the time the user
                        // tries to broadcast. Idempotent — no-op if already running.
                        if let Err(e) = core.ensure_session_started(&session_id).await {
                            warn!(
                                session_id = %session_id,
                                ?e,
                                "ensure_session_started failed — chat will be inactive (check claude auth)"
                            );
                        }
                        let _ = refresh_session_view(&weak, &core, &session_id).await;
                        update_active_session_id(&weak, &session_id);
                        // Inherit awaiting/pending state from the tile we just opened.
                        // Without this, the active-* globals carry over from whichever
                        // session was previously active — "Need user input" showed on
                        // every session you switched to, even if only one needed it.
                        sync_active_from_tile(&weak, &session_id);
                        // Populate the in-chat questions tray from durable storage.
                        refresh_active_questions(&weak, &core, &session_id);
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        app.on_back_to_dashboard({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_back_to_dashboard", &weak_for_safe, AssertUnwindSafe(|| {
                    update_active_session_id(&weak, "");
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_select_doc_tab({
            let weak_for_safe = weak.clone();
            move |letter| {
                ffi_safe("on_select_doc_tab", &weak_for_safe, AssertUnwindSafe(|| {
                    // Phase advancement is agent-driven via the advance_phase
                    // MCP tool — this callback only switches the document-tab
                    // selection and triggers an immediate phase-doc refresh
                    // so the right pane updates without waiting for the 500ms
                    // poll to come around.
                    let Some(window) = weak.upgrade() else {
                        return;
                    };
                    window
                        .global::<SlintAppState>()
                        .set_selected_doc_tab(letter.clone());

                    let session_id = current_session_id(&weak);
                    if session_id.is_empty() {
                        return;
                    }
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    let tab = letter.to_string();
                    rt.spawn(async move {
                        refresh_session_docs(&weak, &core, &session_id, &tab).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_broadcast({
            let weak_for_safe = weak.clone();
            move |text| {
                ffi_safe("on_broadcast", &weak_for_safe, AssertUnwindSafe(|| {
                    // See on_advance_phase: read session_id on the event-loop thread
                    // FIRST, then spawn the async work. Reading inside rt.spawn yields
                    // "" and the broadcast silently no-ops (the user's original bug).
                    let session_id = current_session_id(&weak);
                    let text = text.to_string();
                    if session_id.is_empty() || text.trim().is_empty() {
                        return;
                    }
                    // The user just answered — clear "Need user input" for this session.
                    // mark_awaiting_user set it; without an explicit clear, the banner
                    // stays sticky across sessions forever.
                    clear_awaiting_for(&weak, &session_id);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        if let Err(e) = core.broadcast(&session_id, &text).await {
                            warn!(?e, "broadcast failed");
                        }
                        let _ = refresh_session_view(&weak, &core, &session_id).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_emma_send({
            let weak_for_safe = weak.clone();
            move |text| {
                ffi_safe("on_emma_send", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    let text = text.to_string();
                    // Same as on_broadcast: user reply clears Emma's awaiting flag.
                    clear_emma_awaiting(&weak);
                    rt.spawn(async move {
                        if text.trim().is_empty() {
                            return;
                        }
                        if let Err(e) = core.broadcast("emma", &text).await {
                            warn!(?e, "emma send failed");
                        }
                        let _ = refresh_emma(&weak, &core).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_choice_clicked({
            let weak_for_safe = weak.clone();
            move |choice_id, picked| {
                ffi_safe("on_choice_clicked", &weak_for_safe, AssertUnwindSafe(|| {
                    let core = Arc::clone(&core);
                    let choice_id = choice_id.to_string();
                    let picked = picked.to_string();
                    rt.spawn(async move {
                        if let Err(e) = core.resolve_choice(&choice_id, picked).await {
                            warn!(?e, "resolve_choice");
                        }
                    });
                }));
            }
        });
    }

    // Tray "Send all" — per-card change handler.
    // Each QuestionCard's `changed picked => …` / `changed reply => …` fires
    // this. We mirror it into ANSWER_ACCUMULATOR and recompute submit-ready.
    {
        let weak = weak.clone();
        app.on_set_answer({
            let weak_for_safe = weak.clone();
            move |choice_id, value| {
                ffi_safe("on_set_answer", &weak_for_safe, AssertUnwindSafe(|| {
                    let cid = choice_id.to_string();
                    let val = value.to_string();
                    {
                        let mut acc = ANSWER_ACCUMULATOR.lock().unwrap_or_else(|p| p.into_inner());
                        if val.is_empty() {
                            acc.remove(&cid);
                        } else {
                            acc.insert(cid, val);
                        }
                    }
                    let ready = !ANSWER_ACCUMULATOR.lock().unwrap_or_else(|p| p.into_inner()).is_empty();
                    let _ = weak.upgrade_in_event_loop(move |handle| {
                        handle.global::<SlintAppState>().set_submit_ready(ready);
                    });
                }));
            }
        });
    }

    // Tray "Send all" — submission. Drains ANSWER_ACCUMULATOR, routes each
    // row, then broadcasts a combined user chat message so the agent reads
    // everything as a single turn (also clears the awaiting flag).
    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_submit_questions_batch({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_submit_questions_batch", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        let answers: Vec<(String, String)> = {
                            let mut acc = ANSWER_ACCUMULATOR.lock().unwrap_or_else(|p| p.into_inner());
                            acc.drain().collect()
                        };
                        if answers.is_empty() {
                            return;
                        }
                // Read the active session id from the Slint thread.
                let session_id: String = {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let _ = weak.upgrade_in_event_loop(move |handle| {
                        let _ = tx.send(
                            handle
                                .global::<SlintAppState>()
                                .get_active_session_id()
                                .to_string(),
                        );
                    });
                    rx.await.unwrap_or_default()
                };
                if session_id.is_empty() {
                    warn!("submit-questions-batch fired with no active session");
                    return;
                }
                // Split rows by kind. Choice rows route through resolve_choice
                // which delivers the picked option to the agent directly (via
                // oneshot or — when the agent's tool call timed out — an OOB
                // synthetic user message + input_tx wake). Halt/open_ask rows
                // have no built-in delivery channel; they ride out via the
                // broadcast below as the unified "user replied" message.
                let mut non_choice_lines: Vec<String> = Vec::new();
                for (cid, value) in &answers {
                    let q = match core.storage.get_question(cid).await {
                        Ok(Some(q)) => q,
                        Ok(None) => {
                            warn!(%cid, "submit batch: question missing");
                            continue;
                        }
                        Err(e) => {
                            warn!(?e, %cid, "submit batch: get_question failed");
                            continue;
                        }
                    };
                    match q.kind.as_str() {
                        "choice" => {
                            // resolve_choice handles delivery — don't duplicate
                            // the answer in the broadcast body.
                            if let Err(e) = core.resolve_choice(cid, value.clone()).await {
                                warn!(?e, %cid, "submit batch: resolve_choice");
                            }
                        }
                        kind => {
                            if let Err(e) = core.storage.answer_question(cid, value).await {
                                warn!(?e, %cid, "submit batch: answer_question");
                            }
                            let label = if kind == "halt" { "[halt]" } else { "[open-ask]" };
                            non_choice_lines
                                .push(format!("{label} \"{}\" → {}", q.prompt, value));
                        }
                    }
                }
                // Clear the Send-ready flag AND close the tray now that the
                // accumulator is drained. Closing the tray is the user-visible
                // "I'm done" signal; the conditional in app.slint also hides
                // it once active-questions is empty, but force-closing here
                // makes the dismiss happen synchronously instead of waiting
                // for a refresh round-trip.
                let _ = weak.upgrade_in_event_loop(move |handle| {
                    let app = handle.global::<SlintAppState>();
                    app.set_submit_ready(false);
                    app.set_questions_tray_open(false);
                });
                // Broadcast the residual non-choice answers (halt + open_ask
                // replies) as one combined user message — this is also what
                // clears the bridge's awaiting flag for the session. When the
                // batch was choice-only we still broadcast a minimal "user
                // replied" message so awaiting clears and peer-forwarding
                // resumes without anyone reading a stale halt.
                let body = if non_choice_lines.is_empty() {
                    "Answers submitted from the questions tray.".to_string()
                } else {
                    format!(
                        "Answers submitted from the questions tray:\n\n{}",
                        non_choice_lines.join("\n")
                    )
                };
                if let Err(e) = core.broadcast(&session_id, &body).await {
                    warn!(?e, %session_id, "submit batch: broadcast");
                }
                // Refresh the tray so the answered/withdrawn rows drop out.
                refresh_active_questions(&weak, &core, &session_id);
            });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_create_session({
            let weak_for_safe = weak.clone();
            move |title, working_repo| {
                ffi_safe("on_create_session", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    let title = title.to_string();
                    let path: Option<PathBuf> = if working_repo.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(working_repo.to_string()))
                    };
                    rt.spawn(async move {
                        match core.open_session(title, path).await {
                            Ok(id) => {
                                refresh_dashboard(&weak, &core).await;
                                // refresh_session_view atomically sets session-msgs +
                                // active-session-id in one slint invoke — calling
                                // update_active_session_id separately before it caused
                                // a flash: Slint rendered SessionView with active=new_id
                                // but session-msgs still holding the previous session's
                                // chat between the two invokes.
                                let _ = refresh_session_view(&weak, &core, &id).await;
                            }
                            Err(e) => warn!(?e, "open_session failed"),
                        }
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_set_session_permission({
            let weak_for_safe = weak.clone();
            move |action, granted| {
                ffi_safe("on_set_session_permission", &weak_for_safe, AssertUnwindSafe(|| {
                    // Read active session id on the event-loop thread BEFORE spawning
                    // — Slint property reads from a tokio task return empty.
                    let session_id = current_session_id(&weak);
                    if session_id.is_empty() {
                        return;
                    }
                    let action_str = action.to_string();
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        let parsed = match action_str.as_str() {
                            "commit" => crate::policy::PermissionAction::Commit,
                            "push" => crate::policy::PermissionAction::Push,
                            other => {
                                warn!(action = %other, "set_session_permission: unknown action");
                                return;
                            }
                        };
                        let result = if granted {
                            core.bridge
                                .grant_session_permission(
                                    &session_id,
                                    parsed,
                                    crate::policy::GrantScope::AllBranches,
                                )
                                .await
                        } else {
                            core.bridge
                                .revoke_session_permission(&session_id, parsed)
                                .await
                        };
                        if let Err(e) = result {
                            warn!(?e, "set_session_permission failed");
                            return;
                        }
                        refresh_session_permissions(&weak, &core, &session_id).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_save_agent_config({
            let weak_for_safe = weak.clone();
            move |row| {
                ffi_safe("on_save_agent_config", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    let cfg = DbAgentConfig {
                        agent_name: row.agent_name.to_string(),
                        provider: row.provider.to_string(),
                        model_name: row.model_name.to_string(),
                        base_url: optional(row.base_url.to_string()),
                        auth_token: optional(row.auth_token.to_string()),
                        updated_at: String::new(),
                    };
                    rt.spawn(async move {
                        match core.storage.upsert_agent_config(&cfg).await {
                            Err(e) => warn!(?e, "upsert agent_config"),
                            Ok(()) => {
                                // Session agents (brian/rain) bake env vars at spawn, so a
                                // config edit only affects the next-spawned session. Tell
                                // the user so silent stale-config drift doesn't happen.
                                // Emma has her own Restart button so we skip the toast.
                                if cfg.agent_name == "brian" || cfg.agent_name == "rain" {
                                    show_toast(&weak, "Changes will apply to new sessions.");
                                }
                            }
                        }
                        refresh_agent_configs(&weak, &core).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_restart_emma({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_restart_emma", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        match core.restart_emma().await {
                            Ok(()) => {
                                show_toast(&weak, "Emma restarted with the saved model.");
                            }
                            Err(e) => {
                                warn!(?e, "restart_emma");
                                show_toast(&weak, "Restart failed — check logs.");
                            }
                        }
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_open_file({
            let weak_for_safe = weak.clone();
            move |relative| {
                ffi_safe("on_cl_open_file", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    let rel = relative.to_string();
                    rt.spawn(async move {
                        let path = core.paths.data_dir.join(&rel);
                        let body = std::fs::read_to_string(&path).unwrap_or_default();
                        // Look up index metadata for this path so the description + tags
                        // strip above the editor reflects it.
                        let (project, file_path) = resolve_project_and_path(&rel);
                        let (desc, tags) = match core.storage.get_cl_index(&project, &file_path).await {
                            Ok(Some(entry)) => (entry.description, entry.tags.unwrap_or_default()),
                            _ => (String::new(), String::new()),
                        };
                        update_cl_current(&weak, &rel, &body);
                        update_cl_metadata(&weak, &desc, &tags);
                        clear_cl_dirty(&weak);
                        clear_cl_metadata_dirty(&weak);
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_save_current({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_save_current", &weak_for_safe, AssertUnwindSafe(|| {
                    // Read state on the event-loop thread (we're in a Slint callback)
                    // before spawning. Off-thread reads return empty silently.
                    let (rel, body) = current_cl_state(&weak);
                    let (desc, tags) = current_cl_metadata(&weak);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        if rel.is_empty() {
                            return;
                        }
                        let path = core.paths.data_dir.join(&rel);
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::write(&path, body) {
                            warn!(?e, "CL save body");
                            return;
                        }
                        // Persist metadata as part of the same flush. Description
                        // empty → keep whatever the index had (don't overwrite with
                        // emptiness; users may have content saved without metadata).
                        if !desc.trim().is_empty() {
                            let (project, file_path) = resolve_project_and_path(&rel);
                            // Ensure parent project row exists. Auto-create with a
                            // bare-bones display name if missing — projects are
                            // user-registered but file-system-driven sessions can
                            // populate ahead of registration.
                            let _ = core
                                .storage
                                .upsert_project(&project, &project, None, None, None)
                                .await;
                            let tag_opt = if tags.trim().is_empty() {
                                None
                            } else {
                                Some(tags.trim())
                            };
                            if let Err(e) = core
                                .storage
                                .upsert_cl_index(&project, &file_path, desc.trim(), tag_opt)
                                .await
                            {
                                warn!(?e, "CL save metadata");
                            }
                        }
                        clear_cl_dirty(&weak);
                        clear_cl_metadata_dirty(&weak);
                        refresh_cl_tree(&weak, &core).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_create_file({
            let weak_for_safe = weak.clone();
            move |name, description| {
                ffi_safe("on_cl_create_file", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    let name = name.to_string();
                    let description = description.to_string();
                    rt.spawn(async move {
                        let name = name.trim().trim_start_matches('/');
                        if name.is_empty() {
                            show_toast(&weak, "File path required.");
                            return;
                        }
                        let path = core.paths.data_dir.join(name);
                        if path.exists() {
                            show_toast(&weak, "File already exists.");
                            return;
                        }
                        if let Some(parent) = path.parent() {
                            if let Err(e) = std::fs::create_dir_all(parent) {
                                warn!(?e, "create_dir_all");
                                show_toast(&weak, "Could not create parent directory.");
                                return;
                            }
                        }
                        // Description defaults to the filename stem when blank — speed
                        // is the win, the user refines via the metadata strip later.
                        let stem = std::path::Path::new(name)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(name)
                            .to_string();
                        let final_desc = if description.trim().is_empty() {
                            stem.clone()
                        } else {
                            description.trim().to_string()
                        };
                        let initial = format!("# {}\n\n", final_desc);
                        if let Err(e) = std::fs::write(&path, &initial) {
                            warn!(?e, "CL create");
                            show_toast(&weak, "Could not write file.");
                            return;
                        }
                        let (project, file_path) = resolve_project_and_path(name);
                        let _ = core
                            .storage
                            .upsert_project(&project, &project, None, None, None)
                            .await;
                        if let Err(e) = core
                            .storage
                            .upsert_cl_index(&project, &file_path, &final_desc, None)
                            .await
                        {
                            warn!(?e, "CL create index");
                        }
                        refresh_cl_tree(&weak, &core).await;
                        update_cl_current(&weak, name, &initial);
                        update_cl_metadata(&weak, &final_desc, "");
                        clear_cl_dirty(&weak);
                        clear_cl_metadata_dirty(&weak);
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_delete_file({
            let weak_for_safe = weak.clone();
            move |relative| {
                ffi_safe("on_cl_delete_file", &weak_for_safe, AssertUnwindSafe(|| {
                    let rel = relative.to_string();
                    // Capture current-path here on the event-loop thread so the post-
                    // delete check correctly detects whether the deleted file was open.
                    let currently_open = current_cl_state(&weak).0;
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        let path = core.paths.data_dir.join(&rel);
                        if let Err(e) = std::fs::remove_file(&path) {
                            warn!(?e, "CL delete");
                            show_toast(&weak, "Could not delete file.");
                            return;
                        }
                        let (project, file_path) = resolve_project_and_path(&rel);
                        let _ = core.storage.delete_cl_index(&project, &file_path).await;
                        refresh_cl_tree(&weak, &core).await;
                        let was_open = currently_open == rel;
                        if was_open {
                            update_cl_current(&weak, "", "");
                            update_cl_metadata(&weak, "", "");
                            clear_cl_dirty(&weak);
                            clear_cl_metadata_dirty(&weak);
                        }
                        show_toast(&weak, "File deleted.");
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_rescan({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_rescan", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        // Rescan every known project + the globals bucket.
                        let projects = core.storage.list_projects().await.unwrap_or_default();
                        let mut added = 0usize;
                        let mut touched = 0usize;
                        let mut orphaned = 0usize;
                        for p in projects {
                            if let Ok(report) = core.bridge.cl_rescan(&p.name).await {
                                added += report.added.len();
                                touched += report.touched.len();
                                orphaned += report.orphaned.len();
                            }
                        }
                        refresh_cl_tree(&weak, &core).await;
                        show_toast(
                            &weak,
                            &format!(
                                "Rescan: +{added} new · {touched} touched · {orphaned} orphan(s)"
                            ),
                        );
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_autodescribe({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_autodescribe", &weak_for_safe, AssertUnwindSafe(|| {
                    // Slint property reads only return the live value on the event-
                    // loop thread. The callback fires here ON that thread, so capture
                    // (rel, body) NOW and move them into the tokio task — otherwise
                    // current_cl_state inside rt.spawn returns empty and the user
                    // sees "Open a file first" with a file plainly open.
                    let (rel, body) = current_cl_state(&weak);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        if rel.is_empty() {
                            show_toast(&weak, "Open a file first.");
                            return;
                        }
                        if body.trim().is_empty() {
                            show_toast(&weak, "File is empty — nothing to describe.");
                            return;
                        }
                        // Use Emma as the describer. Send a focused prompt; poll her
                        // chat for the first new text reply. Pollutes Emma's chat
                        // with the request/response which is the v1 trade-off.
                        let snippet: String = body.chars().take(4000).collect();
                        // Sharpened brief: lead with the content itself so Emma's
                        // first read is the file (not a meta instruction), forbid
                        // "I need to read the file" explicitly (her usual reflex),
                        // give a fallback for stub files so empty/placeholder
                        // files still get a useful one-liner.
                        let brief = format!(
                            "Below between the ===FILE-START=== / ===FILE-END=== markers is the ENTIRE content of a CL (context library) file. The content is already in this message — you do NOT need to call any tool or ask me to share the file. Reply with exactly ONE plain sentence summarizing what the file is about. No preface, no quotes, no \"let me\" or \"I'll\" — just the sentence.\n\n\
                             The sentence will be saved into a searchable index that other agents read to decide whether the file is relevant to their task; specific is better than generic.\n\n\
                             If the file is empty or only contains a heading stub (e.g. just `# Title` with nothing else), describe it as a placeholder for what the heading suggests — for example, an empty file titled `# Auth notes` becomes \"Placeholder for auth-related notes (no body yet).\"\n\n\
                             File path: {rel}\n\n\
                             ===FILE-START===\n{snippet}\n===FILE-END===\n\n\
                             One sentence. Plain prose. Reply now."
                        );
                        let weak_apply = weak.clone();
                        poll_emma_description(&core, &weak, "autodescribe", &brief, move |first_line| {
                            update_cl_metadata(&weak_apply, &first_line, "");
                            // The CustomInput edited callback won't fire from
                            // programmatic property changes, so flag dirty here.
                            let _ = weak_apply.upgrade_in_event_loop(move |handle| {
                                handle.global::<SlintAppState>().set_cl_metadata_dirty(true);
                            });
                        })
                        .await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_open_folder({
            let weak_for_safe = weak.clone();
            move |folder| {
                ffi_safe("on_cl_open_folder", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    let rel = folder.to_string();
                    rt.spawn(async move {
                        let folder_abs = core.paths.data_dir.join(&rel);
                        let (owner, folder_path) =
                            resolve_folder_owner(&core.storage, &core.paths.data_dir, &rel).await;
                        let description = core
                            .storage
                            .get_folder(&owner, &folder_path)
                            .await
                            .ok()
                            .flatten()
                            .map(|f| f.description)
                            .unwrap_or_default();
                        // Folder is a registered project when its absolute path
                        // matches some project's `cl_path` (or the default
                        // convention). Pull the project row so we can show
                        // working_repo + the canonical name.
                        let mut is_project = false;
                        let mut project_name = String::new();
                        let mut working_repo = String::new();
                        if let Ok(projects) = core.storage.list_projects().await {
                            for proj in projects {
                                if proj.name == crate::storage::Project::GLOBALS {
                                    continue;
                                }
                                // Match the walker's stricter definition: only
                                // user-bound projects (cl_path or working_repo set)
                                // count as registered. Auto-scanned subdirs do not.
                                let configured = proj
                                    .cl_path
                                    .as_deref()
                                    .is_some_and(|s| !s.is_empty())
                                    || proj
                                        .working_repo_path
                                        .as_deref()
                                        .is_some_and(|s| !s.is_empty());
                                if !configured {
                                    continue;
                                }
                                let abs = match proj.cl_path.as_deref() {
                                    Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
                                    _ => core.paths.data_dir.join("projects").join(&proj.name),
                                };
                                if abs == folder_abs {
                                    is_project = true;
                                    project_name = proj.name.clone();
                                    working_repo = proj.working_repo_path.unwrap_or_default();
                                    break;
                                }
                            }
                        }
                        update_cl_folder(
                            &weak,
                            &rel,
                            &description,
                            is_project,
                            &project_name,
                            &working_repo,
                        );
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_save_folder_description({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_save_folder_description", &weak_for_safe, AssertUnwindSafe(|| {
                    let (folder, description, _is_project, _project_name) =
                        current_cl_folder_state(&weak);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        if folder.is_empty() {
                            return;
                        }
                        let (owner, folder_path) =
                            resolve_folder_owner(&core.storage, &core.paths.data_dir, &folder).await;
                        // Ensure the owning project row exists. _globals is bootstrapped
                        // by the migration; named projects without a row get an empty
                        // shell so the FK doesn't break.
                        if owner != crate::storage::Project::GLOBALS {
                            let _ = core
                                .storage
                                .upsert_project(&owner, &owner, None, None, None)
                                .await;
                        }
                        if let Err(e) = core
                            .storage
                            .upsert_folder_description(&owner, &folder_path, description.trim(), None)
                            .await
                        {
                            warn!(?e, "save folder description");
                            show_toast(&weak, "Could not save folder description.");
                            return;
                        }
                        // Clear dirty + refresh tree so the row's `description` field
                        // reflects the latest write.
                        let _ = weak.upgrade_in_event_loop(move |handle| {
                            handle
                                .global::<SlintAppState>()
                                .set_cl_current_folder_dirty(false);
                        });
                        refresh_cl_tree(&weak, &core).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_register_project_open({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_register_project_open", &weak_for_safe, AssertUnwindSafe(|| {
                    let (folder, _description, _is_project, _project_name) =
                        current_cl_folder_state(&weak);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        if folder.is_empty() {
                            return;
                        }
                        let folder_abs = core.paths.data_dir.join(&folder);
                        let default_name = folder_abs
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&folder)
                            .to_string();
                        let abs = folder_abs.display().to_string();
                        open_register_dialog(&weak, &default_name, &abs, &abs);
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_register_project_submit({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_register_project_submit", &weak_for_safe, AssertUnwindSafe(|| {
                    let (name, cl_path, working_repo) = current_register_dialog(&weak);
                    let folder = weak
                        .upgrade()
                        .map(|h| h.global::<SlintAppState>().get_cl_current_folder().to_string())
                        .unwrap_or_default();
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        let name = name.trim().to_string();
                        let cl_path = cl_path.trim().to_string();
                        let working = working_repo.trim().to_string();
                        if name.is_empty() || cl_path.is_empty() {
                            show_toast(&weak, "Name and CL path are required.");
                            return;
                        }
                        let working_opt = if working.is_empty() {
                            None
                        } else {
                            Some(working.as_str())
                        };
                        if let Err(e) = core
                            .storage
                            .upsert_project(&name, &name, working_opt, None, Some(&cl_path))
                            .await
                        {
                            warn!(?e, "register project");
                            show_toast(&weak, "Could not register project.");
                            return;
                        }
                        // Index every file under the new cl_path into cl_index so
                        // agents find them via cl_index_search immediately. Without
                        // this the project's files only appear in the index on the
                        // next bot-hq restart (when startup_init's rescan loop runs)
                        // or on a manual Refresh — surprising for "I just registered
                        // this folder, why doesn't search see anything in it?".
                        if let Err(e) = core.bridge.cl_rescan(&name).await {
                            warn!(?e, project = %name, "cl_rescan after register failed");
                        }
                        close_register_dialog(&weak);
                        refresh_new_session_projects(&weak, &core).await;
                        refresh_cl_tree(&weak, &core).await;
                        // Re-open the folder view so the panel reflects registered
                        // state without a manual click. invoke_cl_open_folder MUST
                        // be dispatched on the Slint event loop — calling it from
                        // this tokio task silently no-ops.
                        if !folder.is_empty() {
                            let folder_clone = folder.clone();
                            let _ = weak.upgrade_in_event_loop(move |handle| {
                                handle.global::<SlintAppState>().invoke_cl_open_folder(
                                    SharedString::from(folder_clone),
                                );
                            });
                        }
                        show_toast(&weak, "Project registered.");
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_unregister_project({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_unregister_project", &weak_for_safe, AssertUnwindSafe(|| {
                    let (folder, _description, _is_project, project_name) =
                        current_cl_folder_state(&weak);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        if project_name.is_empty() {
                            return;
                        }
                        if let Err(e) = core.storage.unregister_project(&project_name).await {
                            warn!(?e, "unregister project");
                            show_toast(&weak, "Could not unregister project.");
                            return;
                        }
                        refresh_new_session_projects(&weak, &core).await;
                        refresh_cl_tree(&weak, &core).await;
                        if !folder.is_empty() {
                            let folder_clone = folder.clone();
                            let _ = weak.upgrade_in_event_loop(move |handle| {
                                handle.global::<SlintAppState>().invoke_cl_open_folder(
                                    SharedString::from(folder_clone),
                                );
                            });
                        }
                        show_toast(&weak, "Project unregistered.");
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_autodescribe_folder({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_autodescribe_folder", &weak_for_safe, AssertUnwindSafe(|| {
                    let (folder, _description, _is_project, _project_name) =
                        current_cl_folder_state(&weak);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        if folder.is_empty() {
                            show_toast(&weak, "Open a folder first.");
                            return;
                        }
                        let folder_abs = core.paths.data_dir.join(&folder);
                        // Build a snapshot of the folder's immediate contents +
                        // first ~200 chars of each .md inside. Emma reads this to
                        // draft a one-sentence description.
                        let mut listing = String::new();
                        let mut snippets = String::new();
                        if let Ok(entries) = std::fs::read_dir(&folder_abs) {
                            let mut paths: Vec<_> = entries.flatten().collect();
                            paths.sort_by_key(|a| a.file_name());
                            for e in paths.iter().take(40) {
                                let p = e.path();
                                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                if name.starts_with('.') {
                                    continue;
                                }
                                if p.is_dir() {
                                    listing.push_str(&format!("- {name}/\n"));
                                } else {
                                    listing.push_str(&format!("- {name}\n"));
                                    if name.ends_with(".md") {
                                        if let Ok(body) = std::fs::read_to_string(&p) {
                                            let head: String = body.chars().take(200).collect();
                                            snippets.push_str(&format!(
                                                "--- {name} ---\n{head}\n\n"
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        let brief = format!(
                            "Below between the ===FOLDER-START=== / ===FOLDER-END=== markers is a snapshot of a CL (context library) folder's immediate contents. Reply with exactly ONE plain sentence describing what the folder is for. No preface, no quotes, no \"let me\" or \"I'll\" — just the sentence.\n\n\
                             The sentence will be saved into a searchable index that other agents read to decide whether the folder is relevant; specific is better than generic.\n\n\
                             Folder path: {folder}\n\n\
                             ===FOLDER-START===\n\
                             Listing:\n{listing}\n\
                             Snippets:\n{snippets}\
                             ===FOLDER-END===\n\n\
                             One sentence. Plain prose. Reply now."
                        );
                        let weak_apply = weak.clone();
                        poll_emma_description(&core, &weak, "autodescribe-folder", &brief, move |first_line| {
                            let _ = weak_apply.upgrade_in_event_loop(move |handle| {
                                let app = handle.global::<SlintAppState>();
                                app.set_cl_current_folder_description(SharedString::from(first_line));
                                app.set_cl_current_folder_dirty(true);
                            });
                        })
                        .await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        app.on_toggle_emma({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_toggle_emma", &weak_for_safe, AssertUnwindSafe(|| {
                    if let Some(handle) = weak.upgrade() {
                        let app = handle.global::<SlintAppState>();
                        app.set_emma_open(!app.get_emma_open());
                    }
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_refresh({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_refresh", &weak_for_safe, AssertUnwindSafe(|| {
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        refresh_cl_tree(&weak, &core).await;
                    });
                }));
            }
        });
    }

    // ---- VS Code-style inline tree CRUD --------------------------------

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_tree_toggle_collapse({
            let weak_for_safe = weak.clone();
            move |folder_path| {
                ffi_safe("on_cl_tree_toggle_collapse", &weak_for_safe, AssertUnwindSafe(|| {
                    let folder_path = folder_path.to_string();
                    // Read+mutate expanded set on the event-loop thread. Folders
                    // default to COLLAPSED; presence in the set means user-expanded.
                    if let Some(handle) = weak.upgrade() {
                        let app = handle.global::<SlintAppState>();
                        let cur = app.get_cl_tree_expanded();
                        let mut set: Vec<SharedString> = Vec::with_capacity(cur.row_count() + 1);
                        let mut was_expanded = false;
                        for i in 0..cur.row_count() {
                            if let Some(p) = cur.row_data(i) {
                                if p == folder_path {
                                    was_expanded = true; // dropping it = collapse
                                } else {
                                    set.push(p);
                                }
                            }
                        }
                        if !was_expanded {
                            set.push(SharedString::from(folder_path));
                        }
                        app.set_cl_tree_expanded(ModelRc::new(VecModel::from(set)));
                    }
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        refresh_cl_tree(&weak, &core).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_tree_begin_create_file({
            let weak_for_safe = weak.clone();
            move |parent_dir| {
                ffi_safe("on_cl_tree_begin_create_file", &weak_for_safe, AssertUnwindSafe(|| {
                    let parent_dir = parent_dir.to_string();
                    set_editing_state(&weak, "new-file", &parent_dir, "");
                    // The walker injects a ghost row at the create site — refresh
                    // the tree so the user sees the inline input immediately.
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        refresh_cl_tree(&weak, &core).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_tree_begin_create_folder({
            let weak_for_safe = weak.clone();
            move |parent_dir| {
                ffi_safe("on_cl_tree_begin_create_folder", &weak_for_safe, AssertUnwindSafe(|| {
                    let parent_dir = parent_dir.to_string();
                    set_editing_state(&weak, "new-folder", &parent_dir, "");
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        refresh_cl_tree(&weak, &core).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_tree_begin_rename({
            let weak_for_safe = weak.clone();
            move |path| {
                ffi_safe("on_cl_tree_begin_rename", &weak_for_safe, AssertUnwindSafe(|| {
                    let path = path.to_string();
                    let name = std::path::Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    set_editing_state(&weak, "rename", &path, &name);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        refresh_cl_tree(&weak, &core).await;
                    });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        app.on_cl_tree_cancel_edit({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_tree_cancel_edit", &weak_for_safe, AssertUnwindSafe(|| {
                    clear_editing_state(&weak);
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_tree_commit_edit({
            let weak_for_safe = weak.clone();
            move || {
                ffi_safe("on_cl_tree_commit_edit", &weak_for_safe, AssertUnwindSafe(|| {
                    // Read editing state on the event-loop thread.
                    let (mode, target_path, name) = match weak.upgrade() {
                        Some(handle) => {
                            let app = handle.global::<SlintAppState>();
                            (
                                app.get_cl_tree_editing_mode().to_string(),
                                app.get_cl_tree_editing_path().to_string(),
                                app.get_cl_tree_editing_name().to_string(),
                            )
                        }
                        None => return,
                    };
                    let name = name.trim().trim_start_matches('/').to_string();
                    if name.is_empty() {
                        clear_editing_state(&weak);
                        return;
                    }
                    clear_editing_state(&weak);
                    let weak = weak.clone();
                    let core = Arc::clone(&core);
                    rt.spawn(async move {
                        match mode.as_str() {
                            "new-file" => {
                        // For files in `_globals` we don't auto-append .md;
                        // user can type extension. For convenience, if no
                        // extension and no slash, append .md.
                        let final_name = if !name.contains('.') && !name.contains('/') {
                            format!("{}.md", name)
                        } else {
                            name.clone()
                        };
                        let rel = if target_path.is_empty() {
                            final_name.clone()
                        } else {
                            format!("{}/{}", target_path, final_name)
                        };
                        let path = core.paths.data_dir.join(&rel);
                        if path.exists() {
                            show_toast(&weak, "File already exists.");
                            return;
                        }
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        // Seed with an H1 derived from the filename stem so
                        // rescan's auto-extracted description matches.
                        let stem = std::path::Path::new(&final_name)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&final_name)
                            .to_string();
                        if let Err(e) = std::fs::write(&path, format!("# {}\n\n", stem)) {
                            warn!(?e, "inline create");
                            show_toast(&weak, "Could not create file.");
                            return;
                        }
                        // Auto-register index with description = stem (user
                        // refines via metadata strip later).
                        let (project, file_path) = resolve_project_and_path(&rel);
                        let _ = core
                            .storage
                            .upsert_project(&project, &project, None, None, None)
                            .await;
                        let _ = core
                            .storage
                            .upsert_cl_index(&project, &file_path, &stem, None)
                            .await;
                        refresh_cl_tree(&weak, &core).await;
                        // Open the freshly-created file.
                        update_cl_current(&weak, &rel, &format!("# {}\n\n", stem));
                        update_cl_metadata(&weak, &stem, "");
                        clear_cl_dirty(&weak);
                        clear_cl_metadata_dirty(&weak);
                    }
                    "new-folder" => {
                        let rel = if target_path.is_empty() {
                            name.clone()
                        } else {
                            format!("{}/{}", target_path, name)
                        };
                        let path = core.paths.data_dir.join(&rel);
                        if path.exists() {
                            show_toast(&weak, "Folder already exists.");
                            return;
                        }
                        if let Err(e) = std::fs::create_dir_all(&path) {
                            warn!(?e, "inline mkdir");
                            show_toast(&weak, "Could not create folder.");
                            return;
                        }
                        refresh_cl_tree(&weak, &core).await;
                    }
                    "rename" => {
                        let src = core.paths.data_dir.join(&target_path);
                        let parent = std::path::Path::new(&target_path)
                            .parent()
                            .and_then(|p| p.to_str())
                            .unwrap_or("");
                        let dst_rel = if parent.is_empty() {
                            name.clone()
                        } else {
                            format!("{}/{}", parent, name)
                        };
                        let dst = core.paths.data_dir.join(&dst_rel);
                        if dst.exists() && dst != src {
                            show_toast(&weak, "Target name already exists.");
                            return;
                        }
                        if let Err(e) = std::fs::rename(&src, &dst) {
                            warn!(?e, "rename");
                            show_toast(&weak, "Could not rename.");
                            return;
                        }
                        // If a file, update the index (keep same project).
                        if dst.is_file() {
                            let (project_old, file_old) =
                                resolve_project_and_path(&target_path);
                            let (project_new, file_new) =
                                resolve_project_and_path(&dst_rel);
                            // Read old description so we can re-insert under
                            // the new path (delete + insert; no rename op).
                            let desc = core
                                .storage
                                .get_cl_index(&project_old, &file_old)
                                .await
                                .ok()
                                .flatten()
                                .map(|e| (e.description, e.tags))
                                .unwrap_or((String::new(), None));
                            let _ = core
                                .storage
                                .delete_cl_index(&project_old, &file_old)
                                .await;
                            if !desc.0.is_empty() {
                                let _ = core
                                    .storage
                                    .upsert_cl_index(
                                        &project_new,
                                        &file_new,
                                        &desc.0,
                                        desc.1.as_deref(),
                                    )
                                    .await;
                            }
                            // If the renamed file was the open one, follow it.
                            let (open_path, _) = current_cl_state(&weak);
                            if open_path == target_path {
                                update_cl_current(
                                    &weak,
                                    &dst_rel,
                                    &std::fs::read_to_string(&dst).unwrap_or_default(),
                                );
                            }
                        }
                        // For folder renames, child rows update naturally
                        // on the next refresh — no index touch (file paths
                        // are relative-to-project; if the project name
                        // changed because we renamed projects/<x>/, that's
                        // a heavier op we don't try to handle inline.
                        refresh_cl_tree(&weak, &core).await;
                    }
                    _ => {}
                }
            });
                }));
            }
        });
    }

    {
        let weak = weak.clone();
        let core = Arc::clone(&core);
        let rt = rt.clone();
        app.on_cl_tree_delete_path({
            let weak_for_safe = weak.clone();
            move |path| {
                ffi_safe("on_cl_tree_delete_path", &weak_for_safe, AssertUnwindSafe(|| {
                    // Files use the existing delete-confirm dialog (cl-delete-confirm-path).
                    // Folders delete immediately (with a soft confirm via toast).
                    let path = path.to_string();
                    let full = core.paths.data_dir.join(&path);
                    if full.is_dir() {
                        let weak = weak.clone();
                        let core = Arc::clone(&core);
                        rt.spawn(async move {
                            if let Err(e) = std::fs::remove_dir_all(&full) {
                                warn!(?e, "delete folder");
                                show_toast(&weak, "Could not delete folder.");
                                return;
                            }
                            // Drop every index row whose path starts with the
                            // deleted folder. Use rescan to clean orphans; the
                            // bridge.cl_rescan handles per-project sweeps.
                            let projects = core.storage.list_projects().await.unwrap_or_default();
                            for p in projects {
                                let _ = core.bridge.cl_rescan(&p.name).await;
                            }
                            refresh_cl_tree(&weak, &core).await;
                            show_toast(&weak, "Folder deleted.");
                        });
                    } else {
                        // Trigger the existing single-file confirm dialog.
                        if let Some(handle) = weak.upgrade() {
                            handle
                                .global::<SlintAppState>()
                                .set_cl_delete_confirm_path(SharedString::from(path));
                        }
                    }
                }));
            }
        });
    }

    // ---- Signaling event pump ------------------------------------------
    let mut sub = core.subscribe_signaling();
    let weak_for_sub = weak.clone();
    let core_for_sub = Arc::clone(&core);
    rt.spawn(async move {
        loop {
            match sub.recv().await {
                Ok(ev) => handle_signaling_event(&weak_for_sub, &core_for_sub, ev).await,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // ---- Periodic refresh (cheap; re-pulls active session, emma, dashboard, CL) ----
    //
    // CL tree refresh runs at 1/4 cadence (every 2s) — the tree changes rarely
    // (user-edited files, occasional new project), and a full directory walk is
    // costlier than a sqlite query. Counter is plain int because the closure
    // owns it.
    let weak_for_poll = weak.clone();
    let core_for_poll = Arc::clone(&core);
    rt.spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        let mut tick = 0u64;
        loop {
            ticker.tick().await;
            tick = tick.wrapping_add(1);
            // current_session_id needs the event-loop thread; use the async hop.
            // Without this, the polling refresh would never fire because we'd
            // always see session_id="".
            let session_id = current_session_id_async(&weak_for_poll).await;
            if !session_id.is_empty() {
                let _ = refresh_session_view(&weak_for_poll, &core_for_poll, &session_id).await;
                // Also refresh the right-pane DocumentPane for the active
                // tab. Cheap (one indexed query on session_documents);
                // running every 500ms keeps phase-tagged docs surfaced as
                // agents write them without a manual click.
                let tab = current_selected_doc_tab_async(&weak_for_poll).await;
                refresh_session_docs(&weak_for_poll, &core_for_poll, &session_id, &tab).await;
            }
            let _ = refresh_emma(&weak_for_poll, &core_for_poll).await;
            refresh_dashboard(&weak_for_poll, &core_for_poll).await;
            // CL tree refresh every 2 seconds (4 × 500ms). Skip while
            // the user is inline-editing — rebuilding the cl-tree model
            // destroys the row instance (and its focused TextInput),
            // yanking focus out from under the user mid-type. Explicit
            // refreshes (after create / delete / rename) still happen
            // from the action handlers.
            if tick.is_multiple_of(4) {
                let editing = read_tree_state(&weak_for_poll).await.editing_mode;
                if editing.is_empty() {
                    refresh_cl_tree(&weak_for_poll, &core_for_poll).await;
                }
            }
        }
    });

    Ok(())
}

// ---- Helpers -----------------------------------------------------------

fn optional(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Render a tool_use JSON blob as a friendly one-liner.
/// Input shape: `{"tool_use_id":"...","name":"Bash","input":{...}}`.
fn format_tool_use(raw: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Null);
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
    let input = v.get("input").cloned().unwrap_or(serde_json::Value::Null);
    let snippet = tool_input_snippet(name, &input);
    if snippet.is_empty() {
        format!("🔧 {}", name)
    } else {
        format!("🔧 {} · {}", name, snippet)
    }
}

/// Render a tool_result JSON blob as a friendly one-liner.
/// Input shape: `{"tool_use_id":"...","content":"...","is_error":false}`.
fn format_tool_result(raw: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Null);
    let is_error = v.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false);
    let body = match v.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let icon = if is_error { "✗" } else { "✓" };
    let preview = truncate(&body, 240);
    if preview.is_empty() {
        format!("{} result", icon)
    } else {
        format!("{} {}", icon, preview)
    }
}

/// Pick a meaningful snippet from a tool's input. Per-tool heuristics where it
/// helps; generic fall-through otherwise.
fn tool_input_snippet(name: &str, input: &serde_json::Value) -> String {
    let direct_field = match name {
        "Bash" => input.get("command").and_then(|v| v.as_str()),
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            input.get("file_path").and_then(|v| v.as_str())
        }
        "Grep" | "Glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .or_else(|| input.get("path").and_then(|v| v.as_str())),
        "WebFetch" | "WebSearch" => input
            .get("url")
            .and_then(|v| v.as_str())
            .or_else(|| input.get("query").and_then(|v| v.as_str())),
        _ => None,
    };
    if let Some(s) = direct_field {
        return truncate(s, 120);
    }
    // Generic fallback: compact JSON, trimmed.
    truncate(&input.to_string(), 120)
}

fn truncate(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars).collect();
        out.push('…');
        out
    }
}

async fn refresh_dashboard(weak: &Weak<AppWindow>, core: &Arc<CoreAppState>) {
    let sessions = match core.list_active_sessions().await {
        Ok(s) => s,
        Err(e) => {
            warn!(?e, "list_active_sessions");
            return;
        }
    };
    let mut tiles: Vec<TileData> = Vec::with_capacity(sessions.len());
    for s in &sessions {
        if s.id == "emma" {
            continue; // emma handled separately in the half-pane.
        }
        let phase = core
            .current_phase(&s.id)
            .await
            .map(|p| p.chip().to_string())
            .unwrap_or_else(|| "".to_string());
        // Quickview: last non-empty message rendered as "author: snippet…".
        // Cheap query — storage already orders by (created_at, id).
        let quickview = core
            .storage
            .messages_for_session(&s.id, None)
            .await
            .ok()
            .and_then(|msgs| msgs.into_iter().rev().find(|m| !m.content.trim().is_empty()))
            .map(|m| {
                let body = match m.kind.as_str() {
                    "tool_use" => format_tool_use(&m.content),
                    "tool_result" => format_tool_result(&m.content),
                    _ => m.content,
                };
                let first_line: String = body
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(72)
                    .collect();
                let suffix = if body.lines().count() > 1 || body.chars().count() > 72 {
                    "…"
                } else {
                    ""
                };
                format!("{}: {}{}", m.author, first_line, suffix)
            })
            .unwrap_or_default();
        let pending_input_count = core
            .storage
            .pending_question_count(&s.id)
            .await
            .unwrap_or(0) as i32;
        tiles.push(TileData {
            id: s.id.clone(),
            title: s.title.clone(),
            phase,
            last_activity: s.created_at.clone(),
            awaiting: false,
            pending_input_count,
            quickview,
        });
    }
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        // Snapshot per-id pending/awaiting state from the EXISTING model
        // before we rebuild. SignalingEvent::PendingChoice and AwaitingUser
        // set these per-tile via update_tile_*; we don't want this 500ms
        // rebuild to wipe them — that was making the choice dialog flicker
        // away after a fraction of a second on the dashboard.
        let existing = app.get_sessions();
        let mut carry: std::collections::HashMap<String, SessionTile> =
            std::collections::HashMap::new();
        for i in 0..existing.row_count() {
            if let Some(tile) = existing.row_data(i) {
                carry.insert(tile.id.to_string(), tile);
            }
        }
        let rows: Vec<SessionTile> = tiles
            .into_iter()
            .map(|d| {
                let mut new_tile = tile_from_data(d);
                if let Some(prev) = carry.get(&new_tile.id.to_string()) {
                    // Awaiting state survives across rebuilds; pending_input_count
                    // is rebuilt fresh from storage on every tile build.
                    new_tile.awaiting = prev.awaiting;
                }
                new_tile
            })
            .collect();
        app.set_sessions(ModelRc::new(VecModel::from(rows)));
    });
}

async fn refresh_session_permissions(
    weak: &Weak<AppWindow>,
    core: &Arc<CoreAppState>,
    session_id: &str,
) {
    let perms = core.bridge.list_session_permissions(session_id).await;
    let commit_granted = !matches!(perms.commit, crate::policy::GrantScope::None);
    let push_granted = !matches!(perms.push, crate::policy::GrantScope::None);
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_active_commit_granted(commit_granted);
        app.set_active_push_granted(push_granted);
    });
}

async fn refresh_new_session_projects(weak: &Weak<AppWindow>, core: &Arc<CoreAppState>) {
    let projects = match core.storage.list_projects().await {
        Ok(p) => p,
        Err(e) => {
            warn!(?e, "list_projects");
            return;
        }
    };
    // Only surface projects that actually have a working_repo_path. _globals
    // is synthetic (no repo) and unset rows can't drive a useful session.
    let entries: Vec<(String, String)> = projects
        .into_iter()
        .filter_map(|p| {
            p.working_repo_path
                .filter(|s| !s.trim().is_empty())
                .map(|path| (p.name, path))
        })
        .collect();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let rows: Vec<NewSessionProject> = entries
            .into_iter()
            .map(|(name, path)| NewSessionProject {
                name: name.into(),
                path: path.into(),
            })
            .collect();
        handle
            .global::<SlintAppState>()
            .set_new_session_projects(ModelRc::new(VecModel::from(rows)));
    });
}

async fn refresh_agent_configs(weak: &Weak<AppWindow>, core: &Arc<CoreAppState>) {
    let cfgs = match core.storage.list_agent_configs().await {
        Ok(v) => v,
        Err(e) => {
            warn!(?e, "list_agent_configs");
            return;
        }
    };
    let data: Vec<AgentConfigData> = cfgs
        .into_iter()
        .map(|c| AgentConfigData {
            agent_name: c.agent_name,
            provider: c.provider,
            model_name: c.model_name,
            base_url: c.base_url.unwrap_or_default(),
            auth_token: c.auth_token.unwrap_or_default(),
        })
        .collect();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let rows: Vec<AgentConfigRow> = data.into_iter().map(agent_cfg_from_data).collect();
        handle
            .global::<SlintAppState>()
            .set_agent_configs(ModelRc::new(VecModel::from(rows)));
    });
}

async fn refresh_cl_tree(weak: &Weak<AppWindow>, core: &Arc<CoreAppState>) {
    // Snapshot collapsed-folder set + inline-edit state from Slint on the
    // event-loop thread so the walker can honor them. read_tree_state itself
    // requires the event loop to be RUNNING (it hops in via
    // upgrade_in_event_loop + oneshot). For callers that may be invoked
    // before window.run() — i.e., the initial install_view_model pull —
    // use refresh_cl_tree_with_state(TreeState::default()) instead, or you
    // will deadlock waiting for a hop that can never complete.
    let snapshot = read_tree_state(weak).await;
    refresh_cl_tree_with_state(weak, core, &snapshot).await;
}

/// Worker variant: skips the event-loop hop for state. Callers that already
/// have a TreeState (or know it's the default — e.g., startup, before the
/// event loop runs) should call this directly to avoid the deadlock.
async fn refresh_cl_tree_with_state(
    weak: &Weak<AppWindow>,
    core: &Arc<CoreAppState>,
    state: &TreeState,
) {
    let root = core.paths.data_dir.clone();
    let lookups = load_cl_lookups(core, &root).await;
    let entries = walk_cl(&root, state, &lookups);
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let rows: Vec<CLFileEntry> = entries.into_iter().map(cl_entry_from_data).collect();
        handle
            .global::<SlintAppState>()
            .set_cl_tree(ModelRc::new(VecModel::from(rows)));
    });
}

/// Snapshot the storage facts the tree walker needs: which absolute paths
/// are registered-project CL roots, and which (project, folder_path) pairs
/// have a stored description. Loaded once per refresh so the walker stays
/// sync + cheap.
#[derive(Default)]
struct ClTreeLookups {
    /// abs_path → project name. Includes both `cl_path` overrides and
    /// default-convention `<data_dir>/projects/<name>/` paths. Excludes
    /// `_globals` (it's the data_dir itself, never a "registered folder").
    project_roots: HashMap<std::path::PathBuf, String>,
    /// (project, folder_path-relative-to-cl-root) → description.
    folder_descs: HashMap<(String, String), String>,
}

async fn load_cl_lookups(core: &Arc<CoreAppState>, data_dir: &std::path::Path) -> ClTreeLookups {
    let mut out = ClTreeLookups::default();
    let projects = match core.storage.list_projects().await {
        Ok(p) => p,
        Err(e) => {
            warn!(?e, "list_projects failed during CL tree refresh");
            return out;
        }
    };
    for proj in &projects {
        if proj.name == crate::storage::Project::GLOBALS {
            continue;
        }
        // "Registered" means the user actively bound this project via the
        // UI (cl_path or working_repo_path set). Auto-scanned subdirs from
        // startup_init that the user never touched have both NULL and are
        // NOT considered registered — Unregister clears both fields so the
        // visual reverts cleanly without a hard DELETE.
        let configured = proj
            .cl_path
            .as_deref()
            .is_some_and(|s| !s.is_empty())
            || proj
                .working_repo_path
                .as_deref()
                .is_some_and(|s| !s.is_empty());
        if !configured {
            continue;
        }
        let abs = match proj.cl_path.as_deref() {
            Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
            _ => data_dir.join("projects").join(&proj.name),
        };
        out.project_roots.insert(abs, proj.name.clone());
    }
    let folders = match core.storage.cl_folder_search(None, None).await {
        Ok(f) => f,
        Err(e) => {
            warn!(?e, "cl_folder_search failed during CL tree refresh");
            return out;
        }
    };
    for f in folders {
        out.folder_descs
            .insert((f.project_id, f.folder_path), f.description);
    }
    out
}

#[derive(Default, Clone)]
struct TreeState {
    /// Folder relative-paths the user has EXPLICITLY EXPANDED this session.
    /// Folders default to collapsed at startup; presence here means "the
    /// user clicked to open this folder during the current session". The
    /// property persists across tab switches but resets on app restart.
    expanded: std::collections::HashSet<String>,
    /// Current inline-edit mode + target. Drives ghost-row injection.
    editing_mode: String,
    editing_path: String,
}

async fn read_tree_state(weak: &Weak<AppWindow>) -> TreeState {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let mut state = TreeState::default();
        let app = handle.global::<SlintAppState>();
        let expanded = app.get_cl_tree_expanded();
        for i in 0..expanded.row_count() {
            if let Some(p) = expanded.row_data(i) {
                state.expanded.insert(p.to_string());
            }
        }
        state.editing_mode = app.get_cl_tree_editing_mode().to_string();
        state.editing_path = app.get_cl_tree_editing_path().to_string();
        let _ = tx.send(state);
    });
    rx.await.unwrap_or_default()
}

/// Synthetic relative-path used for the inline new-file/new-folder placeholder
/// row. Composed as `<parent>/__ghost__` (or just `__ghost__` at root). The
/// Slint side checks `is_ghost: true` to swap to a TextInput.
fn ghost_relative_path(parent: &str) -> String {
    if parent.is_empty() {
        "__ghost__".to_string()
    } else {
        format!("{}/__ghost__", parent)
    }
}

fn walk_cl(
    root: &std::path::Path,
    state: &TreeState,
    lookups: &ClTreeLookups,
) -> Vec<CLEntryData> {
    fn recurse(
        out: &mut Vec<CLEntryData>,
        dir: &std::path::Path,
        root: &std::path::Path,
        depth: i32,
        state: &TreeState,
        lookups: &ClTreeLookups,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut sorted: Vec<_> = entries.flatten().collect();
        // Directories first, then files; alphabetical within each group.
        sorted.sort_by(|a, b| {
            let ad = a.path().is_dir();
            let bd = b.path().is_dir();
            match (ad, bd) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.file_name().cmp(&b.file_name()),
            }
        });
        for e in sorted {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue;
            }
            let rel = p.strip_prefix(root).unwrap_or(&p).display().to_string();
            let is_dir = p.is_dir();
            // Folders default to collapsed; only show children if the user
            // has explicitly expanded this folder during the session.
            let expanded = is_dir && state.expanded.contains(&rel);
            let (is_registered_project, description) = if is_dir {
                resolve_folder_metadata(&p, root, lookups)
            } else {
                (false, String::new())
            };
            out.push(CLEntryData {
                relative_path: rel.clone(),
                display_name: name.to_string(),
                is_dir,
                depth,
                expanded,
                is_ghost: false,
                is_registered_project,
                description,
            });
            // Inject ghost row for inline create directly under this folder
            // when the user is creating something there. Ghost row sits at
            // depth+1, right after the folder header, BEFORE the recurse so
            // it appears at the top of the listing (matches VS Code).
            if is_dir
                && matches!(state.editing_mode.as_str(), "new-file" | "new-folder")
                && state.editing_path == rel
            {
                out.push(CLEntryData {
                    relative_path: ghost_relative_path(&rel),
                    display_name: String::new(),
                    is_dir: state.editing_mode == "new-folder",
                    depth: depth + 1,
                    expanded: false,
                    is_ghost: true,
                    is_registered_project: false,
                    description: String::new(),
                });
            }
            if is_dir && expanded {
                recurse(out, &p, root, depth + 1, state, lookups);
            }
        }
    }
    let mut out = Vec::new();
    // Root-level ghost row (when create-here was triggered at the bot-hq root
    // — empty editing_path means "create at root").
    if matches!(state.editing_mode.as_str(), "new-file" | "new-folder")
        && state.editing_path.is_empty()
    {
        out.push(CLEntryData {
            relative_path: ghost_relative_path(""),
            display_name: String::new(),
            is_dir: state.editing_mode == "new-folder",
            depth: 0,
            expanded: false,
            is_ghost: true,
            is_registered_project: false,
            description: String::new(),
        });
    }
    recurse(&mut out, root, root, 0, state, lookups);
    out
}

/// For a folder at `abs_path`, decide:
/// - whether it's a registered project's CL root (its absolute path matches
///   some `projects.cl_path` or default convention)
/// - the stored description (from `cl_folders`) — looked up against the
///   owning project + relative folder path. Folders outside any registered
///   project map to `_globals` (the data-dir itself) so descriptions for
///   shared CL slots (`agents/`, etc.) still surface.
fn resolve_folder_metadata(
    abs_path: &std::path::Path,
    root: &std::path::Path,
    lookups: &ClTreeLookups,
) -> (bool, String) {
    let is_registered = lookups.project_roots.contains_key(abs_path);
    let (project_id, folder_path) = if let Some(pid) = lookups.project_roots.get(abs_path) {
        (pid.clone(), String::new())
    } else if let Some((pid, owning_root)) = lookups
        .project_roots
        .iter()
        .filter(|(pr, _)| abs_path.starts_with(pr))
        .max_by_key(|(pr, _)| pr.as_os_str().len())
        .map(|(pr, pid)| (pid.clone(), pr.clone()))
    {
        let rel = abs_path
            .strip_prefix(&owning_root)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        (pid, rel)
    } else {
        // Falls under the bot-hq root with no enclosing registered project
        // → attribute to _globals so agents can describe shared slots.
        let rel = abs_path
            .strip_prefix(root)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        (
            crate::storage::Project::GLOBALS.to_string(),
            rel,
        )
    };
    let description = lookups
        .folder_descs
        .get(&(project_id, folder_path))
        .cloned()
        .unwrap_or_default();
    (is_registered, description)
}

async fn refresh_session_view(
    weak: &Weak<AppWindow>,
    core: &Arc<CoreAppState>,
    session_id: &str,
) -> anyhow::Result<()> {
    let msgs = core
        .storage
        .messages_for_session(session_id, None)
        .await?;
    // Short-circuit on no-change: avoid rebuilding the Slint model when nothing
    // arrived since the last poll. Prevents the 500ms re-render from tearing
    // down TextInput rows mid-selection (highlighting was being cleared).
    //
    // BUT: also force-reload on session switch. The fingerprint cache is keyed
    // by session_id, so switching A→B→A would skip set_session_msgs when A's
    // fingerprint hasn't changed — leaving B's chat visible in A's view.
    let fp = fingerprint(&msgs);
    let switched = displayed_session_changed(session_id);
    let content_changed = switched || fingerprint_changed(session_id, fp);
    let session_row = core.storage.get_session(session_id).await?;
    let title = session_row
        .as_ref()
        .map(|s| s.title.clone())
        .unwrap_or_default();
    let brian_model = session_row
        .as_ref()
        .and_then(|s| s.brian_model_at_spawn.clone())
        .unwrap_or_default();
    let rain_model = session_row
        .as_ref()
        .and_then(|s| s.rain_model_at_spawn.clone())
        .unwrap_or_default();
    let phase = core
        .current_phase(session_id)
        .await
        .map(|p| p.chip().to_string())
        .unwrap_or_else(|| "".to_string());

    // Single chronological projection — storage already returns rows ordered by
    // (session_id, created_at, id). Everyone sees one feed: brian, rain, user,
    // phase_change events all interleaved.
    let chrono: Vec<ChatMsgData> = if content_changed {
        msgs.iter().map(to_chat_data).collect()
    } else {
        Vec::new()
    };

    // Read live session permissions for the chips. AllBranches/Specific both
    // count as "granted" — the chip is binary; the user can narrow via chat
    // ("you can push on main") but the chip click toggles AllBranches/None.
    let perms = core.bridge.list_session_permissions(session_id).await;
    let commit_granted = !matches!(perms.commit, crate::policy::GrantScope::None);
    let push_granted = !matches!(perms.push, crate::policy::GrantScope::None);

    let session_id = session_id.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        if content_changed {
            let rows: Vec<ChatMsg> = chrono.into_iter().map(chat_from_data).collect();
            app.set_session_msgs(ModelRc::new(VecModel::from(rows)));
            // Tick the auto-scroll counter — slint side reacts and pins
            // viewport to bottom. Only when content actually changed so
            // we don't yank the user back to bottom on every 500ms poll.
            app.set_session_scroll_tick(app.get_session_scroll_tick().wrapping_add(1));
        }
        app.set_active_title(SharedString::from(title));
        app.set_active_phase(SharedString::from(phase));
        app.set_active_session_id(SharedString::from(session_id));
        app.set_active_brian_model(SharedString::from(brian_model));
        app.set_active_rain_model(SharedString::from(rain_model));
        app.set_active_commit_granted(commit_granted);
        app.set_active_push_granted(push_granted);
    });
    Ok(())
}

/// Push the external MCP URL + token into Slint AppState so the Settings tab
/// can render them. Reads the token from disk (it was generated by
/// `paths.init()`) and the bound address from `core.external_server`. When
/// the server failed to start (port busy / disabled), URL stays empty and the
/// Settings panel shows an "unavailable" status.
async fn seed_external_mcp_panel(weak: &Weak<AppWindow>, core: &Arc<CoreAppState>) {
    let addr = core
        .external_server
        .lock()
        .await
        .as_ref()
        .map(|s| s.local_addr.to_string());
    let url = addr.map(|a| format!("http://{a}/mcp")).unwrap_or_default();
    let token = core.paths.read_mcp_token().unwrap_or_default();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_external_mcp_url(SharedString::from(url));
        app.set_external_mcp_token(SharedString::from(token));
    });
}

/// Refresh the SessionView's right-pane DocumentPane for the currently-
/// selected IPAV tab. Filters `session_documents` by `phase = <tab>` and
/// surfaces the newest matching doc, plus a count for the "N more" chip
/// (wired in B10). Empty-state copy lands in the empty_msg property.
///
/// For the Apply tab (B7 minimal): falls back to phase='apply' docs only.
/// B8 layers in the `git diff <session_start_sha>` path before this fallback.
async fn refresh_session_docs(
    weak: &Weak<AppWindow>,
    core: &Arc<CoreAppState>,
    session_id: &str,
    selected_tab: &str,
) {
    let phase = match selected_tab {
        "I" => "investigate",
        "P" => "plan",
        "A" => "apply",
        "V" => "verify",
        _ => return,
    };
    let empty_msg = match selected_tab {
        "I" => "No investigation notes yet.",
        "P" => "No plan written yet.",
        "A" => "No changes applied yet.",
        "V" => "No verification notes yet.",
        _ => "",
    }
    .to_string();

    let docs = core
        .bridge
        .session_doc_search(session_id, None, Some(phase))
        .await
        .unwrap_or_default();

    let (content, slug, updated_at, count) = match docs.first() {
        Some(newest) => (
            newest.body.clone(),
            newest.slug.clone(),
            newest.updated_at.clone(),
            docs.len() as i32,
        ),
        None => (String::new(), String::new(), String::new(), 0),
    };

    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_active_doc_content(SharedString::from(content));
        app.set_active_doc_slug(SharedString::from(slug));
        app.set_active_doc_updated_at(SharedString::from(updated_at));
        app.set_active_doc_count(count);
        app.set_active_doc_empty_msg(SharedString::from(empty_msg));
    });
}

async fn refresh_emma(weak: &Weak<AppWindow>, core: &Arc<CoreAppState>) -> anyhow::Result<()> {
    let msgs = core.storage.messages_for_session("emma", None).await?;
    // Same skip-if-unchanged as refresh_session_view — keep highlight alive.
    let fp = fingerprint(&msgs);
    if !fingerprint_changed("emma", fp) {
        return Ok(());
    }
    let data: Vec<ChatMsgData> = msgs.iter().map(to_chat_data).collect();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        let rows: Vec<ChatMsg> = data.into_iter().map(chat_from_data).collect();
        app.set_emma_msgs(ModelRc::new(VecModel::from(rows)));
        // Pin Emma's scroll to bottom on content change (see SessionView).
        app.set_emma_scroll_tick(app.get_emma_scroll_tick().wrapping_add(1));
    });
    Ok(())
}

/// Poll Emma's chat for the first new text reply after broadcasting a brief
/// asking for a one-line description. Used by autodescribe (file + folder).
/// `context` tags failure-mode log messages.
async fn poll_emma_description<F>(
    core: &Arc<CoreAppState>,
    weak: &Weak<AppWindow>,
    context: &str,
    brief: &str,
    apply: F,
) where
    F: FnOnce(String) + Send + 'static,
{
    let last_id = core
        .storage
        .messages_for_session("emma", None)
        .await
        .ok()
        .and_then(|m| m.iter().map(|x| x.id).max())
        .unwrap_or(0);
    if let Err(e) = core.broadcast("emma", brief).await {
        warn!(?e, "{context}: send to emma");
        show_toast(weak, "Emma is unreachable.");
        return;
    }
    // Poll for a new text message. Cap at ~30s.
    let mut tries = 30;
    while tries > 0 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let msgs = match core
            .storage
            .messages_for_session("emma", Some(last_id))
            .await
        {
            Ok(m) => m,
            Err(_) => break,
        };
        if let Some(reply) = msgs.iter().find(|m| {
            m.author == "emma" && m.kind == "text" && !m.content.trim().is_empty()
        }) {
            let first_line = reply
                .content
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
            apply(first_line);
            show_toast(weak, "Description suggested by Emma.");
            return;
        }
        tries -= 1;
    }
    show_toast(weak, "Timed out waiting for Emma.");
}

fn to_chat_data(m: &Message) -> ChatMsgData {
    // Tool-use / tool-result rows are stored as JSON blobs (see core::duo +
    // core::session::pump_emma_agent). The UI shouldn't see raw JSON; format
    // friendly prose with an icon prefix here.
    let content = match m.kind.as_str() {
        "tool_use" => format_tool_use(&m.content),
        "tool_result" => format_tool_result(&m.content),
        _ => m.content.clone(),
    };
    ChatMsgData {
        author: m.author.clone(),
        kind: m.kind.clone(),
        content,
        created_at: m.created_at.clone(),
    }
}

async fn handle_signaling_event(
    weak: &Weak<AppWindow>,
    core: &Arc<CoreAppState>,
    ev: SignalingEvent,
) {
    match ev {
        SignalingEvent::MessagePersisted { .. } => {
            // Slint UI polls storage on its own ticker — it doesn't need this
            // event. Subscribers for `wait_for_change` come in via the
            // external server's own broadcast::Receiver.
        }
        SignalingEvent::AgentAdvancePhase {
            session_id,
            agent,
            target,
        } => {
            let Some(phase) = crate::core::ipav::IpavPhase::parse(&target) else {
                tracing::warn!(%session_id, %agent, %target, "agent advance_phase: bad target");
                return;
            };
            tracing::info!(%session_id, %agent, %target, "agent self-advanced phase");
            if let Err(e) = core.advance_phase(&session_id, phase).await {
                tracing::warn!(?e, %session_id, "agent self-advance_phase failed");
                return;
            }
            refresh_dashboard(weak, core).await;
        }
        SignalingEvent::SessionCloseRequest {
            session_id,
            agent,
            archive,
        } => {
            tracing::info!(
                session_id = %session_id,
                agent = %agent,
                archive,
                "agent-initiated session close"
            );
            if let Err(e) = core.close_session(&session_id, archive).await {
                tracing::warn!(?e, %session_id, "agent-initiated close_session failed");
                return;
            }
            // Refresh dashboard so the closed session disappears from tiles.
            refresh_dashboard(weak, core).await;
            // If the closed session was the active one, kick back to dashboard.
            let closed_id = session_id.clone();
            let _ = weak.upgrade_in_event_loop(move |handle| {
                let app = handle.global::<SlintAppState>();
                if app.get_active_session_id() == closed_id {
                    app.set_active_session_id(SharedString::new());
                    app.set_active_awaiting(false);
                }
            });
        }
        SignalingEvent::PendingChoice(p) => {
            let session_id = p.session_id.clone();
            let core_for_tray = Arc::clone(core);
            let session_for_tray = session_id.clone();
            let _ = weak.upgrade_in_event_loop(move |handle| {
                let app = handle.global::<SlintAppState>();
                // Emma has its own inline panel surface — populate the
                // emma-pending-* properties she still renders from.
                if session_id == "emma" {
                    let opts: Vec<SharedString> =
                        p.options.iter().map(|s| SharedString::from(s.clone())).collect();
                    app.set_emma_pending_choice(true);
                    app.set_emma_pending_question(SharedString::from(p.question.clone()));
                    app.set_emma_pending_options(ModelRc::new(VecModel::from(opts)));
                    app.set_emma_pending_choice_id(SharedString::from(p.choice_id.clone()));
                }
                // Duo sessions (brian + rain) render through the tray
                // exclusively — refresh handled below.
            });
            // Refresh the durable-storage-backed questions tray when this is
            // the active session. Storage write happens in the bridge BEFORE
            // PendingChoice fires, so the read here sees the new row.
            refresh_active_questions(weak, &core_for_tray, &session_for_tray);
        }
        SignalingEvent::AwaitingUser {
            session_id,
            agent: _,
            reason: _,
        } => {
            let session_for_tray = session_id.clone();
            let core_for_tray = Arc::clone(core);
            let _ = weak.upgrade_in_event_loop(move |handle| {
                let app = handle.global::<SlintAppState>();
                if session_id == "emma" {
                    app.set_emma_awaiting(true);
                    return;
                }
                if app.get_active_session_id() == session_id {
                    app.set_active_awaiting(true);
                }
                update_tile_awaiting(&app, &session_id, true);
            });
            // mark_awaiting_user writes a `halt` row that should appear in
            // the tray too — refresh.
            refresh_active_questions(weak, &core_for_tray, &session_for_tray);
        }
        SignalingEvent::ChoiceResolved { .. } => {
            let core_for_tray = Arc::clone(core);
            let _ = weak.upgrade_in_event_loop(move |handle| {
                let app = handle.global::<SlintAppState>();
                // Clear Emma's inline panel state. Duo sessions render
                // through the tray, which refreshes from storage below.
                app.set_emma_pending_choice(false);
                app.set_emma_pending_question(SharedString::new());
                app.set_emma_pending_options(ModelRc::new(VecModel::from(
                    Vec::<SharedString>::new(),
                )));
                app.set_emma_pending_choice_id(SharedString::new());
            });
            // ChoiceResolved doesn't carry session_id (bridge limitation), so
            // refresh the tray for the currently-active session. The storage
            // row was already updated to status=answered in resolve_choice
            // BEFORE this event fires, so the next read drops the answered
            // question from the pending list.
            let weak_outer = weak.clone();
            let core_inner = Arc::clone(&core_for_tray);
            Handle::current().spawn(async move {
                let active = {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let _ = weak_outer.upgrade_in_event_loop(move |handle| {
                        let _ = tx.send(
                            handle.global::<SlintAppState>().get_active_session_id().to_string(),
                        );
                    });
                    rx.await.unwrap_or_default()
                };
                if !active.is_empty() {
                    refresh_active_questions(&weak_outer, &core_inner, &active);
                }
            });
        }
    }
}

/// Plain-Rust mirror of `PendingQuestion` — Send + 'static, safe to ship
/// across the tokio→slint thread hop. Converted to the Slint struct inside
/// the event-loop closure where ModelRc::new is safe to call.
#[derive(Clone)]
struct QuestionData {
    choice_id: String,
    kind: String,
    agent: String,
    prompt: String,
    options: Vec<String>,
    asked_at: String,
}

/// Refresh `AppState.active-questions` from the durable `session_questions`
/// table. Called when the session view opens AND whenever a question-changing
/// event fires (PendingChoice / ChoiceResolved / AwaitingUser) for the
/// currently-active session. Filters to status=pending so the tray shows
/// exactly what still needs the user.
pub(crate) fn refresh_active_questions(
    weak: &Weak<AppWindow>,
    core: &Arc<CoreAppState>,
    session_id: &str,
) {
    let weak = weak.clone();
    let core = Arc::clone(core);
    let session_id = session_id.to_string();
    Handle::current().spawn(async move {
        let rows = match core.storage.questions_for_session(&session_id).await {
            Ok(rs) => rs,
            Err(e) => {
                warn!(?e, %session_id, "questions_for_session failed");
                return;
            }
        };
        let pending: Vec<QuestionData> = rows
            .into_iter()
            .filter(|r| r.status == "pending")
            .map(|r| QuestionData {
                options: r.options().unwrap_or_default(),
                choice_id: r.choice_id,
                kind: r.kind,
                agent: r.agent,
                prompt: r.prompt,
                asked_at: r.asked_at,
            })
            .collect();
        let session_id_clone = session_id.clone();
        let _ = weak.upgrade_in_event_loop(move |handle| {
            let app = handle.global::<SlintAppState>();
            // Only paint if this session is still the active one — guards
            // against a late storage read landing after the user navigated.
            if app.get_active_session_id() == session_id_clone {
                let mapped: Vec<PendingQuestion> = pending
                    .into_iter()
                    .map(|q| {
                        let opts: Vec<SharedString> =
                            q.options.into_iter().map(SharedString::from).collect();
                        PendingQuestion {
                            choice_id: SharedString::from(q.choice_id),
                            kind: SharedString::from(q.kind),
                            agent: SharedString::from(q.agent),
                            prompt: SharedString::from(q.prompt),
                            options: ModelRc::new(VecModel::from(opts)),
                            asked_at: SharedString::from(q.asked_at),
                        }
                    })
                    .collect();
                app.set_active_questions(ModelRc::new(VecModel::from(mapped)));
            }
        });
    });
}

/// Copy a session tile's (awaiting, pending_*) state into the global
/// active-* properties. Called when the user opens a session so the
/// AttentionBanner + choice prompt reflect THIS session's state, not
/// whatever was active before.
fn sync_active_from_tile(weak: &Weak<AppWindow>, session_id: &str) {
    let session_id = session_id.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        let model = app.get_sessions();
        for i in 0..model.row_count() {
            if let Some(tile) = model.row_data(i) {
                if tile.id == session_id {
                    app.set_active_awaiting(tile.awaiting);
                    return;
                }
            }
        }
        // No matching tile (e.g., emma) → clear awaiting.
        app.set_active_awaiting(false);
    });
}

/// Clear the awaiting-user flag for a session — both the global active-*
/// pair (if it's the active session) and the matching dashboard tile.
/// Called when the user sends a message answering the request.
fn clear_awaiting_for(weak: &Weak<AppWindow>, session_id: &str) {
    let session_id = session_id.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        if app.get_active_session_id() == session_id {
            app.set_active_awaiting(false);
        }
        let model = app.get_sessions();
        for i in 0..model.row_count() {
            if let Some(mut tile) = model.row_data(i) {
                if tile.id == session_id {
                    tile.awaiting = false;
                    model.set_row_data(i, tile);
                    break;
                }
            }
        }
    });
}

/// Emma counterpart of clear_awaiting_for. Emma has no dashboard tile so we
/// only clear the (future) emma-specific awaiting state. Today emma's
/// awaiting flag lives on active-awaiting when emma is the active session,
/// but emma is more typically the side panel. Clearing both is harmless.
fn clear_emma_awaiting(weak: &Weak<AppWindow>) {
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_emma_awaiting(false);
        app.set_emma_pending_choice(false);
        app.set_emma_pending_question(SharedString::new());
        app.set_emma_pending_options(ModelRc::new(VecModel::from(
            Vec::<SharedString>::new(),
        )));
        app.set_emma_pending_choice_id(SharedString::new());
    });
}

fn update_tile_awaiting(app: &SlintAppState, session_id: &str, awaiting: bool) {
    // The dashboard tile's awaiting flag is set here on AwaitingUser events
    // and cleared by clear_awaiting_for when the user replies. pending_input_count
    // is independently rebuilt from storage every refresh and drives the
    // tile's [Need User Input · N] chip.
    let model = app.get_sessions();
    for i in 0..model.row_count() {
        if let Some(mut tile) = model.row_data(i) {
            if tile.id == session_id {
                tile.awaiting = awaiting;
                model.set_row_data(i, tile);
            }
        }
    }
}

fn update_active_session_id(weak: &Weak<AppWindow>, id: &str) {
    let id = id.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        handle
            .global::<SlintAppState>()
            .set_active_session_id(SharedString::from(id));
    });
}

fn show_toast(weak: &Weak<AppWindow>, text: &str) {
    let text = text.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_toast_text(SharedString::from(text));
        app.set_toast_visible(true);
    });
}

fn current_session_id(weak: &Weak<AppWindow>) -> String {
    // Synchronous read — only valid on the event-loop thread. Returns "" off-
    // thread (Weak::upgrade fails). Call from inside a Slint callback, BEFORE
    // spawning the tokio task. For off-thread reads (e.g., the polling loop)
    // use `current_session_id_async` which hops to the event loop.
    weak.upgrade()
        .map(|handle| {
            handle
                .global::<SlintAppState>()
                .get_active_session_id()
                .to_string()
        })
        .unwrap_or_default()
}

/// Async-safe read of `active-session-id` from off the event loop. Hops via
/// `upgrade_in_event_loop` + oneshot. Use inside `rt.spawn` tasks.
async fn current_session_id_async(weak: &Weak<AppWindow>) -> String {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = weak.upgrade_in_event_loop(move |h| {
        let id = h.global::<SlintAppState>().get_active_session_id().to_string();
        let _ = tx.send(id);
    });
    rx.await.unwrap_or_default()
}

/// Async-safe read of `selected-doc-tab` (I/P/A/V) from off the event loop.
/// Defaults to "I" if the read fails, matching the Slint default initializer.
async fn current_selected_doc_tab_async(weak: &Weak<AppWindow>) -> String {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = weak.upgrade_in_event_loop(move |h| {
        let t = h.global::<SlintAppState>().get_selected_doc_tab().to_string();
        let _ = tx.send(t);
    });
    rx.await.unwrap_or_else(|_| "I".to_string())
}

fn current_cl_state(weak: &Weak<AppWindow>) -> (String, String) {
    weak.upgrade()
        .map(|handle| {
            let app = handle.global::<SlintAppState>();
            (
                app.get_cl_current_path().to_string(),
                app.get_cl_current_body().to_string(),
            )
        })
        .unwrap_or_default()
}

fn current_cl_metadata(weak: &Weak<AppWindow>) -> (String, String) {
    weak.upgrade()
        .map(|handle| {
            let app = handle.global::<SlintAppState>();
            (
                app.get_cl_current_description().to_string(),
                app.get_cl_current_tags().to_string(),
            )
        })
        .unwrap_or_default()
}

fn update_cl_metadata(weak: &Weak<AppWindow>, description: &str, tags: &str) {
    let description = description.to_string();
    let tags = tags.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_cl_current_description(SharedString::from(description));
        app.set_cl_current_tags(SharedString::from(tags));
    });
}

fn clear_cl_metadata_dirty(weak: &Weak<AppWindow>) {
    let _ = weak.upgrade_in_event_loop(move |handle| {
        handle.global::<SlintAppState>().set_cl_metadata_dirty(false);
    });
}

fn set_editing_state(weak: &Weak<AppWindow>, mode: &str, path: &str, name: &str) {
    let mode = mode.to_string();
    let path = path.to_string();
    let name = name.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_cl_tree_editing_mode(SharedString::from(mode));
        app.set_cl_tree_editing_path(SharedString::from(path));
        app.set_cl_tree_editing_name(SharedString::from(name));
    });
}

fn clear_editing_state(weak: &Weak<AppWindow>) {
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_cl_tree_editing_mode(SharedString::new());
        app.set_cl_tree_editing_path(SharedString::new());
        app.set_cl_tree_editing_name(SharedString::new());
    });
}


/// Split a relative-to-data_dir path into (project_id, file_path_within_project).
/// Anything under `projects/<name>/...` belongs to that project; everything
/// else belongs to `_globals`.
fn resolve_project_and_path(rel: &str) -> (String, String) {
    let normalized = rel.trim_start_matches('/').to_string();
    if let Some(rest) = normalized.strip_prefix("projects/") {
        if let Some(slash) = rest.find('/') {
            let (project, sub) = rest.split_at(slash);
            return (project.to_string(), sub.trim_start_matches('/').to_string());
        }
    }
    (crate::storage::Project::GLOBALS.to_string(), normalized)
}

fn update_cl_current(weak: &Weak<AppWindow>, rel: &str, body: &str) {
    let rel = rel.to_string();
    let body = body.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_cl_current_path(SharedString::from(rel));
        app.set_cl_current_body(SharedString::from(body));
        // Mutual exclusivity with the folder view — opening a file
        // closes any open folder view in the right pane.
        app.set_cl_current_folder(SharedString::new());
        app.set_cl_current_folder_description(SharedString::new());
        app.set_cl_current_folder_is_project(false);
        app.set_cl_current_folder_project_name(SharedString::new());
        app.set_cl_current_folder_working_repo(SharedString::new());
        app.set_cl_current_folder_dirty(false);
    });
}

/// Push folder-view state onto the right pane. Mutually exclusive with
/// `update_cl_current` — clears the file-view state so only one pane is
/// visible at a time.
#[allow(clippy::too_many_arguments)]
fn update_cl_folder(
    weak: &Weak<AppWindow>,
    folder: &str,
    description: &str,
    is_project: bool,
    project_name: &str,
    working_repo: &str,
) {
    let folder = folder.to_string();
    let description = description.to_string();
    let project_name = project_name.to_string();
    let working_repo = working_repo.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_cl_current_folder(SharedString::from(folder));
        app.set_cl_current_folder_description(SharedString::from(description));
        app.set_cl_current_folder_is_project(is_project);
        app.set_cl_current_folder_project_name(SharedString::from(project_name));
        app.set_cl_current_folder_working_repo(SharedString::from(working_repo));
        app.set_cl_current_folder_dirty(false);
        // Mutual exclusivity with the file view.
        app.set_cl_current_path(SharedString::new());
        app.set_cl_current_body(SharedString::new());
        app.set_cl_current_description(SharedString::new());
        app.set_cl_current_tags(SharedString::new());
        app.set_cl_dirty(false);
        app.set_cl_metadata_dirty(false);
    });
}

fn current_cl_folder_state(
    weak: &Weak<AppWindow>,
) -> (String, String, bool, String) {
    weak.upgrade()
        .map(|handle| {
            let app = handle.global::<SlintAppState>();
            (
                app.get_cl_current_folder().to_string(),
                app.get_cl_current_folder_description().to_string(),
                app.get_cl_current_folder_is_project(),
                app.get_cl_current_folder_project_name().to_string(),
            )
        })
        .unwrap_or_default()
}

fn open_register_dialog(
    weak: &Weak<AppWindow>,
    name: &str,
    cl_path: &str,
    working_repo: &str,
) {
    let name = name.to_string();
    let cl_path = cl_path.to_string();
    let working_repo = working_repo.to_string();
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_cl_register_name(SharedString::from(name));
        app.set_cl_register_cl_path(SharedString::from(cl_path));
        app.set_cl_register_working_repo(SharedString::from(working_repo));
        app.set_cl_register_dialog_open(true);
    });
}

fn close_register_dialog(weak: &Weak<AppWindow>) {
    let _ = weak.upgrade_in_event_loop(move |handle| {
        let app = handle.global::<SlintAppState>();
        app.set_cl_register_dialog_open(false);
        app.set_cl_register_name(SharedString::new());
        app.set_cl_register_cl_path(SharedString::new());
        app.set_cl_register_working_repo(SharedString::new());
    });
}

fn current_register_dialog(weak: &Weak<AppWindow>) -> (String, String, String) {
    weak.upgrade()
        .map(|handle| {
            let app = handle.global::<SlintAppState>();
            (
                app.get_cl_register_name().to_string(),
                app.get_cl_register_cl_path().to_string(),
                app.get_cl_register_working_repo().to_string(),
            )
        })
        .unwrap_or_default()
}

/// Resolve the owning project + folder_path for a tree-relative path. Tree
/// rows are relative to `data_dir`; this finds the registered project whose
/// `cl_path` (or default convention) is the longest prefix of the folder's
/// absolute path. Falls back to `_globals` when no registered project owns
/// the folder — so shared CL slots (e.g. `agents/`) still get described.
async fn resolve_folder_owner(
    storage: &crate::storage::Storage,
    data_dir: &std::path::Path,
    rel_to_data_dir: &str,
) -> (String, String) {
    let folder_abs = data_dir.join(rel_to_data_dir);
    let projects = match storage.list_projects().await {
        Ok(p) => p,
        Err(_) => return (
            crate::storage::Project::GLOBALS.to_string(),
            rel_to_data_dir.to_string(),
        ),
    };
    let mut best: Option<(std::path::PathBuf, String)> = None;
    for proj in projects {
        if proj.name == crate::storage::Project::GLOBALS {
            continue;
        }
        let abs = match proj.cl_path.as_deref() {
            Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
            _ => data_dir.join("projects").join(&proj.name),
        };
        if folder_abs.starts_with(&abs) {
            let take = best
                .as_ref()
                .map(|(p, _)| abs.as_os_str().len() > p.as_os_str().len())
                .unwrap_or(true);
            if take {
                best = Some((abs, proj.name));
            }
        }
    }
    match best {
        Some((root, name)) => {
            let rel = folder_abs
                .strip_prefix(&root)
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            (name, rel)
        }
        None => (
            crate::storage::Project::GLOBALS.to_string(),
            rel_to_data_dir.to_string(),
        ),
    }
}

fn clear_cl_dirty(weak: &Weak<AppWindow>) {
    let _ = weak.upgrade_in_event_loop(move |handle| {
        handle.global::<SlintAppState>().set_cl_dirty(false);
    });
}
