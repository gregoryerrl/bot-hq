//! Spawn a `claude-code` subprocess wired up with stream-json IO + the
//! MCP-signaling server. Returns an `AgentHandle` the core layer drives.
//!
//! This module also owns the vocabulary everything downstream of a spawn
//! speaks — `AgentHandle`, `AgentEvent`, `AgentHealth`, `ContextUsage`,
//! `RetryPolicy`, `SpawnConfig`. It was written so a SECOND backend could build
//! against it rather than duplicate a parallel vocabulary; rc3 D9 deleted that
//! backend, and the separation stays because it is what made the deletion a
//! no-op downstream — nothing past `AgentHandle` ever knew which one it had.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::agents::events;
use crate::agents::input;
use crate::agents::protocol::{ControlRequest, OutgoingUserMessage};
use crate::storage::{AgentConfig, PersistedMessage};

/// Global registry of live claude-code child PIDs. Updated by
/// `spawn_agent` (insert) and the lifecycle task (remove on exit). Read
/// by `reap_all_children` from `main.rs`'s panic hook + signal handler
/// so the children get SIGKILL even when the tokio runtime can't be
/// trusted (panic-abort / SIGTERM paths skip Drop chains entirely).
pub static CHILD_PIDS: LazyLock<Mutex<HashSet<u32>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Sync, signal-safe child reaper. Walks the registered PIDs and
/// force-kills each via the per-platform `kill_child` (unix SIGKILL /
/// Windows TerminateProcess) — no tokio, no async, no Drop chain.
///
/// Uses `try_lock` (not `lock`) so the panic hook can't deadlock against
/// a spawn-in-progress on another thread, and so a same-thread panic
/// mid-`insert()` doesn't recurse. Worst case on contention: one
/// cleanup cycle skipped — preferable to a hang.
pub fn reap_all_children() {
    let pids: Vec<u32> = match CHILD_PIDS.try_lock() {
        Ok(g) => g.iter().copied().collect(),
        Err(_) => return,
    };
    for pid in pids {
        kill_child(pid);
    }
}

/// Force-kill one child by PID. Best-effort: a kill that fails (process
/// already gone, access denied) is skipped, matching the unix kill(2)
/// semantics of ignoring the return value.
#[cfg(unix)]
fn kill_child(pid: u32) {
    // SAFETY: libc::kill is async-signal-safe + thread-safe; valid pids are
    // u32 from std/tokio's child.id() which fits in i32 for every realistic
    // process number on darwin/linux. We signal `-pid` — the process GROUP
    // led by `pid`. Every agent is spawned as a group leader
    // (`process_group(0)`), so this reaps its tool children (npm/pytest/
    // dev-servers) too; they'd otherwise reparent to init and survive.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

/// Windows twin of the SIGKILL path. OpenProcess/TerminateProcess are
/// plain Win32 calls, callable from any thread including a panic hook —
/// no async-signal-safety concept applies on Windows. A NULL handle
/// (process already exited, or access denied) is skipped. Note Windows
/// has no kill-children-on-parent-exit semantics, so this walk is just
/// as load-bearing here as the unix one (Ghost-Brian).
#[cfg(windows)]
fn kill_child(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_TERMINATE,
    };
    // SAFETY: handle is null-checked before use and closed exactly once;
    // TerminateProcess on a PROCESS_TERMINATE handle is documented
    // thread-safe.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

/// Liveness of an agent's retry supervisor, surfaced to the UI as a health dot
/// (B2). Plain enum — the serializable Tauri payload is built at the
/// `tauri_events` boundary via [`AgentHealth::as_str`], so the agents layer
/// stays free of `specta`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentHealth {
    /// Running normally.
    Running,
    /// Hit a transient API error; backing off + auto-resuming.
    Retrying,
    /// Mid-turn but silent too long — no tokens/tool events while busy with no
    /// tool in flight (e.g. an upstream "HTTP 200 empty/malformed" loop the
    /// supervisor can't classify as a retryable status). Recoverable: clears to
    /// Running on the next event (Batch 7 stall watchdog).
    Stalled,
    /// Supervisor gave up — permanent error / exhausted retries / exited.
    Dead,
}

impl AgentHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentHealth::Running => "running",
            AgentHealth::Retrying => "retrying",
            AgentHealth::Stalled => "stalled",
            AgentHealth::Dead => "dead",
        }
    }
}

/// High-level events a session-orchestrator consumes from an agent.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Plain prose chunk from the assistant.
    Text(String),
    /// Agent invoked a tool (typically `ask_user_choice` or `mark_awaiting_user`).
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Tool call's result echoed back into the conversation (after MCP fulfilled it).
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    /// Agent finished its turn (the `result` stream event).
    TurnComplete {
        stop_reason: Option<String>,
        subtype: Option<String>,
        /// True when the turn FAILED — `result.is_error`, a non-`success`
        /// subtype, or a populated `api_error_status` (e.g. an API 400). A
        /// failed turn's buffered text must NOT be peer-forwarded: forwarding
        /// it bounces the error to the peer, the peer replies, and that
        /// re-triggers the failing agent — an unbounded error-spam loop
        /// (Rain on the DeepSeek gateway, 2026-05-29).
        is_error: bool,
        /// Upstream API HTTP status when the turn failed on an API error
        /// (e.g. `529` Overloaded, `503`, `429`). `None` on success or on a
        /// non-API failure. The retry supervisor reads this to decide whether
        /// the failure is transient (auto-resume) or permanent (surface it).
        api_error_status: Option<u16>,
        /// What this turn's `result` event said about the context window —
        /// **including when it said nothing usable** (rc3 P7).
        ///
        /// It was `Option<ContextUsage>`, which threw the absences away: a
        /// gateway that never reports `contextWindow` and an agent that never
        /// finished a turn both arrived as `None`, so the question "does the
        /// window arrive at all on that provider" was unanswerable after the
        /// fact. [`ContextReport::usable`] is still the meter's reading, and it
        /// is derived rather than carried alongside, so the two cannot disagree.
        context: ContextReport,
    },
    /// System/init event — agent is ready and reporting its session metadata.
    /// (The wire `SystemEvent::Init` also carries `model`/`cwd`, but no
    /// consumer reads them, so they are not forwarded here.)
    Init { session_id: Option<String> },
    /// Process exited. Carries exit-status string for log/observability.
    Exited(String),
    /// Retry-supervisor liveness transition (B2), relayed by the participant's
    /// pump to the UI as a health dot. Not produced by the stream-json translator —
    /// emitted directly by `supervise` at running/retrying/dead transitions.
    Health(AgentHealth),
}

/// How full an agent's context window is, as of its last completed turn.
///
/// Raw components rather than a bare percentage: the UI wants "620K / 1M" in a
/// tooltip alongside "62%", and a pre-divided float throws that away. The
/// division is trivial; the operands are not recoverable.
///
/// Two properties worth knowing before building on this:
/// - **Stale mid-turn.** Only refreshed on a `result` event, so it describes
///   the last *completed* turn, not the in-flight one.
/// - **Non-monotonic.** claude-code auto-compacts, which makes `used_tokens`
///   drop. Do not treat a decrease as a bug or design a UI that assumes the
///   number only climbs.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextUsage {
    /// Model id the figures belong to (the `modelUsage` map key).
    pub model: String,
    /// `inputTokens + cacheReadInputTokens + cacheCreationInputTokens`.
    ///
    /// Cached tokens are included deliberately: caching changes what a token
    /// *costs*, not whether it *occupies the window*. Omitting them
    /// under-reports by orders of magnitude (2 vs 23,957 on a measured turn).
    pub used_tokens: u64,
    /// The model's total context window, straight from `contextWindow`.
    /// Guaranteed non-zero — a zero or absent value yields `None` upstream
    /// rather than a division by zero.
    pub context_window: u64,
}

impl ContextUsage {
    /// Occupancy in the range 0.0..=1.0 (values >1.0 are possible in principle
    /// and are the caller's problem to clamp for display).
    /// Test-only since round 7 (2026-08-17): no production caller — kept as a test seam, not shipped.
    #[cfg(test)]
    pub fn fraction(&self) -> f64 {
        self.used_tokens as f64 / self.context_window as f64
    }
}

/// Why a `result` event's context figures are, or are not, a meter reading.
///
/// Recorded per reading (rc3 P7) because the three failures are different
/// facts about the provider, and a bare "no reading" conflates them with an
/// agent that simply has not finished a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextVerdict {
    /// Both operands present and plausible — the meter moves.
    Usable,
    /// No `modelUsage` entry carried a non-zero `contextWindow`. **This is the
    /// one the 2026-08-12 `Prompt is too long` incident needs distinguished:**
    /// a participant whose provider never sends a window has no meter at all,
    /// so nothing could have warned anyone.
    NoWindow,
    /// A window arrived, but the point-in-time `usage` object did not, so there
    /// is no numerator.
    NoUsage,
    /// `used_tokens` overshoots the reported window by more than the plausible
    /// band — the provider's denominator is wrong, not the agent's occupancy,
    /// and dividing by it produces a confident 100% that means nothing.
    ImplausibleWindow,
}

impl ContextVerdict {
    /// The stored form (`context_readings.verdict`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::NoWindow => "no_window",
            Self::NoUsage => "no_usage",
            Self::ImplausibleWindow => "implausible_window",
        }
    }
}

/// Everything one `result` event reported about the context window, usable or
/// not — the raw operands plus the verdict on them (rc3 **P7**).
///
/// **The operands are kept even when they are unusable.** An implausible
/// window is still what the provider said, and a reading with a numerator but
/// no denominator still tells you the prompt size. Persisting the raw figures
/// is what makes "what was its context doing before it died" answerable after
/// the session is closed; a pre-divided percentage, or a `None`, is not
/// recoverable.
///
/// Nothing here substitutes a configured `models.context_window` for a missing
/// report. Whether it SHOULD is exactly the question this record exists to
/// settle with evidence, and filling it in as though it were measured would
/// destroy that evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextReport {
    /// The `modelUsage` key the figures were read from. `None` when no entry
    /// carried a usable window.
    pub model: Option<String>,
    /// `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`
    /// off the point-in-time `usage` object. `None` when that object is absent.
    pub used_tokens: Option<u64>,
    /// `contextWindow` exactly as reported, including an implausible one.
    /// `None` when absent or zero.
    pub reported_window: Option<u64>,
    pub verdict: ContextVerdict,
}

impl ContextReport {
    /// A report from a `result` event that carried no usable window.
    pub fn none(verdict: ContextVerdict) -> Self {
        Self {
            model: None,
            used_tokens: None,
            reported_window: None,
            verdict,
        }
    }

    /// The meter's reading, or `None` when this turn did not produce one.
    ///
    /// **Derived, never carried beside the operands.** The UI's figure and the
    /// recorded figures are then the same numbers by construction — the pairing
    /// that used to be two fields is the class of drift that shipped three
    /// wrong context numerators.
    pub fn usable(&self) -> Option<ContextUsage> {
        if self.verdict != ContextVerdict::Usable {
            return None;
        }
        Some(ContextUsage {
            model: self.model.clone()?,
            used_tokens: self.used_tokens?,
            context_window: self.reported_window?,
        })
    }
}

/// Classify an upstream API HTTP status as transient (worth an automatic
/// resume + retry) vs. permanent (surface to the user — retrying won't help).
///
/// Transient: overload / rate-limit / gateway / timeout statuses that usually
/// clear on their own within seconds — `408` request timeout, `425` too early,
/// `429` rate limit, `500` internal, `502` bad gateway, `503` unavailable,
/// `504` gateway timeout, `529` overloaded (the Anthropic "API Error:
/// Overloaded" that stranded a session 2026-06-01). Everything else — notably
/// `400`/`401`/`403`/`404`/`413`/`422` — is a permanent/semantic failure where
/// a blind retry just re-fails (e.g. the DeepSeek system-role 400).
pub fn is_transient_api_error(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504 | 529)
}

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub agent_name: String,
    pub config: AgentConfig,
    /// Path to a file holding the assembled system prompt, passed via
    /// `--append-system-prompt-file`. Routing the multi-KB prompt through a file
    /// (not an inline arg) keeps the command line under Windows' 32,767-char
    /// `CreateProcessW` limit. Cross-platform safe.
    pub system_prompt_path: PathBuf,
    pub mcp_config_path: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    /// Override the claude binary (for tests). Defaults to `"claude"`.
    pub claude_bin: Option<String>,
    /// Session this agent belongs to. Exported as `BOT_HQ_SESSION_ID` so
    /// the git pre-push hook can resolve session-scoped approvals.
    pub session_id: String,
    /// claude-code session UUID to resume (per-agent, captured from a prior
    /// spawn's `init` stream-json event and persisted on the bot-hq session
    /// row). When Some, the command line gains `--resume <uuid>` so the
    /// child picks up its previous conversation. When None, claude assigns
    /// a fresh UUID — we capture that one in the next `init` event.
    pub resume_session_id: Option<String>,
    /// Project name (CL / policy key) this session targets, if any. Threaded so
    /// HANDS's injected hooks resolve the project's policy at tool-call time
    /// (push/commit gates; the Tool Gate keyword list itself is global, not
    /// per-project). `None` for the projectless singleton.
    pub project: Option<String>,
    /// bot-hq data dir — the injected PreToolUse hook needs it to resolve the
    /// project's policy at tool-call time.
    pub data_dir: PathBuf,
    // `session_effort` / `session_ultracode` lived here and are GONE. They were
    // the participant's own D12 columns (`p.effort` / `p.ultracode`) carried down
    // under a name that said "session", and their only production readers were
    // the precedence overlay + exclusion rule that `build_command` used to run.
    // `reconcile_spawn_knobs` now runs that at the caller, which holds `Storage`
    // and can therefore RECORD the result; `overrides` below arrives already
    // reconciled.
    /// This participant's invite-time capability snapshot, read from
    /// `session_participants` at spawn.
    ///
    /// It is what decides the child's PERMISSION POSTURE in [`build_command`] —
    /// previously `cfg.agent_name == "rain"`. A role holding
    /// [`Capability::EditFiles`] gets bypass mode plus the Tool Gate hook; a
    /// role without it gets `dontAsk` + the read-only allow/deny lists. That is
    /// the same split the name check made (the seeded HANDS set holds
    /// `edit_files`, the seeded EYES set does not) sourced from the role
    /// instead of from who the agent is.
    ///
    /// [`ResolvedCapabilities::Unreadable`] takes the RESTRICTIVE branch — see
    /// that type for why every gated decision fails closed.
    pub capabilities: crate::agents::ResolvedCapabilities,
    /// This participant's Claude-config overrides, already resolved against its
    /// ROLE (`claude_config::resolve_agent_overrides`).
    ///
    /// Resolved by the caller rather than here because the key is a role slug
    /// and only the spawn path has the roster to reach one. It arrives resolved
    /// for the same reason the system prompt does: `build_command` and
    /// `spawn_agent_for` each used to load and resolve the store separately off
    /// `agent_name`, which is two reads that can disagree and two places for a
    /// key to go stale — and both went stale at once when slugs became
    /// role-derived.
    pub overrides: crate::claude_config::AgentOverride,
}

