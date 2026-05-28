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

use crate::policy::{Policy, ViolationKind, ViolationOutcome, ViolationsLog};
use crate::storage::{Author, ClIndexEntry, MessageKind, Project, Storage};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, Mutex};
use uuid::Uuid;

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
    /// session_id → granted permissions (commit / push grant scopes). The
    /// in-memory cache is the source of truth; we mirror to a JSON file under
    /// `<data_dir>/.local/session-permissions/<session_id>.json` so the git
    /// pre-push hook (a separate subprocess) can read the grant without an
    /// HTTP roundtrip. Dropped + file deleted by `cleanup_session_permissions`
    /// on session close.
    session_permissions: Mutex<HashMap<String, crate::policy::SessionPermissions>>,
    /// Tauri AppHandle, populated from `setup()` once the webview exists.
    /// Internal MCP `webview_*` tools (jsonrpc.rs) reach the webview through
    /// this — bridge is the only shared handle accessible to the per-agent
    /// dispatchers, which don't have CoreAppState. Set-once; `None` in tests
    /// and during the pre-setup window.
    app_handle: once_cell::sync::OnceCell<tauri::AppHandle>,
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
            session_permissions: Mutex::new(HashMap::new()),
            app_handle: once_cell::sync::OnceCell::new(),
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

    async fn set_session_awaiting(&self, session_id: &str) {
        if let Some(flag) = self.session_awaiting.lock().await.get(session_id) {
            flag.store(true, Ordering::Release);
        }
    }

    // ---- Session permissions ----------------------------------------

    /// Lock the session-permissions cache, apply `mutator` to the entry
    /// for `session_id` (creating a default entry if absent), then mirror
    /// the resulting state to `<data_dir>/.local/session-permissions/<sid>.json`
    /// so the pre-push hook can see it. Single source of truth for the
    /// lock→mutate→snapshot→mirror dance shared by every public grant /
    /// revoke / branch-add path.
    async fn mutate_session_permission(
        &self,
        session_id: &str,
        mutator: impl FnOnce(&mut crate::policy::SessionPermissions),
    ) -> Result<()> {
        let mut map = self.session_permissions.lock().await;
        let perm = map
            .entry(session_id.to_string())
            .or_insert_with(crate::policy::SessionPermissions::default);
        mutator(perm);
        let snapshot = perm.clone();
        drop(map);
        if let Some(data_dir) = &self.data_dir {
            crate::policy::session_permissions::write_session_permission(
                data_dir,
                session_id,
                &snapshot,
            )?;
        }
        Ok(())
    }

    /// Set the grant scope for `action` on this session. Overwrites any prior
    /// grant for the same action. Mirrors the new cache state to
    /// `<data_dir>/.local/session-permissions/<session_id>.json` so the
    /// pre-push hook can see it.
    pub async fn grant_session_permission(
        &self,
        session_id: &str,
        action: crate::policy::PermissionAction,
        scope: crate::policy::GrantScope,
    ) -> Result<()> {
        self.mutate_session_permission(session_id, |perm| perm.set(action, scope))
            .await
    }

    /// Revoke (reset to None) the grant for `action`. Idempotent on absent
    /// grants. Re-mirrors the file.
    pub async fn revoke_session_permission(
        &self,
        session_id: &str,
        action: crate::policy::PermissionAction,
    ) -> Result<()> {
        self.mutate_session_permission(session_id, |perm| {
            perm.set(action, crate::policy::GrantScope::None)
        })
        .await
    }

    /// Read the current permissions for this session. Returns the default
    /// (no grants) if nothing has been recorded.
    pub async fn list_session_permissions(
        &self,
        session_id: &str,
    ) -> crate::policy::SessionPermissions {
        self.session_permissions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Drop the cache entry + delete the mirrored file. Called by
    /// `core::state::close_session` when the session closes.
    pub async fn cleanup_session_permissions(&self, session_id: &str) -> Result<()> {
        self.session_permissions
            .lock()
            .await
            .remove(session_id);
        if let Some(data_dir) = &self.data_dir {
            crate::policy::session_permissions::delete_session_permission(
                data_dir,
                session_id,
            )?;
        }
        Ok(())
    }

    /// Internal helper: add `branch` to the existing push grant's Specific
    /// branch list, or upgrade None → Specific{[branch]}. AllBranches is left
    /// untouched (no point narrowing). Used by `resolve_choice` to record
    /// per-push approvals as a session-level grant.
    async fn add_branch_to_session_grant(
        &self,
        session_id: &str,
        action: crate::policy::PermissionAction,
        branch: String,
    ) -> Result<()> {
        self.mutate_session_permission(session_id, move |perm| {
            use crate::policy::GrantScope;
            let current = match action {
                crate::policy::PermissionAction::Commit => &mut perm.commit,
                crate::policy::PermissionAction::Push => &mut perm.push,
            };
            match current {
                GrantScope::None => {
                    *current = GrantScope::Specific {
                        branches: vec![branch],
                    };
                }
                GrantScope::Specific { branches } => {
                    if !branches.iter().any(|b| b == &branch) {
                        branches.push(branch);
                    }
                }
                GrantScope::AllBranches => { /* already broader */ }
            }
        })
        .await
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

    /// Called by the MCP `tools/call` handler for `ask_user_choice`.
    /// Awaits a response from the UI.
    ///
    /// Auto-supersedes the most recent pending question from this same
    /// `(session_id, agent)` — the new ask replaces the old one in the tray,
    /// the old gets `status='superseded'`, and the new row's `supersedes_id`
    /// points at the old. This kills the client-side-timeout-retry-duplicate
    /// cascade without relying on agent discipline.
    pub async fn ask_user_choice(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
    ) -> Result<String> {
        let supersedes_id = self
            .auto_supersede_prior_pending(&session_id, &agent)
            .await;
        self.ask_user_choice_inner(session_id, agent, question, options, None, supersedes_id)
            .await
    }

    /// Policy-initiated approval request. Same machinery as `ask_user_choice`
    /// (including auto-supersede of the latest pending from this agent) but
    /// carries an [`ApprovalContext`] so the resolve path can write a
    /// violation record.
    pub async fn request_approval(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        ctx: ApprovalContext,
    ) -> Result<String> {
        let supersedes_id = self
            .auto_supersede_prior_pending(&session_id, &agent)
            .await;
        self.ask_user_choice_inner(
            session_id,
            agent,
            question,
            options,
            Some(ctx),
            supersedes_id,
        )
        .await
    }

    /// Explicit supersede: agent passes the choice_id of a stale question
    /// they want to replace + the new question text/options. Same effect as
    /// `ask_user_choice` from the user's perspective (blocking call returning
    /// the eventual pick) but the linkage to a SPECIFIC stale row is
    /// deterministic (vs the auto-supersede heuristic which only catches the
    /// latest). Returns the picked option from the new question.
    pub async fn supersede_question_with_new(
        &self,
        session_id: String,
        agent: String,
        stale_choice_id: String,
        question: String,
        options: Vec<String>,
    ) -> Result<String> {
        // Look up the stale row by choice_id to capture its internal id (for
        // the new row's supersedes_id FK) BEFORE marking it superseded.
        let stale_internal_id = {
            let storage_guard = self.storage.lock().await;
            match storage_guard.as_ref() {
                Some(storage) => storage
                    .get_question(&stale_choice_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|q| q.id),
                None => None,
            }
        };
        // Flip status + drop parked oneshot + fire ChoiceResolved for the UI.
        {
            let storage_guard = self.storage.lock().await;
            if let Some(storage) = storage_guard.as_ref() {
                let _ = storage.supersede_question(&stale_choice_id).await;
            }
        }
        self.pending.lock().await.remove(&stale_choice_id);
        let _ = self.event_tx.send(SignalingEvent::ChoiceResolved {
            choice_id: stale_choice_id,
            picked: "(superseded)".to_string(),
        });
        // Post the new question with the supersedes_id link in place, then
        // block on its oneshot like a normal ask_user_choice would.
        self.ask_user_choice_inner(
            session_id,
            agent,
            question,
            options,
            None,
            stale_internal_id,
        )
        .await
    }

    /// Mark the latest pending question from `(session_id, agent)` as
    /// superseded + remove it from `pending`. Returns the internal id of the
    /// superseded row so the caller can link the new question via
    /// `supersedes_id`. None means no prior pending was found.
    ///
    /// Used by `ask_user_choice` / `request_approval` to auto-deduplicate
    /// when the same agent re-asks (G2 in the per-session-question-cache
    /// design — kills the timeout-retry-duplicate cascade at the bridge
    /// layer regardless of whether the agent remembered to call
    /// `withdraw_question` first).
    async fn auto_supersede_prior_pending(
        &self,
        session_id: &str,
        agent: &str,
    ) -> Option<i64> {
        let storage_guard = self.storage.lock().await;
        let storage = storage_guard.as_ref()?;
        let rows = storage.questions_for_session(session_id).await.ok()?;
        let latest = rows
            .into_iter()
            .rev()
            .find(|q| q.agent == agent && q.status == "pending")?;
        let stale_choice_id = latest.choice_id.clone();
        let stale_internal_id = latest.id;
        // Mark in storage first so the UI tray drops it on its next poll.
        let _ = storage.supersede_question(&stale_choice_id).await;
        drop(storage_guard);
        // Drop the parked oneshot so any (rare) still-listening client gets
        // the standard cancellation.
        self.pending.lock().await.remove(&stale_choice_id);
        // Tell the UI to clear its inline state for this choice.
        let _ = self.event_tx.send(SignalingEvent::ChoiceResolved {
            choice_id: stale_choice_id,
            picked: "(superseded)".to_string(),
        });
        Some(stale_internal_id)
    }

    async fn ask_user_choice_inner(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        approval: Option<ApprovalContext>,
        supersedes_id: Option<i64>,
    ) -> Result<String> {
        let choice_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel::<String>();
        let choice = PendingChoice {
            choice_id: choice_id.clone(),
            session_id: session_id.clone(),
            agent: agent.clone(),
            question: question.clone(),
            options: options.clone(),
            approval,
        };
        self.pending.lock().await.insert(
            choice_id.clone(),
            Parked {
                tx,
                choice: choice.clone(),
            },
        );

        // Mirror into the question cache for durable per-session state. The
        // in-chat tray + dashboard counter read from this table, and the row
        // survives restart even though the parked oneshot in `pending` does
        // not. Best-effort: if storage isn't wired yet (test bridges built
        // via ::new), continue without persisting.
        self.persist_question(
            &session_id,
            &choice_id,
            &agent,
            crate::storage::QuestionKind::Choice,
            &question,
            Some(&options),
            supersedes_id,
        )
        .await;

        // Halt the duo BEFORE emitting the event — the agent's next chunk
        // shouldn't volley to its peer while we wait for the user.
        self.set_session_awaiting(&session_id).await;

        // Best-effort broadcast. If no subscribers, the request still parks
        // until resolve_choice is called (mostly a concern for tests).
        let _ = self.event_tx.send(SignalingEvent::PendingChoice(choice));

        // Caller is the agent; block until UI resolves.
        let picked = rx.await.map_err(|_| {
            anyhow::anyhow!("ask_user_choice canceled before user picked an option")
        })?;
        let _ = self.event_tx.send(SignalingEvent::ChoiceResolved {
            choice_id,
            picked: picked.clone(),
        });
        Ok(picked)
    }

    /// Best-effort write of a question row to storage. The bridge's in-memory
    /// `pending` map is still the source of truth for the blocking oneshot,
    /// but the storage row is what the UI tray reads. Failures are logged
    /// and swallowed so the agent's tool call doesn't fail on a DB hiccup.
    #[allow(clippy::too_many_arguments)]
    async fn persist_question(
        &self,
        session_id: &str,
        choice_id: &str,
        agent: &str,
        kind: crate::storage::QuestionKind,
        prompt: &str,
        options: Option<&[String]>,
        supersedes_id: Option<i64>,
    ) {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return;
        };
        if let Err(e) = storage
            .insert_question(session_id, choice_id, agent, kind, prompt, options, supersedes_id)
            .await
        {
            tracing::warn!(?e, choice_id, "persist_question failed");
        }
    }

    /// Withdraw a pending question (agent abandons it; no answer will arrive).
    /// Removes the parked oneshot AND updates the storage row to status=withdrawn
    /// so the UI tray drops it. Returns true if a question was actually withdrawn,
    /// false if the choice_id was unknown or already resolved.
    pub async fn withdraw_question(&self, choice_id: &str) -> bool {
        let parked = self.pending.lock().await.remove(choice_id);
        let was_parked = parked.is_some();
        // Drop the oneshot — the agent's blocking caller (if any) gets the
        // standard "canceled" error.
        drop(parked);
        let storage_guard = self.storage.lock().await;
        if let Some(storage) = storage_guard.as_ref() {
            if let Err(e) = storage.withdraw_question(choice_id).await {
                tracing::warn!(?e, choice_id, "withdraw_question storage update failed");
            }
        }
        was_parked
    }

    /// Snapshot the questions table for a session. Convenience for the UI
    /// + the agent-facing `list_my_pending_questions` MCP tool.
    pub async fn list_questions_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::storage::SessionQuestion>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage.questions_for_session(session_id).await
    }

    /// Count pending questions for a session — drives the dashboard
    /// `[Need User Input] · N` badge.
    pub async fn pending_question_count(&self, session_id: &str) -> Result<i64> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(0);
        };
        storage.pending_question_count(session_id).await
    }

    /// Called by the UI when the user clicks a choice button.
    pub async fn resolve_choice(&self, choice_id: &str, picked: String) -> Result<ResolveOutcome> {
        // Mark the storage row answered first so the UI/tray updates even if
        // the in-memory parked entry has already been cleaned up (e.g. after
        // a restart that replays from the table).
        {
            let storage_guard = self.storage.lock().await;
            if let Some(storage) = storage_guard.as_ref() {
                if let Err(e) = storage.answer_question(choice_id, &picked).await {
                    tracing::warn!(?e, choice_id, "answer_question storage update failed");
                }
            }
        }
        let parked = self.pending.lock().await.remove(choice_id);
        match parked {
            Some(p) => {
                // Write violation record FIRST (before unblocking the agent)
                // so the audit trail captures the decision even if the agent
                // crashes immediately after receiving the result.
                let outcome = outcome_from_picked(&picked);
                if let (Some(log), Some(ctx)) =
                    (self.violations.as_ref(), &p.choice.approval)
                {
                    let _ = log
                        .record(
                            p.choice.session_id.clone(),
                            p.choice.agent.clone(),
                            ctx.kind,
                            ctx.action.clone(),
                            outcome,
                            ctx.detail.clone(),
                        )
                        .await;
                }

                // Approved push_gate via `request_approval` ALSO records a
                // session-level push grant for the branch — so a second push to
                // the same branch in this session won't re-prompt. The grant
                // persists in the in-memory cache + mirrored JSON file; it is
                // wiped on session close + on bot-hq restart.
                if let (Some(ctx), Some(_)) = (&p.choice.approval, &self.data_dir) {
                    if matches!(ctx.kind, crate::policy::ViolationKind::PushGate)
                        && matches!(outcome, crate::policy::ViolationOutcome::Approved)
                    {
                        if let Some(branch) = parse_push_branch(&ctx.action) {
                            if let Err(e) = self
                                .add_branch_to_session_grant(
                                    &p.choice.session_id,
                                    crate::policy::PermissionAction::Push,
                                    branch.clone(),
                                )
                                .await
                            {
                                tracing::warn!(
                                    ?e,
                                    branch = %branch,
                                    session_id = %p.choice.session_id,
                                    "session-grant update failed; pre-push hook may re-block"
                                );
                            }
                        }
                    }
                }

                match p.tx.send(picked) {
                    Ok(()) => Ok(ResolveOutcome::Delivered),
                    Err(picked) => {
                        // The agent's blocking `ask_user_choice` tool call client-side
                        // timed out before we got the user's pick. The answer is still
                        // captured (violations log + remembered_approvals are already
                        // written above) — persist an out-of-band synthetic user message
                        // so the UI / message-poll callers see the resolution.
                        // CoreAppState::resolve_choice is the one that ALSO routes the
                        // body through the duo input channels to wake the subprocess.
                        let session_id = p.choice.session_id.clone();
                        let body =
                            oob_resolution_body(&p.choice.agent, &p.choice.question, &picked);
                        let storage_guard = self.storage.lock().await;
                        if let Some(storage) = storage_guard.as_ref() {
                            if let Err(e) = storage
                                .insert_message(
                                    &session_id,
                                    Author::User,
                                    MessageKind::Text,
                                    &body,
                                )
                                .await
                            {
                                tracing::warn!(
                                    ?e,
                                    %session_id,
                                    "out-of-band choice-resolution message failed to persist"
                                );
                            }
                        } else {
                            tracing::warn!(
                                %session_id,
                                "resolve_choice: agent receiver dropped AND no storage wired — \
                                 pick recorded but not delivered"
                            );
                        }
                        Ok(ResolveOutcome::AgentReceiverDroppedFellBack { session_id, body })
                    }
                }
            }
            None => {
                // No in-memory parked oneshot for this choice_id. The common
                // cause is the #2 reopened-session bug: the session was closed
                // (subprocess killed, its oneshot dropped) then reopened; the
                // resumed agent re-asked with a NEW choice_id while the user
                // answered the OLD one still shown in their tray. Previously
                // this arm errored, so `answer_question` (above) cleared the
                // tray but the pick never reached the live agent — it waited
                // forever. Instead, reconstruct the question from the durable
                // session_questions row and fall back to OOB stdin delivery so
                // CoreAppState injects the answer into the live (respawned)
                // session. Stdin injection is the only channel to a resumed
                // subprocess — re-parking a oneshot across a PID boundary is
                // impossible.
                let q = {
                    let storage_guard = self.storage.lock().await;
                    match storage_guard.as_ref() {
                        Some(storage) => storage.get_question(choice_id).await?,
                        None => None,
                    }
                };
                let Some(q) = q else {
                    return Err(anyhow::anyhow!("no pending choice with id {choice_id}"));
                };
                let body = oob_resolution_body(&q.agent, &q.prompt, &picked);
                {
                    let storage_guard = self.storage.lock().await;
                    if let Some(storage) = storage_guard.as_ref() {
                        if let Err(e) = storage
                            .insert_message(&q.session_id, Author::User, MessageKind::Text, &body)
                            .await
                        {
                            tracing::warn!(
                                ?e,
                                session_id = %q.session_id,
                                "OOB (reopened-session) choice-resolution message failed to persist"
                            );
                        }
                    }
                }
                Ok(ResolveOutcome::AgentReceiverDroppedFellBack {
                    session_id: q.session_id,
                    body,
                })
            }
        }
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

    /// Snapshot the currently-parked choices. Used by the external MCP driver
    /// so it can see what's waiting for user input + the choice_ids needed to
    /// resolve them.
    pub async fn list_pending_choices(&self) -> Vec<PendingChoice> {
        self.pending
            .lock()
            .await
            .values()
            .map(|p| p.choice.clone())
            .collect()
    }

    /// Called by the MCP `tools/call` handler for `mark_awaiting_user`. This
    /// is async (was previously sync) because we need to set the halt flag
    /// before the agent's next chunk can volley.
    ///
    /// Also writes a `kind=halt` row to `session_questions` so the per-session
    /// tray surfaces the wait alongside any actual choice/open-ask questions
    /// and the dashboard `[Need User Input] · N` counter reflects it.
    pub async fn mark_awaiting_user(&self, session_id: String, agent: String, reason: String) {
        self.set_session_awaiting(&session_id).await;
        let choice_id = Uuid::new_v4().to_string();
        self.persist_question(
            &session_id,
            &choice_id,
            &agent,
            crate::storage::QuestionKind::Halt,
            &reason,
            None,
            None,
        )
        .await;
        let _ = self.event_tx.send(SignalingEvent::AwaitingUser {
            session_id,
            agent,
            reason,
        });
    }

    /// Agent-initiated IPAV phase advance request. Persists a chat message
    /// authored by the requesting agent (so the scroll shows the ask inline)
    /// and a halt question (so the tray + dashboard counter reflect it), then
    /// sets the awaiting flag so the duo's peer-forward halts until the user acts.
    ///
    /// The user has two response paths, both clear the halt:
    ///   1. Click the phase chip → `AppState::advance_phase` (which also
    ///      clears awaiting + answers pending halt rows).
    ///   2. Type a reply → `AppState::broadcast` (which always clears halt
    ///      on user input). Implicit decline — phase stays put.
    pub async fn request_phase_advance(
        &self,
        session_id: String,
        agent: String,
        target: String,
        reason: String,
    ) {
        let body = format!("[PHASE REQUEST -> {target}] {reason}");
        {
            let storage_guard = self.storage.lock().await;
            if let Some(storage) = storage_guard.as_ref() {
                let author = crate::storage::Author::parse(&agent)
                    .unwrap_or(crate::storage::Author::User);
                match storage
                    .insert_message(&session_id, author, crate::storage::MessageKind::Text, &body)
                    .await
                {
                    Ok(id) => self.notify_message_persisted(session_id.clone(), id),
                    Err(e) => {
                        tracing::warn!(?e, "request_phase_advance insert_message failed")
                    }
                }
            }
        }
        self.set_session_awaiting(&session_id).await;
        let choice_id = Uuid::new_v4().to_string();
        self.persist_question(
            &session_id,
            &choice_id,
            &agent,
            crate::storage::QuestionKind::Halt,
            &body,
            None,
            None,
        )
        .await;
        let _ = self.event_tx.send(SignalingEvent::AwaitingUser {
            session_id,
            agent,
            reason: body,
        });
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

    /// For tests / introspection: how many choices are parked?
    pub async fn pending_choice_count(&self) -> usize {
        self.pending.lock().await.len()
    }

    // ---- Context Library (CL) index ------------------------------------

    /// Resolve the on-disk root for a project's CL files. `_globals` maps to
    /// the bot-hq data dir itself; named projects honor `projects.cl_path`
    /// when set, otherwise fall back to `<data_dir>/projects/<name>`.
    /// Returns None only when the bridge has no `data_dir` configured (test
    /// bridges built via `new()`).
    ///
    /// Clones the Storage handle out of the mutex before awaiting so callers
    /// holding the bridge mutex (e.g. `cl_rescan`) don't deadlock when this
    /// method tries to re-lock for its own lookup.
    pub(crate) async fn cl_project_root(&self, project: &str) -> Option<PathBuf> {
        let data_dir = self.data_dir.as_ref()?.clone();
        if project == Project::GLOBALS {
            return Some(data_dir);
        }
        let storage = self.storage.lock().await.clone();
        match storage {
            Some(storage) => storage
                .cl_path_for_project(&data_dir, project)
                .await
                .ok(),
            None => Some(data_dir.join("projects").join(project)),
        }
    }

    /// Read-side discovery for agents. Wraps storage.cl_index_search.
    pub async fn cl_index_search(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClIndexEntry>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage.cl_index_search(project, query).await
    }

    // ---- session_documents -----------------------------------------

    /// Agent-callable: upsert a per-session scratch document by slug.
    /// Optional `phase` tags the doc for the IPAV document tab + phase-filtered search.
    pub async fn session_doc_write(
        &self,
        session_id: &str,
        slug: &str,
        body: &str,
        phase: Option<&str>,
    ) -> Result<i64> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Err(anyhow::anyhow!("storage not configured"));
        };
        storage
            .upsert_session_document(session_id, slug, body, phase)
            .await
    }

    /// Agent-callable: search this session's docs (slug + body substring).
    /// Optional `phase` restricts results to docs tagged with that IPAV phase.
    pub async fn session_doc_search(
        &self,
        session_id: &str,
        query: Option<&str>,
        phase: Option<&str>,
    ) -> Result<Vec<crate::storage::SessionDocument>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage
            .session_documents_for(session_id, query, phase)
            .await
    }

    /// Agent-callable: read one session doc by slug.
    pub async fn session_doc_read(
        &self,
        session_id: &str,
        slug: &str,
    ) -> Result<Option<crate::storage::SessionDocument>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(None);
        };
        storage.session_document_by_slug(session_id, slug).await
    }

    /// Read-side discovery for FOLDER descriptions. Parallel to
    /// [`cl_index_search`]. Returns lightweight rows; empty list when storage
    /// isn't configured (test bridges).
    pub async fn cl_folder_search(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<crate::storage::ClFolder>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage.cl_folder_search(project, query).await
    }

    /// Write-side for agents: upsert a folder description. The jsonrpc layer
    /// gates this to HANDS (brian) + Emma; Rain is denied.
    pub async fn cl_register_folder_description(
        &self,
        project: &str,
        folder_path: &str,
        description: &str,
        tags: Option<&str>,
    ) -> Result<()> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(());
        };
        // Ensure the project row exists. _globals is bootstrapped by the
        // initial migration; named projects might not have an upsert yet.
        if project != Project::GLOBALS {
            storage
                .upsert_project(project, project, None, None, None)
                .await?;
        }
        storage
            .upsert_folder_description(project, folder_path, description, tags)
            .await?;
        Ok(())
    }

    /// Record an audit row: this agent (in this session) read this file.
    /// Looks up the cl_index row by (project, file_path); silently no-ops
    /// if the index doesn't know about the file yet.
    pub async fn cl_register_read(
        &self,
        agent: &str,
        session_id: Option<&str>,
        project: &str,
        file_path: &str,
    ) -> Result<()> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(());
        };
        let Some(entry) = storage.get_cl_index(project, file_path).await? else {
            tracing::debug!(
                project,
                file_path,
                "cl_register_read: no index row for path; skipping audit insert"
            );
            return Ok(());
        };
        storage.record_cl_read(entry.id, session_id, agent).await
    }

    /// Diff a project's on-disk CL directory against the index. Three outcomes:
    ///   - added:    file on disk, no index row → insert with auto-extracted description
    ///   - touched:  index row exists, mtime newer than stored updated_at → bump
    ///   - orphaned: index row exists, file gone → list (but DO NOT auto-delete; user decides)
    pub async fn cl_rescan(&self, project: &str) -> Result<ClRescanReport> {
        let mut report = ClRescanReport::default();
        // Clone the Storage handle out of the bridge mutex BEFORE calling
        // cl_project_root, which also acquires the same mutex. tokio's Mutex
        // is not reentrant — holding the guard across that call deadlocks.
        let storage = match self.storage.lock().await.clone() {
            Some(s) => s,
            None => return Ok(report),
        };
        let Some(root) = self.cl_project_root(project).await else {
            return Ok(report);
        };
        if !root.is_dir() {
            return Ok(report);
        }

        // Walk disk; collect (relative_path, mtime_rfc3339, first_h1_or_snippet).
        let mut on_disk: HashMap<String, (String, String)> = HashMap::new();
        walk_cl_dir(&root, &root, project, &mut on_disk);

        // Existing rows.
        let existing = storage.cl_index_search(Some(project), None).await?;
        let existing_paths: HashSet<String> =
            existing.iter().map(|e| e.file_path.clone()).collect();

        // Adds + touches.
        for (rel, (mtime, snippet)) in &on_disk {
            match existing.iter().find(|e| &e.file_path == rel) {
                None => {
                    storage
                        .upsert_cl_index(project, rel, snippet, None)
                        .await?;
                    report.added.push(rel.clone());
                }
                Some(row) if row.updated_at.as_str() < mtime.as_str() => {
                    storage.touch_cl_index(project, rel, mtime).await?;
                    report.touched.push(rel.clone());
                }
                _ => {}
            }
        }

        // Orphans (index has it, disk doesn't): the file was deleted on disk
        // OUTSIDE bot-hq (via `rm`, an editor, or a user cleanup). The index
        // pointer is now pointing at /dev/null; auto-purge so cl_index_search
        // never returns a row the agent can't actually read.
        for path in &existing_paths {
            if !on_disk.contains_key(path) {
                let _ = storage.delete_cl_index(project, path).await;
                report.orphaned.push(path.clone());
            }
        }
        Ok(report)
    }
}

