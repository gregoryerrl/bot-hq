//! Bridges MCP tool calls to the UI layer.
//!
//! The MCP HTTP handler invokes [`SignalingBridge::ask_user_choice`] /
//! [`SignalingBridge::mark_awaiting_user`]. Those calls fan out two ways:
//!
//! 1. A [`SignalingEvent`] is broadcast over `event_tx`; the UI subscribes and
//!    paints choice buttons or sets the awaiting-user flag.
//! 2. A `oneshot::Sender<String>` is parked in `pending`. For the blocking
//!    `request_approval`, the MCP handler awaits the matching
//!    `oneshot::Receiver` and the chosen option returns as the tool's value.
//!    For the non-blocking `ask_user_choice`, the handler returns a `{parked}`
//!    ack immediately and the user's pick is delivered out-of-band as a
//!    synthetic user message (not the tool's return value). The UI calls
//!    [`SignalingBridge::resolve_choice`] with the chosen option either way.
//!
//! The implementation is split across submodules — each owns one cohesive slice
//! of the bridge's surface and contributes its own `impl SignalingBridge` block:
//!
//! - [`tray`]         — user-blocking tools (ask/resolve/supersede/await/phase)
//! - [`action_gate`]  — Tool-Gate execute-on-approve (the `action_gate` tool)
//! - [`findings`]     — EYES-sign-off review findings + the commit gate
//! - [`cl_facade`]    — Context Library index/folder/rescan reads
//! - [`session_docs`] — per-session scratch documents
//! - [`util`]         — free helper functions

use crate::core::activity::ActivityTracker;
use crate::policy::{Policy, ViolationKind, ViolationsLog};
use crate::storage::Storage;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::{broadcast, oneshot, Mutex};

