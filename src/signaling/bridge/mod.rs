//! Bridges MCP tool calls to the UI layer.
//!
//! The MCP HTTP handler invokes [`SignalingBridge::ask_user_choice`] /
//! [`SignalingBridge::mark_awaiting_user`]. Those calls fan out two ways:
//!
//! 1. A [`SignalingEvent`] is broadcast over `event_tx`; the UI subscribes and
//!    paints choice buttons or sets the awaiting-user flag.
//! 2. For `ask_user_choice`, a fresh `oneshot::Sender<String>` is parked in
//!    `pending`. The MCP handler awaits on the matching `oneshot::Receiver`;
//!    the UI later calls [`SignalingBridge::resolve_choice`] with the chosen
//!    option. The result flows back to the agent as the tool's return value.
//!
//! The implementation is split across submodules — each owns one cohesive slice
//! of the bridge's surface and contributes its own `impl SignalingBridge` block:
//!
//! - [`questions`]    — user-blocking tools (ask/resolve/supersede/await/phase)
//! - [`cl_facade`]    — Context Library index/folder/rescan reads
//! - [`session_docs`] — per-session scratch documents
//! - [`util`]         — free helper functions

use crate::policy::{Policy, ViolationKind, ViolationsLog};
use crate::storage::Storage;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, Mutex};

mod action_gate;
mod cl_facade;
mod questions;
mod session_docs;
mod util;

/// Summary of a single `cl_rescan` pass.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ClRescanReport {
    /// Files newly discovered on disk and inserted into the index.
    pub added: Vec<String>,
    /// Existing index entries whose stored updated_at lagged disk mtime.
    pub touched: Vec<String>,
    /// Index entries pointing at files that no longer exist on disk.
    pub orphaned: Vec<String>,
}

/// What happened when a parked choice was resolved.
///
/// The happy path (`Delivered`) means the agent's blocking tool call was
/// still waiting and received the picked option synchronously. The
/// fallback (`AgentReceiverDroppedFellBack`) means the agent's tool call
/// already client-side timed out, so the bridge persisted an out-of-band
/// `user` message into session storage; the caller (typically
/// `CoreAppState::resolve_choice`) is responsible for **also** sending
/// that body through the duo's input channels so the agent's subprocess
/// wakes up and sees it (clearing the awaiting flag alone won't deliver
/// — the agent is blocked on stdin and needs an actual stdin write).
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    Delivered,
    AgentReceiverDroppedFellBack { session_id: String, body: String },
}

#[derive(Debug, Clone)]
pub enum SignalingEvent {
    PendingChoice(PendingChoice),
    AwaitingUser {
        session_id: String,
        agent: String,
        reason: String,
    },
    /// Resolved (so the UI can clean up its inline rendering).
    ChoiceResolved {
        choice_id: String,
        picked: String,
    },
    /// A new message row was persisted to storage. Fired by the per-agent
    /// pumps (duo + emma) after `storage.insert_message` returns. Lets the
    /// external MCP's `wait_for_change` tool block server-side instead of
    /// asking clients to poll.
    MessagePersisted {
        session_id: String,
        message_id: i64,
    },
    /// Agent asked to close its own session via the `close_session` MCP tool.
    /// AppState picks this up, kills the agent subprocesses, and marks the
    /// session closed/archived in storage. Fire-and-forget — the agent
    /// gets killed before it sees the outcome, which is the right semantics
    /// for "close the session I'm in."
    SessionCloseRequest {
        session_id: String,
        agent: String,
        archive: bool,
    },
    /// Agent self-advanced the IPAV phase via the `advance_phase` MCP tool.
    /// AppState's signaling subscriber parses `target` and calls
    /// `core.advance_phase` so the IpavState updates, transition_notice
    /// fires, and the dashboard chip moves. `target` accepts full names
    /// or single-letter chips (see `IpavPhase::parse`).
    AgentAdvancePhase {
        session_id: String,
        agent: String,
        target: String,
    },
    /// A session document was written/updated (`session_doc_write`). The UI
    /// invalidates its doc queries so a freshly-written phase doc appears
    /// without a manual tab-switch.
    DocChanged { session_id: String },
    /// A session finished closing (after `core.close_session`). The UI
    /// navigates away from a now-closed session and refreshes its lists.
    SessionClosed { session_id: String },
}