/// Walk `dir` recursively; for each text-ish file (.md, .yaml, .txt) populate
/// `out` with (relative_path, mtime_iso8601, description_snippet). Skips
/// hidden files/dirs (anything starting with '.') and a few well-known noise
/// directories (`projects` at the data_dir level is handled by per-project
/// rescans, not here).
fn walk_cl_dir(
    dir: &Path,
    root: &Path,
    project: &str,
    out: &mut HashMap<String, (String, String)>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        // At the _globals root, the per-project subdirectories show up under
        // `projects/` — skip them; they'll be rescanned with their own
        // project name. Same for `.local` (sqlite + per-project lock state).
        if project == Project::GLOBALS && dir == root && (name == "projects" || name == ".local") {
            continue;
        }
        if path.is_dir() {
            walk_cl_dir(&path, root, project, out);
            continue;
        }
        // Only index human-readable text-ish files. Binary / large data files
        // don't belong in the agent's discovery surface.
        let is_text = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "yaml" | "yml" | "txt" | "toml" | "json")
        );
        if !is_text {
            continue;
        }
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        let mtime = match entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(chrono::DateTime::<chrono::Utc>::from)
        {
            Some(t) => t.to_rfc3339(),
            None => continue,
        };
        let snippet = extract_description(&path);
        out.insert(rel, (mtime, snippet));
    }
}