mod action_gate;
mod cl_facade;
mod findings;
mod session_docs;
mod tray;
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
    /// The pick would EXECUTE a gated command (action_gate / ToolBlocklist)
    /// whose requesting agent has moved on (client-side MCP timeout / restart),
    /// and the caller did not pass `confirm_stale`. NOTHING was flipped or
    /// executed — the command may now be invalid or destructive against a
    /// changed repo, so the UI must confirm and re-resolve with
    /// `confirm_stale = true`. Reject / non-executing picks never reach here.
    StaleGateNeedsConfirm {
        command: String,
        asked_at: Option<String>,
    },
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
    /// pumps (duo) after `storage.insert_message` returns. Lets the
    /// external MCP's `wait_for_change` tool block server-side instead of
    /// asking clients to poll.
    MessagePersisted {
        session_id: Arc<str>,
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
    DocChanged {
        session_id: String,
    },
    /// A session's EYES findings changed (`eyes_flag` / `disposition_finding`).
    /// The UI refetches the per-session findings banner so the ⚠ count is live.
    FindingsChanged {
        session_id: String,
    },
    /// A session finished closing (after `core.close_session`). The UI
    /// navigates away from a now-closed session and refreshes its lists.
    SessionClosed {
        session_id: String,
    },
    /// Pending `mark_awaiting_user` halt rows were flipped to answered (by a
    /// user broadcast or a phase advance). The UI invalidates its tray queries
    /// so the "needs input" bell clears — a DB-only clear (clear_pending_halts)
    /// otherwise leaves the `list_pending_tray` cache stale. Scoped to the tray
    /// (not a full resync) per the per-event invalidation policy.
    HaltsCleared {
        session_id: String,
    },
    /// An agent's retry-supervisor liveness changed (B2: running / retrying /
    /// dead). The UI updates the per-agent health dot. `health` is the state
    /// string from `AgentHealth::as_str` — carried as a String so the signaling
    /// layer stays decoupled from the agents enum.
    AgentHealth {
        session_id: String,
        agent: String,
        health: String,
    },
    /// A session's duo activity changed (idle / busy / awaiting-user /
    /// cancelling). Drives the chat-input lock + Cancel button: the UI disables
    /// input while `busy`/`cancelling`, re-enables on `idle`/`awaiting_user`.
    /// `state` is the `SessionActivity::as_str` string — carried as a String so
    /// the signaling layer stays decoupled from the core activity enum.
    /// `brian_busy`/`rain_busy` carry the per-agent busy flags (the derived
    /// `state` collapses them) so the UI can show *which* agent is working —
    /// e.g. a broadcast sets both busy at once.
    SessionActivity {
        session_id: String,
        state: String,
        brian_busy: bool,
        rain_busy: bool,
    },
    /// The per-session peer-forward router's liveness changed. `alive=false` is
    /// emitted by the watchdog when the router task has died while agents are
    /// still live (forwarding is down); `alive=true` on (re)spawn. The UI shows a
    /// router-health dot. Carried as a bool — the signaling layer stays decoupled
    /// from core.
    RouterHealth {
        session_id: String,
        alive: bool,
    },
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

/// A3b: per-session state for the close-delta soft-gate.
#[derive(Default)]
struct CloseGateState {
    /// The agent ran `cl_rescan` this session (proxy for a CL learnings write).
    cl_written: bool,
    /// We've already nudged once on `close_session` — let the next close go.
    close_nudged: bool,
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
    /// session_id → Weak ref to the session's ActivityTracker. Lets
    /// `set_session_awaiting` reflect an awaiting-flag flip into the derived
    /// activity immediately (emit AwaitingUser) instead of waiting for the next
    /// `set_busy`. Weak, not Arc: the tracker holds a strong
    /// `Arc<SignalingBridge>` (activity.rs), so a strong back-ref here would
    /// cycle and leak the tracker past session close; `upgrade()` returns None
    /// after close → a silent no-op.
    session_activity: Mutex<HashMap<String, Weak<ActivityTracker>>>,
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
    /// A3b: per-session close-gate state — whether the agent touched the CL
    /// (`cl_rescan`, a proxy for the write-then-prune learnings delta) and
    /// whether we've already nudged it once on close. Drives the soft two-call
    /// gate in the `close_session` MCP handler.
    session_close_gate: Mutex<HashMap<String, CloseGateState>>,
    /// Batch 7: latest health per (session_id, agent) — the wire string from
    /// `AgentHealth::as_str` ("running"/"retrying"/"stalled"/"dead"). Written by
    /// `notify_agent_health`; read by the fail-closed commit gate to block when a
    /// duo reviewer is Stalled/Dead. `std::sync::Mutex` (not tokio) because
    /// `notify_agent_health` is sync — mirrors ActivityTracker's pattern.
    agent_health: std::sync::Mutex<HashMap<(String, String), String>>,
    /// Batch 7: per-session HANDS override of the reviewer-down commit block —
    /// session_id → reason. Set by `override_reviewer_block`, honored by
    /// `check_open_findings`, auto-cleared when the reviewer recovers to running.
    reviewer_override: std::sync::Mutex<HashMap<String, String>>,
    /// Latest peer-forward-router liveness per session_id (true = alive). Written
    /// by `notify_router_health`; read by `get_session_runtime` to seed the UI
    /// router-health dot on mount (the event fires only on change, like
    /// `agent_health`). Sync `Mutex` — `notify_router_health` is sync.
    router_health: std::sync::Mutex<HashMap<String, bool>>,
    /// session_id → shared open-blocking-findings count. The router reads the
    /// `Arc<AtomicUsize>` LOCK-FREE per peer-forward (for the wire banner) instead
    /// of a per-forward `SELECT COUNT(*)` + storage-`Mutex` acquire; the findings
    /// mutators recompute it after any change via `refresh_open_blocking`.
    /// `std::sync::Mutex` over the MAP (brief, never held across `await`); the
    /// per-session `Arc` is the lock-free read surface the router holds a clone of.
    session_open_blocking: std::sync::Mutex<HashMap<String, Arc<AtomicUsize>>>,
}

impl SignalingBridge {
    fn new_with(violations: Option<ViolationsLog>, data_dir: Option<PathBuf>) -> Arc<Self> {
        // Sized generously: every stream chunk fires MessagePersisted and several
        // consumers share this one channel (the Tauri subscriber, external
        // wait_for_change, the main.rs control handler). A small buffer let a
        // brief consumer stall drop low-frequency-but-critical control events
        // (SessionCloseRequest / AgentAdvancePhase) under a chunk flood. 1024
        // gives wide headroom; the main.rs handler also no longer blocks its
        // recv loop on slow work (it hands off to a serial worker).
        let (event_tx, _) = broadcast::channel(1024);
        Arc::new(Self {
            event_tx,
            pending: Mutex::new(HashMap::new()),
            violations,
            data_dir,
            session_projects: Mutex::new(HashMap::new()),
            session_awaiting: Mutex::new(HashMap::new()),
            session_activity: Mutex::new(HashMap::new()),
            storage: Mutex::new(None),
            app_handle: std::sync::OnceLock::new(),
            session_close_gate: Mutex::new(HashMap::new()),
            agent_health: std::sync::Mutex::new(HashMap::new()),
            reviewer_override: std::sync::Mutex::new(HashMap::new()),
            router_health: std::sync::Mutex::new(HashMap::new()),
            session_open_blocking: std::sync::Mutex::new(HashMap::new()),
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
        self.session_projects
            .lock()
            .await
            .insert(session_id, project);
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

    /// Hand the bridge a Weak ref to the session's ActivityTracker so
    /// `set_session_awaiting` can refresh the derived activity the moment it
    /// flips the awaiting flag (emit AwaitingUser without waiting for the next
    /// `set_busy`). Weak — see the `session_activity` field doc.
    pub async fn register_session_activity(&self, session_id: String, tracker: Weak<ActivityTracker>) {
        self.session_activity.lock().await.insert(session_id, tracker);
    }

    /// Register a session's open-blocking-findings count cache and return the
    /// shared `Arc` the router reads LOCK-FREE per forward. Seeds from storage so a
    /// re-spawned session with pre-existing findings starts at the right value (not
    /// 0). Mirrors `register_session_awaiting`.
    pub async fn register_open_blocking(&self, session_id: String) -> Arc<AtomicUsize> {
        let count = self.open_blocking_count(&session_id).await;
        let arc = Arc::new(AtomicUsize::new(count));
        self.session_open_blocking
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session_id, Arc::clone(&arc));
        arc
    }

    /// Recompute a session's open-blocking-findings count from storage into its
    /// cached `Arc` (no-op if the session isn't registered — headless / tests).
    /// COLD path: called only by the findings mutators after a change, never per
    /// forward. The map lock is released BEFORE the storage query, so it's never
    /// held across the `await`.
    pub async fn refresh_open_blocking(&self, session_id: &str) {
        let arc = self
            .session_open_blocking
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .cloned();
        let Some(arc) = arc else { return };
        let count = self.open_blocking_count(session_id).await;
        arc.store(count, Ordering::Release);
    }

    /// Clear the awaiting flag for a session — called by core.broadcast when
    /// the user sends a message (which resumes the duo).
    pub async fn clear_session_awaiting(&self, session_id: &str) {
        if let Some(flag) = self.session_awaiting.lock().await.get(session_id) {
            flag.store(false, Ordering::Release);
        }
    }

    /// Drop ALL of a session's bridge-side per-session map state when it closes.
    /// Without this, the per-session maps grow unbounded across open→close cycles —
    /// each closed session leaks an entry (and dangling `Arc`s) for the process
    /// lifetime. Idempotent — absent entries are fine. Called from
    /// `core::close_session`.
    pub async fn unregister_session(&self, session_id: &str) {
        self.session_projects.lock().await.remove(session_id);
        self.session_awaiting.lock().await.remove(session_id);
        self.session_activity.lock().await.remove(session_id);
        self.session_close_gate.lock().await.remove(session_id);
        self.agent_health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(s, _), _| s != session_id);
        self.reviewer_override
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id);
        // router_health — std::Mutex (mirrors reviewer_override above); the
        // forward-path `insert` is never otherwise paired with a remove.
        self.router_health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id);
        self.session_open_blocking
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id);
        // pending — tokio Mutex of parked choices; a non-blocking ask_user_choice
        // leaves a Parked entry whose receiver was dropped. Drop this session's.
        self.pending
            .lock()
            .await
            .retain(|_, p| p.choice.session_id != session_id);
    }

    /// A3b: record that the agent ran `cl_rescan` this session — a proxy for
    /// "appended a learnings delta", which lifts the close-delta gate.
    pub async fn mark_cl_rescan(&self, session_id: &str) {
        self.session_close_gate
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .cl_written = true;
    }

    /// A3b: should the agent's `close_session` be soft-gated with a
    /// write-then-prune reminder instead of closing? True only on the FIRST
    /// close when adherence nudges are on and no CL write happened this session;
    /// records the nudge so the agent's NEXT `close_session` proceeds. False
    /// when nudges are off, the CL was touched, or we already nudged once.
    pub async fn should_nudge_close(&self, session_id: &str) -> bool {
        let storage = self.storage.lock().await.clone();
        let Some(storage) = storage else {
            return false; // no storage wired (test/pre-init) — never gate
        };
        if !storage.adherence_nudges_enabled().await {
            return false;
        }
        let mut gate = self.session_close_gate.lock().await;
        let state = gate.entry(session_id.to_string()).or_default();
        if state.cl_written || state.close_nudged {
            return false;
        }
        state.close_nudged = true;
        true
    }

    // ---- Project helpers --------------------------------------------

    /// Best-effort lookup. Returns the registered project (if any) or None
    /// if the session isn't registered yet.
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

    /// Audit `<data_dir>/config/general-policy.yaml` + the project's policy.yaml for
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
    pub fn notify_message_persisted(&self, session_id: Arc<str>, message_id: i64) {
        let _ = self.event_tx.send(SignalingEvent::MessagePersisted {
            session_id,
            message_id,
        });
    }

    /// Fire a `HaltsCleared` event after pending awaiting-halt rows were flipped
    /// to answered, so the UI refetches `list_pending_tray` and the bell badge
    /// clears. Callers guard on `cleared > 0` so this only fires when a halt was
    /// actually pending. Fire-and-forget.
    pub fn notify_halts_cleared(&self, session_id: String) {
        let _ = self
            .event_tx
            .send(SignalingEvent::HaltsCleared { session_id });
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

    /// Publish an agent's retry-supervisor liveness change (B2). Fire-and-forget;
    /// the UI subscriber maps it to a `session:agent_health` event. `health` is
    /// the `AgentHealth::as_str` string ("running" / "retrying" / "dead").
    pub fn notify_agent_health(&self, session_id: String, agent: &str, health: &str) {
        // Batch 7: cache the latest health so the fail-closed commit gate can read
        // it (a Stalled/Dead duo reviewer blocks commit). Write BEFORE the move.
        self.agent_health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((session_id.clone(), agent.to_string()), health.to_string());
        // Batch 7: a recovered reviewer auto-clears any reviewer-down override —
        // the override is scoped to one down-incident, never persistent.
        if agent == "rain" && health == "running" {
            self.reviewer_override
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&session_id);
        }
        let _ = self.event_tx.send(SignalingEvent::AgentHealth {
            session_id,
            agent: agent.to_string(),
            health: health.to_string(),
        });
    }

    /// Latest cached health for an agent ("running"/"retrying"/"stalled"/"dead"),
    /// or `None` if no transition has been reported (assume running — events fire
    /// only on change). Backs the Batch 7 fail-closed commit gate.
    pub fn current_agent_health(&self, session_id: &str, agent: &str) -> Option<String> {
        self.agent_health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&(session_id.to_string(), agent.to_string()))
            .cloned()
    }

    /// Publish the peer-forward router's liveness change. Fire-and-forget; the UI
    /// subscriber maps it to `session:router_health`. Caches the latest state so
    /// `get_session_runtime` can seed the dot on a fresh mount.
    pub fn notify_router_health(&self, session_id: String, alive: bool) {
        self.router_health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session_id.clone(), alive);
        let _ = self
            .event_tx
            .send(SignalingEvent::RouterHealth { session_id, alive });
    }

    /// Latest cached router liveness for a session, or `None` if never reported
    /// (assume alive — the event fires only on change).
    pub fn current_router_health(&self, session_id: &str) -> Option<bool> {
        self.router_health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .copied()
    }

    /// Batch 7: HANDS records an explicit override of the reviewer-down commit
    /// block, with a reason (logged + surfaced in the gate response). The
    /// fail-closed escape valve — mirrors a finding rebuttal; never wedged.
    pub fn override_reviewer_block(&self, session_id: &str, reason: &str) -> String {
        tracing::warn!(
            session = %session_id,
            reason = %reason,
            "reviewer-down commit block OVERRIDDEN by HANDS"
        );
        self.reviewer_override
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session_id.to_string(), reason.to_string());
        format!(
            "reviewer-down block overridden — commit allowed. Logged reason: {reason}. \
             (Auto-clears when the reviewer recovers.)"
        )
    }

    /// The active reviewer-down override reason for a session, if any.
    pub fn reviewer_override_reason(&self, session_id: &str) -> Option<String> {
        self.reviewer_override
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .cloned()
    }

    /// Publish a session's duo-activity change (idle / busy / awaiting-user /
    /// cancelling). Fire-and-forget; the UI subscriber maps it to a
    /// `session:activity` event that gates the chat input + Cancel button.
    /// `state` is the `SessionActivity::as_str` string; `brian_busy`/`rain_busy`
    /// are the per-agent flags the UI uses to label which agent is working.
    pub fn notify_session_activity(
        &self,
        session_id: String,
        state: &str,
        brian_busy: bool,
        rain_busy: bool,
    ) {
        let _ = self.event_tx.send(SignalingEvent::SessionActivity {
            session_id,
            state: state.to_string(),
            brian_busy,
            rain_busy,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_health_registry_round_trips() {
        // Batch 7: notify_agent_health caches health per (session, agent); the
        // fail-closed commit gate reads it via current_agent_health.
        let bridge = SignalingBridge::new();
        assert_eq!(
            bridge.current_agent_health("s1", "rain"),
            None,
            "unset = None (assume running; events fire only on change)"
        );
        bridge.notify_agent_health("s1".into(), "rain", "stalled");
        assert_eq!(
            bridge.current_agent_health("s1", "rain").as_deref(),
            Some("stalled")
        );
        // Latest write wins (recovery overwrites).
        bridge.notify_agent_health("s1".into(), "rain", "running");
        assert_eq!(
            bridge.current_agent_health("s1", "rain").as_deref(),
            Some("running")
        );
        // Distinct agents + sessions stay independent.
        assert_eq!(bridge.current_agent_health("s1", "brian"), None);
        assert_eq!(bridge.current_agent_health("s2", "rain"), None);
    }
}