/// One participant's stdin, reachable only with a receipt for a row in THIS
/// participant's session.
///
/// The sender is private and the public way in is [`deliver`](Self::deliver),
/// which takes a [`PersistedMessage`]. That is the whole point of the type:
/// before B5 Task 2 the host paths pushed a `String` at an agent with no row
/// behind it, so what the agent read was invisible to the user, and nothing but
/// discipline stopped the next one. Now the argument has to be proof of a row.
///
/// A `ParticipantInput` built from a channel of your own is harmless — it
/// writes to that channel, not to an agent. The only senders that reach a live
/// subprocess come from [`spawn_agent`].
#[derive(Clone, Debug)]
pub struct ParticipantInput {
    /// The session whose rows this stdin accepts — see [`deliver`](Self::deliver).
    ///
    /// `Arc<str>` to match [`PersistedMessage::session_id`], so the compare is
    /// against the same representation and a clone of this input (the watchdog
    /// and the turn sequencer each hold one) is a refcount bump.
    session_id: Arc<str>,
    tx: mpsc::Sender<OutgoingUserMessage>,
}

impl ParticipantInput {
    pub(crate) fn new(
        session_id: impl Into<Arc<str>>,
        tx: mpsc::Sender<OutgoingUserMessage>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            tx,
        }
    }

    /// Write a persisted row to this participant's stdin. Returns whether it
    /// landed: `false` means either the receipt was for another session or the
    /// input pump has exited (the subprocess is gone). Neither is something a
    /// caller here can remediate — but several want to log or skip a busy-flag
    /// flip, so it is reported rather than swallowed.
    ///
    /// The wire is [`PersistedMessage::wire`] and nothing else. A caller with
    /// something to add to the text has to add it to the ROW, before the insert.
    ///
    /// ## The session-scope check lives here, and only here
    ///
    /// A receipt carries the session its row was written into, and delivering it
    /// into a DIFFERENT session's agent wires one session's text into another's
    /// process while the row sits in the wrong channel. The check used to sit on
    /// `SessionHandle::send_to_all`, which was the only caller that knew both
    /// ids — and that left two receipt-carrying routes past it:
    /// `SessionAgent::deliver`, and the three-hop
    /// `agent.handle.input().deliver(&receipt)` reachable through three `pub`
    /// items. Receipt-gated is not scope-gated, and those two were receipt-gated
    /// only.
    ///
    /// Both of those routes END here, so this is the narrow point: give the
    /// stdin its own session id and every receipt-carrying write is compared,
    /// with one copy of the comparison. (`send_to_all` and `SessionAgent::deliver`
    /// were deleted in round 7 as callerless; every remaining route is this one.)
    ///
    /// Be exact about the size of the claim. Within this type there are four
    /// writes to `tx` — this one, [`deliver_batch`](Self::deliver_batch) and
    /// the private [`relay`](Self::relay) (`send_unrouted`, the fourth, went
    /// with the router) — so what holds is: **every write to a participant's
    /// stdin that carries a receipt is scope-checked.** `deliver_batch` is the
    /// other receipt-carrying one and runs this same comparison per row. `relay`
    /// carries no receipt and puts no row on record: it has one call site that
    /// authors its own text — see `relay`'s doc. **One unrecorded
    /// stdin writes, not one.** Neither is touched here.
    ///
    /// And this is a check on the receipt, not on the channel. Two capabilities
    /// have to be told apart, because they are not equally reachable:
    ///
    /// - **Minting an input under a session id of your choosing** is reachable
    ///   from OUTSIDE this crate, not just in-crate as this paragraph used to
    ///   say. `ParticipantInput::new` is `pub(crate)`, but
    ///   [`AgentHandle::from_parts`] is `pub`, re-exported from `crate::agents`,
    ///   takes the session id as a plain parameter, and
    ///   [`AgentHandle::input`] hands the result back — so any consumer of the
    ///   `bot_hq` lib target (`tests/` included) can mint one. Compiled and run
    ///   as an integration test before this was written; it needs no
    ///   `pub(crate)` item.
    /// - **Pointing that input at a LIVE agent's stdin** additionally needs that
    ///   agent's raw `Sender<OutgoingUserMessage>`, and no public API returns
    ///   one: the field on this type is private, so is `AgentHandle::input_tx`,
    ///   and no function anywhere in the crate has that sender as a return type.
    ///   The senders that reach a subprocess are created inside [`spawn_agent`]
    ///   and `spawn_supervised_agent`, so misfiling one stays a build-time
    ///   obligation on those two — the same obligation
    ///   [`crate::core::sequencer::SequencerDeps::inputs`] carries for its map
    ///   keys.
    ///
    /// So an outside forge writes into its own channel (harmless, as the type
    /// doc says); an in-crate one can write into another agent's. What this
    /// check rules out, in both cases, is a receipt from session B reaching an
    /// input constructed for session A.
    ///
    /// Drops rather than panics, and warns. A mismatch is a routing bug in the
    /// caller; the containment that matters is that the wrong agent does not
    /// read it, and killing the process on top of that helps nobody. It cannot
    /// pass silently: the row is already written, so the text is still in its
    /// own channel and the warning names both sides.
    pub async fn deliver(&self, msg: &PersistedMessage) -> bool {
        if msg.session_id() != &*self.session_id {
            warn!(
                session = %self.session_id,
                receipt_session = %msg.session_id(),
                message_id = msg.message_id(),
                "refusing to deliver a receipt from another session"
            );
            return false;
        }
        self.tx
            .send(OutgoingUserMessage::text(msg.wire()))
            .await
            .is_ok()
    }

    /// Write a whole BATCH of persisted rows to this participant's stdin as ONE
    /// message. Returns whether it landed, on the same terms as
    /// [`deliver`](Self::deliver).
    ///
    /// ## Why this exists, and why it is not a loop over `deliver`
    ///
    /// One [`OutgoingUserMessage`] is one stream-json line
    /// ([`pump_inputs`](crate::agents::input::pump_inputs)), and claude-code
    /// opens a TURN on the first line it reads. So delivering a nine-row backlog
    /// row-at-a-time did not hand the participant nine rows to read — it handed
    /// it one row and then interrupted it eight times, mid-turn. Measured across
    /// four sessions on 2026-08-13: the user's own message arrived somewhere
    /// other than the front of the batch **37 times out of 44**, including row 9
    /// of 9. One session's reviewer spent its turn on a peer's test run while the
    /// user's actual instruction sat unread at the end of the batch.
    ///
    /// Coalescing is therefore not a performance tweak. It is what makes the
    /// order the ring already establishes — ascending id, so the newest row last
    /// — the order the participant actually reads in.
    ///
    /// ## All or nothing, deliberately
    ///
    /// A mismatched receipt refuses the WHOLE batch rather than skipping that
    /// row. A mismatch is a routing bug in the caller (see
    /// [`deliver`](Self::deliver)); delivering the remainder would put a
    /// partially-correct transcript in front of the agent, which is harder to
    /// reason about than none of it. Same for the send: it either lands whole or
    /// not at all, and the caller's cursor moves accordingly.
    ///
    /// An empty batch sends nothing and reports success — there is no row to
    /// fail to deliver. The turn path never calls it that way (it returns on an
    /// empty page first), so this is a total function rather than a live case.
    ///
    /// ## The receipt gate
    ///
    /// This is the FOURTH write to `tx` in this type, and the second one that
    /// carries receipts — see the size-of-the-claim paragraph on
    /// [`deliver`](Self::deliver). It is scope-checked per receipt, so the claim
    /// there is unchanged in substance: every write to a participant's stdin
    /// that carries a receipt is compared against the channel it is for.
    pub async fn deliver_batch(&self, msgs: &[PersistedMessage]) -> bool {
        // Checked BEFORE anything is sent, across the whole batch, so a bad row
        // in the middle cannot leave a prefix on the wire.
        for msg in msgs {
            if msg.session_id() != &*self.session_id {
                warn!(
                    session = %self.session_id,
                    receipt_session = %msg.session_id(),
                    message_id = msg.message_id(),
                    batch = msgs.len(),
                    "refusing to deliver a batch containing a receipt from another session"
                );
                return false;
            }
        }
        if msgs.is_empty() {
            return true;
        }
        self.tx
            .send(OutgoingUserMessage::text(PersistedMessage::wire_batch(msgs)))
            .await
            .is_ok()
    }

    /// True once the receiving half is gone — a permanent API error or an
    /// exhausted retry budget drops the supervisor's receiver.
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Hand a message to the CURRENT incarnation. Private to this module
    /// because it is supervisor plumbing, not a send.
    ///
    /// **Only one of its two call sites is re-pointing, though — this doc said
    /// both were.** They differ in exactly the way that matters for whether the
    /// text is on record:
    ///
    /// - the bridge in [`supervise`]'s select, which forwards whatever came off
    ///   `out_input_rx`. That message WAS authored one channel upstream, and if
    ///   it arrived via [`deliver`](Self::deliver) it has a row behind it. Pure
    ///   re-pointing;
    /// - the `pending_nudge` write at the top of `supervise`'s outer loop. That
    ///   string is authored INSIDE `supervise`, where the transient-API retry
    ///   sets it, and nothing in this file writes it to storage — `supervise`
    ///   holds no `Storage` at all. So the resumed child reads a `[bot-hq]`
    ///   instruction that appears in no channel and no transcript.
    ///
    /// That makes the nudge the one ungated, unrecorded write to a
    /// participant's stdin (`send_unrouted`, once the other, went with the
    /// router) — see the size-of-the-claim paragraph on [`deliver`](Self::deliver).
    /// Recording it means giving `supervise` a way to write a row, which it has
    /// no dependency on today; it is noted here rather than fixed.
    async fn relay(
        &self,
        msg: OutgoingUserMessage,
    ) -> Result<(), mpsc::error::SendError<OutgoingUserMessage>> {
        self.tx.send(msg).await
    }
}

/// Driver handle for one running agent subprocess.
pub struct AgentHandle {
    pub name: String,
    pub event_rx: mpsc::Receiver<AgentEvent>,
    /// Private so the only way to this agent's stdin is [`AgentHandle::input`],
    /// which hands back a [`ParticipantInput`] — receipt-gated by construction.
    input_tx: ParticipantInput,
    /// Out-of-band stdin channel for `control_request` interrupts (the cancel
    /// path). Separate from `input_tx` so an interrupt preempts queued user
    /// messages, exactly as the binary's control protocol expects.
    pub control_tx: mpsc::Sender<ControlRequest>,
    kill_tx: Option<oneshot::Sender<()>>,
    /// The turn epoch that was live the last time bot-hq ITSELF interrupted this
    /// agent (`NO_INTERRUPT_EPOCH` = never). Shared with the agent's pump, which
    /// reads it when a turn completes `is_error:true`: claude-code reports an
    /// aborted turn as an error (`result` with `terminal_reason:
    /// "aborted_streaming"`, `is_error:true` — `signaling/protocol.rs`), so
    /// without this a user Pause or the agent's own `halt` was indistinguishable
    /// from an API failure, and two of them in a row fired the pump's
    /// "turns are failing back-to-back … close the session" halt over the reason
    /// the agent had just declared (measured 3× on 2026-08-17, every one false).
    ///
    /// Keyed on the EPOCH rather than a bare flag on purpose: an interrupt to an
    /// idle agent is a documented harmless no-op (the typed-Send preempt fires
    /// it at every agent), and a flag set with no turn in flight would survive to
    /// swallow the NEXT turn's genuine error. The epoch stamped while idle is the
    /// one this agent last completed with, which no future completion can carry.
    interrupted_epoch: Arc<std::sync::atomic::AtomicU64>,
}

/// [`AgentHandle::interrupted_epoch`]'s "never interrupted" value.
pub const NO_INTERRUPT_EPOCH: u64 = u64::MAX;