/// First H1 (`# ...`) line; failing that, the first non-empty line trimmed
/// to 80 chars. Used to seed `cl_index.description` when an entry is auto-
/// added during a rescan. User can edit later via the UI.
fn extract_description(path: &Path) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.trim().to_string();
        }
    }
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() <= 80 {
            return trimmed.to_string();
        }
        return trimmed.chars().take(80).collect::<String>() + "…";
    }
    "(empty file)".to_string()
}

/// Parse the branch from a push-gate `action` string. Accepts shapes like
/// `"git push origin <branch>"`, `"git push --force origin <branch>"`, and
/// `"git push origin <branch>:<remote-ref>"`. Returns None for unparseable
/// inputs (no false-positive remembered approvals).
fn parse_push_branch(action: &str) -> Option<String> {
    let after_push = action.trim().strip_prefix("git push")?.trim_start();
    let tokens: Vec<&str> = after_push.split_whitespace().collect();
    // Refuse if the user is doing something other than "push this branch":
    // --delete/-d removes a remote ref, which we don't want to silently
    // remember as an approved push target.
    if tokens.iter().any(|t| *t == "--delete" || *t == "-d") {
        return None;
    }
    // Strip flag-shaped tokens. The first two remaining positional tokens are
    // <remote> <ref>. If only one is present (e.g. `git push -u origin`), bail
    // rather than guess.
    let positionals: Vec<&str> = tokens.iter().copied().filter(|t| !t.starts_with('-')).collect();
    let branch_arg = match positionals.as_slice() {
        [_remote, branch, ..] => *branch,
        _ => return None,
    };
    let branch = branch_arg.split(':').next().unwrap_or(branch_arg).trim();
    if branch.is_empty() {
        return None;
    }
    Some(branch.to_string())
}