#[derive(Debug, Clone)]
pub struct PendingChoice {
    pub choice_id: String,
    pub session_id: String,
    pub agent: String,
    pub question: String,
    pub options: Vec<String>,
    /// Optional richer-context fields for policy-initiated approval requests.
    /// `None` for plain `ask_user_choice` calls.
    pub approval: Option<ApprovalContext>,
}

/// Side-channel context for policy-initiated approval requests. Lets the UI
/// render the prompt differently (e.g., red border for `force_push`) and
/// gives `resolve_choice` enough metadata to write a proper violation record.
#[derive(Debug, Clone)]
pub struct ApprovalContext {
    pub kind: ViolationKind,
    pub action: String,
    pub detail: Option<String>,
}

/// Parked state for a pending choice. The oneshot resolves the agent's wait;
/// the cloned `choice` lets external readers (`list_pending_choices`) see the
/// question + options without losing the resolve-time-only approval context.
struct Parked {
    tx: oneshot::Sender<String>,
    choice: PendingChoice,
}

/// Shared signaling state.
pub struct SignalingBridge {
    event_tx: broadcast::Sender<SignalingEvent>,
    pending: Mutex<HashMap<String, Parked>>,
    violations: Option<ViolationsLog>,
    /// `<data_dir>` for resolving policy.yaml on demand. None disables
    /// policy-aware tools (`check_commit_message` returns "ok" trivially).
    data_dir: Option<PathBuf>,
    /// session_id → optional project slug. Sessions register themselves at
    /// spawn time so policy-aware MCP tools can look up the right policy.
    session_projects: Mutex<HashMap<String, Option<String>>>,
    /// session_id → "duo is waiting for user input" flag, shared with the
    /// duo pump so it can halt peer-forwarding while flag is set. When any
    /// user-blocking tool (mark_awaiting_user / ask_user_choice / request_approval)
    /// fires, the bridge sets the flag synchronously BEFORE returning so
    /// Brian's next chunk doesn't volley to Rain before the halt takes effect.
    session_awaiting: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Storage handle for out-of-band message injection. Set once via
    /// `set_storage` at startup. When a `resolve_choice` lands after the
    /// agent's blocking `ask_user_choice` tool call already client-side
    /// timed out (claude-code's MCP tool timeout is shorter than typical
    /// user-response latency), the answer is otherwise lost. We persist a
    /// synthetic user message so the duo sees the resolution on its next
    /// message poll. None on bridges constructed before storage is wired
    /// (test bridges + the pre-storage window in main).
    storage: Mutex<Option<Storage>>,
    /// Tauri AppHandle, populated from `setup()` once the webview exists.
    /// Internal MCP `webview_*` tools (jsonrpc.rs) reach the webview through
    /// this — bridge is the only shared handle accessible to the per-agent
    /// dispatchers, which don't have CoreAppState. Set-once; `None` in tests
    /// and during the pre-setup window.
    app_handle: std::sync::OnceLock<tauri::AppHandle>,
}

