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
use std::sync::atomic::{AtomicBool, Ordering};
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
/// `Delivered` means a BLOCKING call (`request_approval`, the pre-push gate)
/// was still waiting and received the pick synchronously. `DeliveredOutOfBand`
/// is the normal outcome for every `ask_user_choice` (non-blocking since rc3
/// D35: the receiver is dropped at park time) and for any durable row whose
/// park predates a restart: the bridge persisted the answer as the user's row
/// in session storage; the caller (`CoreAppState::resolve_choice`) wakes an
/// idle ring so the row is read at the next boundary — the row IS the
/// delivery, nothing is written to a stdin here.
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    Delivered,
    DeliveredOutOfBand {
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
    /// pumps (duo) after `storage.insert_message` returns. Lets a
    /// `wait_for_change` caller block server-side instead of polling — the
    /// external driver's tool originally, and since its removal (2026-08-17)
    /// the plugin proxy's (`tauri_cmd/plugin_api.rs`).
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
    /// The ring reached a turn boundary with a staged user response pending
    /// (the Stage toggle, 2026-08-15). Routed by main.rs to
    /// `AppState::deliver_staged`, which takes the stored content and sends
    /// it through the ordinary `send_user_response` path — answers first,
    /// message last, one release. Internal plumbing: the UI never sees this.
    StagedDeliveryDue {
        session_id: String,
    },
    /// A staged user response was delivered (posted + released). The UI
    /// clears its Stage toggle, drops the draft, and un-stages tray picks.
    StageDelivered {
        session_id: String,
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
    /// `slot0_busy`/`slot1_busy` carry the per-agent busy flags (the derived
    /// `state` collapses them) so the UI can show *which* agent is working —
    /// e.g. a broadcast sets both busy at once.
    SessionActivity {
        session_id: String,
        state: String,
        slot0_busy: bool,
        slot1_busy: bool,
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
    /// `session_id:slug` → how many times that participant has called
    /// `pass_turn` in the turn it currently holds (rc3 **D25**).
    ///
    /// **A turn carries at most one pass, and the second is incoherent by
    /// construction**: the first already recorded the whole of what a pass says.
    /// Counted here rather than in the pump because the refusal has to reach the
    /// AGENT, and the tool result is the only thing it reads synchronously.
    ///
    /// Cleared when the ring starts a turn for that participant, so the count is
    /// per-turn rather than per-session. It deliberately does NOT clear on
    /// anything the participant itself does — a wedged turn that never ends is
    /// exactly when this has to keep refusing.
    turn_passes: std::sync::Mutex<HashMap<String, u32>>,
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
    /// choice_id → (session_id, reason): reviewer-override REQUESTS parked and
    /// not yet decided. Written by `override_reviewer_block`, consumed by
    /// `resolve_choice_confirmable` (Approve → the reason moves into
    /// `reviewer_override`; Reject → dropped), voided by reviewer recovery.
    pub(super) pending_override_requests: std::sync::Mutex<HashMap<String, (String, String)>>,
    /// session_id → reason. Set by the USER approving an override request
    /// (`resolve_choice_confirmable` consumes `pending_override_requests`),
    /// honored by `check_open_findings`, auto-cleared when the reviewer
    /// recovers to running.
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
    /// Latest idle-unflagged attention state per session_id (value = the
    /// attention kind, e.g. "idle_unflagged"; absent = clear). Written by
    /// `notify_session_attention`; read by `get_session_runtime` to seed the
    /// UI chip on mount (the event fires only on change, mirroring
    /// `agent_health`). Sync `Mutex` — the notify path is sync.
    session_attention: std::sync::Mutex<HashMap<String, String>>,
    /// `session_id:agent` → the secret that agent's own mcp-config carries
    /// (C1-1). Minted per spawn, never persisted: it is only meaningful while
    /// the subprocess that holds it is alive, and a fresh spawn mints a fresh
    /// one.
    ///
    /// Absent means "this pair has no secret registered", which is NOT the same
    /// as "the secret is wrong" — see [`SignalingBridge::mcp_token_matches`] for
    /// why that distinction is what keeps a mid-upgrade session working.
    mcp_tokens: std::sync::Mutex<HashMap<String, String>>,
}

/// What an `advance_phase` call actually did — **the tool's honest answer**.
///
/// It used to be the literal `"phase advanced"`, unconditionally, which was true
/// only while an advance always happened. Once a vote gates the phase, the
/// common answer is "not yet", and the agent ACTS on this string: told it
/// advanced, it writes the next phase's document and starts mutating while the
/// session is still in the previous phase and the reviewer has not voted. The
/// vote would gate the phase while behaviour followed the tool's word, which
/// defeats the feature exactly.
///
/// So all three variants are distinguishable by their FIRST TOKEN, because a
/// model skimming a tool result reads the opening and moves on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseAdvanceOutcome {
    /// Every active participant voted; the phase moved. The only variant that
    /// reads like success.
    Advanced { target: String },
    /// The vote is recorded and **the phase has not moved**.
    VoteRecorded { target: String, voted: usize, of: usize },
    /// The caller is outside the electorate, so its vote would count toward
    /// nothing while appearing to advance the phase.
    Refused { reason: String },
}

impl PhaseAdvanceOutcome {
    /// The text handed to the agent.
    ///
    /// Four properties, each answering a way the previous wording failed:
    /// refusal is the first token (every other tool here returns a terse success
    /// token, so the SHAPE alone read as done); the CURRENT phase is named, not
    /// just the target (reading the target as current is the exact failure); the
    /// prohibition is concrete rather than abstract; and the invalidation rule is
    /// stated so re-voting is expected rather than surprising.
    pub fn message(&self, current_phase: &str) -> String {
        match self {
            Self::Advanced { target } => format!("ADVANCED — the session is now in {target}."),
            Self::VoteRecorded { target, voted, of } => format!(
                "NOT ADVANCED — the session is still in {current_phase}.\n\
                 Your vote to advance to {target} is recorded ({voted} of {of} participants).\n\
                 The phase moves only when every active participant has voted for this same \
                 state of the work. Do NOT act as though you are in {target}: no {target}-phase \
                 edits, no work that belongs to it. Your peers read this turn's output when the \
                 ring hands them their next turn; expect their votes then. If a peer posts \
                 findings, address them — changing the work (including a phase-doc write) \
                 invalidates the votes cast on the old state, including your own, and everyone \
                 re-votes."
            ),
            Self::Refused { reason } => format!(
                "REFUSED — your vote was not recorded, and the session is still in \
                 {current_phase}. Reason: {reason}. Participants outside the voting roster \
                 cannot move the phase; ask an active participant to call `advance_phase`, or \
                 raise it in the channel."
            ),
        }
    }
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
            pending_override_requests: std::sync::Mutex::new(HashMap::new()),
            session_reviewers: std::sync::Mutex::new(HashMap::new()),
            session_attention: std::sync::Mutex::new(HashMap::new()),
            turn_passes: std::sync::Mutex::new(HashMap::new()),
            mcp_tokens: std::sync::Mutex::new(HashMap::new()),
        })
    }

    pub fn new() -> Arc<Self> {
        Self::new_with(None, None)
    }

    /// Construct a bridge with a violations log attached. Approval-class
    /// resolutions write a record after the user picks an option.
    /// Test-only since round 7 (2026-08-17): no production caller — kept as a test seam, not shipped.
    #[cfg(test)]
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
    /// `SequencerCommand::HaltDeclared` was written, documented and tested for
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
    /// Shipped broken on 2026-08-13 for about two hours: `HaltDeclared` was
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
    /// Tell the ring an approval gate opened (`opened = true`) or resolved
    /// (`false`) — rc3 **D35**: while any gate is open the ring deals no turns.
    /// `try_send` like every ring notify: this runs inside tool calls and
    /// resolve paths that must not block on the ring's queue. A miss on
    /// `opened` is healed at the next respawn (the latch is seeded from the
    /// durable rows); a miss on `resolved` is healed the same way, and the
    /// user's release still drains the session's rows.
    pub async fn notify_ring_gate(&self, session_id: &str, choice_id: &str, opened: bool) {
        let seq = self.session_sequencer.lock().await.get(session_id).cloned();
        if let Some(tx) = seq {
            let choice_id = choice_id.to_string();
            let cmd = if opened {
                crate::core::sequencer::SequencerCommand::GateOpened { choice_id }
            } else {
                crate::core::sequencer::SequencerCommand::GateResolved { choice_id }
            };
            if tx.try_send(cmd).is_err() {
                tracing::warn!(
                    session_id,
                    opened,
                    "a gate notification did not reach the ring; the latch reseeds on respawn"
                );
            }
        }
    }

    /// Mint-and-remember the secret one agent's mcp-config will carry (C1-1).
    ///
    /// Called at spawn, once per agent, with the value written into that
    /// agent's own config file and nowhere else.
    pub fn register_mcp_token(&self, session_id: &str, agent: &str, token: &str) {
        self.mcp_tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(format!("{session_id}:{agent}"), token.to_string());
    }

    /// Does this caller hold the secret registered for the pair it claims to be?
    ///
    /// **Absent registration ALLOWS, and that is deliberate.** A pair with no
    /// secret on file is a session whose agents were spawned before this
    /// existed: refusing it would break a live session at upgrade time, which is
    /// a worse failure than the impersonation it would prevent — and the window
    /// closes on its own, since every spawn from here registers one. A pair WITH
    /// a secret is checked strictly, and that is every live session after the
    /// next relaunch.
    ///
    /// Constant-time compare, mirroring `external_server`: a timing oracle on a
    /// localhost secret is a stretch, but the cost of using the right primitive
    /// is nothing.
    ///
    /// **SCOPE — this is not an adversarial boundary between same-user agents,
    /// and should not be mistaken for one later.** The secret is written with
    /// `std::fs::write` into the per-agent temp dir at default permissions, and
    /// every participant runs as the same uid holding `Read` and `Bash`: a peer
    /// that goes LOOKING can read a sibling's config and present its token. What
    /// this closes is everything short of that — misrouting, guessing, a URL
    /// copied from a log, and any caller with no filesystem access to that
    /// sibling directory (a plugin webview, an external process). Making it hold
    /// against a determined peer needs the secret out of a readable file
    /// (per-agent 0600 at minimum, or handed over a channel only that
    /// subprocess can read), which is a separate change.
    pub fn mcp_token_matches(&self, session_id: &str, agent: &str, presented: Option<&str>) -> bool {
        use subtle::ConstantTimeEq;
        let expected = self
            .mcp_tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&format!("{session_id}:{agent}"))
            .cloned();
        match (expected, presented) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(expected), Some(presented)) => {
                expected.as_bytes().ct_eq(presented.as_bytes()).into()
            }
        }
    }

    /// **Stop the ring where it stands** — rc3's Pause, minted at last.
    ///
    /// `SequencerCommand::Pause` and the whole held-queue machinery behind it
    /// shipped with NO producer: `cancel_session_turn` flipped the activity
    /// latch and nothing told the ring, so after an honoured interrupt the
    /// holder's `result` still completed its turn, the ring dealt the NEXT
    /// participant, and the user watched a fresh turn start under a ⏸ Paused
    /// banner. (If that turn then failed to settle, the escalation SIGKILLed
    /// everyone.)
    ///
    /// There is deliberately no `Resume` producer: a user message IS the
    /// release, and the ring's own `UserMessage`-while-paused arm does it —
    /// re-queuing the message so it keeps the mentions the user typed. A second
    /// release path would be two mechanisms for one job, which in this codebase
    /// is how the older one silently disables the newer.
    ///
    /// `try_send` like every ring notify: this runs inside the Stop path, which
    /// must not block on the ring's queue. A miss leaves the session in the
    /// behaviour it had before this existed.
    pub async fn notify_ring_pause(&self, session_id: &str) {
        let seq = self.session_sequencer.lock().await.get(session_id).cloned();
        if let Some(tx) = seq {
            if tx
                .try_send(crate::core::sequencer::SequencerCommand::Pause)
                .is_err()
            {
                tracing::warn!(
                    session_id,
                    "the pause did not reach the ring; it may deal one more turn"
                );
            }
        }
    }

    /// The ring reached a boundary with a stage pending: hand delivery to the
    /// app layer (main.rs routes this to `AppState::deliver_staged`).
    pub fn notify_staged_delivery_due(&self, session_id: &str) {
        let _ = self.event_tx.send(SignalingEvent::StagedDeliveryDue {
            session_id: session_id.to_string(),
        });
    }

    /// A staged response was posted + released — the UI clears its toggle,
    /// draft, and staged tray picks.
    pub fn notify_stage_delivered(&self, session_id: &str) {
        let _ = self.event_tx.send(SignalingEvent::StageDelivered {
            session_id: session_id.to_string(),
        });
    }

    /// Tell the ring a STAGED user response now exists (or was withdrawn) —
    /// the Stage toggle (2026-08-15). The content stays in `AppState`; the
    /// ring holds only the flag and emits `StagedDeliveryDue` at the next
    /// turn boundary, where delivery cannot supersede a turn in flight.
    pub async fn notify_ring_stage(&self, session_id: &str, staged: bool) {
        let seq = self.session_sequencer.lock().await.get(session_id).cloned();
        if let Some(tx) = seq {
            let cmd = if staged {
                crate::core::sequencer::SequencerCommand::MessageStaged
            } else {
                crate::core::sequencer::SequencerCommand::MessageUnstaged
            };
            if tx.try_send(cmd).is_err() {
                tracing::warn!(
                    session_id,
                    staged,
                    "a stage notification did not reach the ring; the staged \
                     message waits for the next boundary the flag does reach"
                );
            }
        }
    }

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

    /// Record a `pass_turn` and say how many this participant has now made in
    /// the turn it holds (rc3 **D25**). `1` is the first and is legitimate.
    ///
    /// **The bound exists because the round cap cannot see this.** The cap counts
    /// LAPS of the ring, so a participant looping inside ONE turn — which is what
    /// a stale-epoch turn that never ends produces — spends real model calls
    /// while the counter it is supposed to trip stays at zero. Measured in
    /// `s-a4e9a1b4` on 2026-08-13: 141 `pass_turn` calls in the eight minutes
    /// after the last handover, one every two seconds, and the only thing that
    /// stopped it was the user watching.
    pub fn record_pass(&self, session_id: &str, agent: &str) -> u32 {
        let mut map = self
            .turn_passes
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let n = map.entry(format!("{session_id}:{agent}")).or_insert(0);
        *n += 1;
        *n
    }

    /// Forget a participant's pass count — called by the ring when it hands that
    /// participant a turn, which is the only event that makes a new pass
    /// legitimate.
    pub fn clear_passes(&self, session_id: &str, agent: &str) {
        self.turn_passes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&format!("{session_id}:{agent}"));
    }

    /// Whether a turn ring is reachable for this session — the observable half
    /// of [`Self::register_session_sequencer`], so the spawn-time join can be
    /// pinned by a test.
    /// Test-only since round 7 (2026-08-17): no production caller — kept as a test seam, not shipped.
    #[cfg(test)]
    pub async fn session_sequencer_registered(&self, session_id: &str) -> bool {
        self.session_sequencer.lock().await.contains_key(session_id)
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
        // pending — tokio Mutex of parked choices; a non-blocking ask_user_choice
        // leaves a Parked entry whose receiver was dropped. Drop this session's.
        self.pending
            .lock()
            .await
            .retain(|_, p| p.choice.session_id != session_id);
        // **The six that were never removed.** Sixteen per-session maps, each
        // needing its own line here, and six of them had none — found by reading
        // the registrations against this function rather than by anything going
        // wrong, which is the shape the audit calls "one map, one line, no type
        // saying so" (the fix for that is the PerSession registry, a refactor).
        //
        // `session_sequencer` is the expensive one: its `Sender` is the ring
        // task's last, so holding it here means `run_sequencer` never sees its
        // channel close and the task lives for the life of the process — one
        // orphan ring per closed session. `session_attention` is the visible
        // one: `get_session_runtime` seeds the UI chip from it, so REOPENING a
        // session id showed the previous run's attention badge.
        self.session_sequencer.lock().await.remove(session_id);
        self.turn_passes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|k, _| !k.starts_with(&format!("{session_id}:")));
        self.agent_rpc_seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(s, _), _| s != session_id);
        self.pending_override_requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, (s, _)| s != session_id);
        self.session_reviewers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id);
        self.session_attention
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id);
        self.mcp_tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|k, _| !k.starts_with(&format!("{session_id}:")));
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
    /// row id. `wait_for_change` subscribes to these so callers need not poll —
    /// the plugin proxy's copy (`tauri_cmd/plugin_api.rs`) since the external
    /// driver was removed (2026-08-17).
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

    /// Cast a participant's vote to advance the phase, and say what happened.
    ///
    /// **The phase moves only when every active participant has voted for the
    /// same state of the work** (migration 0062, the user's design). So this is
    /// a vote first and a transition second, and the caller has to be told which
    /// of the two it just did — the tool used to answer the literal string
    /// `"phase advanced"` unconditionally, which was true while the advance
    /// always happened and becomes false in the COMMON case now.
    ///
    /// ## Why this resolves synchronously instead of awaiting AppState
    ///
    /// Every input is in STORAGE, which this bridge already holds: the epoch and
    /// the electorate (0062), the fingerprint (`session_documents`), the target
    /// (the caller's argument). The live phase is in-memory `AppState` and is
    /// needed only to PERFORM an advance — which the agent does not wait on.
    ///
    /// The alternative was a `oneshot` on the event, and it is wrong for a
    /// reason recorded three lines from the handler it would attach to:
    /// `main.rs` says *"the slow work (close kills subprocesses) runs on a
    /// SEPARATE serial worker fed by an unbounded queue"*, and that worker takes
    /// `AgentAdvancePhase` and `SessionCloseRequest` off one FIFO. An agent
    /// awaiting there waits behind whatever is queued, including a close killing
    /// subprocesses. The rule in `tray.rs` is not "never block on a human" — it
    /// is **never block a tool call on something whose completion time you do
    /// not control**, and a serial worker carrying the slow work is exactly
    /// that.
    ///
    /// ## The one consequence, owned
    ///
    /// A crash between the vote landing and the event firing leaves a complete
    /// tally with the phase unmoved. It self-heals: the next `advance_phase`
    /// re-reads the same complete tally and re-fires. That is strictly better
    /// than the failure it replaces, which was telling an agent "advanced" when
    /// nothing had happened at all.
    pub async fn agent_advance_phase(
        &self,
        session_id: String,
        agent: String,
        target: String,
    ) -> PhaseAdvanceOutcome {
        let storage = { self.storage.lock().await.clone() };
        let Some(storage) = storage else {
            // No storage wired (tests, early boot): preserve the old
            // fire-and-forget behaviour rather than refusing an advance.
            let _ = self.event_tx.send(SignalingEvent::AgentAdvancePhase {
                session_id,
                agent,
                target: target.clone(),
            });
            return PhaseAdvanceOutcome::Advanced { target };
        };

        let me = match storage.participant_by_slug(&session_id, &agent).await {
            Ok(Some(p)) => p,
            _ => {
                // An unknown caller cannot be in the electorate, so it cannot
                // vote — but refusing outright would wedge a session whose
                // roster read failed. Fall back to the old behaviour.
                let _ = self.event_tx.send(SignalingEvent::AgentAdvancePhase {
                    session_id,
                    agent,
                    target: target.clone(),
                });
                return PhaseAdvanceOutcome::Advanced { target };
            }
        };

        // **`on_mention` holds the tool but sits outside the electorate.** Its
        // vote would count toward nothing while the tool said the phase moved,
        // and the actives would never vote because the advance looked done — a
        // dead phase with no signal. Refusing names the reason instead.
        if me.participation_mode != crate::storage::MODE_ACTIVE || !me.enabled {
            return PhaseAdvanceOutcome::Refused {
                reason: format!(
                    "you are `{}` in this session, which is outside the voting roster",
                    me.participation_mode
                ),
            };
        }

        let epoch = storage.phase_epoch(&session_id).await.unwrap_or(0);
        let fingerprint = storage
            .phase_artifact_fingerprint(&session_id)
            .await
            .unwrap_or_default();
        if let Err(e) = storage
            .cast_phase_vote(&session_id, me.id, &target, &fingerprint, epoch)
            .await
        {
            tracing::warn!(?e, %session_id, %agent, "casting a phase vote failed");
        }
        let (voted, of) = storage
            .phase_vote_tally(&session_id, &target, &fingerprint, epoch)
            .await
            .unwrap_or((0, 0));

        match storage
            .all_active_voted_to_advance(&session_id, &target, &fingerprint, epoch)
            .await
        {
            Ok(true) => {
                let _ = self.event_tx.send(SignalingEvent::AgentAdvancePhase {
                    session_id,
                    agent,
                    target: target.clone(),
                });
                PhaseAdvanceOutcome::Advanced { target }
            }
            Ok(false) => PhaseAdvanceOutcome::VoteRecorded { target, voted, of },
            Err(e) => {
                tracing::warn!(?e, %session_id, "reading the phase tally failed");
                PhaseAdvanceOutcome::VoteRecorded { target, voted, of }
            }
        }
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
    pub fn notify_agent_health(self: &Arc<Self>, session_id: String, agent: &str, health: &str) {
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
            // A recovered reviewer also VOIDS any override request still
            // parked for the user — the down-incident it asked about is over,
            // and an approval landing later must not lift a future block.
            // Withdraw the tray row too (it un-parks the ring's gate latch);
            // spawned because this method is sync and the withdraw is not.
            let voided: Vec<String> = {
                let mut pending = self
                    .pending_override_requests
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                let ids: Vec<String> = pending
                    .iter()
                    .filter(|(_, (sid, _))| sid == &session_id)
                    .map(|(cid, _)| cid.clone())
                    .collect();
                for cid in &ids {
                    pending.remove(cid);
                }
                ids
            };
            if !voided.is_empty() {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let bridge = Arc::clone(self);
                    handle.spawn(async move {
                        for cid in voided {
                            tracing::info!(
                                choice_id = %cid,
                                "reviewer recovered; withdrawing the parked override request"
                            );
                            let _ = bridge.withdraw_question(&cid, None).await;
                        }
                    });
                }
            }
        }
        // **A verdict that only ever reached the UI could not be read back**
        // (rc3 D26). The health map is in memory and the event is transient, so
        // once a window closed nothing recorded what bot-hq had CONCLUDED about
        // a participant — and a post-hoc dissection could not tell "the watchdog
        // called it stalled" from "the watchdog never ran". On 2026-08-13 that
        // ambiguity produced a wrong diagnosis of a live session: silence in the
        // log was read as the watchdog not firing, when the log was never going
        // to show it either way.
        //
        // INFO rather than debug: a transition is rare (a participant goes
        // stalled or recovers), it is exactly what someone reading a stalled
        // session wants, and `./start` tees the log to a file for that purpose.
        tracing::info!(
            session_id = %session_id,
            agent,
            health,
            "agent health"
        );
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

    /// Publish an idle-unflagged attention change. Deduped here (not at the
    /// caller): the watchdog re-evaluates every poll, so it calls this every
    /// 10s while the condition holds — only an actual transition reaches the
    /// wire. `state=None` clears. Mirrors `notify_agent_health`.
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

    /// The reviewer-down escape valve, as a REQUEST (vision alignment,
    /// 2026-08-14): the agent no longer overrides the block itself — it parks
    /// an Approve/Reject decision for the user and the session waits at the
    /// gate. Measured in `s-f6a441ff` (12:26Z): HANDS self-overrode with a
    /// four-line justification that was reasonable AND was still an agent
    /// asserting a path at a junction — "agents recommend; the user decides"
    /// is the vision's line, and a dead reviewer is a junction. The override
    /// takes effect only when the user picks Approve
    /// (`resolve_choice_confirmable` consumes the pending request); a
    /// recovered reviewer voids the request before it is answered.
    pub async fn override_reviewer_block(&self, session_id: &str, agent: &str, reason: &str) -> String {
        // One pending request per session — a duplicate call returns the
        // existing gate rather than stacking confusable prompts (the
        // action-gate's own rule).
        {
            let pending = self
                .pending_override_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some((existing_choice, _)) = pending
                .iter()
                .find(|(_, (sid, _))| sid == session_id)
                .map(|(c, v)| (c.clone(), v.clone()))
            {
                return format!(
                    "override already requested (gate {existing_choice} pending) — the \
                     user's Approve/Reject decides; the outcome arrives out-of-band."
                );
            }
        }
        let parked = match self
            .ask_user_choice_inner(
                session_id.to_string(),
                agent.to_string(),
                format!(
                    "Reviewer is down — {agent} asks to override the review block.\n\n\
                     Reason: {reason}\n\nApprove lifts the commit block for this \
                     down-incident (auto-clears when the reviewer recovers); Reject \
                     keeps commits blocked until the reviewer is back."
                ),
                vec!["Approve".to_string(), "Reject".to_string()],
                Some(ApprovalContext {
                    kind: crate::policy::ViolationKind::GenericApproval,
                    action: format!("override reviewer-down block: {reason}"),
                    detail: Some("reviewer-override".to_string()),
                }),
                None,
                false,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return format!("could not park the override request: {e}");
            }
        };
        let choice_id = serde_json::from_str::<serde_json::Value>(&parked)
            .ok()
            .and_then(|v| {
                v.get("choice_id")
                    .and_then(|c| c.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        tracing::info!(
            session = %session_id,
            agent,
            reason = %reason,
            choice_id = %choice_id,
            "reviewer-down override REQUESTED; parked for the user's approval"
        );
        self.pending_override_requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                choice_id.clone(),
                (session_id.to_string(), reason.to_string()),
            );
        format!(
            "override REQUESTED — parked for the user's approval (gate {choice_id}); \
             the session waits at the gate. If approved, the block lifts for this \
             down-incident; if rejected, restore the reviewer or wait."
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
    /// `state` is the `SessionActivity::as_str` string; `slot0_busy`/`slot1_busy`
    /// are the per-agent flags the UI uses to label which agent is working.
    pub fn notify_session_activity(
        &self,
        session_id: String,
        state: &str,
        slot0_busy: bool,
        slot1_busy: bool,
    ) {
        self.persist_activity_event(&session_id, state, slot0_busy, slot1_busy);
        let _ = self.event_tx.send(SignalingEvent::SessionActivity {
            session_id,
            state: state.to_string(),
            slot0_busy,
            slot1_busy,
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
        slot0_busy: bool,
        slot1_busy: bool,
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
                .insert_activity_event(&session_id, &state, slot0_busy, slot1_busy)
                .await
            {
                tracing::warn!(?e, %session_id, %state, "persisting activity event failed");
            }
        });
    }
}

#[cfg(test)]
mod phase_vote_tests {
    use super::*;
    use crate::storage::Storage;

    async fn seeded(n: usize) -> (std::sync::Arc<SignalingBridge>, Storage) {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.ensure_session_roster("s1", n).await.unwrap();
        (bridge, storage)
    }

    /// **The feature, as one assertion.** The executor voting does NOT move the
    /// phase, and the tool says so — this is the turn the reviewer gets, and
    /// the string is what stops the agent acting as though it already advanced.
    #[tokio::test]
    async fn one_vote_of_two_records_without_advancing() {
        let (bridge, _s) = seeded(crate::storage::MAX_SESSION_PARTICIPANTS).await;
        let out = bridge
            .agent_advance_phase("s1".into(), "hands".into(), "Plan".into())
            .await;
        assert_eq!(
            out,
            PhaseAdvanceOutcome::VoteRecorded { target: "Plan".into(), voted: 1, of: 2 }
        );
        let msg = out.message("Investigate");
        assert!(msg.starts_with("NOT ADVANCED"), "the refusal must be the first token: {msg}");
        assert!(msg.contains("still in Investigate"), "it must name the CURRENT phase: {msg}");
        assert!(msg.contains("Do NOT act as though you are in Plan"), "{msg}");
    }

    /// The second vote completes it, and only then does the phase move.
    #[tokio::test]
    async fn the_last_vote_advances() {
        let (bridge, _s) = seeded(crate::storage::MAX_SESSION_PARTICIPANTS).await;
        let mut sub = bridge.subscribe();
        bridge.agent_advance_phase("s1".into(), "hands".into(), "Plan".into()).await;
        assert!(
            sub.try_recv().is_err(),
            "an incomplete tally must NOT fire the advance event"
        );
        let out = bridge
            .agent_advance_phase("s1".into(), "eyes".into(), "Plan".into())
            .await;
        assert_eq!(out, PhaseAdvanceOutcome::Advanced { target: "Plan".into() });
        assert!(
            matches!(sub.try_recv(), Ok(SignalingEvent::AgentAdvancePhase { .. })),
            "a complete tally is what fires the advance"
        );
        assert!(out.message("Investigate").starts_with("ADVANCED"));
    }

    /// **`on_mention` holds the tool and is outside the electorate.** Accepting
    /// its vote would count it toward nothing while the tool reported an
    /// advance, and the actives would never vote because the phase looked moved
    /// — a dead phase with no signal anywhere.
    #[tokio::test]
    async fn an_on_mention_participant_is_refused_not_silently_counted() {
        let (bridge, storage) = seeded(crate::storage::MAX_SESSION_PARTICIPANTS).await;
        sqlx::query("UPDATE session_participants SET participation_mode = 'on_mention' WHERE slug = 'eyes'")
            .execute(storage.pool())
            .await
            .unwrap();
        let out = bridge
            .agent_advance_phase("s1".into(), "eyes".into(), "Plan".into())
            .await;
        assert!(matches!(out, PhaseAdvanceOutcome::Refused { .. }), "{out:?}");
        assert!(out.message("Investigate").starts_with("REFUSED"));
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM phase_votes")
            .fetch_one(storage.pool())
            .await
            .unwrap();
        assert_eq!(n.0, 0, "a refused vote must not be recorded");
    }

    /// Solo: the one active participant IS the electorate, so its vote advances
    /// immediately. Without this the feature would deadlock every solo session.
    #[tokio::test]
    async fn a_solo_session_advances_on_the_first_vote() {
        let (bridge, _s) = seeded(1).await;
        let out = bridge
            .agent_advance_phase("s1".into(), "hands".into(), "Plan".into())
            .await;
        assert_eq!(out, PhaseAdvanceOutcome::Advanced { target: "Plan".into() });
    }

    /// **Editing the work invalidates the votes cast on it** — end to end, not
    /// at the storage layer. The reviewer votes, the author rewrites the doc,
    /// and the tally reopens because the fingerprint moved.
    #[tokio::test]
    async fn rewriting_the_work_reopens_a_completed_tally() {
        let (bridge, storage) = seeded(crate::storage::MAX_SESSION_PARTICIPANTS).await;
        bridge.agent_advance_phase("s1".into(), "eyes".into(), "Plan".into()).await;
        storage
            .upsert_session_document("s1", "investigate", "findings", Some("investigate"))
            .await
            .unwrap();
        let out = bridge
            .agent_advance_phase("s1".into(), "hands".into(), "Plan".into())
            .await;
        assert_eq!(
            out,
            PhaseAdvanceOutcome::VoteRecorded { target: "Plan".into(), voted: 1, of: 2 },
            "the reviewer's vote was cast on work that has since changed, so only \
             the fresh vote counts and the tally reopens"
        );
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

    /// **Every per-session map is actually emptied** (round-2 audit, R2).
    ///
    /// `unregister_session` has sixteen drain lines and, before this, ONE was
    /// pinned — `closing_a_session_unregisters_its_phase` below asserts on
    /// `session_phase` alone, and `session_sequencer` turned out to be covered
    /// from `sequencer.rs`'s own module. The reviewer cut the other fourteen in
    /// one go, with `session_phase` deliberately left in the cut set as a
    /// positive control, and the suite reported `1164 passed; 1 failed` — the
    /// control. Fourteen drains deleted green, including `session_attention`,
    /// whose absence was a USER-VISIBLE bug batch 1 had just fixed (reopening a
    /// session id showed the previous run's attention badge).
    ///
    /// Seeded by writing the maps directly rather than through sixteen
    /// registration APIs: this is a test of the DRAIN, and routing through the
    /// registrations would make it fail for reasons that are not the drain's.
    #[tokio::test]
    async fn unregister_session_empties_every_per_session_map() {
        let b = SignalingBridge::new();
        let sid = "s-doomed";
        let keep = "s-survivor";

        // Seed both sessions into all sixteen maps. `keep` is the negative
        // control: a drain that over-reaches (clear() instead of remove()) is
        // as wrong as one that never runs, and only a second session shows it.
        for s in [sid, keep] {
            b.session_projects
                .lock()
                .await
                .insert(s.into(), Some("p".into()));
            b.session_awaiting
                .lock()
                .await
                .insert(s.into(), Arc::new(AtomicBool::new(true)));
            b.session_activity.lock().await.insert(s.into(), Weak::new());
            b.session_phase.lock().await.insert(s.into(), Weak::new());
            b.session_close_gate
                .lock()
                .await
                .insert(s.into(), CloseGateState::default());
            b.session_sequencer
                .lock()
                .await
                .insert(s.into(), tokio::sync::mpsc::channel(1).0);
            b.pending.lock().await.insert(
                format!("{s}-choice"),
                Parked {
                    tx: tokio::sync::oneshot::channel().0,
                    choice: PendingChoice {
                        choice_id: format!("{s}-choice"),
                        session_id: s.into(),
                        agent: "hands".into(),
                        question: "q".into(),
                        options: vec!["a".into()],
                        approval: None,
                    },
                },
            );
            b.agent_health
                .lock()
                .unwrap()
                .insert((s.into(), "hands".into()), "running".into());
            b.agent_rpc_seen
                .lock()
                .unwrap()
                .insert((s.into(), "hands".into()), std::time::Instant::now());
            b.reviewer_override
                .lock()
                .unwrap()
                .insert(s.into(), "why".into());
            b.turn_passes.lock().unwrap().insert(format!("{s}:hands"), 1);
            b.pending_override_requests
                .lock()
                .unwrap()
                .insert(format!("{s}-req"), (s.into(), "hands".into()));
            b.session_reviewers
                .lock()
                .unwrap()
                .insert(s.into(), vec!["eyes".into()]);
            b.session_attention
                .lock()
                .unwrap()
                .insert(s.into(), "idle_unflagged".into());
            b.mcp_tokens
                .lock()
                .unwrap()
                .insert(format!("{s}:hands"), "tok".into());
        }

        b.unregister_session(sid).await;

        // `(map, still-holds-doomed, still-holds-survivor)` — both directions
        // per map, because a drain can fail two ways and only one of them is
        // the absent line. `clear()` where `remove()` was meant empties the map
        // for every LIVE session too, and a length check cannot tell the two
        // apart.
        let mut report: Vec<(&str, bool, bool)> = Vec::new();
        // The predicate is a token pattern, not a closure argument: a closure
        // passed INTO a macro has no call site to infer its parameter from, so
        // every one of the sixteen would need a spelled-out map type.
        macro_rules! check {
            ($name:literal, $guard:expr, |$m:ident, $s:ident| $body:expr) => {{
                let g = $guard;
                let probe = |$s: &str| -> bool {
                    let $m = &*g;
                    $body
                };
                report.push(($name, probe(sid), probe(keep)));
            }};
        }
        check!("session_projects", b.session_projects.lock().await, |m, s| {
            m.contains_key(s)
        });
        check!("session_awaiting", b.session_awaiting.lock().await, |m, s| {
            m.contains_key(s)
        });
        check!("session_activity", b.session_activity.lock().await, |m, s| {
            m.contains_key(s)
        });
        check!("session_phase", b.session_phase.lock().await, |m, s| {
            m.contains_key(s)
        });
        check!(
            "session_close_gate",
            b.session_close_gate.lock().await,
            |m, s| m.contains_key(s)
        );
        check!(
            "session_sequencer",
            b.session_sequencer.lock().await,
            |m, s| m.contains_key(s)
        );
        check!("pending", b.pending.lock().await, |m, s| {
            m.values().any(|p| p.choice.session_id == s)
        });
        check!("agent_health", b.agent_health.lock().unwrap(), |m, s| {
            m.keys().any(|(k, _)| k == s)
        });
        check!("agent_rpc_seen", b.agent_rpc_seen.lock().unwrap(), |m, s| {
            m.keys().any(|(k, _)| k == s)
        });
        check!(
            "reviewer_override",
            b.reviewer_override.lock().unwrap(),
            |m, s| m.contains_key(s)
        );
        check!("turn_passes", b.turn_passes.lock().unwrap(), |m, s| {
            m.keys().any(|k| k.starts_with(&format!("{s}:")))
        });
        check!(
            "pending_override_requests",
            b.pending_override_requests.lock().unwrap(),
            |m, s| m.values().any(|(owner, _)| owner == s)
        );
        check!(
            "session_reviewers",
            b.session_reviewers.lock().unwrap(),
            |m, s| m.contains_key(s)
        );
        check!(
            "session_attention",
            b.session_attention.lock().unwrap(),
            |m, s| m.contains_key(s)
        );
        check!("mcp_tokens", b.mcp_tokens.lock().unwrap(), |m, s| {
            m.keys().any(|k| k.starts_with(&format!("{s}:")))
        });

        // Tracks THIS test's manual `check!` list, so it moves when a map is
        // legitimately added or removed — 16 until round 6 deleted
        // `session_open_blocking` with the rest of the router's orphaned cache.
        assert_eq!(report.len(), 15, "a map was dropped from this test's sweep");
        let undrained: Vec<&str> = report
            .iter()
            .filter(|(_, doomed, _)| *doomed)
            .map(|(n, _, _)| *n)
            .collect();
        assert!(
            undrained.is_empty(),
            "map(s) still holding the CLOSED session — each one outlives the \
             session it belongs to: {undrained:?}"
        );
        let over_reached: Vec<&str> = report
            .iter()
            .filter(|(_, _, survivor)| !*survivor)
            .map(|(n, _, _)| *n)
            .collect();
        assert!(
            over_reached.is_empty(),
            "map(s) that also dropped an UNRELATED live session — `clear()` \
             where `remove()` was meant: {over_reached:?}"
        );
    }

    /// **A seventeenth map cannot be added without a drain line** (round-2, R2).
    ///
    /// The behavioural test above pins the sixteen that exist; nothing pins the
    /// one somebody adds next, and THAT is the failure this function keeps
    /// having — six maps once had no drain line at all, and batch 1's own fix
    /// introduced a seventeenth (`mcp_tokens`) while its commit message said
    /// "all sixteen". The audit's structural answer is a `PerSession` registry
    /// so the type system says it; until that lands, this says it from the
    /// source.
    ///
    /// Derived, not restated: the expectation is computed from the struct's own
    /// fields, so the only way to pass is to agree with the declaration. There
    /// is deliberately NO exemption list — every `HashMap` field on this struct
    /// is per-session today, and a list of sanctioned absences is precisely the
    /// thing that rots into permission for the next one.
    ///
    /// **What this proves, exactly.** It answers one question — *is there a
    /// field `unregister_session` never mentions?* — and nothing more. It is a
    /// TEXT search over this file's source, so it cannot tell a drain from a
    /// comment: deleting the `session_attention` line and leaving a comment
    /// that says `self.session_attention` passes here, measured. The
    /// behavioural test above is what catches that, and only the PAIR is the
    /// guarantee; neither half is on its own.
    ///
    /// The `self.` prefix on the needle is load-bearing and must survive any
    /// refactor of this test: a bare-name needle is already satisfied by this
    /// function's own comment block, which writes `session_sequencer` and
    /// `session_attention` in backticks. That is round 1's "backticked NAME"
    /// doc-guard defect in a new place — a needle a mention can satisfy.
    #[test]
    fn every_hashmap_field_is_named_in_unregister_session() {
        let src = include_str!("mod.rs");

        let struct_start = src
            .find("pub struct SignalingBridge {")
            .expect("the struct declaration moved");
        let struct_body = &src[struct_start..][..src[struct_start..]
            .find("\n}\n")
            .expect("unterminated struct")];

        let fn_start = src
            .find("pub async fn unregister_session(&self, session_id: &str) {")
            .expect("unregister_session's signature moved");
        let fn_body = &src[fn_start..][..src[fn_start..]
            .find("\n    }\n")
            .expect("unterminated fn")];

        // Field name → the text of its declared type, joined across the
        // multi-line declarations (`session_sequencer` is one).
        let mut current: Option<(String, String)> = None;
        let mut fields: Vec<(String, String)> = Vec::new();
        for line in struct_body.lines().skip(1) {
            let t = line.trim_start();
            if t.starts_with("///") || t.starts_with("//") || t.starts_with('#') {
                continue;
            }
            let decl = t
                .strip_prefix("pub(super) ")
                .or_else(|| t.strip_prefix("pub "))
                .unwrap_or(t);
            match decl.split_once(':') {
                Some((name, rest))
                    if !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()) =>
                {
                    if let Some(f) = current.take() {
                        fields.push(f);
                    }
                    current = Some((name.to_string(), rest.to_string()));
                }
                _ => {
                    if let Some((_, ty)) = current.as_mut() {
                        ty.push_str(t);
                    }
                }
            }
        }
        if let Some(f) = current.take() {
            fields.push(f);
        }

        let maps: Vec<&str> = fields
            .iter()
            .filter(|(_, ty)| ty.contains("HashMap"))
            .map(|(n, _)| n.as_str())
            .collect();
        // The floor exists so a silently-broken field parser cannot make the
        // sweep below vacuous. It used to be a hardcoded `>= 16`, which is a
        // tripwire on every legitimate field DELETION rather than a check on
        // the parser — round 6 removed one map and this fired reading "the
        // parser broke, not the code" when the code was exactly what changed.
        // Derived from the struct text instead, by a different rule than the
        // parser uses, so the two can disagree only if the parser is wrong.
        let declared = struct_body
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with('#') && t.contains("HashMap")
            })
            .count();
        assert!(declared >= 8, "the struct text scan found {declared} HashMap lines — both counts broke");
        assert_eq!(
            maps.len(),
            declared,
            "the field parser found {} maps but {declared} lines declare one — \
             the parser broke, not the code: {maps:?}",
            maps.len()
        );

        let undrained: Vec<&&str> = maps
            .iter()
            .filter(|n| !fn_body.contains(&format!("self.{n}")))
            .collect();
        assert!(
            undrained.is_empty(),
            "per-session map(s) `unregister_session` never MENTIONS (this is a \
             source-text check — that they are mentioned is not proof they are \
             drained; `unregister_session_empties_every_per_session_map` proves \
             that). An unmentioned map is a leak that outlives its session, and \
             the last two found this way were an orphan ring task per closed \
             session and a stale attention badge on reopen: {undrained:?}"
        );
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
