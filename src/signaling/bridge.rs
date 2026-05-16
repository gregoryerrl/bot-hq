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
use crate::storage::{Author, MessageKind, Storage};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, Mutex};
use uuid::Uuid;

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
}

impl SignalingBridge {
    pub fn new() -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            event_tx,
            pending: Mutex::new(HashMap::new()),
            violations: None,
            data_dir: None,
            session_projects: Mutex::new(HashMap::new()),
            session_awaiting: Mutex::new(HashMap::new()),
            storage: Mutex::new(None),
        })
    }

    /// Construct a bridge with a violations log attached. Approval-class
    /// resolutions write a record after the user picks an option.
    pub fn with_violations_log(violations: ViolationsLog) -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            event_tx,
            pending: Mutex::new(HashMap::new()),
            violations: Some(violations),
            data_dir: None,
            session_projects: Mutex::new(HashMap::new()),
            session_awaiting: Mutex::new(HashMap::new()),
            storage: Mutex::new(None),
        })
    }

    /// Full-featured constructor: violations log + policy resolution root.
    /// Used in production; tests can use [`new`] or [`with_violations_log`]
    /// for partial setups.
    pub fn with_policy(violations: ViolationsLog, data_dir: PathBuf) -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            event_tx,
            pending: Mutex::new(HashMap::new()),
            violations: Some(violations),
            data_dir: Some(data_dir),
            session_projects: Mutex::new(HashMap::new()),
            session_awaiting: Mutex::new(HashMap::new()),
            storage: Mutex::new(None),
        })
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

    /// Load (resolve) policy for the given session. Falls back to default
    /// policy if data_dir isn't configured or the session isn't registered.
    /// Parse errors propagate — callers should map to a JSON-RPC error.
    pub async fn resolve_policy_for(&self, session_id: &str) -> Result<Policy> {
        let Some(data_dir) = self.data_dir.as_ref() else {
            return Ok(Policy::default());
        };
        let project = self.project_for_session(session_id).await;
        Policy::resolve(data_dir, project.as_deref())
    }

    /// Direct access to the violations log (e.g., for the UI's recent-events
    /// panel). None when the bridge was constructed without a log.
    pub fn violations_log(&self) -> Option<&ViolationsLog> {
        self.violations.as_ref()
    }

    /// CL root path — used by callers that need to read auxiliary files
    /// (policy hash cache, etc.). None on test bridges built via `new()`.
    pub fn data_dir(&self) -> Option<&PathBuf> {
        self.data_dir.as_ref()
    }

    /// Subscribe to all signaling events. The UI layer uses this to paint.
    pub fn subscribe(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    /// Called by the MCP `tools/call` handler for `ask_user_choice`.
    /// Awaits a response from the UI.
    pub async fn ask_user_choice(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
    ) -> Result<String> {
        self.ask_user_choice_inner(session_id, agent, question, options, None)
            .await
    }

    /// Policy-initiated approval request. Same machinery as `ask_user_choice`
    /// but carries an [`ApprovalContext`] so the resolve path can write a
    /// violation record.
    pub async fn request_approval(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        ctx: ApprovalContext,
    ) -> Result<String> {
        self.ask_user_choice_inner(session_id, agent, question, options, Some(ctx))
            .await
    }

    async fn ask_user_choice_inner(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        approval: Option<ApprovalContext>,
    ) -> Result<String> {
        let choice_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel::<String>();
        let choice = PendingChoice {
            choice_id: choice_id.clone(),
            session_id: session_id.clone(),
            agent,
            question,
            options,
            approval,
        };
        self.pending.lock().await.insert(
            choice_id.clone(),
            Parked {
                tx,
                choice: choice.clone(),
            },
        );

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

    /// Called by the UI when the user clicks a choice button.
    pub async fn resolve_choice(&self, choice_id: &str, picked: String) -> Result<()> {
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
                            outcome.clone(),
                            ctx.detail.clone(),
                        )
                        .await;
                }

                // Persist a push_gate approval to the `.remembered-approvals`
                // side-file so the git pre-push hook (which calls
                // `Policy::resolve` fresh on every push) sees the branch and
                // lets subsequent pushes proceed. Without this step, the MCP
                // approval is audit-only and the hook re-blocks indefinitely.
                if let (Some(ctx), Some(data_dir)) = (&p.choice.approval, &self.data_dir) {
                    if matches!(ctx.kind, crate::policy::ViolationKind::PushGate)
                        && matches!(outcome, crate::policy::ViolationOutcome::Approved)
                    {
                        if let Some(branch) = parse_push_branch(&ctx.action) {
                            let project = self
                                .session_projects
                                .lock()
                                .await
                                .get(&p.choice.session_id)
                                .cloned()
                                .flatten();
                            if let Err(e) = crate::policy::Policy::append_remembered_approval(
                                data_dir,
                                project.as_deref(),
                                &branch,
                            ) {
                                tracing::warn!(
                                    ?e,
                                    branch = %branch,
                                    "append_remembered_approval failed; pre-push hook may re-block"
                                );
                            }
                        }
                    }
                }

                if let Err(picked) = p.tx.send(picked) {
                    // The agent's blocking `ask_user_choice` tool call client-side
                    // timed out before we got the user's pick. The answer is still
                    // captured (violations log + remembered_approvals are already
                    // written above) — surface it to the agent out-of-band so they
                    // see the resolution on next message poll instead of waiting
                    // forever on a tool result that will never arrive.
                    let agent_label = p.choice.agent.clone();
                    let question = p.choice.question.clone();
                    let session_id = p.choice.session_id.clone();
                    let storage_guard = self.storage.lock().await;
                    if let Some(storage) = storage_guard.as_ref() {
                        let body = format!(
                            "(out-of-band) Your earlier `ask_user_choice` for {agent_label} resolved while \
                             you were no longer waiting on the tool call.\n\n\
                             **Question:** {question}\n\
                             **User picked:** {picked}\n\n\
                             Treat this as the user's reply. Continue from here."
                        );
                        if let Err(e) = storage
                            .insert_message(&session_id, Author::User, MessageKind::Text, &body)
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
                }
                Ok(())
            }
            None => Err(anyhow::anyhow!("no pending choice with id {choice_id}")),
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
    pub async fn mark_awaiting_user(&self, session_id: String, agent: String, reason: String) {
        self.set_session_awaiting(&session_id).await;
        let _ = self.event_tx.send(SignalingEvent::AwaitingUser {
            session_id,
            agent,
            reason,
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

    /// For tests / introspection: how many choices are parked?
    pub async fn pending_choice_count(&self) -> usize {
        self.pending.lock().await.len()
    }
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
        let bridge = SignalingBridge::new();
        let err = bridge
            .resolve_choice("nope", "x".into())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no pending choice"));
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
        bridge
            .resolve_choice(&choice_id, "A".into())
            .await
            .expect("resolve_choice should succeed even when agent receiver dropped");

        // Verify the out-of-band message landed.
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
    async fn approved_push_gate_persists_branch_to_remembered_approvals() {
        // Round-trip the bridge.resolve_choice → policy side-file → Policy::resolve
        // path. After an approved push_gate, the branch must land in
        // remembered_approvals so the pre-push hook stops blocking subsequent
        // pushes.
        let dir = tempfile::tempdir().unwrap();
        let project = "test-project";
        // Seed a project policy with per_branch_approval mode + no approvals yet
        let proj_dir = dir.path().join("projects").join(project);
        std::fs::create_dir_all(&proj_dir).unwrap();
        std::fs::write(
            proj_dir.join("policy.yaml"),
            "push_gate:\n  mode: per_branch_approval\n",
        )
        .unwrap();

        let log = ViolationsLog::new(dir.path());
        let bridge =
            SignalingBridge::with_policy(log.clone(), dir.path().to_path_buf());
        bridge
            .register_session(
                "session-A".to_string(),
                Some(project.to_string()),
            )
            .await;

        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "session-A".into(),
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

        // Branch is now in the side-file.
        let approvals = std::fs::read_to_string(
            proj_dir.join(".remembered-approvals"),
        )
        .unwrap();
        assert!(approvals.contains("346-streamline-onboarding-process"));

        // Policy::resolve now picks it up.
        let resolved = crate::policy::Policy::resolve(dir.path(), Some(project)).unwrap();
        assert!(resolved
            .push_gate
            .remembered_approvals
            .iter()
            .any(|b| b == "346-streamline-onboarding-process"));

        // Idempotent — second approval for the same branch doesn't duplicate.
        let bridge_clone = Arc::clone(&bridge);
        let ask2 = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "session-A".into(),
                    "brian".into(),
                    "Push again?".into(),
                    vec!["Approve push".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push origin 346-streamline-onboarding-process".into(),
                        detail: None,
                    },
                )
                .await
                .unwrap()
        });
        // Drain any pending events (ChoiceResolved from the first round) until
        // we land on the second PendingChoice.
        let cid2 = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        bridge
            .resolve_choice(&cid2, "Approve push".into())
            .await
            .unwrap();
        let _ = ask2.await.unwrap();
        let approvals_again =
            std::fs::read_to_string(proj_dir.join(".remembered-approvals")).unwrap();
        let occurrences = approvals_again
            .lines()
            .filter(|l| l.trim() == "346-streamline-onboarding-process")
            .count();
        assert_eq!(occurrences, 1, "branch should appear exactly once");
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

        let approvals_path = proj_dir.join(".remembered-approvals");
        assert!(
            !approvals_path.exists() || std::fs::read_to_string(&approvals_path).unwrap().is_empty(),
            ".remembered-approvals should not contain denied branch"
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
}