impl SignalingBridge {
    fn new_with(
        violations: Option<ViolationsLog>,
        data_dir: Option<PathBuf>,
    ) -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            event_tx,
            pending: Mutex::new(HashMap::new()),
            violations,
            data_dir,
            session_projects: Mutex::new(HashMap::new()),
            session_awaiting: Mutex::new(HashMap::new()),
            storage: Mutex::new(None),
            app_handle: std::sync::OnceLock::new(),
        })
    }

    pub fn new() -> Arc<Self> {
        Self::new_with(None, None)
    }

    /// Construct a bridge with a violations log attached. Approval-class
    /// resolutions write a record after the user picks an option.
    pub fn with_violations_log(violations: ViolationsLog) -> Arc<Self> {
        Self::new_with(Some(violations), None)
    }

    /// Full-featured constructor: violations log + policy resolution root.
    /// Used in production; tests can use [`new`] or [`with_violations_log`]
    /// for partial setups.
    pub fn with_policy(violations: ViolationsLog, data_dir: PathBuf) -> Arc<Self> {
        Self::new_with(Some(violations), Some(data_dir))
    }

    /// Called by the session spawn code so the bridge can resolve the right
    /// project policy when this session's agents call policy-aware MCP tools.
    /// Idempotent — re-registering overwrites.
    pub async fn register_session(&self, session_id: String, project: Option<String>) {
        self.session_projects.lock().await.insert(session_id, project);
    }

    /// Wire the storage handle so the bridge can write out-of-band messages
    /// when a `resolve_choice` arrives after the agent's tool call already
    /// timed out. Called once at startup. Idempotent (overwrites).
    pub async fn set_storage(&self, storage: Storage) {
        *self.storage.lock().await = Some(storage);
    }

    /// Hand the bridge a shared awaiting-flag pointer owned by the SessionHandle.
    /// The duo pump reads this same flag to decide whether to forward chunks
    /// to the peer. Setting it from inside the bridge (in mark_awaiting_user /
    /// ask_user_choice) is what gives us a race-free halt.
    pub async fn register_session_awaiting(&self, session_id: String, flag: Arc<AtomicBool>) {
        self.session_awaiting.lock().await.insert(session_id, flag);
    }

    /// Clear the awaiting flag for a session — called by core.broadcast when
    /// the user sends a message (which resumes the duo).
    pub async fn clear_session_awaiting(&self, session_id: &str) {
        if let Some(flag) = self.session_awaiting.lock().await.get(session_id) {
            flag.store(false, Ordering::Release);
        }
    }

    /// Drop a session's bridge-side state (project mapping + awaiting flag) when
    /// it closes. Without this, `session_projects` + `session_awaiting` grow
    /// unbounded across open→close cycles — each closed session leaks a map entry
    /// (and a dangling `Arc<AtomicBool>`) for the process lifetime. Idempotent —
    /// absent entries are fine. Called from `core::close_session`.
    pub async fn unregister_session(&self, session_id: &str) {
        self.session_projects.lock().await.remove(session_id);
        self.session_awaiting.lock().await.remove(session_id);
    }

    // ---- Project helpers --------------------------------------------

    /// Best-effort lookup. Returns the registered project (if any) or None
    /// if the session isn't registered yet (e.g., the seeded `"emma"` row).
    pub async fn project_for_session(&self, session_id: &str) -> Option<String> {
        self.session_projects
            .lock()
            .await
            .get(session_id)
            .cloned()
            .flatten()
    }

    /// Look up the registered project for `session_id` and, when a project
    /// is registered, resolve its CL root via storage's `cl_path_for_project`.
    /// Returns both because the callers that resolve project_root also pass
    /// the project name through to the underlying policy/audit fns.
    async fn resolve_project_and_root(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> (Option<String>, Option<PathBuf>) {
        let project = self.project_for_session(session_id).await;
        let project_root = match project.as_deref() {
            Some(p) => {
                let storage = self.storage.lock().await.clone();
                match storage {
                    Some(storage) => storage.cl_path_for_project(data_dir, p).await.ok(),
                    None => None,
                }
            }
            None => None,
        };
        (project, project_root)
    }

    /// Load (resolve) policy for the given session. Falls back to default
    /// policy if data_dir isn't configured or the session isn't registered.
    /// Parse errors propagate — callers should map to a JSON-RPC error.
    pub async fn resolve_policy_for(&self, session_id: &str) -> Result<Policy> {
        let Some(data_dir) = self.data_dir.as_ref() else {
            return Ok(Policy::default());
        };
        let (project, project_root) = self.resolve_project_and_root(data_dir, session_id).await;
        Policy::resolve_at_root(
            data_dir,
            project.as_deref(),
            project_root.as_deref(),
            Some(session_id),
        )
    }

    /// Delete the canonical session-policy snapshot. Called by
    /// `core::state::close_session` when the session closes — the snapshot is
    /// per-session state that must not leak into the next session (which
    /// re-seeds from the current blueprints). Idempotent; no-ops silently when
    /// the bridge has no `data_dir` (test bridges).
    pub async fn cleanup_session_policy(&self, session_id: &str) -> Result<()> {
        if let Some(data_dir) = &self.data_dir {
            crate::policy::session_policy::delete_session_policy(data_dir, session_id)?;
        }
        Ok(())
    }

    /// Direct access to the violations log (e.g., for the UI's recent-events
    /// panel). None when the bridge was constructed without a log.
    pub fn violations_log(&self) -> Option<&ViolationsLog> {
        self.violations.as_ref()
    }

    /// Audit `<data_dir>/general-policy.yaml` + the project's policy.yaml for
    /// mutations, honoring a non-default `projects.cl_path` when set. Wraps
    /// [`crate::policy::audit_policy_files_at_root`] for callers that only
    /// have a `(session_id, agent)` pair and don't want to thread storage
    /// through themselves. No-ops silently when the bridge has no `data_dir`.
    pub async fn audit_policy_files_for_session(
        &self,
        session_id: &str,
        caller_agent: &str,
    ) -> Result<()> {
        let Some(data_dir) = self.data_dir.as_ref() else {
            return Ok(());
        };
        let (project, project_root) = self.resolve_project_and_root(data_dir, session_id).await;
        crate::policy::audit_policy_files_at_root(
            data_dir,
            project.as_deref(),
            project_root.as_deref(),
            self.violations.as_ref(),
            session_id,
            caller_agent,
        )?;
        Ok(())
    }

    /// CL root path — used by callers that need to read auxiliary files
    /// (policy hash cache, etc.). None on test bridges built via `new()`.
    pub fn data_dir(&self) -> Option<&PathBuf> {
        self.data_dir.as_ref()
    }

    /// Stash the Tauri AppHandle once `setup()` has it. Idempotent — silently
    /// ignores a second call. Tests don't set this; internal MCP webview_*
    /// tools error with "AppHandle not initialized" when unset.
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        let _ = self.app_handle.set(handle);
    }

    /// Get the stashed AppHandle. None until `setup()` runs, or in tests.
    pub fn app_handle(&self) -> Option<&tauri::AppHandle> {
        self.app_handle.get()
    }

    /// Subscribe to all signaling events. The UI layer uses this to paint.
    pub fn subscribe(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    /// Fire a `MessagePersisted` event. Called by the per-agent pumps + the
    /// user-broadcast helper after `storage.insert_message` returns the new
    /// row id. The external MCP's `wait_for_change` tool subscribes for these
    /// so clients don't need to poll.
    pub fn notify_message_persisted(&self, session_id: String, message_id: i64) {
        let _ = self.event_tx.send(SignalingEvent::MessagePersisted {
            session_id,
            message_id,
        });
    }

    /// Fire a `SessionClosed` event after a session finished closing, so the UI
    /// can leave the closed session view + refresh its lists. Fire-and-forget.
    pub fn notify_session_closed(&self, session_id: String) {
        let _ = self
            .event_tx
            .send(SignalingEvent::SessionClosed { session_id });
    }

    /// Called by the MCP `tools/call` handler for `close_session`. Broadcasts
    /// a request; AppState's signaling subscriber processes it (kills agents,
    /// marks closed in storage). Fire-and-forget — by the time the agent
    /// reads our "ok" response, the subprocess might already be dying.
    pub fn request_session_close(&self, session_id: String, agent: String, archive: bool) {
        let _ = self.event_tx.send(SignalingEvent::SessionCloseRequest {
            session_id,
            agent,
            archive,
        });
    }

    /// Called by the MCP `tools/call` handler for `advance_phase`. Broadcasts
    /// the request; AppState's subscriber routes to `core.advance_phase`
    /// which updates IpavState, fires transition_notice into both agents,
    /// and clears any awaiting halt. Fire-and-forget — the agent's tool
    /// call returns immediately; the phase update lands on the next event
    /// loop tick.
    pub fn agent_advance_phase(&self, session_id: String, agent: String, target: String) {
        let _ = self.event_tx.send(SignalingEvent::AgentAdvancePhase {
            session_id,
            agent,
            target,
        });
    }
}
