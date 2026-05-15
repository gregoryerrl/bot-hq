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
/// the optional approval context is consumed on resolve to write the log.
struct Parked {
    tx: oneshot::Sender<String>,
    approval: Option<ApprovalContext>,
    session_id: String,
    agent: String,
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
        })
    }

    /// Called by the session spawn code so the bridge can resolve the right
    /// project policy when this session's agents call policy-aware MCP tools.
    /// Idempotent — re-registering overwrites.
    pub async fn register_session(&self, session_id: String, project: Option<String>) {
        self.session_projects.lock().await.insert(session_id, project);
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
        self.pending.lock().await.insert(
            choice_id.clone(),
            Parked {
                tx,
                approval: approval.clone(),
                session_id: session_id.clone(),
                agent: agent.clone(),
            },
        );

        // Halt the duo BEFORE emitting the event — the agent's next chunk
        // shouldn't volley to its peer while we wait for the user.
        self.set_session_awaiting(&session_id).await;

        // Best-effort broadcast. If no subscribers, the request still parks
        // until resolve_choice is called (mostly a concern for tests).
        let _ = self.event_tx.send(SignalingEvent::PendingChoice(PendingChoice {
            choice_id: choice_id.clone(),
            session_id,
            agent,
            question,
            options,
            approval,
        }));

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
                if let (Some(log), Some(ctx)) = (self.violations.as_ref(), &p.approval) {
                    let outcome = outcome_from_picked(&picked);
                    let _ = log
                        .record(
                            p.session_id.clone(),
                            p.agent.clone(),
                            ctx.kind,
                            ctx.action.clone(),
                            outcome,
                            ctx.detail.clone(),
                        )
                        .await;
                }
                p.tx.send(picked)
                    .map_err(|_| anyhow::anyhow!("agent receiver dropped before choice resolved"))?;
                Ok(())
            }
            None => Err(anyhow::anyhow!("no pending choice with id {choice_id}")),
        }
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