#[cfg(test)]
#[test]
fn parse_push_branch_shapes() {
    assert_eq!(
        parse_push_branch("git push origin 346-streamline-onboarding-process"),
        Some("346-streamline-onboarding-process".into())
    );
    assert_eq!(
        parse_push_branch("git push origin main:release"),
        Some("main".into())
    );
    assert_eq!(parse_push_branch("git push --force origin main"), Some("main".into()));
    assert_eq!(parse_push_branch("not a push command"), None);
    assert_eq!(parse_push_branch("git push origin --delete x"), None); // safety: refuse flag-looking branches
}

/// Map a picked option string to an outcome enum. Anything that starts with
/// "approve" (case-insensitive) counts as Approved; everything else Denied.
/// Abandoned isn't reachable via resolve_choice (that path requires a pick).
fn outcome_from_picked(picked: &str) -> ViolationOutcome {
    let lower = picked.to_lowercase();
    if lower.starts_with("approve") || lower == "ok" || lower == "yes" {
        ViolationOutcome::Approved
    } else {
        ViolationOutcome::Denied
    }
}

/// Build the out-of-band "your question resolved" message body fed back to an
/// agent that is no longer blocked on the original `ask_user_choice` tool
/// call — either because the MCP call timed out client-side, or because the
/// session was closed + reopened and the asking subprocess was replaced.
/// Shared by both resolve_choice fallbacks (dropped-receiver and the
/// reopened-session `None` path) so the wording stays identical.
fn oob_resolution_body(agent_label: &str, question: &str, picked: &str) -> String {
    format!(
        "(out-of-band) Your earlier `ask_user_choice` for {agent_label} resolved while \
         you were no longer waiting on the tool call.\n\n\
         **Question:** {question}\n\
         **User picked:** {picked}\n\n\
         Treat this as the user's reply. Continue from here."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ask_user_choice_round_trip() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s1".into(),
                    "brian".into(),
                    "pick".into(),
                    vec!["a".into(), "b".into()],
                )
                .await
                .unwrap()
        });
        // First event should be PendingChoice.
        let ev = sub.recv().await.unwrap();
        let choice_id = match ev {
            SignalingEvent::PendingChoice(p) => p.choice_id,
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge.resolve_choice(&choice_id, "b".into()).await.unwrap();
        let picked = ask.await.unwrap();
        assert_eq!(picked, "b");
        // Next event should be ChoiceResolved.
        let ev2 = sub.recv().await.unwrap();
        assert!(matches!(ev2, SignalingEvent::ChoiceResolved { picked: p, .. } if p == "b"));
    }

    #[tokio::test]
    async fn mark_awaiting_user_broadcasts() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        bridge
            .mark_awaiting_user("s1".into(), "brian".into(), "ping".into())
            .await;
        let ev = sub.recv().await.unwrap();
        assert!(matches!(ev, SignalingEvent::AwaitingUser { session_id, agent, reason }
            if session_id == "s1" && agent == "brian" && reason == "ping"));
    }

    #[tokio::test]
    async fn resolve_unknown_choice_errors() {
        // No storage + no parked oneshot → genuinely unknown id → error.
        let bridge = SignalingBridge::new();
        let err = bridge
            .resolve_choice("nope", "x".into())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no pending choice"));
    }

    #[tokio::test]
    async fn resolve_reopened_session_choice_falls_back_to_oob() {
        // #2: after close+reopen the user may answer a choice_id whose parked
        // oneshot died with the old subprocess. The durable question row still
        // exists. resolve_choice must NOT error — it reconstructs the question
        // and returns the OOB fallback so the answer reaches the live agent.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s-reopen", "t", None).await.unwrap();
        let opts = vec!["Yes".to_string(), "No".to_string()];
        storage
            .insert_question(
                "s-reopen",
                "old-choice-id",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Ship it?",
                Some(&opts),
                None,
            )
            .await
            .unwrap();

        // No parked oneshot in the in-memory map (post-reopen state).
        let outcome = bridge
            .resolve_choice("old-choice-id", "Yes".into())
            .await
            .expect("reopened-session resolve should fall back, not error");
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { session_id, body } => {
                assert_eq!(session_id, "s-reopen");
                assert!(body.contains("Ship it?"), "body: {body}");
                assert!(body.contains("Yes"), "body: {body}");
            }
            other => panic!("expected OOB fallback, got {other:?}"),
        }
        // OOB message persisted for the agent to read on its next turn.
        let msgs = storage.messages_for_session("s-reopen", None).await.unwrap();
        assert!(msgs
            .iter()
            .any(|m| m.content.contains("(out-of-band)") && m.content.contains("Yes")));
        // Question row marked answered so the tray clears.
        let q = storage.get_question("old-choice-id").await.unwrap().unwrap();
        assert_eq!(q.status, "answered");
    }

    #[tokio::test]
    async fn resolve_after_agent_drop_falls_back_to_message() {
        // Simulates: agent calls ask_user_choice → claude-code MCP client
        // times out → drops the receiver. Some time later the orchestrator
        // calls resolve_choice. We expect Ok + a synthetic user message
        // persisted to storage so the agent learns the answer on next poll.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        // Seed a session row so the FK in messages is satisfied.
        storage
            .create_session("s-fallback", "title", None)
            .await
            .unwrap();

        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let asker = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s-fallback".into(),
                    "brian".into(),
                    "Pick something?".into(),
                    vec!["A".into(), "B".into()],
                )
                .await
        });
        // Grab the choice_id from the broadcast event.
        let choice_id = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        // Simulate client-side timeout: abort the agent's future, then yield
        // so the drop runs and the oneshot::Receiver is gone.
        asker.abort();
        let _ = asker.await; // collect the JoinError; we expect Aborted.
        tokio::task::yield_now().await;

        // Orchestrator resolves the (now-orphaned) choice.
        let outcome = bridge
            .resolve_choice(&choice_id, "A".into())
            .await
            .expect("resolve_choice should succeed even when agent receiver dropped");

        // Verify we surfaced the wake info to the caller so CoreAppState can
        // route the body through input_tx and actually unblock the subprocess.
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { session_id, body } => {
                assert_eq!(session_id, "s-fallback");
                assert!(body.contains("User picked:"));
                assert!(body.contains("A"));
            }
            other => panic!("expected AgentReceiverDroppedFellBack, got {other:?}"),
        }

        // Verify the out-of-band message also landed in storage (for UI + poll).
        let msgs = storage
            .messages_for_session("s-fallback", None)
            .await
            .unwrap();
        let oob = msgs
            .iter()
            .find(|m| m.content.contains("(out-of-band)"))
            .expect("expected synthetic out-of-band message");
        assert_eq!(oob.author, "user");
        assert!(oob.content.contains("User picked:"));
        assert!(oob.content.contains("A"));
    }

    #[tokio::test]
    async fn delivered_outcome_when_agent_receives() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s1".into(),
                    "brian".into(),
                    "pick".into(),
                    vec!["a".into(), "b".into()],
                )
                .await
                .unwrap()
        });
        let choice_id = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        let outcome = bridge.resolve_choice(&choice_id, "b".into()).await.unwrap();
        let _ = ask.await.unwrap();
        assert!(matches!(outcome, ResolveOutcome::Delivered));
    }

    #[tokio::test]
    async fn request_approval_records_violation_on_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "s1".into(),
                    "brian".into(),
                    "Approve push?".into(),
                    vec!["Approve once".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push origin main".into(),
                        detail: Some("per_branch_approval".into()),
                    },
                )
                .await
                .unwrap()
        });
        let ev = sub.recv().await.unwrap();
        let choice_id = match ev {
            SignalingEvent::PendingChoice(p) => {
                assert!(p.approval.is_some());
                p.choice_id
            }
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge
            .resolve_choice(&choice_id, "Approve once".into())
            .await
            .unwrap();
        let picked = ask.await.unwrap();
        assert_eq!(picked, "Approve once");

        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, ViolationKind::PushGate);
        assert_eq!(recs[0].outcome, ViolationOutcome::Approved);
        assert_eq!(recs[0].action, "git push origin main");
    }

    #[tokio::test]
    async fn approved_push_gate_records_session_grant() {
        // After an approved push_gate via request_approval, the bridge records
        // a session-level push grant for that branch — so a second push to the
        // same branch in this session won't re-prompt. The grant lives in the
        // in-memory cache + a mirrored JSON file the hook can read.
        let dir = tempfile::tempdir().unwrap();
        let project = "test-project";
        let session_id = "session-A";
        let proj_dir = dir.path().join("projects").join(project);
        std::fs::create_dir_all(&proj_dir).unwrap();
        std::fs::write(
            proj_dir.join("policy.yaml"),
            "push_gate:\n  mode: per_branch_approval\n",
        )
        .unwrap();

        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_policy(log, dir.path().to_path_buf());
        bridge
            .register_session(session_id.to_string(), Some(project.to_string()))
            .await;

        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    session_id.into(),
                    "brian".into(),
                    "Approve push?".into(),
                    vec!["Approve push".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push origin 346-streamline-onboarding-process".into(),
                        detail: Some("per_branch_approval".into()),
                    },
                )
                .await
                .unwrap()
        });
        let ev = sub.recv().await.unwrap();
        let cid = match ev {
            SignalingEvent::PendingChoice(p) => p.choice_id,
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge
            .resolve_choice(&cid, "Approve push".into())
            .await
            .unwrap();
        let _ = ask.await.unwrap();

        // Cache: push grant now includes the branch.
        let perm = bridge.list_session_permissions(session_id).await;
        assert!(perm.allows_push("346-streamline-onboarding-process"));
        assert!(!perm.allows_push("some-other-branch"));

        // File mirror: the hook can read the same grant.
        let mirrored = crate::policy::session_permissions::read_session_permission(
            dir.path(),
            session_id,
        )
        .unwrap()
        .unwrap();
        assert!(mirrored.allows_push("346-streamline-onboarding-process"));

        // cleanup_session_permissions wipes both cache + file.
        bridge.cleanup_session_permissions(session_id).await.unwrap();
        let after = bridge.list_session_permissions(session_id).await;
        assert!(!after.allows_push("346-streamline-onboarding-process"));
        assert!(crate::policy::session_permissions::read_session_permission(
            dir.path(),
            session_id,
        )
        .unwrap()
        .is_none());
    }

    #[tokio::test]
    async fn denied_push_gate_does_not_persist() {
        let dir = tempfile::tempdir().unwrap();
        let project = "deny-test";
        let proj_dir = dir.path().join("projects").join(project);
        std::fs::create_dir_all(&proj_dir).unwrap();
        std::fs::write(
            proj_dir.join("policy.yaml"),
            "push_gate:\n  mode: per_branch_approval\n",
        )
        .unwrap();

        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_policy(log, dir.path().to_path_buf());
        bridge
            .register_session("s".to_string(), Some(project.to_string()))
            .await;

        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "s".into(),
                    "brian".into(),
                    "?".into(),
                    vec!["Approve push".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push origin denied-branch".into(),
                        detail: None,
                    },
                )
                .await
                .unwrap()
        });
        let ev = sub.recv().await.unwrap();
        let cid = match ev {
            SignalingEvent::PendingChoice(p) => p.choice_id,
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge.resolve_choice(&cid, "Deny".into()).await.unwrap();
        let _ = ask.await.unwrap();

        // No session grant should have been recorded for the denied branch.
        let perm = bridge.list_session_permissions("s").await;
        assert!(!perm.allows_push("denied-branch"));
        assert!(
            crate::policy::session_permissions::read_session_permission(
                dir.path(),
                "s",
            )
            .unwrap()
            .is_none(),
            "no permission file should be written for a denied push"
        );
    }

    #[tokio::test]
    async fn deny_picked_records_denied_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "s1".into(),
                    "brian".into(),
                    "Approve force-push?".into(),
                    vec!["Approve".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::ForcePush,
                        action: "git push --force origin main".into(),
                        detail: None,
                    },
                )
                .await
                .unwrap()
        });
        let ev = sub.recv().await.unwrap();
        let choice_id = match ev {
            SignalingEvent::PendingChoice(p) => p.choice_id,
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge.resolve_choice(&choice_id, "Deny".into()).await.unwrap();
        ask.await.unwrap();
        let recs = log.read_all().unwrap();
        assert_eq!(recs[0].outcome, ViolationOutcome::Denied);
    }

    #[tokio::test]
    async fn ask_user_choice_does_not_write_violation() {
        let dir = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s1".into(),
                    "brian".into(),
                    "pick".into(),
                    vec!["a".into(), "b".into()],
                )
                .await
                .unwrap()
        });
        let ev = sub.recv().await.unwrap();
        let choice_id = match ev {
            SignalingEvent::PendingChoice(p) => {
                assert!(p.approval.is_none());
                p.choice_id
            }
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge.resolve_choice(&choice_id, "a".into()).await.unwrap();
        ask.await.unwrap();
        let recs = log.read_all().unwrap();
        assert!(recs.is_empty(), "plain ask_user_choice should not log");
    }

    #[tokio::test]
    async fn ask_user_choice_auto_supersedes_prior_pending() {
        // G2: when the same agent re-asks in the same session, the prior
        // pending question gets flipped to 'superseded' automatically and
        // the new row links back via supersedes_id. Without this, a
        // timed-out re-ask would accumulate duplicates in the tray.
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("test.db")).await.unwrap();
        storage.create_session("s1", "test", None).await.unwrap();

        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;

        let bridge_clone = Arc::clone(&bridge);
        let first = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s1".into(),
                    "brian".into(),
                    "first".into(),
                    vec!["a".into(), "b".into()],
                )
                .await
        });
        // Give the first ask a moment to land in storage.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Second ask from same agent should auto-supersede the first.
        let bridge_clone = Arc::clone(&bridge);
        let second = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s1".into(),
                    "brian".into(),
                    "second".into(),
                    vec!["x".into(), "y".into()],
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let rows = storage.questions_for_session("s1").await.unwrap();
        assert_eq!(rows.len(), 2, "two question rows expected");
        let first_row = &rows[0];
        let second_row = &rows[1];
        assert_eq!(first_row.prompt, "first");
        assert_eq!(first_row.status, "superseded");
        assert_eq!(second_row.prompt, "second");
        assert_eq!(second_row.status, "pending");
        assert_eq!(
            second_row.supersedes_id,
            Some(first_row.id),
            "new row should link back to the superseded row"
        );

        // The first ask's oneshot was dropped by auto-supersede; its task
        // will resolve with an error. The second task is still parked —
        // resolve it so the test cleans up.
        bridge
            .resolve_choice(&second_row.choice_id, "x".into())
            .await
            .unwrap();
        let _ = first.await.unwrap();
        let _ = second.await.unwrap();
    }

    #[tokio::test]
    async fn supersede_question_links_old_to_new() {
        // G1: the explicit supersede tool replaces a SPECIFIC stale by
        // choice_id and links the new row to it via supersedes_id.
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("test.db")).await.unwrap();
        storage.create_session("s1", "test", None).await.unwrap();

        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;

        // Seed a stale question directly via storage (skip the auto-
        // supersede path so we have a clean "stale exists, nothing else
        // pending" state for the explicit tool to target).
        storage
            .insert_question(
                "s1",
                "stale-cid",
                "brian",
                crate::storage::QuestionKind::Choice,
                "stale prompt",
                Some(&["a".to_string(), "b".to_string()]),
                None,
            )
            .await
            .unwrap();

        let bridge_clone = Arc::clone(&bridge);
        let supersede = tokio::spawn(async move {
            bridge_clone
                .supersede_question_with_new(
                    "s1".into(),
                    "brian".into(),
                    "stale-cid".into(),
                    "rephrased".into(),
                    vec!["x".into(), "y".into()],
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let rows = storage.questions_for_session("s1").await.unwrap();
        assert_eq!(rows.len(), 2);
        let stale = &rows[0];
        let fresh = &rows[1];
        assert_eq!(stale.choice_id, "stale-cid");
        assert_eq!(stale.status, "superseded");
        assert_eq!(fresh.prompt, "rephrased");
        assert_eq!(fresh.status, "pending");
        assert_eq!(fresh.supersedes_id, Some(stale.id));

        bridge
            .resolve_choice(&fresh.choice_id, "x".into())
            .await
            .unwrap();
        let picked = supersede.await.unwrap().unwrap();
        assert_eq!(picked, "x");
    }
}