impl AgentHandle {
    /// Assemble a handle from channels a caller owns, rather than from a spawn.
    ///
    /// **Its production caller was the native loop, which rc3 D9 deleted.** What
    /// still uses it is `core::session`'s test scaffolding, and it stays `pub`
    /// for the same reason it was written: an alternative agent implementation
    /// needs a way in, and the handle is a pure channel struct so anything that
    /// produces one plugs into `supervise` and the participant's pump unchanged.
    /// `kill_tx` stays private, hence this constructor.
    ///
    /// `session_id` is the session this agent belongs to, and it is what
    /// [`ParticipantInput::deliver`] compares a receipt against. It is a
    /// parameter rather than something read off the channels because the
    /// channels carry no identity — the caller is the last place that knows.
    /// Test-only since round 7 (2026-08-17): no production caller — kept as a test seam, not shipped.
    #[cfg(test)]
    pub fn from_parts(
        name: String,
        session_id: impl Into<Arc<str>>,
        event_rx: mpsc::Receiver<AgentEvent>,
        input_tx: mpsc::Sender<OutgoingUserMessage>,
        control_tx: mpsc::Sender<ControlRequest>,
        kill_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            name,
            event_rx,
            input_tx: ParticipantInput::new(session_id, input_tx),
            control_tx,
            kill_tx: Some(kill_tx),
            interrupted_epoch: Arc::new(std::sync::atomic::AtomicU64::new(NO_INTERRUPT_EPOCH)),
        }
    }

    /// This agent's stdin. Clone it to hand a long-lived task (the idle
    /// watchdog, the turn sequencer) its own way in — every clone carries
    /// the session id with it, so a clone is still scope-checked as well as
    /// receipt-gated. Those are two different guarantees: see
    /// [`ParticipantInput::deliver`].
    pub fn input(&self) -> &ParticipantInput {
        &self.input_tx
    }

    /// Best-effort kill. Idempotent (subsequent calls no-op).
    pub fn kill(&mut self) {
        if let Some(tx) = self.kill_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Best-effort interrupt: queue a `control_request` to abort the in-flight
    /// turn WITHOUT killing the process (warm cache, no `--resume`). Returns
    /// whether it was queued; a full or closed control channel returns `false`
    /// and the caller escalates to [`kill`](Self::kill). `request_id` correlates
    /// the `control_response` ACK.
    ///
    /// Private on purpose: the only way to interrupt from outside is
    /// [`interrupt_at`](Self::interrupt_at), which records the epoch — so a
    /// caller cannot compile the version that forgets to. Core reaches it via
    /// `core::session::SessionAgent::interrupt`.
    fn interrupt(&self, request_id: impl Into<String>) -> bool {
        self.control_tx
            .try_send(ControlRequest::interrupt(request_id))
            .is_ok()
    }

    /// [`interrupt`](Self::interrupt), recording the turn epoch that is live for
    /// this agent as it happens, so the pump can tell the aborted turn's
    /// `is_error` completion from a real failure. Every host-side interrupt of a
    /// participant goes through `core::session::SessionAgent::interrupt`, which
    /// supplies the epoch off the participant's own cell.
    pub fn interrupt_at(&self, request_id: impl Into<String>, epoch: u64) -> bool {
        self.interrupted_epoch
            .store(epoch, std::sync::atomic::Ordering::Release);
        self.interrupt(request_id)
    }

    /// The cell [`interrupt_at`](Self::interrupt_at) writes — handed to this
    /// agent's pump at spawn.
    pub fn interrupted_epoch(&self) -> Arc<std::sync::atomic::AtomicU64> {
        Arc::clone(&self.interrupted_epoch)
    }
}

impl Drop for AgentHandle {
    fn drop(&mut self) {
        self.kill();
    }
}

pub async fn spawn_agent(cfg: SpawnConfig) -> Result<AgentHandle> {
    ensure_claude_runnable(cfg.claude_bin.as_deref().unwrap_or("claude"))?;

    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(256);
    let (input_tx, input_rx) = mpsc::channel::<OutgoingUserMessage>(64);
    let (control_tx, control_rx) = mpsc::channel::<ControlRequest>(8);
    let (kill_tx, kill_rx) = oneshot::channel::<()>();

    let mut cmd = build_command(&cfg);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Put each agent (and every tool child it spawns via Bash — npm/pytest/
    // dev-servers) in its OWN process group, so a cancel/close/crash-reap can
    // kill the whole group instead of just the parent. Without this, a
    // long-running tool child reparents to init on kill and keeps running
    // (CPU, file locks). `process_group(0)` makes the child a group LEADER
    // with PGID == its PID, so the registered `child.id()` doubles as the
    // group id; the kill paths signal `-pid`. Unix-only — Windows job-object
    // reaping is a tracked follow-up (the single-PID `kill_child` stands in).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.as_std_mut().process_group(0);
    }

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "spawning claude-code for agent {}; bin={}",
            cfg.agent_name,
            cfg.claude_bin.as_deref().unwrap_or("claude")
        )
    })?;

    // Register PID for crash-path reaping. None on platforms that don't
    // expose pids (we only ship darwin/linux) or after the child has
    // already been reaped — the registration is best-effort either way.
    let child_pid = child.id();
    if let Some(pid) = child_pid {
        CHILD_PIDS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(pid);
    }

    let stdin = child.stdin.take().context("subprocess missing stdin")?;
    let stdout = child.stdout.take().context("subprocess missing stdout")?;
    let stderr = child.stderr.take().context("subprocess missing stderr")?;

    tokio::spawn(events::pump_events(stdout, event_tx.clone()));
    tokio::spawn(events::pump_stderr(stderr, cfg.agent_name.clone()));
    tokio::spawn(input::pump_inputs(
        stdin,
        input_rx,
        control_rx,
        cfg.agent_name.clone(),
    ));

    let event_tx_for_lifecycle = event_tx.clone();
    let agent_name = cfg.agent_name.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = kill_rx => {
                info!(agent = %agent_name, "kill signalled");
                // Reap the whole process group, not just the leader: tool
                // children the agent spawned via Bash share its PGID and would
                // otherwise reparent to init and keep running. SIGKILL the
                // group (`-pid`, valid because process_group(0) made the child
                // a group leader), then let tokio reap the leader zombie.
                #[cfg(unix)]
                if let Some(pid) = child_pid {
                    // SAFETY: kill(2) is thread-safe; -pid targets the group.
                    unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
                }
                let _ = child.kill().await;
                if let Some(pid) = child_pid {
                    CHILD_PIDS.lock().unwrap_or_else(|p| p.into_inner()).remove(&pid);
                }
                let _ = event_tx_for_lifecycle
                    .send(AgentEvent::Exited("killed by supervisor".into()))
                    .await;
            }
            res = child.wait() => {
                if let Some(pid) = child_pid {
                    CHILD_PIDS.lock().unwrap_or_else(|p| p.into_inner()).remove(&pid);
                }
                let msg = match res {
                    Ok(status) => format!("status={status:?}"),
                    Err(e) => format!("wait error: {e}"),
                };
                warn!(agent = %agent_name, msg = %msg, "agent exited");
                let _ = event_tx_for_lifecycle.send(AgentEvent::Exited(msg)).await;
            }
        }
    });

    info!(agent = %cfg.agent_name, "agent spawned");

    Ok(AgentHandle {
        name: cfg.agent_name,
        event_rx,
        input_tx: ParticipantInput::new(cfg.session_id, input_tx),
        control_tx,
        kill_tx: Some(kill_tx),
        interrupted_epoch: Arc::new(std::sync::atomic::AtomicU64::new(NO_INTERRUPT_EPOCH)),
    })
}

/// Retry policy for the agent supervisor: how many consecutive transient API
/// failures to absorb (auto-resume) before surfacing the error and stopping,
/// plus the backoff schedule between attempts. A successful turn resets the
/// budget.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        // ~2s, 4s, 8s, 16s, 30s — ≈60s of patience over 5 attempts, which
        // comfortably outlasts a typical Anthropic "Overloaded" blip, then
        // gives up with a clear message so a real outage doesn't loop forever.
        Self {
            max_retries: 5,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Backoff before the Nth retry (1-based): `base * 2^(n-1)`, capped at
    /// `max_delay`.
    pub fn backoff(&self, attempt: u32) -> Duration {
        let shift = attempt.saturating_sub(1).min(16);
        self.base_delay
            .saturating_mul(1u32 << shift)
            .min(self.max_delay)
    }
}

/// Spawn an agent under a retry supervisor. The returned `AgentHandle` exposes
/// STABLE event/input channels: when a child dies on a *transient* upstream API
/// error (e.g. `529` Overloaded), the supervisor auto-resumes it
/// (`--resume <uuid>`) with capped backoff and a continue-nudge — transparently
/// to the caller and the peer pump, with no channel rewiring. A permanent error
/// (e.g. `400`), a clean exit, or exhausting `max_retries` ends the supervisor
/// and closes the channels (the peer pump then unwinds on its own).
///
/// The first incarnation is spawned synchronously so spawn failures surface to
/// the caller via `?`, matching `spawn_agent`'s contract.
pub async fn spawn_supervised_agent(cfg: SpawnConfig, policy: RetryPolicy) -> Result<AgentHandle> {
    let (out_event_tx, out_event_rx) = mpsc::channel::<AgentEvent>(256);
    let (out_input_tx, out_input_rx) = mpsc::channel::<OutgoingUserMessage>(64);
    let (out_control_tx, out_control_rx) = mpsc::channel::<ControlRequest>(8);
    let (kill_tx, kill_rx) = oneshot::channel::<()>();

    let name = cfg.agent_name.clone();
    // Cloned before `cfg` moves into `supervise`. The OUTER handle's stdin is
    // what every caller holds — each respawned incarnation is bridged onto it —
    // so this is the input whose session id has to be right.
    let session_id: Arc<str> = Arc::from(cfg.session_id.as_str());
    let first = spawn_agent(cfg.clone()).await?;

    tokio::spawn(supervise(
        cfg,
        policy,
        first,
        out_event_tx,
        out_input_rx,
        out_control_rx,
        kill_rx,
        spawn_agent,
    ));

    Ok(AgentHandle {
        name,
        event_rx: out_event_rx,
        input_tx: ParticipantInput::new(session_id, out_input_tx),
        control_tx: out_control_tx,
        kill_tx: Some(kill_tx),
        interrupted_epoch: Arc::new(std::sync::atomic::AtomicU64::new(NO_INTERRUPT_EPOCH)),
    })
}

/// Supervisor task body. Bridges one child incarnation at a time onto the
/// stable outer channels, retrying transient API failures. Generic over the
/// respawn fn so the retry logic is testable with fake incarnations.
#[allow(clippy::too_many_arguments)]
async fn supervise<S, Fut>(
    mut cfg: SpawnConfig,
    policy: RetryPolicy,
    first: AgentHandle,
    out_event_tx: mpsc::Sender<AgentEvent>,
    mut out_input_rx: mpsc::Receiver<OutgoingUserMessage>,
    mut out_control_rx: mpsc::Receiver<ControlRequest>,
    mut kill_rx: oneshot::Receiver<()>,
    mut spawn_next: S,
) where
    S: FnMut(SpawnConfig) -> Fut + Send,
    Fut: std::future::Future<Output = Result<AgentHandle>> + Send,
{
    let agent = cfg.agent_name.clone();
    let mut incarnation = first;
    let mut consecutive_transient: u32 = 0;
    let mut pending_nudge: Option<String> = None;
    // Disabled once `out_control_rx` closes (handle dropped) so the closed branch
    // can't busy-loop on `None`. The user-input `None` arm tears the loop down first.
    let mut control_open = true;

    loop {
        // A freshly respawned `--resume` child idles until it receives input —
        // nudge it to pick up the interrupted turn.
        if let Some(nudge) = pending_nudge.take() {
            let _ = incarnation
                .input_tx
                .relay(OutgoingUserMessage::text(nudge))
                .await;
        }

        let mut last_error_status: Option<u16> = None;

        // Bridge this incarnation until its event channel CLOSES. Closure (not
        // the `Exited` event) is the end-of-incarnation signal: the channel
        // closes only once both the stdout pump and the lifecycle task have
        // dropped their senders, so every event — including the final
        // `TurnComplete` carrying the error status — has already been received.
        // This makes classification race-free regardless of Exited/Result order.
        loop {
            tokio::select! {
                biased;
                _ = &mut kill_rx => {
                    incarnation.kill();
                    return;
                }
                ctl = out_control_rx.recv(), if control_open => {
                    match ctl {
                        // Relay the interrupt to the CURRENT incarnation's stdin
                        // (follows respawns automatically). Best-effort: a full or
                        // closed control channel just drops it and the cancel path
                        // escalates to SIGKILL. Placed above user input so an
                        // interrupt preempts any queued messages.
                        Some(ctl) => {
                            let _ = incarnation.control_tx.try_send(ctl);
                        }
                        None => control_open = false,
                    }
                }
                msg = out_input_rx.recv() => {
                    match msg {
                        Some(msg) => {
                            if let Err(e) = incarnation.input_tx.relay(msg).await {
                                // The incarnation's stdin pump has died (its
                                // receiver dropped), so the child is now deaf to
                                // ALL input — yet its event channel can stay open
                                // while stdout lingers, so the `None => break`
                                // path below would NOT catch it. Bridging on would
                                // silently drop every user/peer message (the #4
                                // user→HANDS desync, invisible + unrecoverable).
                                // Tear down instead: dropping `out_input_rx` closes
                                // the public sender → `is_stale()` → the next
                                // `ensure_session_started` evicts + respawns.
                                warn!(agent = %agent, error = %e, "incarnation input pump died; terminating supervisor so the session goes stale and respawns");
                                incarnation.kill();
                                return;
                            }
                        }
                        None => {
                            // Caller dropped the handle → tear down.
                            incarnation.kill();
                            return;
                        }
                    }
                }
                ev = incarnation.event_rx.recv() => {
                    match ev {
                        // Suppress Exited: forwarding it would make the peer
                        // pump terminate before a possible retry. Channel close
                        // below is the real signal.
                        Some(AgentEvent::Exited(reason)) => {
                            debug!(agent = %agent, %reason, "incarnation exited; awaiting channel close");
                        }
                        Some(ev) => {
                            match &ev {
                                AgentEvent::Init { session_id: Some(id) } => {
                                    cfg.resume_session_id = Some(id.clone());
                                }
                                AgentEvent::TurnComplete { is_error, api_error_status, .. } => {
                                    if *is_error {
                                        last_error_status = *api_error_status;
                                    } else {
                                        // A healthy turn clears the retry budget;
                                        // if we'd been retrying, signal recovery.
                                        if consecutive_transient > 0 {
                                            let _ = out_event_tx
                                                .send(AgentEvent::Health(AgentHealth::Running))
                                                .await;
                                        }
                                        consecutive_transient = 0;
                                        last_error_status = None;
                                    }
                                }
                                _ => {}
                            }
                            let _ = out_event_tx.send(ev).await;
                        }
                        None => break, // incarnation fully ended
                    }
                }
            }
        }

        let transient = last_error_status
            .map(is_transient_api_error)
            .unwrap_or(false);

        if transient && consecutive_transient < policy.max_retries {
            consecutive_transient += 1;
            let status = last_error_status.unwrap_or(0);
            let delay = policy.backoff(consecutive_transient);
            warn!(
                agent = %agent, status, attempt = consecutive_transient,
                delay_ms = delay.as_millis() as u64,
                "agent hit transient API error; auto-resuming after backoff"
            );
            let _ = out_event_tx
                .send(AgentEvent::Health(AgentHealth::Retrying))
                .await;
            tokio::select! {
                _ = &mut kill_rx => return,
                _ = tokio::time::sleep(delay) => {}
            }
            pending_nudge = Some(format!(
                "[bot-hq] Your previous turn was interrupted by a transient upstream API error \
                 (HTTP {status}) and has been automatically resumed. Continue exactly where you \
                 left off — re-issue the action you were about to take. Do NOT repeat work you \
                 already completed or committed."
            ));
            match spawn_next(cfg.clone()).await {
                Ok(next) => {
                    incarnation = next;
                    continue;
                }
                Err(e) => {
                    warn!(agent = %agent, error = %e, "respawn failed after transient error");
                    let _ = out_event_tx
                        .send(AgentEvent::Text(format!(
                            "⚠️ Could not resume after a transient API error (HTTP {status}): {e}. \
                             Reopen the session to retry."
                        )))
                        .await;
                    return;
                }
            }
        }

        if transient {
            // Budget exhausted — a real outage, not a blip. Surface it.
            let status = last_error_status.unwrap_or(0);
            warn!(agent = %agent, status, retries = consecutive_transient, "transient API errors exhausted retry budget");
            let _ = out_event_tx
                .send(AgentEvent::Text(format!(
                    "⚠️ Stopped after {consecutive_transient} consecutive transient API errors \
                     (last: HTTP {status}). The upstream API stayed unavailable — reopen the \
                     session to resume from here."
                )))
                .await;
        }
        // Clean exit / permanent error / retries exhausted: returning drops
        // `out_event_tx`, so the peer pump sees its channel close and unwinds.
        return;
    }
}

