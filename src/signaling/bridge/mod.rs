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
use crate::storage::{PersistedMessage, Storage};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::{broadcast, oneshot, Mutex};

mod action_gate;
mod cl_facade;
mod cl_push;
pub use cl_push::{scan_then_push, PushOutcome};
mod cl_refs;
mod cl_staleness;
mod cl_write;
mod feedback;
mod findings;
mod session_docs;
mod terminal_tools;
mod tray;
pub use tray::{gate_age_secs, STALE_GATE_MAX_AGE_SECS};
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
/// `CoreAppState::resolve_choice`) is responsible for **also** delivering
/// that row through the duo's input channels so the agent's subprocess
/// wakes up and sees it (clearing the awaiting flag alone won't deliver
/// — the agent is blocked on stdin and needs an actual stdin write).
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    Delivered,
    AgentReceiverDroppedFellBack {
        session_id: String,
        /// What the answer SAYS — the composed replay text, before the phase
        /// envelope. Retained beside the receipt because it is meaningful even
        /// when nothing was recorded, and because it is what the composition
        /// tests assert on; when `receipt` is `Some` this is its `body()`.
        body: String,
        /// The row that authorizes wiring it, envelope included.
        ///
        /// `None` when the bridge had no storage or the insert failed. The
        /// caller then delivers nothing: B5 Task 2's invariant is that a
        /// message with no row does not reach an agent, and this is the one
        /// path where that changes behaviour — it used to wake the agent with
        /// text that existed nowhere else.
        receipt: Option<PersistedMessage>,
    },
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
    /// An agent finished a turn and claude-code reported how full its context
    /// window is. Drives the per-agent context meter in the session header so
    /// the user can decide when to wrap a session rather than discovering the
    /// ceiling by hitting it.
    ///
    /// Only emitted when the *denominator* is known: claude-code reports
    /// `contextWindow` on the `result` event's `modelUsage` map, and a gateway
    /// provider may omit it. No event is better than a guessed percentage.
    AgentContext {
        session_id: String,
        agent: String,
        used_tokens: u64,
        context_window: u64,
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
    /// HANDS declared (or ended) harness-background work via `declare_working`.
    /// `reason=Some(..)` shows the neutral WORKING badge; `None` clears it
    /// (TTL expiry, user broadcast, or session close — never activity
    /// transitions: a declared state persists across turns).
    SessionWorking {
        session_id: String,
        reason: Option<String>,
    },
    /// Session-level attention flag from the idle-unflagged watchdog.
    /// `state=Some("idle_unflagged")` when the session sat Idle past grace with
    /// no tray flag parked after the first user prompt; `state=None` when the
    /// condition cleared (activity resumed or the user spoke). The UI shows a
    /// "needs direction" chip. String-typed so future attention kinds don't
    /// need a wire change.
    SessionAttention {
        session_id: String,
        state: Option<String>,
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
    /// #31: `(project, term)` concepts this session's CL writes RETIRED —
    /// present in a file's old body, gone from its new one. Seeds the
    /// close-out staleness sweep.
    retired: Vec<(String, String)>,
    /// We've already surfaced the staleness sweep once — like `close_nudged`,
    /// this makes the sweep advisory: it can never hold a close shut.
    sweep_nudged: bool,
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
    /// Per-session turn-ring control channel, so parking a question can HALT the
    /// ring and not merely set a flag. See [`Self::register_session_sequencer`].
    session_sequencer: Mutex<
        HashMap<String, tokio::sync::mpsc::Sender<crate::core::sequencer::SequencerCommand>>,
    >,
    /// Per-session `declare_working` flag, registered at spawn (mirrors
    /// `session_awaiting`). `Some((until, reason))` while active. Tuple —
    /// not a core type — so the signaling layer stays decoupled from core.
    /// The bridge sets it (`declare_working`); `AppState::broadcast` clears
    /// it; the watchdog expires it.
    #[allow(clippy::type_complexity)]
    session_working_flag:
        Mutex<HashMap<String, Arc<std::sync::Mutex<Option<(std::time::Instant, String)>>>>>,
    /// Latest emitted WORKING badge state per session (dedupe registry +
    /// `get_session_runtime` seed), exactly mirroring `session_attention`.
    session_working: std::sync::Mutex<HashMap<String, String>>,
    /// session_id → Weak ref to the session's ActivityTracker. Lets
    /// `set_session_awaiting` reflect an awaiting-flag flip into the derived
    /// activity immediately (emit AwaitingUser) instead of waiting for the next
    /// `set_busy`. Weak, not Arc: the tracker holds a strong
    /// `Arc<SignalingBridge>` (activity.rs), so a strong back-ref here would
    /// cycle and leak the tracker past session close; `upgrade()` returns None
    /// after close → a silent no-op.
    session_activity: Mutex<HashMap<String, Weak<ActivityTracker>>>,
    /// session_id → Weak ref to the session's IPAV state.
    ///
    /// Registered for exactly one reader: `deliver_oob` needs the current phase
    /// to put in the ENVELOPE of the out-of-band answer it posts. Before B5
    /// Task 2 the phase was applied in `CoreAppState::resolve_choice` after the
    /// row was already written, so the row and the agent's stdin disagreed by a
    /// `[PHASE: X]` line. The envelope has to be known at post time, so the
    /// phase has to be readable where the post happens.
    ///
    /// Weak, but NOT for `session_activity`'s reason — nothing here cycles, the
    /// IPAV state holds no bridge ref. It is Weak so that a session whose handle
    /// is dropped without a clean `unregister_session` (crash-reap, a close path
    /// that missed) leaves a dead ref rather than pinning its phase state for the
    /// process lifetime; `upgrade()` then returns None and the envelope goes out
    /// untagged. `unregister_session` still removes the entry on the normal path.
    session_phase: Mutex<HashMap<String, Weak<Mutex<crate::core::ipav::IpavState>>>>,
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
    /// Per-session PTY registry shared with `AppState.terminals` — the
    /// `terminal_exec` / `terminal_read` MCP handlers reach the same PTYs the
    /// Terminal subtab renders. Set once at setup, like `app_handle`; `None`
    /// in tests and the pre-setup window.
    terminals: std::sync::OnceLock<Arc<crate::core::TerminalRegistry>>,
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
    /// (session_id, agent) → last time this agent made ANY bridge RPC call.
    /// Ground truth for "is the reviewer actually there": the health map above
    /// is event-derived and has reported an agent Stalled 4ms after its own
    /// tool call (2026-07-27 archive study, s-32196a61) — an agent talking to
    /// the bridge is alive regardless of what the health events last said.
    agent_rpc_seen: std::sync::Mutex<HashMap<(String, String), std::time::Instant>>,
    /// Batch 7: per-session HANDS override of the reviewer-down commit block —
    /// session_id → reason. Set by `override_reviewer_block`, honored by
    /// `check_open_findings`, auto-cleared when the reviewer recovers to running.
    reviewer_override: std::sync::Mutex<HashMap<String, String>>,
    /// session_id → the slugs of the participants that can file findings.
    ///
    /// **What "the reviewer" means now that no agent is called Rain** (rc3
    /// D10/D11). The commit gate used to ask `current_agent_health(session,
    /// "rain")` and the override auto-clear used to fire on `agent == "rain"`;
    /// with role-derived slugs both would silently stop matching, and the gate
    /// fails OPEN — a review that cannot have happened would stop blocking the
    /// commit, with nothing to notice.
    ///
    /// Registered at spawn from the roster's capability snapshots — a reviewer is
    /// a participant that holds `file_finding`, which is bot-hq's own definition
    /// and not a guess about what a role MEANS. Empty (or unregistered) = this
    /// session has no reviewer, which is exactly what a solo session was.
    session_reviewers: std::sync::Mutex<HashMap<String, Vec<String>>>,
    /// Latest peer-forward-router liveness per session_id (true = alive). Written
    /// by `notify_router_health`; read by `get_session_runtime` to seed the UI
    /// router-health dot on mount (the event fires only on change, like
    /// `agent_health`). Sync `Mutex` — `notify_router_health` is sync.
    router_health: std::sync::Mutex<HashMap<String, bool>>,
    /// Latest idle-unflagged attention state per session_id (value = the
    /// attention kind, e.g. "idle_unflagged"; absent = clear). Written by
    /// `notify_session_attention`; read by `get_session_runtime` to seed the
    /// UI chip on mount (the event fires only on change, mirroring
    /// `router_health`). Sync `Mutex` — the notify path is sync.
    session_attention: std::sync::Mutex<HashMap<String, String>>,
    /// session_id → shared open-blocking-findings count. The router reads the
    /// `Arc<AtomicUsize>` LOCK-FREE per peer-forward (for the wire banner) instead
    /// of a per-forward `SELECT COUNT(*)` + storage-`Mutex` acquire; the findings
    /// mutators recompute it after any change via `refresh_open_blocking`.
    /// `std::sync::Mutex` over the MAP (brief, never held across `await`); the
    /// per-session `Arc` is the lock-free read surface the router holds a clone of.
    session_open_blocking: std::sync::Mutex<HashMap<String, Arc<AtomicUsize>>>,
}

impl SignalingBridge {
    pub(crate) fn new_with(violations: Option<ViolationsLog>, data_dir: Option<PathBuf>) -> Arc<Self> {
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
            session_sequencer: Mutex::new(HashMap::new()),
            session_activity: Mutex::new(HashMap::new()),
            session_phase: Mutex::new(HashMap::new()),
            storage: Mutex::new(None),
            app_handle: std::sync::OnceLock::new(),
            terminals: std::sync::OnceLock::new(),
            session_close_gate: Mutex::new(HashMap::new()),
            agent_health: std::sync::Mutex::new(HashMap::new()),
            agent_rpc_seen: std::sync::Mutex::new(HashMap::new()),
            reviewer_override: std::sync::Mutex::new(HashMap::new()),
            session_reviewers: std::sync::Mutex::new(HashMap::new()),
            router_health: std::sync::Mutex::new(HashMap::new()),
            session_attention: std::sync::Mutex::new(HashMap::new()),
            session_working_flag: Mutex::new(HashMap::new()),
            session_working: std::sync::Mutex::new(HashMap::new()),
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

    /// The wired storage handle, or `None` before `set_storage`.
    ///
    /// Exposed so the tool gate can read the caller's participant row
    /// (`jsonrpc::resolve_caller_capabilities`) without the bridge growing a
    /// method per query. `None` is a real answer there, not an inconvenience:
    /// it is what makes "the roster could not be read" a distinct outcome from
    /// "the roster says you hold nothing".
    pub async fn storage_handle(&self) -> Option<Storage> {
        self.storage.lock().await.clone()
    }

    /// Hand the bridge a shared awaiting-flag pointer owned by the SessionHandle.
    /// The duo pump reads this same flag to decide whether to forward chunks
    /// to the peer. Setting it from inside the bridge (in mark_awaiting_user /
    /// ask_user_choice) is what gives us a race-free halt.
    pub async fn register_session_awaiting(&self, session_id: String, flag: Arc<AtomicBool>) {
        self.session_awaiting.lock().await.insert(session_id, flag);
    }

    /// Hand the bridge the session's turn-ring control channel.
    ///
    /// **This is what makes a parked question actually halt the cycle.** The
    /// awaiting FLAG alone only stops cursors advancing, so before this the ring
    /// kept handing out turns while the session was blocked on a human: each
    /// participant woke with nothing new delivered, had no legal move, and
    /// passed. Observed live on 2026-08-12 as ~15 model calls in 1m44s, both
    /// participants alternating "standing by" — and it could not self-terminate,
    /// because a pass retracts its own done vote and any prose at all counts as
    /// substantive output, which clears the whole tally.
    ///
    /// `SequencerCommand::QuestionParked` was written, documented and tested for
    /// exactly this, and had no production sender until now.
    pub async fn register_session_sequencer(
        &self,
        session_id: String,
        tx: tokio::sync::mpsc::Sender<crate::core::sequencer::SequencerCommand>,
    ) {
        self.session_sequencer.lock().await.insert(session_id, tx);
    }

    /// **Tell the ring the user spoke.** This is the RELEASE for a halt — the
    /// only one the sequencer has — and without it a parked question stops the
    /// cycle permanently.
    ///
    /// Shipped broken on 2026-08-13 for about two hours: `QuestionParked` was
    /// wired with no release path, so the first `mark_awaiting_user` of a session
    /// halted the ring and nothing could restart it. The participants then ran
    /// on their initial prompt with no turn-taking and no delivery at all — 105
    /// messages and zero `participant_deliveries` rows in the session that caught
    /// it. Halting and releasing are two halves of one mechanism; ship them
    /// together or neither.
    ///
    /// **`mentions` is who the user named** (rc3 D17), already resolved to
    /// participant ids by the caller — the only caller that can, since it is the
    /// one holding the text and the session. Empty is the ordinary case.
    pub async fn notify_ring_user_message(&self, session_id: &str, mentions: Vec<i64>) {
        let seq = self.session_sequencer.lock().await.get(session_id).cloned();
        if let Some(tx) = seq {
            if tx
                .try_send(crate::core::sequencer::SequencerCommand::UserMessage { mentions })
                .is_err()
            {
                tracing::warn!(
                    session_id,
                    "a user message did not reach the ring — a halted cycle will stay halted"
                );
            }
        }
    }

    /// Whether a turn ring is reachable for this session — the observable half
    /// of [`Self::register_session_sequencer`], so the spawn-time join can be
    /// pinned by a test.
    pub async fn session_sequencer_registered(&self, session_id: &str) -> bool {
        self.session_sequencer.lock().await.contains_key(session_id)
    }

    /// Register the session's `declare_working` flag at spawn (mirrors
    /// `register_session_awaiting`).
    #[allow(clippy::type_complexity)]
    pub async fn register_session_working(
        &self,
        session_id: String,
        flag: Arc<std::sync::Mutex<Option<(std::time::Instant, String)>>>,
    ) {
        self.session_working_flag
            .lock()
            .await
            .insert(session_id, flag);
    }

    /// HANDS declares harness-background work: set the flag until `ttl` from
    /// now and emit the WORKING badge. Re-declaring extends/replaces. Returns
    /// the clamped TTL actually applied, or None if the session isn't
    /// registered (spawn incomplete / already closed).
    pub async fn declare_working(
        &self,
        session_id: &str,
        reason: &str,
        ttl: std::time::Duration,
    ) -> Option<std::time::Duration> {
        let flag = self
            .session_working_flag
            .lock()
            .await
            .get(session_id)
            .cloned()?;
        *flag.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((std::time::Instant::now() + ttl, reason.to_string()));
        self.notify_session_working(session_id.to_string(), Some(reason));
        Some(ttl)
    }

    /// Clear the `declare_working` flag (user broadcast / session close) and
    /// drop the badge. No-op when nothing was declared.
    pub async fn clear_session_working(&self, session_id: &str) {
        if let Some(flag) = self.session_working_flag.lock().await.get(session_id) {
            let had = flag
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .is_some();
            if had {
                self.notify_session_working(session_id.to_string(), None);
            }
        }
    }

    /// Emit a WORKING badge change, deduped like `notify_session_attention` —
    /// the watchdog's expiry path and re-declares may re-call with the same
    /// state; only actual transitions reach the wire.
    pub fn notify_session_working(&self, session_id: String, reason: Option<&str>) {
        {
            let mut map = self
                .session_working
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let changed = match reason {
                Some(r) => map.insert(session_id.clone(), r.to_string()).as_deref() != Some(r),
                None => map.remove(&session_id).is_some(),
            };
            if !changed {
                return;
            }
        }
        let _ = self.event_tx.send(SignalingEvent::SessionWorking {
            session_id,
            reason: reason.map(str::to_string),
        });
    }

    /// Latest cached WORKING badge state (`None` = clear / never declared).
    pub fn current_session_working(&self, session_id: &str) -> Option<String> {
        self.session_working
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .cloned()
    }

    /// Hand the bridge a Weak ref to the session's ActivityTracker so
    /// `set_session_awaiting` can refresh the derived activity the moment it
    /// flips the awaiting flag (emit AwaitingUser without waiting for the next
    /// `set_busy`). Weak — see the `session_activity` field doc.
    pub async fn register_session_activity(&self, session_id: String, tracker: Weak<ActivityTracker>) {
        self.session_activity.lock().await.insert(session_id, tracker);
    }

    /// Hand the bridge a Weak ref to the session's IPAV state — see the
    /// `session_phase` field for why the bridge needs to read a phase at all.
    pub async fn register_session_phase(
        &self,
        session_id: String,
        ipav: Weak<Mutex<crate::core::ipav::IpavState>>,
    ) {
        self.session_phase.lock().await.insert(session_id, ipav);
    }

    /// The session's current IPAV phase, or `None` if it was never registered
    /// (headless / tests) or has since closed. A `None` means the envelope goes
    /// out without a phase tag, which is honest: nothing knows what phase a dead
    /// session is in, and the row then records the untagged wire it will get.
    pub async fn current_session_phase(
        &self,
        session_id: &str,
    ) -> Option<crate::core::ipav::IpavPhase> {
        let ipav = self.session_phase.lock().await.get(session_id)?.upgrade()?;
        let phase = ipav.lock().await.current_phase;
        Some(phase)
    }

    /// Register a session's open-blocking-findings count cache and return the
    /// shared `Arc` the router reads LOCK-FREE per forward. Seeds from storage so a
    /// re-spawned session with pre-existing findings starts at the right value (not
    /// 0). Mirrors `register_session_awaiting`.
    /// Record which participants of a session can file findings — the reviewers
    /// the commit gate watches. Called once per spawn with the roster's own
    /// answer; an empty list means this session has no reviewer at all.
    pub fn register_session_reviewers(&self, session_id: String, slugs: Vec<String>) {
        self.session_reviewers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session_id, slugs);
    }

    /// This session's reviewer slugs (empty when none are registered).
    pub(crate) fn session_reviewers(&self, session_id: &str) -> Vec<String> {
        self.session_reviewers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    fn is_reviewer(&self, session_id: &str, agent: &str) -> bool {
        self.session_reviewers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .is_some_and(|slugs| slugs.iter().any(|s| s == agent))
    }

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
        self.session_working_flag.lock().await.remove(session_id);
        self.session_working
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id);
        self.session_activity.lock().await.remove(session_id);
        self.session_phase.lock().await.remove(session_id);
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

    /// A3b: record that the agent ran `cl_rescan` or `cl_write_file` this
    /// session — a proxy for "persisted a learnings delta", which lifts the
    /// close-delta gate.
    pub async fn mark_cl_rescan(&self, session_id: &str) {
        self.session_close_gate
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .cl_written = true;
    }

    /// #31: remember the concepts a CL write retired, so `staleness_sweep` can
    /// check the rest of the project's library for files still citing them.
    /// Bounded per session — a long session of rewrites can't grow unboundedly.
    pub async fn record_retired_terms(&self, session_id: &str, project: &str, terms: Vec<String>) {
        const MAX_RETIRED_PER_SESSION: usize = 60;
        if terms.is_empty() {
            return;
        }
        let mut gate = self.session_close_gate.lock().await;
        let state = gate.entry(session_id.to_string()).or_default();
        for term in terms {
            if state.retired.len() >= MAX_RETIRED_PER_SESSION {
                break;
            }
            let entry = (project.to_string(), term);
            if !state.retired.contains(&entry) {
                state.retired.push(entry);
            }
        }
    }

    /// #31 close-out staleness sweep: which OTHER CL files still cite a concept
    /// this session retired? Returns a capped, human-readable report the
    /// `close_session` handler surfaces ONCE, or `None` when there's nothing to
    /// say (no retired terms, no surviving hits, or already surfaced).
    ///
    /// Advisory by construction — it never blocks the close, it never edits, and
    /// it fires at most once per session. The gap it closes is mechanical, not
    /// disciplinary: the 2026-08-05 framing-rule session landed its decision in
    /// `decisions.md` and left `conventions.md:3` contradicting it for hours,
    /// with the "grep the old terms" rule live the whole time.
    pub async fn staleness_sweep(&self, session_id: &str) -> Option<String> {
        let retired = {
            let mut gate = self.session_close_gate.lock().await;
            let state = gate.entry(session_id.to_string()).or_default();
            if state.sweep_nudged || state.retired.is_empty() {
                return None;
            }
            state.sweep_nudged = true;
            state.retired.clone()
        };
        // Group by project so each library root is walked once.
        let mut by_project: HashMap<String, Vec<String>> = HashMap::new();
        for (project, term) in retired {
            by_project.entry(project).or_default().push(term);
        }
        let mut hits: Vec<String> = Vec::new();
        for (project, terms) in by_project {
            let Some(root) = self.cl_project_root(&project).await else {
                continue;
            };
            let found = tokio::task::spawn_blocking(move || {
                cl_write::sweep_project(&root, &project, &terms)
            })
            .await
            .unwrap_or_default();
            hits.extend(found);
        }
        if hits.is_empty() {
            return None;
        }
        let total = hits.len();
        hits.truncate(cl_write::SWEEP_MAX_HITS);
        let more = total.saturating_sub(hits.len());
        let tail = if more > 0 {
            format!("\n…and {more} more.")
        } else {
            String::new()
        };
        Some(format!(
            "Close-out staleness sweep — this session's CL writes retired terms that \
             OTHER library files still use:\n{}{tail}\n\nEach hit is either (a) a file \
             that should have been updated with the change, or (b) a legitimate \
             historical mention. Fix the (a)s with cl_write_file, then call \
             close_session again — this check does not repeat and will not hold the \
             close.",
            hits.join("\n")
        ))
    }

    /// `(cl_written, close_nudged)` for a session — the two facts rc3 D15's
    /// close epilogue needs to decide whether this session has already had its
    /// chance to write a learnings delta.
    ///
    /// **Read it BEFORE teardown.** [`Self::unregister_session`] drops the gate
    /// entry, so a caller that tears the session down first sees `(false,
    /// false)` and would ask an agent that already declined. A session with no
    /// entry has never touched the CL and was never nudged, which is what the
    /// default says.
    pub async fn close_gate_flags(&self, session_id: &str) -> (bool, bool) {
        let gate = self.session_close_gate.lock().await;
        gate.get(session_id)
            .map(|s| (s.cl_written, s.close_nudged))
            .unwrap_or((false, false))
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

    /// Stash the shared per-session terminal registry (same Arc as
    /// `AppState.terminals`). Idempotent, set once at setup.
    pub fn set_terminal_registry(&self, registry: Arc<crate::core::TerminalRegistry>) {
        let _ = self.terminals.set(registry);
    }

    /// The shared terminal registry. None until setup, or in tests — the
    /// terminal_* MCP tools error cleanly in that window.
    pub(crate) fn terminal_registry(&self) -> Option<&Arc<crate::core::TerminalRegistry>> {
        self.terminals.get()
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

    /// Publish an agent's context-window occupancy after a completed turn.
    /// Fire-and-forget; the UI subscriber maps it to `session:agent_context`.
    ///
    /// Call only with a known window — `context_window == 0` is rejected here
    /// as a last line of defence against a divide-by-zero reaching the UI.
    pub fn notify_agent_context(
        &self,
        session_id: String,
        agent: &str,
        used_tokens: u64,
        context_window: u64,
    ) {
        if context_window == 0 {
            return;
        }
        let _ = self.event_tx.send(SignalingEvent::AgentContext {
            session_id,
            agent: agent.to_string(),
            used_tokens,
            context_window,
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
        // the override is scoped to one down-incident, never persistent. Keyed on
        // the session's REGISTERED reviewers (rc3 D10) rather than on the literal
        // slug `rain`, which no participant is called any more.
        if health == "running" && self.is_reviewer(&session_id, agent) {
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

    /// Stamp "this agent just made a bridge RPC call". Called from the JSON-RPC
    /// tool dispatch — the single choke point every agent tool call crosses.
    pub fn note_agent_rpc(&self, session_id: &str, agent: &str) {
        self.agent_rpc_seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                (session_id.to_string(), agent.to_string()),
                std::time::Instant::now(),
            );
    }

    /// Whether the agent made any bridge RPC call within `within`. Overrides an
    /// event-derived Stalled/Dead verdict in the reviewer gate: activity on the
    /// wire is stronger evidence of liveness than the last health event.
    pub fn agent_rpc_recent(
        &self,
        session_id: &str,
        agent: &str,
        within: std::time::Duration,
    ) -> bool {
        self.agent_rpc_seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&(session_id.to_string(), agent.to_string()))
            .is_some_and(|t| t.elapsed() <= within)
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

    /// Publish an idle-unflagged attention change. Deduped here (not at the
    /// caller): the watchdog re-evaluates every poll, so it calls this every
    /// 10s while the condition holds — only an actual transition reaches the
    /// wire. `state=None` clears. Mirrors `notify_router_health`.
    pub fn notify_session_attention(&self, session_id: String, state: Option<&str>) {
        {
            let mut map = self
                .session_attention
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let changed = match state {
                Some(s) => map.insert(session_id.clone(), s.to_string()).as_deref() != Some(s),
                None => map.remove(&session_id).is_some(),
            };
            if !changed {
                return;
            }
        }
        let _ = self.event_tx.send(SignalingEvent::SessionAttention {
            session_id,
            state: state.map(str::to_string),
        });
    }

    /// Latest cached attention state for a session (`None` = clear / never set).
    pub fn current_session_attention(&self, session_id: &str) -> Option<String> {
        self.session_attention
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .cloned()
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
        self.persist_activity_event(&session_id, state, brian_busy, rain_busy);
        let _ = self.event_tx.send(SignalingEvent::SessionActivity {
            session_id,
            state: state.to_string(),
            brian_busy,
            rain_busy,
        });
    }

    /// Mirror an activity transition into `activity_events` so the state side of
    /// the timeline outlives the UI that consumed it (migration 0042).
    ///
    /// Called from `ActivityTracker::recompute_locked`, which is SYNCHRONOUS and
    /// holds its state mutex — so the write is detached rather than awaited. Two
    /// consequences, both deliberate:
    ///
    /// * `Handle::try_current()` rather than a bare `tokio::spawn`. The tracker's
    ///   mutators (`set_busy`, `set_paused`, `refresh`) are plain `&self` methods
    ///   and nothing guarantees a runtime is entered at every call site; a bare
    ///   spawn would panic there. No runtime → no row, never a panic.
    /// * Ordering between detached writes is not guaranteed, which is why the
    ///   row carries its own `recorded_at` and queries sort by `id`.
    ///
    /// Fail-open throughout: this decorates a signal that gates the chat input,
    /// and losing telemetry must never disturb it.
    fn persist_activity_event(
        &self,
        session_id: &str,
        state: &str,
        brian_busy: bool,
        rain_busy: bool,
    ) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        // `try_lock`, not `lock().await`: this fn is sync. Every holder of this
        // mutex does clone-and-drop, so the window is nanoseconds and a miss is
        // vanishingly rare — but when it happens the row is dropped rather than
        // blocking the activity signal on it. `Storage` is a cheap handle clone
        // (connection pool), so what crosses into the task is not the mutex.
        let Some(storage) = self.storage.try_lock().ok().and_then(|g| g.clone()) else {
            return;
        };
        let session_id = session_id.to_string();
        let state = state.to_string();
        handle.spawn(async move {
            if let Err(e) = storage
                .insert_activity_event(&session_id, &state, brian_busy, rain_busy)
                .await
            {
                tracing::warn!(?e, %session_id, %state, "persisting activity event failed");
            }
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

    #[tokio::test]
    async fn closing_a_session_unregisters_its_phase() {
        // `session_phase` joins the maps `unregister_session` drains. Missing
        // from that list it would leak one entry per open→close cycle, and the
        // `Weak` does not save it: the key outlives the value.
        //
        // The Arc is held ALIVE across the unregister on purpose. If the test
        // dropped it, `current_session_phase` would return None either way and
        // pass whether or not the entry was ever removed — which is exactly the
        // hole the `drop(ipav)` case in `tray.rs` leaves open.
        let bridge = SignalingBridge::new();
        let ipav = Arc::new(Mutex::new(crate::core::ipav::IpavState::default()));
        ipav.lock().await.advance(crate::core::ipav::IpavPhase::Verify);
        bridge
            .register_session_phase("s1".into(), Arc::downgrade(&ipav))
            .await;
        bridge
            .register_session_phase("s2".into(), Arc::downgrade(&ipav))
            .await;
        assert_eq!(
            bridge.current_session_phase("s1").await,
            Some(crate::core::ipav::IpavPhase::Verify)
        );

        bridge.unregister_session("s1").await;
        assert!(
            bridge.session_phase.lock().await.get("s1").is_none(),
            "the map entry itself must go, not just the value behind the Weak"
        );
        assert_eq!(bridge.current_session_phase("s1").await, None);
        // Scoped to the session that closed.
        assert_eq!(
            bridge.current_session_phase("s2").await,
            Some(crate::core::ipav::IpavPhase::Verify)
        );
        assert!(Arc::strong_count(&ipav) > 0, "the live Arc is untouched");
    }
}