/// A participant without `Capability::EditFiles` runs read-only under
/// `--permission-mode dontAsk`. Tools with a
/// read form worth keeping (`git branch`, `gh`) are denied BY WRITE VERB rather
/// than by blanket noun, so the read forms fall through to the allowed `Bash`
/// (deny wins over allow, so a blanket `Bash(gh issue:*)` / `Bash(git branch:*)`
/// would also kill the reads). These const lists are the single source of truth:
/// both `build_read_only_disallowed_tools` AND the `eyes_denies_*` tests iterate
/// them, so enforcement and its test can't drift. New write verbs go here.
///
/// Mutating `git branch` forms. Read forms (bare / `--show-current` / `-a` / `-r`
/// / `--list` / `--contains`) fall through. 2026-06-17: replaced the blanket
/// `Bash(git branch:*)` deny that was false-blocking EYES's read-only listing.
const GIT_BRANCH_WRITE_VERBS: &[&str] = &[
    "-d",
    "-D",
    "--delete",
    "-m",
    "-M",
    "--move",
    "-c",
    "-C",
    "--copy",
    "-f",
    "--force",
    "-u",
    "--set-upstream-to",
    "--unset-upstream",
    "--track",
    "--no-track",
    "--edit-description",
];
/// Mutating `gh pr` verbs (read forms — view/diff/list/status/checks — fall through).
const GH_PR_WRITE_VERBS: &[&str] = &[
    "create", "edit", "close", "reopen", "merge", "ready", "review", "comment", "lock", "unlock",
    "delete", "checkout",
];
/// Mutating `gh issue` verbs (read forms — view/list — fall through).
const GH_ISSUE_WRITE_VERBS: &[&str] = &[
    "create", "edit", "close", "reopen", "comment", "delete", "transfer", "pin", "unpin", "lock",
    "unlock", "develop",
];
/// Mutating `gh release` verbs (read forms — view/list — fall through).
const GH_RELEASE_WRITE_VERBS: &[&str] = &["create", "edit", "delete", "upload", "download"];
/// Mutating `gh repo` verbs (read forms — view — fall through).
const GH_REPO_WRITE_VERBS: &[&str] = &[
    "create", "edit", "delete", "fork", "sync", "rename", "archive", "clone",
];

/// Build space-joined `Bash(<tool> <verb>:*)` deny patterns for a
/// deny-by-write-verb tool. Read forms (no listed verb) fall through to allowed `Bash`.
fn deny_write_verbs(tool: &str, verbs: &[&str]) -> String {
    verbs
        .iter()
        .map(|v| format!("Bash({tool} {v}:*)"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `--disallowedTools` value for a participant that may not edit files:
/// static denies (Edit/Write/Task + the
/// full-noun git mutations that have no read form worth preserving) plus the
/// deny-by-write-verb collections for `git branch` and `gh`. `gh api` is fully
/// denied — the POST/PATCH/DELETE escape hatch. Covered by the `eyes_denies_*`
/// tests, which assert against the same `*_WRITE_VERBS` consts.
///
/// Selected by CAPABILITY, never by slug — the caller is
/// `if !cfg.capabilities.grants(Capability::EditFiles)`. The name said `rain`
/// until round-4 F6, which was cosmetic but read as if a deleted slug still
/// gated enforcement.
fn build_read_only_disallowed_tools() -> String {
    let mut parts: Vec<String> = [
        "Edit",
        "Write",
        "NotebookEdit",
        "Task",
        "Bash(git commit:*)",
        "Bash(git push:*)",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    parts.push(deny_write_verbs("git branch", GIT_BRANCH_WRITE_VERBS));

    // Full-noun git mutations — no read form worth preserving, so deny the noun.
    parts.extend(
        [
            "Bash(git checkout:*)",
            "Bash(git switch:*)",
            "Bash(git reset:*)",
            "Bash(git merge:*)",
            "Bash(git rebase:*)",
            "Bash(git add:*)",
            "Bash(git stash:*)",
            "Bash(git restore:*)",
            "Bash(git rm:*)",
            "Bash(git tag:*)",
            "Bash(git cherry-pick:*)",
            "Bash(git apply:*)",
        ]
        .iter()
        .map(|s| s.to_string()),
    );

    parts.push(deny_write_verbs("gh pr", GH_PR_WRITE_VERBS));
    parts.push(deny_write_verbs("gh issue", GH_ISSUE_WRITE_VERBS));
    parts.push(deny_write_verbs("gh release", GH_RELEASE_WRITE_VERBS));
    parts.push(deny_write_verbs("gh repo", GH_REPO_WRITE_VERBS));
    parts.push("Bash(gh api:*)".to_string());

    parts.join(" ")
}

/// Pre-flight: ensure the `claude` binary is launchable on this platform, with
/// an actionable error when it isn't.
///
/// On Windows `std::process::Command` finds `claude.exe` but NOT npm's
/// `claude.cmd` (it appends `.exe` and ignores `PATHEXT`), and routing our
/// invocation through `cmd.exe` is unreliable. So we require a native `.exe` on
/// PATH and turn the otherwise-cryptic OS "program not found" into guidance
/// pointing at the native installer. No-op on unix, where a bare `claude` on
/// PATH is found and launched directly.
#[cfg(windows)]
pub(crate) fn ensure_claude_runnable(bin: &str) -> Result<()> {
    // An explicit path (custom bin / test override) is trusted as-is.
    if bin.contains('\\') || bin.contains('/') {
        return Ok(());
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
    // A native `.exe` on PATH is exactly what `Command::new(bin)` will launch.
    if dirs.iter().any(|d| d.join(format!("{bin}.exe")).is_file()) {
        return Ok(());
    }
    // npm installs a cmd-shim — detect it for a precise message.
    let has_shim = dirs
        .iter()
        .any(|d| d.join(format!("{bin}.cmd")).is_file() || d.join(format!("{bin}.bat")).is_file());
    if has_shim {
        anyhow::bail!(
            "claude-code is installed as an npm shim ('{bin}.cmd'), which bot-hq can't \
             launch reliably on Windows. Install the native build: run \
             `irm https://claude.ai/install.ps1 | iex` in PowerShell, then restart bot-hq."
        );
    }
    anyhow::bail!(
        "claude-code ('{bin}') was not found on PATH. Install it: run \
         `irm https://claude.ai/install.ps1 | iex` in PowerShell, then restart bot-hq."
    );
}

/// No-op on non-Windows: a bare `claude` on PATH is found and launched directly.
#[cfg(not(windows))]
pub(crate) fn ensure_claude_runnable(_bin: &str) -> Result<()> {
    Ok(())
}

/// Overlay the participant's own effort/ultracode picks onto the persistent
/// per-role overrides, reconcile the pair, then floor it — **the whole
/// precedence chain, in one place.**
///
/// The chain is two steps and a floor (no-inherit, 2026-08-25): the per-run
/// pick, else the role's default (`per_role[slug]` — `resolve_agent_overrides`
/// no longer lets effort/ultracode fall through to `_all`), else
/// [`crate::claude_config::DEFAULT_EFFORT`]. Every spawn therefore emits a
/// concrete `CLAUDE_CODE_EFFORT_LEVEL`, so the user's own settings.json knob
/// never reaches an agent. `pick_effort` / `pick_ultracode` are the
/// participant's D12 columns, and they WIN — a choice made for this run beats
/// a standing default.
///
/// ## Why the reconciliation exists
///
/// claude-code treats max-effort and ultracode as mutually exclusive (ultracode
/// implies xhigh + workflow orchestration; `max` is a distinct effort posture,
/// and emitting BOTH — env `CLAUDE_CODE_EFFORT_LEVEL=max` plus `"ultracode":
/// true` in `--settings` — is undefined). Each surface's UI already stops a user
/// picking both, but a **cross-layer** overlay can still resolve to both:
/// persistent `max` under a per-run ultracode, or the reverse. Whichever knob was
/// EXPLICITLY picked for this run wins; with neither picked, ultracode wins.
///
/// ## Why it is a free function rather than inline in `build_command`
///
/// It ran there until round 4, inside a synchronous fn with no `Storage` — so
/// the resolved value existed only as a local, was emitted to the child process,
/// and was then unrecoverable. The UI could not say what a participant was
/// actually spawned with, and re-deriving it later answers a different question
/// ("what it WOULD be spawned with now"), which diverges the moment Claude Config
/// is edited mid-session. Hoisting it to the caller that holds `Storage` is what
/// lets the answer be RECORDED, the same call `slot0_model_at_spawn` already
/// made for the sibling fact.
///
/// Returns the effective pair. The caller persists it and hands `overrides` to
/// [`SpawnConfig`] already reconciled.
pub fn reconcile_spawn_knobs(
    persistent: &mut crate::claude_config::AgentOverride,
    pick_effort: Option<&str>,
    pick_ultracode: Option<bool>,
) {
    if pick_effort.is_some() {
        persistent.effort = pick_effort.map(str::to_string);
    }
    if pick_ultracode.is_some() {
        persistent.ultracode = pick_ultracode;
    }
    if persistent.ultracode == Some(true) && persistent.effort.as_deref() == Some("max") {
        // `pick_effort.is_some() && pick_ultracode.is_none()` is "this run chose
        // max and said nothing about ultracode", so the `max` is the deliberate
        // one and the inherited ultracode yields. Every other shape leaves
        // ultracode standing.
        if pick_effort.is_some() && pick_ultracode.is_none() {
            persistent.ultracode = None;
        } else {
            persistent.effort = None;
        }
    }
    if persistent.ultracode == Some(true) {
        // Ultracode pins effort to xhigh at runtime; record and emit the pair
        // explicitly so a role that never receives `--settings` (no edit_files)
        // still spawns at a truthful CLAUDE_CODE_EFFORT_LEVEL, and so the
        // user's own settings.json effort cannot collide with the flag.
        persistent.effort = Some("xhigh".into());
    } else if persistent.effort.is_none() {
        // The no-inherit floor: nothing picked, role default absent.
        persistent.effort = Some(crate::claude_config::DEFAULT_EFFORT.into());
    }
}

#[cfg(all(test, not(windows)))]
mod ensure_claude_runnable_tests {
    use super::ensure_claude_runnable;
    #[test]
    fn noop_off_windows() {
        assert!(ensure_claude_runnable("claude").is_ok());
        assert!(ensure_claude_runnable("does-not-exist-xyz").is_ok());
    }
}

fn build_command(cfg: &SpawnConfig) -> Command {
    let bin = cfg.claude_bin.as_deref().unwrap_or("claude");
    let mut cmd = Command::new(bin);
    cmd.arg("-p")
        .args(["--input-format", "stream-json"])
        .args(["--output-format", "stream-json"])
        // `--verbose` is REQUIRED when combining `-p` + stream-json IO.
        // See docs/stream-json-events.md.
        .arg("--verbose")
        .args([
            "--append-system-prompt-file",
            &cfg.system_prompt_path.display().to_string(),
        ]);

    // Per-role Claude-config overrides (Settings → Claude Config), already
    // resolved AND reconciled by the caller — see [`reconcile_spawn_knobs`] and
    // `core/session.rs`'s `spawn_plan`. This function CONSUMES the effective
    // set; it does not derive it.
    //
    // It used to derive it here, from two `SpawnConfig` fields the caller filled
    // from the participant row. That put the only implementation of the
    // precedence chain inside a synchronous fn with no `Storage`, which is why
    // the resolved value could not be recorded — and recording it is the only
    // way the UI can say what a participant was actually spawned with, since
    // re-resolving at read time answers "what it would be spawned with now".
    let agent_override = cfg.overrides.clone();

    if let Some(mcp) = &cfg.mcp_config_path {
        cmd.args(["--mcp-config", &mcp.display().to_string()])
            .arg("--strict-mcp-config");
    }

    // Resume a prior claude-code conversation for this agent if we have its
    // UUID stored. Lets a user close bot-hq and reopen the same session
    // without losing the agent's accumulated context. `--resume` coexists
    // with `-p` (`--help`: bracketed value skips the interactive picker).
    if let Some(resume_id) = &cfg.resume_session_id {
        cmd.args(["--resume", resume_id]);
    }

    // Permission posture is CAPABILITY-dependent — it asks whether this
    // participant's role was granted `edit_files`, not what the agent is called.
    //
    // A role that MAY edit runs with `--dangerously-skip-permissions`:
    // bot-hq is its permission layer (policy.yaml + UI dialogs + git hooks),
    // and letting claude-code prompt in parallel would double-gate, leak
    // prompts into stream-json (never reaching our UI), and hang the agent.
    //
    // A role that may NOT edit is review-only and must be MECHANICALLY unable
    // to mutate. A prompt instruction alone failed (2026-05-28: the reviewer of
    // the day ran Edit + git commit + gh issue create on a client repo).
    // `--dangerously-skip-
    // permissions` (bypass mode) CANNOT be used to enforce this because bypass
    // mode disables the permission layer entirely — deny rules are ignored.
    // Instead: `dontAsk` (no prompts, deny-by-default) + an allowlist of read-
    // only tools + an explicit denylist of the mutation surface. Deny wins
    // over allow, so `Bash` is allowed wholesale for read-only investigation
    // while mutating git/gh invocations are blocked (verified: colon-form
    // `Bash(cmd:*)` matching holds under dontAsk on claude 2.1.x). The
    // internal MCP server `bot-hq-signaling` is allowed as a unit; its gated
    // tools are checked server-side against this same set
    // (signaling/jsonrpc.rs).
    //
    // Parity: `edit_files` is in the seeded HANDS set and absent from the
    // seeded EYES set, so the two roles land on exactly the branches
    // `agent_name == "rain"` used to send them to. What changes is that a THIRD
    // role now lands somewhere deliberate instead of silently getting bypass
    // mode for not being called "rain".
    if !cfg.capabilities.grants(crate::agents::Capability::EditFiles) {
        // A read-only participant may reach its model through a third-party
        // Anthropic-compatible gateway (DeepSeek, via ANTHROPIC_BASE_URL). claude-code >= 2.1.156
        // serializes a SessionStart hook's `additionalContext` (a plugin's
        // SessionStart hook injects one) as a `role:"system"` entry inside
        // the request's `messages` array. The real Anthropic API tolerates
        // that; DeepSeek's gateway only accepts user/assistant roles and
        // rejects it ("unknown variant `system`, expected user or assistant"
        // → API Error 400). The LOAD-BEARING fix is the local normalizing
        // proxy (`agents::llm_proxy`): such a participant's ANTHROPIC_BASE_URL routes
        // through it and EVERY role:"system" entry in `messages[]` is hoisted
        // into the top-level `system` field before it reaches DeepSeek —
        // source-agnostic, so it also catches the plugin-sync injection that
        // running full (non-bare) mode brings back.
        //
        // We deliberately do NOT pass `--bare`. `--bare` (minimal mode,
        // CLAUDE_CODE_SIMPLE=1) was once kept as belt-and-suspenders against
        // that injection, but it ALSO disables claude-code's deferred-tool
        // loader (`ToolSearch`) — which left the reviewer with Grep/Glob/WebFetch/
        // ToolSearch/TodoWrite all inert ("exists but is not enabled in this
        // context"), i.e. its whole read-investigation surface beyond Read/
        // Bash. Since the proxy already neutralizes the role:"system"
        // injection --bare was guarding against, dropping --bare restores the
        // tool loader at no safety cost. Auth + routing are unaffected:
        // ANTHROPIC_AUTH_TOKEN + ANTHROPIC_BASE_URL are set as env below
        // regardless of mode. Read-only enforcement lives in `dontAsk` + the
        // allow/deny lists, NOT in --bare. (Trade-off: without --bare a
        // read-only participant syncs plugins + autodiscovers
        // CLAUDE.md/auto-memory the same as an editing one — heavier startup;
        // suppress per-agent via the override env if needed.)
        cmd.args(["--permission-mode", "dontAsk"]);
        cmd.args([
            "--allowedTools",
            "Read Grep Glob WebFetch WebSearch ToolSearch TodoWrite BashOutput KillShell Bash mcp__bot-hq-signaling",
        ]);
        // Read-only enforcement for EYES: deny BY WRITE VERB (not blanket noun)
        // for tools whose read forms we keep (`git branch`, `gh`), so the reads
        // fall through to the allowed `Bash` (deny wins over allow). Verb lists +
        // rationale live on the `*_WRITE_VERBS` consts above; the value is
        // assembled by `build_read_only_disallowed_tools`, and the `eyes_denies_*`
        // tests assert against the SAME consts so enforcement + test can't drift.
        let disallowed = build_read_only_disallowed_tools();
        cmd.args(["--disallowedTools", &disallowed]);
    } else {
        cmd.arg("--dangerously-skip-permissions");

        // Mechanical backstop for a role that may edit. It runs in bypass mode,
        // where claude-code's native deny rules are IGNORED — so the only thing
        // that can hard-stop an outward/mutating command is a hook. Inject a
        // PreToolUse Bash hook that calls back into THIS binary's `policy-check
        // tool-gate` to match each Bash command against the GLOBAL Tool Gate
        // keyword config BEFORE it executes: a `gate` keyword blocks the direct
        // call (exit 2) and routes the agent to the `action_gate` MCP tool,
        // which surfaces Approve/Reject and runs the command on approval; an
        // `auto_allow`/unmatched command is allowed through. This replaces the
        // per-project `tool_blocklist` role after the 2026-05-29 fabricated-
        // comment incident. A role WITHOUT `edit_files` is exempt: this hook is
        // injected only on this branch, and that role is already mechanically
        // read-only via the deny list above (its mutation surface is blocked
        // regardless of any hook). Injected via `--settings` (a process arg) so
        // NOTHING is written into the working repo's tree — it lives bot-hq-side, never in the working repo.
        match std::env::current_exe() {
            Ok(exe) => {
                let mut hook_cmd = format!(
                    "\"{}\" policy-check tool-gate --data-dir \"{}\"",
                    exe.display(),
                    cfg.data_dir.display(),
                );
                if let Some(project) = &cfg.project {
                    hook_cmd.push_str(&format!(" --project \"{project}\""));
                }
                hook_cmd.push_str(&format!(" --session \"{}\"", cfg.session_id));
                let mut settings = serde_json::json!({
                    "hooks": {
                        "PreToolUse": [{
                            "matcher": "Bash",
                            "hooks": [{ "type": "command", "command": hook_cmd }],
                        }],
                    }
                });
                // Fold in the agent's override fragment (skillOverrides /
                // enabledPlugins / ultracode). Built with serde_json so the
                // payload is always valid — avoids claude-code's silent-ignore
                // of malformed `--settings` in `-p` mode.
                if let serde_json::Value::Object(ref mut map) = settings {
                    for (k, v) in
                        crate::claude_config::overrides::settings_fragment(&agent_override)
                    {
                        map.insert(k, v);
                    }
                }
                cmd.args(["--settings", &settings.to_string()]);
            }
            Err(e) => warn!(
                agent = %cfg.agent_name,
                error = %e,
                "current_exe() failed — tool-gate PreToolUse hook NOT injected; \
                 falling back to prompt-level gating only"
            ),
        }
    }

    // Env-vars per ARCHITECTURE.md "Agents" section.
    cmd.env("ANTHROPIC_MODEL", &cfg.config.model_name);
    // BOT_HQ_SESSION_ID is read by the git pre-push hook to overlay
    // session-scoped approvals onto the resolved policy.
    cmd.env("BOT_HQ_SESSION_ID", &cfg.session_id);
    // BOT_HQ_AGENT lets the pre-push hook attribute the push-approval prompt to
    // the pushing agent (a read-only participant cannot push).
    // All agents route through build_command, so this lands for every participant.
    cmd.env("BOT_HQ_AGENT", &cfg.agent_name);
    if let Some(token) = &cfg.config.auth_token {
        if !token.is_empty() {
            cmd.env("ANTHROPIC_AUTH_TOKEN", token);
        }
    }
    // Route a custom (non-Anthropic) gateway through the local normalizing
    // proxy so any `role:"system"` message claude-code injects at request-
    // build time is hoisted out before it reaches a stricter gateway that
    // would 400 on it (a DeepSeek-backed participant). See `agents::llm_proxy`
    // for the full rationale. Falls back to the raw base_url if the proxy
    // didn't start. Agents with no base_url (the first-party API) get no
    // override and never touch the proxy.
    if let Some(base) = crate::agents::llm_proxy::resolve_anthropic_base_url(
        cfg.config.base_url.as_deref(),
        crate::agents::llm_proxy::proxy_addr(),
    ) {
        cmd.env("ANTHROPIC_BASE_URL", base);
    }

    // Per-agent override env (effort / auto-memory / CLAUDE.md suppression).
    // Applied to ALL agents. The skill/plugin `--settings` fragments above are
    // editing-participant-only (a read-only one gets no --settings), but these
    // ENV overrides are the lever to keep it lean now that nothing runs --bare.
    for (k, v) in crate::claude_config::overrides::env_vars(&agent_override) {
        cmd.env(k, v);
    }

    // Always pin the subprocess cwd. A repo-less session must not inherit
    // the app's own cwd — in dev that's the bot-hq repo itself, and the
    // claude-code child would adopt that repo's CLAUDE.md + user-scope
    // auto-memory as session context (observed bleed: s-79f8aafe quoted
    // stale memory). data_dir always exists by spawn time (paths.rs boot
    // init creates it).
    let wd = cfg.working_dir.as_deref().unwrap_or(&cfg.data_dir);
    cmd.current_dir(wd);

    cmd
}

/// The environment `build_command` would hand the child, as owned strings.
///
/// The sibling of [`debug_command`] for the half that does not ride on argv:
/// effort and the auto-memory/CLAUDE.md switches are env vars, so a test that
/// only reads args cannot see them. Exists so a spawn assembled in
/// `core::session` can be asserted through the ACTUAL command rather than
/// through the `SpawnConfig` field it was built from — the difference between
/// pinning a wire and pinning its two halves.
#[cfg(test)]
pub fn debug_env(cfg: &SpawnConfig) -> Vec<(String, String)> {
    let cmd = build_command(cfg);
    cmd.as_std()
        .get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().to_string(),
                v?.to_string_lossy().to_string(),
            ))
        })
        .collect()
}

/// Build the path-string form of the claude command for diagnostics / logging.
/// Not used by spawn; tests use it to assert flag set.
#[cfg(test)]
pub fn debug_command(cfg: &SpawnConfig) -> Vec<String> {
    let cmd = build_command(cfg);
    let std_cmd = cmd.as_std();
    let mut out = vec![std_cmd.get_program().to_string_lossy().to_string()];
    for arg in std_cmd.get_args() {
        out.push(arg.to_string_lossy().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::AgentConfig;
    use std::path::Path;

    #[tokio::test]
    async fn a_receipt_from_another_session_never_reaches_stdin() {
        // The check used to live on `SessionHandle::send_to_all`, which left
        // `SessionAgent::deliver` and `agent.handle.input().deliver(&receipt)`
        // as receipt-gated but NOT scope-gated routes to the same stdin. Both
        // end here, so this is the test that covers all three.
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s-a", "a", None).await.unwrap();
        storage.create_session("s-b", "b", None).await.unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        let input = ParticipantInput::new("s-a", tx);

        let kind = crate::storage::MessageKind::Text.as_str();
        let from_b = storage
            .post_to_channel("s-b", "user", None, kind, "meant for the other session", None)
            .await
            .unwrap();
        assert!(
            !input.deliver(&from_b).await,
            "a receipt from another session is refused, and the refusal is reported"
        );
        assert!(
            rx.try_recv().is_err(),
            "session A's agent must not read session B's row"
        );

        // A scope check, not a blanket refusal: this input's own session lands.
        let from_a = storage
            .post_to_channel("s-a", "user", None, kind, "meant for this session", None)
            .await
            .unwrap();
        assert!(input.deliver(&from_a).await);
        assert_eq!(
            rx.try_recv().unwrap().message.content,
            // rc3 D23: the wire says who wrote it. `[user]` here, and that is
            // the point of the label — a receipt from another session would
            // arrive looking identical without it.
            "[user] meant for this session"
        );
    }

    #[tokio::test]
    async fn a_batch_carrying_one_foreign_receipt_is_refused_whole() {
        // The batch form of the test above, and the property is deliberately
        // stronger than "the foreign row is dropped": a mismatch is a routing
        // bug in the caller, and handing the agent the REST of the batch would
        // put a partially-correct transcript in front of it — harder to reason
        // about, from either side, than none of it. So nothing goes out, and
        // `deliver_backlog`'s cursor stays put because the write reported
        // failure.
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s-a", "a", None).await.unwrap();
        storage.create_session("s-b", "b", None).await.unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        let input = ParticipantInput::new("s-a", tx);
        let kind = crate::storage::MessageKind::Text.as_str();

        let mine = storage
            .post_to_channel("s-a", "user", None, kind, "first", None)
            .await
            .unwrap();
        let theirs = storage
            .post_to_channel("s-b", "user", None, kind, "not mine", None)
            .await
            .unwrap();
        let also_mine = storage
            .post_to_channel("s-a", "user", None, kind, "third", None)
            .await
            .unwrap();

        assert!(
            !input
                .deliver_batch(&[mine.clone(), theirs, also_mine.clone()])
                .await,
            "one out-of-scope receipt refuses the batch"
        );
        assert!(
            rx.try_recv().is_err(),
            "and refusing means NOTHING was written — not the good rows either"
        );

        // A scope check, not a blanket refusal.
        assert!(input.deliver_batch(&[mine, also_mine]).await);
        assert_eq!(
            rx.try_recv().unwrap().message.content,
            format!("[user] first{}[user] third", crate::storage::WIRE_JOIN),
            "the batch is each row's own wire, joined — no batch-level decoration"
        );
    }

    #[tokio::test]
    async fn an_empty_batch_writes_nothing_and_reports_success() {
        // Total rather than live: `deliver_backlog` returns on an empty page
        // before it gets here. Reporting failure would be the wrong answer
        // anyway — there is no row that failed to arrive — and it would stop a
        // drain that has nothing left to do.
        let (tx, mut rx) = mpsc::channel(4);
        let input = ParticipantInput::new("s-a", tx);
        assert!(input.deliver_batch(&[]).await);
        assert!(rx.try_recv().is_err(), "an empty batch is not an empty line");
    }

    fn cfg() -> SpawnConfig {
        SpawnConfig {
            agent_name: "hands".into(),
            config: AgentConfig {
                agent_name: "hands".into(),
                provider: "anthropic".into(),
                model_name: "claude-opus-4-7".into(),
                base_url: None,
                auth_token: Some("sk-test".into()),
                updated_at: String::new(),
                context_window: None,
            },
            system_prompt_path: Path::new("/tmp/bot-hq-test-prompt.txt").to_path_buf(),
            mcp_config_path: Some(Path::new("/tmp/mcp.json").to_path_buf()),
            working_dir: Some(Path::new("/tmp/repo").to_path_buf()),
            claude_bin: Some("claude".into()),
            session_id: "test-session".into(),
            resume_session_id: None,
            project: Some("acme-app-exporter".into()),
            data_dir: Path::new("/tmp/data").to_path_buf(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::preset_hands(),
            ),
            overrides: crate::claude_config::AgentOverride::default(),
        }
    }

    /// A config for a role WITHOUT `edit_files` — the read-only spawn posture.
    ///
    /// It still sets the agent name, but the name no longer decides anything
    /// here: it is kept because the per-agent claude-overrides lookup
    /// (`resolve_agent_overrides`) is keyed on it. What moves the posture is the
    /// capability set — `posture_follows_the_capability_set_not_the_name` is the
    /// test that proves the name is inert.
    fn eyes_cfg() -> SpawnConfig {
        let mut c = cfg();
        c.agent_name = "eyes".into();
        c.config.agent_name = "eyes".into();
        c.capabilities = crate::agents::ResolvedCapabilities::Known(
            crate::agents::CapabilitySet::preset_eyes(),
        );
        c
    }

    #[test]
    fn agent_health_wire_strings() {
        // B2: the as_str values are the wire contract with the frontend
        // (session:agent_health payload + HealthDot styling) — lock them.
        assert_eq!(AgentHealth::Running.as_str(), "running");
        assert_eq!(AgentHealth::Retrying.as_str(), "retrying");
        assert_eq!(AgentHealth::Stalled.as_str(), "stalled");
        assert_eq!(AgentHealth::Dead.as_str(), "dead");
    }

    /// Batch 1 guarantee: killing an agent reaps its TOOL CHILDREN, not just the
    /// parent. We spawn a group leader that backgrounds a long-lived grandchild,
    /// group-kill via `kill_child(leader_pid)`, and assert the grandchild dies —
    /// proving we signal the process GROUP (`-pid`), not the lone parent. Without
    /// `process_group(0)` + the `-pid` signal this orphan would survive on init.
    #[cfg(unix)]
    #[test]
    fn kill_child_reaps_the_whole_process_group() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::process::CommandExt;
        use std::process::{Command as StdCommand, Stdio};

        // `sleep 600 &` = backgrounded grandchild; `echo $!` prints its pid; the
        // leader then blocks so the group stays alive until we kill it. `sh -c`
        // runs without job control, so the bg job shares the leader's PGID.
        let mut leader = StdCommand::new("sh")
            .arg("-c")
            .arg("sleep 600 & echo $!; sleep 600")
            .process_group(0) // mirror spawn_agent: leader is its group's leader
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn sh leader");

        let stdout = leader.stdout.take().expect("piped stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read grandchild pid");
        let grandchild: i32 = line.trim().parse().expect("parse grandchild pid");

        // Alive before the kill.
        assert_eq!(
            unsafe { libc::kill(grandchild, 0) },
            0,
            "grandchild should be alive pre-kill"
        );

        // Group-kill through the production reaper.
        kill_child(leader.id());

        // Poll for the grandchild to vanish (signal delivery + init reaping the
        // reparented zombie is near-instant; the slack just avoids flakiness).
        let gone = (0..300).any(|_| {
            if unsafe { libc::kill(grandchild, 0) } != 0 {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            false
        });
        let _ = leader.wait(); // reap the leader zombie
        assert!(gone, "group-kill must reap the grandchild (tool child)");
    }

    #[test]
    fn transient_api_statuses_are_retryable() {
        for s in [408, 425, 429, 500, 502, 503, 504, 529] {
            assert!(is_transient_api_error(s), "{s} should be transient");
        }
    }

    #[test]
    fn permanent_api_statuses_are_not_retryable() {
        // 400 = the DeepSeek system-role rejection; auth/forbidden/not-found
        // and semantic 4xx never clear on a blind retry.
        for s in [400, 401, 403, 404, 409, 413, 422, 451] {
            assert!(!is_transient_api_error(s), "{s} should be permanent");
        }
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let p = RetryPolicy {
            max_retries: 5,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
        };
        assert_eq!(p.backoff(1), Duration::from_secs(2));
        assert_eq!(p.backoff(2), Duration::from_secs(4));
        assert_eq!(p.backoff(3), Duration::from_secs(8));
        assert_eq!(p.backoff(4), Duration::from_secs(16));
        assert_eq!(p.backoff(5), Duration::from_secs(30)); // 32 → capped
        assert_eq!(p.backoff(99), Duration::from_secs(30));
    }

    #[test]
    fn repo_less_spawn_falls_back_to_data_dir_cwd() {
        let mut c = cfg();
        c.working_dir = None;
        let cmd = build_command(&c);
        assert_eq!(cmd.as_std().get_current_dir(), Some(Path::new("/tmp/data")));
    }

    #[test]
    fn pinned_working_dir_wins_over_data_dir_fallback() {
        let cmd = build_command(&cfg());
        assert_eq!(cmd.as_std().get_current_dir(), Some(Path::new("/tmp/repo")));
    }

    /// **The agent subprocess carries the session id the git hooks read** —
    /// CODEBASE.md seam 6 listed this producer as UNPINNED ("cut ⇒ findings
    /// gate silently skipped, pushes under `ask` blocked") until round 8.
    /// Kill-tested: comment out the `BOT_HQ_SESSION_ID` env line in
    /// `build_command` and this goes red.
    #[test]
    fn build_command_sets_the_session_id_the_hooks_read() {
        let c = cfg();
        let cmd = build_command(&c);
        let sid = cmd.as_std().get_envs().find_map(|(k, v)| {
            (k == std::ffi::OsStr::new("BOT_HQ_SESSION_ID")).then(|| v.map(|v| v.to_owned()))
        });
        assert_eq!(
            sid.flatten().as_deref(),
            Some(std::ffi::OsStr::new(c.session_id.as_str())),
            "BOT_HQ_SESSION_ID must be set to the session id on every agent spawn"
        );
    }

    #[test]
    fn overrides_merge_into_settings_and_env() {
        use crate::claude_config::{save_overrides, ClaudeOverrides, SkillVisibility};
        let dir = tempfile::tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        let hands = store.per_role.entry("hands".into()).or_default();
        hands
            .skills
            .insert("my-skill".into(), SkillVisibility::UserInvocableOnly);
        hands.effort = Some("high".into());
        save_overrides(dir.path(), &store).unwrap();

        // The spawn path resolves the store against the ROLE and hands the
        // result to `build_command` — mirrored here by the caller.
        let mut c = cfg();
        c.data_dir = dir.path().to_path_buf();
        c.overrides = crate::claude_config::resolve_agent_overrides(
            &crate::claude_config::load_overrides(dir.path()),
            Some("hands"),
        );

        // The injected --settings carries the override fragment alongside the hook.
        let args = debug_command(&c);
        let settings_arg = args
            .iter()
            .skip_while(|a| *a != "--settings")
            .nth(1)
            .expect("--settings present");
        assert!(
            settings_arg.contains("skillOverrides"),
            "got {settings_arg}"
        );
        assert!(
            settings_arg.contains("user-invocable-only"),
            "got {settings_arg}"
        );
        assert!(
            settings_arg.contains("PreToolUse"),
            "hook must survive merge"
        );

        // Effort override is injected as env.
        let cmd = build_command(&c);
        let has_effort = cmd.as_std().get_envs().any(|(k, v)| {
            k == std::ffi::OsStr::new("CLAUDE_CODE_EFFORT_LEVEL")
                && v == Some(std::ffi::OsStr::new("high"))
        });
        assert!(has_effort, "effort env should be set from override");
    }

    /// The persistent half of a cross-layer overlay, resolved the way the spawn
    /// path resolves it.
    ///
    /// **The three reconcile tests below used to write this to a file under
    /// `c.data_dir` and nothing read it back.** Overrides stopped being loaded
    /// inside `build_command` when they moved onto `SpawnConfig.overrides`
    /// (resolved by `core::session::resolve_participant_overrides`), so a store
    /// on disk that no resolver is pointed at is a fixture the code never sees:
    /// every "persistent" premise was absent, and each test was asserting only
    /// its session half. Verified by mutation on 2026-08-12 — the whole
    /// cross-layer reconcile block could be disabled with all 1049 tests green.
    ///
    /// Routing through the REAL `resolve_agent_overrides` (rather than building
    /// an `AgentOverride` by hand) keeps the `_all` → per-role layering in the
    /// path, which is what makes the premise a persistent override rather than
    /// a struct literal that happens to look like one.
    fn persistent_for_hands(
        f: impl FnOnce(&mut crate::claude_config::AgentOverride),
    ) -> crate::claude_config::AgentOverride {
        use crate::claude_config::{resolve_agent_overrides, ClaudeOverrides};
        let mut store = ClaudeOverrides::default();
        f(store.per_role.entry("hands".into()).or_default());
        resolve_agent_overrides(&store, Some("hands"))
    }

    /// `CLAUDE_CODE_EFFORT_LEVEL` as the built command carries it. `None` means
    /// the variable is not set AT ALL — a different answer from "set to
    /// something else", and the difference is what the exclusion turns on.
    fn effort_env(cfg: &SpawnConfig) -> Option<String> {
        build_command(cfg)
            .as_std()
            .get_envs()
            .find(|(k, _)| *k == std::ffi::OsStr::new("CLAUDE_CODE_EFFORT_LEVEL"))
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().to_string())
    }

    /// The `--settings` JSON a role holding `edit_files` always gets (it carries
    /// the PreToolUse hook), so a test can ask whether `ultracode` rode along.
    fn settings_fragment(cfg: &SpawnConfig) -> String {
        debug_command(cfg)
            .iter()
            .skip_while(|a| *a != "--settings")
            .nth(1)
            .expect("--settings present")
            .clone()
    }

    /// The three tests below were written against `SpawnConfig.session_effort` /
    /// `.session_ultracode` and asserted through `build_command`. Round 4 moved
    /// the precedence chain out to [`reconcile_spawn_knobs`] so its result could
    /// be RECORDED, which deleted those two fields — and deleting the fields
    /// stops these compiling.
    ///
    /// **The tempting repair was to delete them, and they are the only coverage
    /// of the exclusion rule** — a rule that silently flips a user's effort
    /// setting. Ported onto the pure function instead, which makes them faster
    /// and more precise; the end-to-end half is kept separately below, because a
    /// pure-function test cannot see a reconciled value failing to reach the
    /// child.
    #[test]
    fn a_per_run_pick_wins_over_the_persistent_override() {
        let mut o = persistent_for_hands(|o| o.effort = Some("high".into()));
        // Control: with no pick, the persistent effort survives untouched.
        // Without it the test cannot tell "the pick won" from "the persistent
        // value was never there" — the failure this test shipped with.
        reconcile_spawn_knobs(&mut o, None, None);
        assert_eq!(o.effort.as_deref(), Some("high"), "the persistent override must survive");

        reconcile_spawn_knobs(&mut o, Some("max"), None);
        assert_eq!(o.effort.as_deref(), Some("max"), "the per-run pick must win");
    }

    #[test]
    fn a_per_run_ultracode_clears_inherited_max_effort() {
        // Cross-layer collision: persistent effort=max under a per-run ultracode
        // pick (effort left on Inherit). Ultracode wins; the inherited max must
        // not survive alongside it.
        let mut o = persistent_for_hands(|o| o.effort = Some("max".into()));
        reconcile_spawn_knobs(&mut o, None, None);
        assert_eq!(o.effort.as_deref(), Some("max"), "control: the inherited max is live");

        let mut o = persistent_for_hands(|o| o.effort = Some("max".into()));
        reconcile_spawn_knobs(&mut o, None, Some(true));
        assert_eq!(
            o.effort.as_deref(),
            Some("xhigh"),
            "the inherited max must be cleared — and the floor records ultracode's implied xhigh"
        );
        assert_eq!(o.ultracode, Some(true), "the per-run ultracode is what survives");
    }

    #[test]
    fn a_per_run_max_clears_inherited_ultracode() {
        // The reverse: persistent ultracode under an explicit per-run max.
        let mut o = persistent_for_hands(|o| o.ultracode = Some(true));
        reconcile_spawn_knobs(&mut o, None, None);
        assert_eq!(o.ultracode, Some(true), "control: the inherited ultracode is live");
        assert_eq!(
            o.effort.as_deref(),
            Some("xhigh"),
            "a legacy ultracode-only override floors to its implied xhigh"
        );

        let mut o = persistent_for_hands(|o| o.ultracode = Some(true));
        reconcile_spawn_knobs(&mut o, Some("max"), None);
        assert_eq!(o.effort.as_deref(), Some("max"), "the explicit max wins");
        assert_eq!(o.ultracode, None, "ultracode must be cleared by it");
    }

    /// Both knobs picked in the same run is NOT a collision — the user said both
    /// explicitly, and `max` yields because ultracode is the stronger posture.
    /// Pinned because it is the one branch the two collision tests never reach:
    /// they each leave one pick on Default.
    #[test]
    fn picking_both_in_one_run_keeps_ultracode() {
        let mut o = persistent_for_hands(|_| {});
        reconcile_spawn_knobs(&mut o, Some("max"), Some(true));
        assert_eq!(o.ultracode, Some(true));
        assert_eq!(
            o.effort.as_deref(),
            Some("xhigh"),
            "ultracode is the stronger posture, so max yields to its implied xhigh"
        );
    }

    /// The no-inherit floor (2026-08-25): nothing picked, nothing configured
    /// for the role → `DEFAULT_EFFORT`, never `None`. This is what guarantees
    /// every spawn emits a concrete `CLAUDE_CODE_EFFORT_LEVEL` and the user's
    /// own settings.json knob stops reaching agents.
    #[test]
    fn nothing_picked_nothing_configured_floors_to_default_effort() {
        let mut o = persistent_for_hands(|_| {});
        reconcile_spawn_knobs(&mut o, None, None);
        assert_eq!(o.effort.as_deref(), Some(crate::claude_config::DEFAULT_EFFORT));
        assert_eq!(o.ultracode, None);

        // A per-run level pick sent with ultracode:false (the dialog's shape
        // for every concrete pick) clears a role-default ultracode.
        let mut o = persistent_for_hands(|o| {
            o.effort = Some("xhigh".into());
            o.ultracode = Some(true);
        });
        reconcile_spawn_knobs(&mut o, Some("high"), Some(false));
        assert_eq!(o.effort.as_deref(), Some("high"));
        assert_eq!(o.ultracode, Some(false), "the explicit clear survives");
    }

    /// **The equality this whole redesign rests on.** Recording the reconciled
    /// pair only means something if the row describes the process — so assert the
    /// reconciled value against what the built `Command` actually carries, in
    /// both directions of the exclusion.
    ///
    /// A test that only checked "something was persisted" would pass while the
    /// row and the child disagreed, which is the failure at-spawn recording was
    /// chosen to prevent.
    #[test]
    fn the_reconciled_pair_is_what_reaches_the_child() {
        // Ultracode wins: env carries the implied xhigh (never the cleared max
        // — and never nothing, so a role that gets no --settings still spawns
        // at a truthful level), --settings carries ultracode.
        let mut o = persistent_for_hands(|o| o.effort = Some("max".into()));
        reconcile_spawn_knobs(&mut o, None, Some(true));
        let mut c = cfg();
        c.overrides = o.clone();
        assert_eq!(
            effort_env(&c).as_deref(),
            Some("xhigh"),
            "ultracode must reach the env as its implied xhigh, not as the cleared max"
        );
        assert!(
            settings_fragment(&c).contains("ultracode"),
            "the surviving ultracode must reach --settings"
        );

        // Max wins: env carries max, --settings carries no ultracode.
        let mut o = persistent_for_hands(|o| o.ultracode = Some(true));
        reconcile_spawn_knobs(&mut o, Some("max"), None);
        let mut c = cfg();
        c.overrides = o;
        assert_eq!(effort_env(&c).as_deref(), Some("max"), "the surviving max must reach the env");
        assert!(
            !settings_fragment(&c).contains("ultracode"),
            "a cleared ultracode must not reach --settings; got {}",
            settings_fragment(&c)
        );
    }

    #[test]
    fn a_read_only_role_gets_override_env_but_no_settings_fragment() {
        use crate::claude_config::{load_overrides, resolve_agent_overrides, save_overrides,
                                   ClaudeOverrides};
        let dir = tempfile::tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        store.all.disable_auto_memory = Some(true); // fan-out default
        save_overrides(dir.path(), &store).unwrap();

        let mut c = eyes_cfg();
        c.data_dir = dir.path().to_path_buf();
        c.overrides = resolve_agent_overrides(&load_overrides(dir.path()), Some("eyes"));

        let args = debug_command(&c);
        assert!(
            !args.iter().any(|a| a == "--settings"),
            "a role without edit_files gets no --settings (the tool-gate PreToolUse hook \
             rides with the permissive posture)"
        );
        // env-based overrides still apply to it.
        let cmd = build_command(&c);
        let has = cmd.as_std().get_envs().any(|(k, v)| {
            k == std::ffi::OsStr::new("CLAUDE_CODE_DISABLE_AUTO_MEMORY")
                && v == Some(std::ffi::OsStr::new("1"))
        });
        assert!(has, "the _all fan-out reaches a read-only role too");
    }

    // ---- supervisor retry logic (fake incarnations, no real subprocess) ----

    /// A fake `AgentHandle` whose event stream the test drives directly. Push
    /// events via `ev_tx`; close the incarnation by dropping it. Observe the
    /// resume-nudge (and any peer input) via `in_rx`.
    fn fake_incarnation() -> (
        AgentHandle,
        mpsc::Sender<AgentEvent>,
        mpsc::Receiver<OutgoingUserMessage>,
    ) {
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(16);
        let (in_tx, in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (control_tx, _control_rx) = mpsc::channel::<ControlRequest>(8);
        let (kill_tx, _kill_rx) = oneshot::channel::<()>();
        let handle = AgentHandle {
            name: "fake".into(),
            event_rx: ev_rx,
            input_tx: ParticipantInput::new("test-session", in_tx),
            control_tx,
            kill_tx: Some(kill_tx),
            interrupted_epoch: Arc::new(std::sync::atomic::AtomicU64::new(NO_INTERRUPT_EPOCH)),
        };
        (handle, ev_tx, in_rx)
    }

    fn errored_turn(status: u16) -> AgentEvent {
        AgentEvent::TurnComplete {
            stop_reason: None,
            subtype: Some("error_during_execution".into()),
            is_error: true,
            api_error_status: Some(status),
            context: ContextReport::none(ContextVerdict::NoWindow),
        }
    }

    fn clean_turn() -> AgentEvent {
        AgentEvent::TurnComplete {
            stop_reason: Some("end_turn".into()),
            subtype: Some("success".into()),
            is_error: false,
            api_error_status: None,
            context: ContextReport::none(ContextVerdict::NoWindow),
        }
    }

    fn instant_policy(max_retries: u32) -> RetryPolicy {
        RetryPolicy {
            max_retries,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn supervisor_resumes_after_transient_then_stops_clean() {
        let (h1, ev1, _in1) = fake_incarnation();
        let (h2, ev2, mut in2) = fake_incarnation();

        let mut queue = std::collections::VecDeque::new();
        queue.push_back(h2);
        let spawn_next = move |_c: SpawnConfig| {
            let h = queue.pop_front().expect("unexpected extra respawn");
            async move { Ok(h) }
        };

        let (out_ev_tx, mut out_ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (_out_in_tx, out_in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (_out_ctl_tx, out_ctl_rx) = mpsc::channel::<ControlRequest>(8);
        let (_kill_tx, kill_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(supervise(
            cfg(),
            instant_policy(5),
            h1,
            out_ev_tx,
            out_in_rx,
            out_ctl_rx,
            kill_rx,
            spawn_next,
        ));

        // Incarnation 1 hits a transient 529, then exits.
        ev1.send(errored_turn(529)).await.unwrap();
        drop(ev1);

        // The resumed incarnation is nudged to continue.
        let nudge = in2
            .recv()
            .await
            .expect("resumed incarnation should be nudged");
        assert!(
            nudge.message.content.contains("529"),
            "nudge names the status"
        );
        assert!(nudge.message.content.to_lowercase().contains("resumed"));

        // Incarnation 2 does real work and finishes cleanly.
        ev2.send(AgentEvent::Text("resumed work".into()))
            .await
            .unwrap();
        ev2.send(clean_turn()).await.unwrap();
        drop(ev2);

        task.await.unwrap();

        let mut got = Vec::new();
        while let Some(ev) = out_ev_rx.recv().await {
            got.push(ev);
        }
        assert!(
            matches!(
                got.first(),
                Some(AgentEvent::TurnComplete { is_error: true, .. })
            ),
            "errored turn is forwarded to the peer pump"
        );
        assert!(got
            .iter()
            .any(|e| matches!(e, AgentEvent::Text(t) if t == "resumed work")));
        assert!(got.iter().any(|e| matches!(
            e,
            AgentEvent::TurnComplete {
                is_error: false,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn supervisor_terminates_when_incarnation_input_pump_dies() {
        // The incarnation's stdin pump death = its input receiver dropped, while
        // its EVENT channel stays open (child still emitting). The supervisor must
        // NOT bridge to a now-deaf child forever (the #4 user→HANDS desync) — it
        // tears down so the public input channel closes (the is_stale signal),
        // WITHOUT a respawn-in-place.
        let (h1, _ev1, in1) = fake_incarnation();
        drop(in1); // kill the incarnation's stdin pump (receiver gone)

        let mut queue: std::collections::VecDeque<AgentHandle> = std::collections::VecDeque::new();
        let spawn_next = move |_c: SpawnConfig| {
            let h = queue
                .pop_front()
                .expect("input-pump death must NOT trigger a respawn-in-place");
            async move { Ok(h) }
        };

        let (out_ev_tx, _out_ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (out_in_tx, out_in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (_out_ctl_tx, out_ctl_rx) = mpsc::channel::<ControlRequest>(8);
        let (_kill_tx, kill_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(supervise(
            cfg(),
            instant_policy(5),
            h1,
            out_ev_tx,
            out_in_rx,
            out_ctl_rx,
            kill_rx,
            spawn_next,
        ));

        // A user message arrives; forwarding it to the dead incarnation pump
        // fails, which must terminate the supervisor. `_ev1` is kept alive so the
        // event channel stays OPEN — only the input-pump path can end the loop.
        out_in_tx
            .send(OutgoingUserMessage::text("hello"))
            .await
            .unwrap();

        task.await.unwrap();

        assert!(
            out_in_tx.is_closed(),
            "input-pump death must terminate the supervisor so the session goes stale"
        );
    }

    #[tokio::test]
    async fn supervisor_does_not_resume_permanent_error() {
        let (h1, ev1, _in1) = fake_incarnation();
        // Empty queue: any respawn pops-and-panics, failing the test.
        let mut queue: std::collections::VecDeque<AgentHandle> = std::collections::VecDeque::new();
        let spawn_next = move |_c: SpawnConfig| {
            let h = queue
                .pop_front()
                .expect("permanent error must NOT trigger a respawn");
            async move { Ok(h) }
        };

        let (out_ev_tx, mut out_ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (_out_in_tx, out_in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (_out_ctl_tx, out_ctl_rx) = mpsc::channel::<ControlRequest>(8);
        let (_kill_tx, kill_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(supervise(
            cfg(),
            instant_policy(5),
            h1,
            out_ev_tx,
            out_in_rx,
            out_ctl_rx,
            kill_rx,
            spawn_next,
        ));

        ev1.send(errored_turn(400)).await.unwrap(); // permanent
        drop(ev1);

        task.await.unwrap(); // returns without a respawn

        // A terminated supervisor drops its input receiver, so the stable
        // sender now reads closed — exactly the signal `SessionHandle::is_stale`
        // uses to evict + re-spawn a crashed session.
        assert!(
            _out_in_tx.is_closed(),
            "terminated supervisor must close the input channel (is_stale signal)"
        );

        let mut got = Vec::new();
        while let Some(ev) = out_ev_rx.recv().await {
            got.push(ev);
        }
        assert!(got.iter().any(|e| matches!(
            e,
            AgentEvent::TurnComplete {
                is_error: true,
                api_error_status: Some(400),
                ..
            }
        )));
        assert!(
            !got.iter()
                .any(|e| matches!(e, AgentEvent::Text(t) if t.contains("Stopped after"))),
            "permanent error must not emit the transient give-up message"
        );
    }

    #[tokio::test]
    async fn supervisor_gives_up_after_max_retries() {
        // max_retries = 2 → initial + 2 respawns = 3 incarnations, then surface.
        let (h1, ev1, _in1) = fake_incarnation();
        let (h2, ev2, mut in2) = fake_incarnation();
        let (h3, ev3, mut in3) = fake_incarnation();

        let mut queue = std::collections::VecDeque::new();
        queue.push_back(h2);
        queue.push_back(h3);
        let spawn_next = move |_c: SpawnConfig| {
            let h = queue.pop_front().expect("more respawns than budget allows");
            async move { Ok(h) }
        };

        let (out_ev_tx, mut out_ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (_out_in_tx, out_in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (_out_ctl_tx, out_ctl_rx) = mpsc::channel::<ControlRequest>(8);
        let (_kill_tx, kill_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(supervise(
            cfg(),
            instant_policy(2),
            h1,
            out_ev_tx,
            out_in_rx,
            out_ctl_rx,
            kill_rx,
            spawn_next,
        ));

        ev1.send(errored_turn(529)).await.unwrap();
        drop(ev1);
        in2.recv().await.expect("nudge to incarnation 2");
        ev2.send(errored_turn(503)).await.unwrap();
        drop(ev2);
        in3.recv().await.expect("nudge to incarnation 3");
        ev3.send(errored_turn(529)).await.unwrap();
        drop(ev3);

        task.await.unwrap();

        let mut got = Vec::new();
        while let Some(ev) = out_ev_rx.recv().await {
            got.push(ev);
        }
        assert!(
            got.iter().any(|e| matches!(
                e,
                AgentEvent::Text(t) if t.contains("Stopped after") && t.contains('2')
            )),
            "expected the give-up message after exhausting 2 retries; got {got:?}"
        );
    }

    #[test]
    fn command_has_required_flags() {
        let argv = debug_command(&cfg());
        assert_eq!(argv[0], "claude");
        assert!(argv.iter().any(|a| a == "-p"));
        assert!(argv.iter().any(|a| a == "--verbose"));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--input-format" && w[1] == "stream-json"));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--output-format" && w[1] == "stream-json"));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--mcp-config" && w[1] == "/tmp/mcp.json"));
        assert!(argv.iter().any(|a| a == "--strict-mcp-config"));
        assert!(argv.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(argv.windows(2).any(|w| w[0] == "--append-system-prompt-file"
            && w[1].ends_with("bot-hq-test-prompt.txt")));
        // The inline form must never be used — it would put the multi-KB prompt
        // on the command line and trip Windows' 32,767-char limit.
        assert!(!argv.iter().any(|a| a == "--append-system-prompt"));
        // No resume flag when SpawnConfig.resume_session_id is None.
        assert!(!argv.iter().any(|a| a == "--resume"));
    }

    #[test]
    fn posture_follows_the_capability_set_not_the_name() {
        // The reframe, at the spawn seam. `agent_name` used to pick the branch;
        // now `edit_files` does, and these four cases are what "the name is
        // inert" means concretely. Without this, the gate could quietly go on
        // keying off the name for every session the user actually runs — because
        // in those sessions the name and the capability agree.
        use crate::agents::{CapabilitySet, ResolvedCapabilities};

        let bypass = |c: &SpawnConfig| {
            debug_command(c)
                .iter()
                .any(|a| a == "--dangerously-skip-permissions")
        };

        // Named "rain", but granted edit_files → the permissive posture.
        let mut c = eyes_cfg();
        c.capabilities = ResolvedCapabilities::Known(CapabilitySet::preset_hands());
        assert!(
            bypass(&c),
            "a role holding `edit_files` must get bypass mode whatever it is called"
        );

        // Named "brian", but NOT granted edit_files → the restrictive posture.
        let mut c = cfg();
        c.capabilities = ResolvedCapabilities::Known(CapabilitySet::preset_eyes());
        assert!(
            !bypass(&c),
            "a role without `edit_files` must not get bypass mode whatever it is called"
        );

        // A third role nobody hardcoded, granted nothing → restrictive. Under
        // the name check this landed in the `else` branch and silently got
        // bypass mode for the sole reason that it was not called "rain".
        let mut c = cfg();
        c.agent_name = "scout".into();
        c.config.agent_name = "scout".into();
        c.capabilities = ResolvedCapabilities::Known(CapabilitySet::default());
        assert!(
            !bypass(&c),
            "an unrecognised role with no grants must not get bypass mode"
        );

        // An unreadable roster → restrictive. Fail closed, same as the gate.
        let mut c = cfg();
        c.capabilities = ResolvedCapabilities::Unreadable {
            reason: "no participant row",
        };
        assert!(
            !bypass(&c),
            "an unreadable capability set must not get bypass mode"
        );
    }

    #[test]
    fn eyes_gets_deny_by_default_not_bypass() {
        // EYES enforcement: Rain must NOT get bypass mode (which nullifies
        // deny rules); she gets dontAsk + an allowlist + a mutation denylist.
        let c = eyes_cfg();
        let argv = debug_command(&c);

        assert!(
            !argv.iter().any(|a| a == "--dangerously-skip-permissions"),
            "Rain must not run in bypass mode (it ignores deny rules): {argv:?}"
        );
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "--permission-mode" && w[1] == "dontAsk"),
            "expected `--permission-mode dontAsk`: {argv:?}"
        );
        // Allowlist keeps read-only investigation + the signaling MCP.
        let allowed = argv
            .windows(2)
            .find(|w| w[0] == "--allowedTools")
            .map(|w| w[1].clone())
            .expect("--allowedTools present");
        // Web/reference tools must match Rain's role prompt (prompts.rs) — the
        // prompt promises WebFetch/WebSearch/ToolSearch, so the allowlist must
        // grant all three or claude-code silently blocks what the prompt offers.
        for t in [
            "Read",
            "Grep",
            "Glob",
            "Bash",
            "mcp__bot-hq-signaling",
            "WebFetch",
            "WebSearch",
            "ToolSearch",
        ] {
            assert!(allowed.contains(t), "allowlist missing {t}: {allowed}");
        }
        // Denylist covers the mutation surface from the 2026-05-28 incident.
        let denied = argv
            .windows(2)
            .find(|w| w[0] == "--disallowedTools")
            .map(|w| w[1].clone())
            .expect("--disallowedTools present");
        for t in [
            "Edit",
            "Write",
            "NotebookEdit",
            "Bash(git commit:*)",
            "Bash(git push:*)",
            "Bash(gh issue create:*)",
            "Bash(gh pr merge:*)",
        ] {
            assert!(denied.contains(t), "denylist missing {t}: {denied}");
        }
    }

    #[test]
    fn eyes_denies_gh_write_allows_gh_read() {
        // Issue (2026-06-05): Rain should keep read-only `gh` (view/list/diff)
        // while every mutating `gh` form stays blocked. Deny wins over allow, so
        // the denylist must NOT contain a blanket `gh <noun>:*` (that would also
        // kill the read forms) and MUST enumerate the write verbs.
        let c = eyes_cfg();
        let argv = debug_command(&c);
        let denied = argv
            .windows(2)
            .find(|w| w[0] == "--disallowedTools")
            .map(|w| w[1].clone())
            .expect("--disallowedTools present");

        // Every mutating gh verb is blocked — asserted against the SAME consts
        // the production deny-list is built from, so the two can't drift.
        for (noun, verbs) in [
            ("gh pr", GH_PR_WRITE_VERBS),
            ("gh issue", GH_ISSUE_WRITE_VERBS),
            ("gh release", GH_RELEASE_WRITE_VERBS),
            ("gh repo", GH_REPO_WRITE_VERBS),
        ] {
            for v in verbs {
                let pat = format!("Bash({noun} {v}:*)");
                assert!(denied.contains(&pat), "gh write verb not denied: {pat}\n{denied}");
            }
        }
        // The escape hatch — gh api can POST/PATCH/DELETE anything.
        assert!(
            denied.contains("Bash(gh api:*)"),
            "gh api must be denied:\n{denied}"
        );

        // No blanket noun deny survives (it would block the read forms).
        for blanket in [
            "Bash(gh pr:*)",
            "Bash(gh issue:*)",
            "Bash(gh repo:*)",
            "Bash(gh release:*)",
        ] {
            assert!(
                !denied.contains(blanket),
                "blanket gh deny would block read forms: {blanket}"
            );
        }

        // Read forms have no dedicated deny entry, so they fall through to the
        // allowed `Bash` (a `view`/`list`/`diff` substring must not appear as a
        // denied pattern).
        for read in [
            "Bash(gh issue view:*)",
            "Bash(gh pr view:*)",
            "Bash(gh pr diff:*)",
            "Bash(gh repo view:*)",
        ] {
            assert!(
                !denied.contains(read),
                "read form should not be explicitly denied: {read}"
            );
        }
    }

    #[test]
    fn eyes_denies_git_branch_write_allows_read() {
        // 2026-06-17 cross-model survey: the blanket `Bash(git branch:*)` deny
        // blocked read-only listing too — DeepSeek-EYES hit 10+ false denials on
        // legit `git branch --show-current`/`-a` reads (incl. compound
        // `git branch … && echo …`). Mirror the gh deny-by-write-verb shape: only
        // mutating git-branch forms denied, read forms fall through to allowed Bash.
        let c = eyes_cfg();
        let argv = debug_command(&c);
        let denied = argv
            .windows(2)
            .find(|w| w[0] == "--disallowedTools")
            .map(|w| w[1].clone())
            .expect("--disallowedTools present");

        // Every mutating git-branch form is blocked — asserted against the SAME
        // const the production deny-list is built from, so the two can't drift.
        for v in GIT_BRANCH_WRITE_VERBS {
            let pat = format!("Bash(git branch {v}:*)");
            assert!(
                denied.contains(&pat),
                "git branch write form not denied: {pat}\n{denied}"
            );
        }

        // The blanket noun deny must NOT survive (it blocked read-only listing).
        assert!(
            !denied.contains("Bash(git branch:*)"),
            "blanket git branch deny would block read forms: {denied}"
        );

        // Read forms have no dedicated deny entry — they fall through to allowed Bash.
        for read in ["Bash(git branch --show-current:*)", "Bash(git branch -a:*)"] {
            assert!(
                !denied.contains(read),
                "read form should not be explicitly denied: {read}"
            );
        }
    }

    #[test]
    fn hands_still_gets_bypass() {
        // HANDS keeps full power — bypass mode, no allow/deny lists.
        let argv = debug_command(&cfg()); // cfg() is brian
        assert!(argv.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(!argv.iter().any(|a| a == "--permission-mode"));
        assert!(!argv.iter().any(|a| a == "--allowedTools"));
        assert!(!argv.iter().any(|a| a == "--disallowedTools"));
        // Brian hits the real Anthropic API, which tolerates the system-role
        // message claude-code injects from plugin SessionStart hooks, so he
        // does NOT need --bare (and would lose CLAUDE.md/LSP if he had it).
        assert!(!argv.iter().any(|a| a == "--bare"));
    }

    #[test]
    fn eyes_runs_without_bare_so_tool_loader_works() {
        // Rain must NOT run `--bare`. `--bare` (CLAUDE_CODE_SIMPLE=1) disables
        // claude-code's deferred-tool loader (`ToolSearch`), which left Rain's
        // Grep/Glob/WebFetch/ToolSearch/TodoWrite inert ("exists but is not
        // enabled in this context") — her whole read surface beyond Read/Bash.
        // The role:"system" injection --bare once guarded against is
        // neutralized by `llm_proxy` (it hoists every such entry out of
        // `messages[]` into the top-level `system` field), so dropping --bare
        // restores the tool surface at no safety cost.
        let c = eyes_cfg();
        let argv = debug_command(&c);
        assert!(
            !argv.iter().any(|a| a == "--bare"),
            "Rain must NOT run --bare (it disables the ToolSearch tool loader); \
             the llm_proxy handles the role:system injection instead: {argv:?}"
        );
    }

    #[test]
    fn resume_session_id_emits_resume_flag() {
        let mut c = cfg();
        c.resume_session_id = Some("abc-123-uuid".into());
        let argv = debug_command(&c);
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "--resume" && w[1] == "abc-123-uuid"),
            "expected `--resume abc-123-uuid` in argv: {argv:?}"
        );
    }

    #[test]
    fn hands_gets_tool_gate_pretooluse_hook() {
        let argv = debug_command(&cfg()); // cfg() is brian
        let settings = argv
            .windows(2)
            .find(|w| w[0] == "--settings")
            .map(|w| w[1].clone())
            .expect("hands must get --settings carrying the PreToolUse hook");
        assert!(settings.contains("PreToolUse"), "settings: {settings}");
        assert!(
            settings.contains("policy-check tool-gate"),
            "hook must call the gate subcommand: {settings}"
        );
        assert!(
            settings.contains("acme-app-exporter"),
            "hook must be bound to the session's project: {settings}"
        );
        assert!(
            settings.contains("\"matcher\":\"Bash\""),
            "hook must match the Bash tool: {settings}"
        );
    }

    #[test]
    fn eyes_does_not_get_tool_gate_hook() {
        // The tool-gate PreToolUse hook is injected via --settings in the HANDS
        // (Brian) branch only; Rain is already mechanically read-only via the
        // deny list, so she gets no --settings at all.
        let c = eyes_cfg();
        let argv = debug_command(&c);
        assert!(
            !argv.iter().any(|a| a == "--settings"),
            "Rain must NOT get --settings: {argv:?}"
        );
    }
}
