//! Per-agent event pump. Persists agent events to storage, fans text chunks
//! out to the peer with the IPAV buffer rule.

use crate::agents::{AgentEvent, AgentHealth};
use crate::core::activity::ActivityTracker;
use crate::core::ipav::{IpavPhase, IpavState};
use crate::signaling::SignalingBridge;
use crate::storage::{MessageKind, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
// Test-only since Batch 6 removed the buffered-window timer (the sole non-test
// `Duration` user); the test sleeps below still need it.
#[cfg(test)]
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, warn};

/// Borrow-serialized row shapes for the message log (O3): serialized directly with
/// `serde_json::to_string` instead of building an intermediate `serde_json::json!`
/// `Value` (which re-boxes the already-owned `input`/`content`) only to
/// `.to_string()` it. Fields are declared in the key order `serde_json` emits for a
/// `json!` map (alphabetical — no `preserve_order` feature), so the stored JSON is
/// byte-identical to the previous output.
#[derive(serde::Serialize)]
struct ToolUseRow<'a> {
    input: &'a serde_json::Value,
    name: &'a str,
    tool_use_id: &'a str,
}

#[derive(serde::Serialize)]
struct ToolResultRow<'a> {
    content: &'a str,
    is_error: bool,
    tool_use_id: &'a str,
}

#[derive(Clone)]
pub struct PumpConfig {
    /// `Arc<str>` (not `String`): cloned once per persisted message on the hottest
    /// path (`notify_persisted` fires on every Text / ToolUse / ToolResult), so a
    /// refcount bump beats a heap copy. Threaded as `Arc<str>` through
    /// `MessagePersisted` into the `BatchEmitter` dirty-set / watermark keys (O5).
    pub session_id: Arc<str>,
    /// This participant's roster slug — its `messages.author` string, its
    /// `ActivityTracker` key, and its handle in the tray.
    pub slug: Arc<str>,
    /// `session_participants.id` for this pump's agent.
    ///
    /// `None` on the test/hardcoded paths and whenever the roster read failed —
    /// same degradation as [`SessionAgent::participant_id`].
    pub participant_id: Option<i64>,
    /// Does this participant hold `edit_files`? The capability predicate that
    /// replaced `matches!(cfg.author, Author::Brian)` on the pre-Apply mutation
    /// nudge — bot-hq must gate on the ticked boxes, never on which role a name
    /// implies (rc3 D11).
    pub edits_files: bool,
    /// Optional bridge for firing MessagePersisted events after every
    /// successful storage.insert_message. None in tests that don't need
    /// event-driven readers.
    pub bridge: Option<Arc<SignalingBridge>>,
    /// The agent's OWN stdin sender (distinct from the router-owned peer-forward
    /// path), for A3a self-nudges — e.g. nudging Brian when he mutates during
    /// Investigate/Plan. `None` disables self-nudging (Rain; tests that don't
    /// need it). Set only for Brian's pump at spawn.
    pub self_input_tx: Option<crate::agents::ParticipantInput>,
    /// Per-session activity tracker (interrupt redesign, Batch 2). The pump
    /// clears this agent's `busy` on `TurnComplete`/`Exited`, and sets the
    /// PEER's `busy` when it forwards a chunk. `None` in tests / solo configs
    /// that don't drive the input lock.
    pub activity: Option<Arc<ActivityTracker>>,
    /// Shared "this agent is mid-atomic-tool" flag (interrupt redesign, Batch
    /// 3.1 Part 1). The pump sets it on an atomic `ToolUse` (git commit/push/
    /// migration) and clears it on the matching `ToolResult`/`TurnComplete`, so
    /// `cancel_session_turn` can DEFER a kill until the op completes (no
    /// half-written worktree). Shared session-level; only HANDS trips it. `None`
    /// in tests / solo configs that don't drive cancel deferral.
    pub in_atomic_tool: Option<Arc<AtomicBool>>,
    /// Per-agent liveness for the Batch 7 stall watchdog — the pump touches it on
    /// every event and tracks tools-in-flight. `None` in tests / solo configs
    /// that don't run the watchdog.
    pub liveness: Option<Arc<crate::core::watchdog::AgentLiveness>>,
    /// Sender to the turn sequencer (`core::sequencer`), the B5 replacement for
    /// `router_tx`. The pump emits one `TurnComplete` per finished turn, carrying
    /// the consensus vote `turn_ending()` derives from the same `peer_ack` signals
    /// the Forward above carries.
    ///
    /// **Both may be set during the changeover**, and that is deliberate: it lets
    /// one session run the ring while the router still drives the rest, which is
    /// how the sequencer earns the right to task 14's deletion. `None` = this
    /// pump does not feed a ring.
    pub sequencer_tx: Option<mpsc::Sender<crate::core::sequencer::SequencerCommand>>,
    /// The epoch of the turn this participant currently holds, written by the
    /// sequencer at handover.
    ///
    /// **Read at the START of a turn, never at its end**, and the difference is
    /// the whole reason this is a cell rather than a value on the completion. A
    /// user message mid-turn resets the ring and moves the epoch while this
    /// participant still holds; a completion that read the cell on its way out
    /// would carry the NEW epoch, pass the sequencer's guard, and step a ring
    /// that had just been re-pointed at it — two participants on a turn at once,
    /// the one invariant that loop exists to keep. Snapshotting on the first
    /// event of the turn makes the stale completion carry the OLD epoch, which is
    /// exactly what the guard is there to reject.
    ///
    /// # A STRAGGLER must not open a turn (rc3 D24)
    ///
    /// "The first event after a completion" is not the same thing as "the first
    /// event of the next turn", and treating them as one wedged a live session.
    /// A participant that emits anything in the gap between completing and being
    /// handed its next turn snapshots the cell as it stands — which is still the
    /// epoch it just completed with. The real turn then arrives, the guard sees
    /// `turn_epoch` already set, and every completion from that point carries a
    /// number the ring retired. They are all discarded, the ring cannot step past
    /// a participant it is waiting on, and nothing in the loop recovers.
    ///
    /// Measured in `s-206e8921`: the reviewer completed at 03:56:01 carrying
    /// epoch 9, was handed epoch 11 at 03:56:28, and completed again at 04:01:51
    /// **still carrying 9**. A 27-second window was all it took, and the session
    /// stopped dead for the twenty minutes until the user noticed.
    ///
    /// The fix is `pump_agent`'s `last_completed_epoch`: a cell that still reads
    /// what this pump last completed with means no new turn has been handed out,
    /// so the event is a straggler and opens nothing. The epoch strictly
    /// increases at every handover, so "unchanged" is an exact test rather than a
    /// heuristic.
    pub turn_epoch: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// True while this participant is ORIENTING rather than holding a turn
    /// (rc3 **D21**). Set before the primer goes out, cleared before the ring
    /// hands out turn one.
    ///
    /// **The explicit signal D21 asks for, replacing an inference that is simply
    /// wrong during boot.** The pump learns a turn started from its own first
    /// event ([`Self::turn_epoch`]) — but during boot no turn has been handed
    /// out, so the cell still reads its initial `0`, `last_completed_epoch` is
    /// `None`, and the pump happily opens a turn on epoch 0. The completion that
    /// follows carries 0, which the ring discards forever: precisely the class
    /// D24 fixed, reached through a different door. D21 names this the hard part
    /// and *"where this will break if rushed"*.
    ///
    /// An `AtomicBool` rather than a message because the pump reads it once per
    /// EVENT and must not take a channel in that path, and because one store
    /// flips every participant at once — boot ends for the session, not per
    /// agent.
    pub booting: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Where this pump reports that it finished orienting (rc3 **D21**),
    /// carrying its participant id.
    ///
    /// The boot counterpart of [`Self::sequencer_tx`], and deliberately NOT the
    /// same channel: a `TurnComplete` during boot would carry epoch 0 and be
    /// discarded, so the ring would never learn anyone was ready. D21 §4 needs
    /// exactly this signal — *"when every participant has finished orienting …
    /// the ring starts"*.
    pub boot_done: Option<tokio::sync::mpsc::Sender<i64>>,
}

impl PumpConfig {
    pub fn new(session_id: impl Into<Arc<str>>, slug: impl Into<Arc<str>>) -> Self {
        Self {
            session_id: session_id.into(),
            slug: slug.into(),
            edits_files: false,
            participant_id: None,
            bridge: None,
            self_input_tx: None,
            activity: None,
            in_atomic_tool: None,
            liveness: None,
            sequencer_tx: None,
            turn_epoch: None,
            booting: None,
            boot_done: None,
        }
    }

    /// Whether this participant is orienting rather than holding a turn (rc3
    /// D21). `false` whenever no flag was wired, so every existing caller keeps
    /// today's behaviour exactly.
    fn is_booting(&self) -> bool {
        self.booting
            .as_ref()
            .is_some_and(|b| b.load(std::sync::atomic::Ordering::Acquire))
    }

    fn notify_persisted(&self, message_id: i64) {
        if let Some(bridge) = &self.bridge {
            bridge.notify_message_persisted(self.session_id.clone(), message_id);
        }
    }
}

/// True for a tool call that performs an atomic, hard-to-resume mutation — a
/// `git commit`/`git push` or a DB migration. A cancel arriving mid-flight
/// should DEFER the agent kill until such an op finishes, so the working tree /
/// repo isn't left half-written (interrupt redesign, Batch 3.1 Part 1). Matches
/// HANDS's two atomic-op surfaces: a direct `Bash` command, or an `action_gate`
/// (a gated command — surfaced MCP-prefixed as
/// `mcp__bot-hq-signaling__action_gate`, so match by suffix). Rain is read-only
/// and never trips this. The `migrate` match is deliberately broad (sqlx /
/// artisan / rails / npm): a false positive only defers a kill briefly (8s-
/// capped, self-clears on the ToolResult); a false negative is the exact bug
/// this prevents.
fn is_atomic_command(name: &str, input: &serde_json::Value) -> bool {
    let is_command_surface = name == "Bash" || name.ends_with("action_gate");
    if !is_command_surface {
        return false;
    }
    let cmd = input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    cmd.contains("git commit") || cmd.contains("git push") || cmd.contains("migrate")
}

/// True for the `peer_ack` MCP tool call — the bare alias (tests) or the
/// MCP-prefixed wire name (`mcp__bot-hq-signaling__peer_ack`). When the pump
/// sees this ToolUse, it suppresses the turn's peer-forward: the agent
/// explicitly acknowledged its peer without wanting to wake it for a full turn.
/// Behavioral happy-path layer ON TOP of the L2 volley-breaker, never a
/// replacement (weak models that never call it still hit L2).
fn is_peer_ack_tool(name: &str) -> bool {
    name == "peer_ack" || name.ends_with("__peer_ack")
}

/// True for a `peer_ack` call that passed `final: true` — the agent asserting
/// "this turn is my closing statement; record it, don't wake my peer".
///
/// Without it the router can only INFER substance from length
/// (`PEER_ACK_MAX_SUPPRESSED_LEN`), and that proxy misfires on the exact turn
/// shape that ENDS a volley: "I agree, and here is the one reason why" runs past
/// 200 chars, so it forwards, so the peer wakes and replies. Filed from a live
/// session as feedback #6 with worked examples.
///
/// Suppression is safe to make explicit because it has never destroyed content —
/// the turn's text is persisted by the `AgentEvent::Text` arm as it arrives,
/// independent of whether a Forward is later emitted. `final` skips the WAKE,
/// not the record.
fn peer_ack_is_final(name: &str, input: &serde_json::Value) -> bool {
    is_peer_ack_tool(name) && input.get("final").and_then(|v| v.as_bool()) == Some(true)
}

/// True for the `pass_turn` MCP tool call — the bare alias (tests) or the
/// MCP-prefixed wire name (`mcp__bot-hq-signaling__pass_turn`).
///
/// Observed here rather than acted on bridge-side for the same reason
/// [`is_peer_ack_tool`] is: what a turn MEANT is a property of the whole turn,
/// and the pump is the only place that sees the turn end. The bridge handler
/// cannot know yet whether the pass will be overridden by text the agent has
/// not written.
///
/// **Suffix match, not `contains`.** `ends_with` is what stops a tool merely
/// NAMED after this one — or a differently-prefixed gateway's
/// `foo__pass_turn_v2` — from being read as a pass; the peer_ack matrix above
/// found the same class of near-miss worth pinning, and
/// `is_pass_turn_tool_matches_bare_and_prefixed` pins it here.
fn is_pass_turn_tool(name: &str) -> bool {
    name == "pass_turn" || name.ends_with("__pass_turn")
}

/// The row a pass posts, so the pass is VISIBLE (design §1: "the pass is
/// recorded in the channel so it is visible").
///
/// Prose (`MessageKind::Text`) under `origin = 'participant'`, which is the rc3
/// decision, and it is what makes the row render today: `ChatMessage` dispatches
/// on `kind`, and every kind it does not special-case falls through to the
/// prose branch anyway — so a `pass` kind would render identically while
/// costing a migration this slice does not take. `system_notice` was the
/// alternative and is wrong twice over: it is documented as HOST-emitted and
/// every writer of it in this crate posts under `origin = 'system'`, which is
/// the origin rc3 decided against; and D7 already prices that lane as carrying
/// five injections at one-line sizing.
///
/// Phrased as the participant's own line because that is whose row it is: the
/// author header above it names them.
const PASS_NOTICE: &str = "(passed — nothing to add this round)";

/// Provider quota/limit phrases, matched case-insensitively against each text
/// chunk. Deliberately a plain substring net over ALL provider eras: the
/// archive study found these render as ordinary agent speech — Brian sat dead
/// 3h13m across two quota deaths while the session looked merely quiet, and
/// the reviewer kept reviewing into the void. The net is over TEXT, not over a
/// status code, which is why it covered a second backend without a second
/// implementation and why it keeps working now there is one.
/// Misclassification cost is a spurious tray notice.
const PROVIDER_LIMIT_PATTERNS: &[&str] = &[
    "out of usage credits",
    "hit your session limit",
    "usage limit reached",
    "insufficient balance",
    "payment required",
    "quota exceeded",
    "credit balance is too low",
];

/// A provider error arrives as a terse, standalone chunk; agent ANALYSIS that
/// quotes one arrives inside prose. Only chunks at or under this length are
/// candidates — without the bound, an agent discussing a quota incident (or a
/// reviewer quoting the detector's own patterns) self-trips the halt.
const PROVIDER_LIMIT_MAX_CHUNK: usize = 240;

/// The first line of `text` containing a provider-limit phrase, if any.
/// Terse chunks only (see [`PROVIDER_LIMIT_MAX_CHUNK`]).
fn detect_provider_limit(text: &str) -> Option<String> {
    if text.trim().len() > PROVIDER_LIMIT_MAX_CHUNK {
        return None;
    }
    let lower = text.to_lowercase();
    if !PROVIDER_LIMIT_PATTERNS.iter().any(|p| lower.contains(p)) {
        return None;
    }
    text.lines()
        .find(|l| {
            let ll = l.to_lowercase();
            PROVIDER_LIMIT_PATTERNS.iter().any(|p| ll.contains(p))
        })
        .map(|l| l.trim().to_string())
}

/// Re-notification window for a single limit incident: a quota death can emit
/// its message on several consecutive nudged turns; one notice per window.
const LIMIT_NOTICE_DEDUPE: std::time::Duration = std::time::Duration::from_secs(600);

/// Pump events from one agent. Each text chunk is persisted; the peer-forward
/// path depends on the current IPAV phase. `TurnComplete` flushes pending
/// buffered text immediately regardless of phase.
pub async fn pump_agent(
    cfg: PumpConfig,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    storage: Storage,
    ipav_state: Arc<Mutex<IpavState>>,
) {
    let mut buffer = String::new();
    // peer_ack (behavioral layer): set when the agent calls the `peer_ack` tool
    // during this turn; consumed at the turn's flush to suppress that turn's
    // peer-forward. Per-turn — reset after every TurnComplete (success OR error).
    let mut peer_ack_pending = false;
    let mut peer_ack_final_pending = false;
    // pass_turn (design §1): set when the agent declines this turn. Per-turn and
    // reset alongside the peer_ack pair below — a pass that leaked into the next
    // turn would make a participant that DID speak look like it had passed, and
    // the pass is the one ending that leaves the tally standing.
    let mut pass_pending = false;
    // A3a: one-shot guard so Brian gets at most one "you're mutating before
    // Apply" nudge per session (delivered to his own stdin via self_input_tx).
    let mut mutate_nudged = false;
    // Batch 3.1 Part 1: the tool_use_id of an in-flight atomic op (git commit/
    // push/migration), so a cancel can defer the kill until it completes. We
    // match the clearing ToolResult by id — claude-code can emit parallel tool
    // calls, so clearing on ANY result would race a still-running commit.
    let mut atomic_tool_id: Option<String> = None;
    // Provider-limit detection: the first matching line seen this turn, and the
    // last time a notice fired (per-incarnation dedupe — one notice per
    // incident, not one per nudged retry).
    let mut limit_line: Option<String> = None;
    let mut last_limit_notice: Option<std::time::Instant> = None;
    // s-f6a441ff: consecutive errored turns for THIS pump. ONE errored turn
    // ends `Spoke` and the ring steps past it — a failure is not a claim there
    // is nothing left to do (see the ending derivation below). But a SECOND in
    // a row means the participant cannot work at all, and letting the ring
    // keep dealing turned a context-blown pair into an error volley: 11
    // "Prompt is too long" turns in 5 minutes before the text-repeat net
    // halted the cycle — silently. Two in a row → host-declared halt with the
    // error as the visible reason, the provider-limit stall's route.
    let mut consecutive_errored_turns: usize = 0;
    // B5: the epoch of the turn in flight, snapshotted from `cfg.turn_epoch` on
    // this turn's FIRST event and cleared when it completes. See the field's doc
    // for why reading it at completion time instead would defeat the guard it
    // exists to pass.
    let mut turn_epoch: Option<u64> = None;
    // The epoch this pump last COMPLETED with (rc3 D24). A cell still reading
    // this value means the ring has not handed out a turn since, so whatever
    // event is being processed is a straggler from the turn that ended and must
    // not open a new one. See `PumpConfig::turn_epoch`.
    let mut last_completed_epoch: Option<u64> = None;
    // Why this pump stopped, when it said so. `AgentEvent::Exited` carries the
    // process's own account; a channel that simply closes carries none, and the
    // post-loop says so rather than inventing one.
    let mut exit_msg: Option<String> = None;
    // When this pump last had a turn CLOSED, so the next turn's first event can
    // report how long the model took to produce anything (rc3 D26). Reset at
    // completion rather than at delivery because the pump is not told about the
    // handover — its first event IS how it learns.
    let mut turn_opened_at = std::time::Instant::now();

    loop {
        let Some(event) = event_rx.recv().await else { break };

        // Batch 7: any event means the agent is alive — reset the stall timer.
        if let Some(liveness) = &cfg.liveness {
            liveness.touch();
        }
        // First event of a turn: bind it to whichever epoch the sequencer had
        // handed out when the agent started speaking. Deliberately BEFORE the
        // match, so every event kind opens a turn — the agent may lead with a
        // tool call rather than prose, and a turn opened only by text would
        // snapshot late and miss exactly the reset this guards against.
        // **Boot opens no turn** (rc3 D21). No turn has been handed out, so the
        // cell still reads its initial 0 and the straggler guard below cannot
        // see that: `last_completed_epoch` is `None`, `Some(0) != None`, and the
        // pump would bind epoch 0 and then complete with it — discarded forever
        // by the ring, which is the exact class D24 fixed. The flag is the
        // explicit signal D21 asks for in place of that inference.
        if turn_epoch.is_none() && !cfg.is_booting() {
            if let Some(cell) = &cfg.turn_epoch {
                let live = cell.load(std::sync::atomic::Ordering::Acquire);
                // **Unchanged since this pump's last completion = no new turn.**
                // Binding here would tie the NEXT turn to a retired epoch, and
                // every completion after it would be discarded — see the field
                // doc for the session that died this way.
                if last_completed_epoch == Some(live) {
                    debug!(
                        agent = %cfg.slug,
                        epoch = live,
                        "straggler event after a completed turn; not opening a turn on it"
                    );
                } else {
                    turn_epoch = Some(live);
                    // **How long the model took to say anything** (rc3 D26).
                    // The gap between the ring handing a turn out and its first
                    // event is the one stretch bot-hq records nothing for, and
                    // it is exactly the stretch a user stares at wondering
                    // whether the session is thinking or wedged. A live one ran
                    // 565 seconds on 2026-08-13 and the only way to find out
                    // afterwards was to diff two tables.
                    //
                    // INFO because it is once per turn and it is the number
                    // `scripts/turn-latency.py` calls `start` — measurable from
                    // the log now, not only by reconstruction.
                    tracing::info!(
                        agent = %cfg.slug,
                        epoch = live,
                        waited_ms = turn_opened_at.elapsed().as_millis() as u64,
                        "turn opened: first event after the ring handed it over"
                    );
                }
            }
        }

        match event {
            AgentEvent::Text(text) => {
                match storage
                    // `text` is read again below (limit detection, buffer), so
                    // this one borrows; the tool payloads further down move.
                    // The session id is an `Arc<str>` clone — a refcount bump,
                    // not the per-chunk allocation `&*cfg.session_id` would
                    // have cost once the parameter stopped being `&str`.
                    // rc3 D21: what a participant says while ORIENTING is a
                    // `boot` row — persisted and shown to the user, filtered out
                    // of every peer's backlog by `channel_page`.
                    .post_to_channel(
                        cfg.session_id.clone(),
                        "participant",
                        Some(&cfg.slug),
                        if cfg.is_booting() {
                            MessageKind::Boot.as_str()
                        } else {
                            MessageKind::Text.as_str()
                        },
                        &text,
                        None,
                    )
                    .await
                {
                    Ok(m) => cfg.notify_persisted(m.message_id()),
                    Err(e) => warn!(?e, "persisting text"),
                }
                if limit_line.is_none() {
                    limit_line = detect_provider_limit(&text);
                }

                buffer.push_str(&text);
                buffer.push('\n');
            }
            AgentEvent::ToolUse { id, name, input } => {
                // peer_ack: the agent explicitly acked its peer this turn — flag it
                // so this turn's Forward tells the router to suppress the wake.
                if is_peer_ack_tool(&name) {
                    peer_ack_pending = true;
                    // `final: true` = the agent ASSERTS this is its closing turn,
                    // so the router suppresses regardless of length instead of
                    // inferring substance from a byte count.
                    if peer_ack_is_final(&name, &input) {
                        peer_ack_final_pending = true;
                    }
                }
                // pass_turn: the agent declined this turn. Flagged, not acted on
                // — `turn_ending` decides at the flush whether the pass stands
                // or the turn's own text overrode it.
                if is_pass_turn_tool(&name) {
                    pass_pending = true;
                }
                // Batch 7: a tool call started — suppress stall detection until
                // its ToolResult (a long build/install emits no events meanwhile).
                if let Some(liveness) = &cfg.liveness {
                    liveness.tool_started();
                }
                // Batch 3.1 Part 1: flag an atomic op (git commit/push/
                // migration) so a cancel defers the kill until it completes.
                // Shared session flag; only HANDS trips it (Rain is read-only).
                if let Some(flag) = cfg.in_atomic_tool.as_ref() {
                    if is_atomic_command(&name, &input) {
                        flag.store(true, Ordering::Release);
                        atomic_tool_id = Some(id.clone());
                    }
                }
                // A3a (adherence): catch Brian mutating before the Apply phase —
                // a one-time self-nudge to advance first. Brian-only (Rain can't
                // mutate), gated by adherence_nudges, fired at most once.
                if !mutate_nudged
                    && cfg.edits_files
                    && matches!(name.as_str(), "Edit" | "Write" | "NotebookEdit")
                {
                    // The guard stays on `self_input_tx`: it is what says this
                    // pump belongs to a participant that HAS a stdin — a live
                    // agent rather than a test harness — and the nudge is only
                    // meaningful for one. The reminder itself no longer writes
                    // to it (see the persist below).
                    if cfg.self_input_tx.is_some() {
                        let phase = ipav_state.lock().await.current_phase;
                        if matches!(phase, IpavPhase::Investigate | IpavPhase::Plan)
                            && storage.adherence_nudges_enabled().await
                        {
                            // Host-authored, so it posts as `system` with a NULL
                            // participant: it is not Brian's turn output even
                            // though it lands on Brian's stdin. No envelope —
                            // this site never wrapped the text, and B5 Task 2 is
                            // a plumbing change, not a prompt change.
                            match storage
                                .post_to_channel(
                                    cfg.session_id.clone(),
                                    "system",
                                    None,
                                    MessageKind::SystemNotice.as_str(),
                                    "🔔 You're editing files before the Apply phase. Per IPAV, \
                                     mutations belong in Apply — call advance_phase(\"Apply\") \
                                     first, or note why this edit is intentional. (One-time \
                                     reminder.)",
                                    None,
                                )
                                .await
                            {
                                Ok(m) => {
                                    cfg.notify_persisted(m.message_id());
                                    // Persisted only. The direct write went into
                                    // the stdin of the agent that is mid-EDIT,
                                    // which cannot read it mid-generation
                                    // anyway: it opened a fresh generation the
                                    // ring never dealt, whose completion was
                                    // discarded, and the row then arrived again
                                    // off the cursor. Read at this agent's next
                                    // dealt turn instead — later than the edit,
                                    // and still the first moment it can act
                                    // (advance the phase, or say why the edit
                                    // was intended).
                                    // Burnt on a successful POST, and a failed
                                    // delivery still burns it — same as before,
                                    // when the send's error was discarded. A
                                    // dead stdin is not something the next Edit
                                    // would fix.
                                    mutate_nudged = true;
                                }
                                // Not burnt: nothing was recorded and nothing
                                // was sent, so the one-shot is still unspent.
                                Err(e) => warn!(?e, "persisting the pre-Apply mutation nudge"),
                            }
                        }
                    }
                }
                let payload = serde_json::to_string(&ToolUseRow {
                    input: &input,
                    name: &name,
                    tool_use_id: &id,
                })
                .unwrap_or_else(|_| "{}".to_string());
                match storage
                    // `payload` MOVES: it is not read again, and a tool_use
                    // input can be large. Borrowing here would copy the whole
                    // body into the receipt on every tool call.
                    .post_to_channel(
                        cfg.session_id.clone(),
                        "participant",
                        Some(&cfg.slug),
                        MessageKind::ToolUse.as_str(),
                        payload,
                        None,
                        )
                    .await
                {
                    Ok(m) => cfg.notify_persisted(m.message_id()),
                    Err(e) => warn!(?e, "persisting tool_use"),
                }
            }
            AgentEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                // Batch 7: tool result returned — one fewer tool in flight.
                if let Some(liveness) = &cfg.liveness {
                    liveness.tool_finished();
                }
                // Batch 3.1 Part 1: clear the atomic-op flag once THIS op's
                // result returns (id-matched → parallel-call safe).
                if atomic_tool_id.as_deref() == Some(tool_use_id.as_str()) {
                    if let Some(flag) = cfg.in_atomic_tool.as_ref() {
                        flag.store(false, Ordering::Release);
                    }
                    atomic_tool_id = None;
                }
                let payload = serde_json::to_string(&ToolResultRow {
                    content: content.as_str(),
                    is_error,
                    tool_use_id: &tool_use_id,
                })
                .unwrap_or_else(|_| "{}".to_string());
                match storage
                    // `payload` MOVES — same reason, and this is the biggest
                    // body of the three: a tool result can carry a whole file.
                    .post_to_channel(
                        cfg.session_id.clone(),
                        "participant",
                        Some(&cfg.slug),
                        MessageKind::ToolResult.as_str(),
                        payload,
                        None,
                        )
                    .await
                {
                    Ok(m) => cfg.notify_persisted(m.message_id()),
                    Err(e) => warn!(?e, "persisting tool_result"),
                }
            }
            AgentEvent::TurnComplete {
                is_error, context, ..
            } => {
                // Context occupancy rides the turn-complete event because that
                // is the only place claude-code reports `contextWindow`.
                // Publish it BEFORE the error branch below: a failed turn still
                // consumed context, and the meter going stale exactly when a
                // session starts erroring would hide the most useful reading.
                if let (Some(c), Some(bridge)) = (context.usable(), &cfg.bridge) {
                    bridge.notify_agent_context(
                        cfg.session_id.to_string(),
                        &cfg.slug,
                        c.used_tokens,
                        c.context_window,
                    );
                }
                // …and record EVERY report, usable or not (rc3 P7). The meter
                // above is live-only: it is forwarded to a UI that may not be
                // open, it is overwritten by the next turn, and it dies with the
                // session — which is why a participant that died with `Prompt is
                // too long` on 2026-08-12 left no evidence of what its context
                // was doing beforehand. The unusable reports are the load-bearing
                // half: without a row for them, "the provider never sent a
                // window" and "the agent never finished a turn" are the same
                // empty query result.
                //
                // Best-effort, exactly like the rows above it: a failed insert is
                // warned about and never interrupts a turn.
                if let Err(e) = storage
                    .record_context_reading(&cfg.session_id, &cfg.slug, &context)
                    .await
                {
                    warn!(?e, agent = %cfg.slug, "persisting context reading");
                }
                // Provider limit hit this turn: surface it as a real state
                // instead of letting it pass as agent speech. Peer notice FIRST
                // (the awaiting flag set below suppresses later forwards — this
                // one wake is deliberate, so the reviewer stops reviewing into
                // the void), then health + a tray halt so the user sees a
                // needs-input signal instead of a merely-quiet session.
                if let Some(line) = limit_line.take() {
                    let deduped = last_limit_notice
                        .is_some_and(|t| t.elapsed() < LIMIT_NOTICE_DEDUPE);
                    if !deduped {
                        last_limit_notice = Some(std::time::Instant::now());
                        warn!(agent = %cfg.slug, %line, "provider limit detected; pausing session on the user");
                        let notice = format!(
                            "⚠ [bot-hq] {} hit a provider limit and is paused: \
                             \"{line}\". Do not expect replies from them, and do \
                             not take over their work — the session waits on the \
                             user to resume.",
                            &cfg.slug
                        );
                        // Host-authored, so the row posts as `system` with a NULL
                        // participant like the other host injections. The ring
                        // delivers it to every peer off its cursor — no separate
                        // wire copy, and none of the hold/drop ladder the router
                        // used to put between this row and the peer reading it.
                        match storage
                            .post_to_channel(
                                cfg.session_id.clone(),
                                "system",
                                None,
                                MessageKind::SystemNotice.as_str(),
                                notice.as_str(),
                                None,
                            )
                            .await
                        {
                            Ok(m) => cfg.notify_persisted(m.message_id()),
                            // No row. The health mark and the tray halt below are
                            // deliberately NOT gated on it: those tell the USER
                            // the session is parked, which stays true either way.
                            Err(e) => warn!(?e, "persisting the provider-limit notice"),
                        }
                        if let Some(bridge) = &cfg.bridge {
                            bridge.notify_agent_health(
                                cfg.session_id.to_string(),
                                &cfg.slug,
                                "stalled",
                            );
                            // Host-initiated halt: discard the repeat-halt hint.
                            // That warning is for an AGENT yielding twice on one
                            // state; a provider-limit stall is the host parking
                            // the session and there is no agent turn to advise.
                            let _ = bridge
                                .mark_awaiting_user(
                                    cfg.session_id.to_string(),
                                    cfg.slug.to_string(),
                                    format!(
                                        "⚠ Provider limit: \"{line}\" — the agent can't \
                                         continue until it resets. Send any message (e.g. \
                                         'proceed') once it's resumable."
                                    ),
                                )
                                .await;
                        }
                    }
                }
                // The router owns self-idle on the forward path (it sequences
                // peer-busy BEFORE this agent's idle → no momentary Idle flicker).
                // The pump owns self-idle only when it does NOT hand a Forward to
                // the router: an errored turn, an empty buffer, or a solo session.
                // B5: what this ending MEANS, derived before the buffer is taken
                // below. An errored turn ends `Spoke` — it produced nothing, but
                // the ring has to step or the cycle stalls on a participant that
                // already failed; a failure is not a claim that there is nothing
                // left to do.
                //
                // **Nor is it a PASS**, and the distinction is the reason
                // `pass_pending` is not consulted here. A pass is a deliberate
                // "not me this round" that leaves every other participant's done
                // vote standing; an errored turn is a participant that could not
                // speak at all, and letting a crash preserve a tally it never
                // read would be a halt built on votes cast about a session the
                // failed participant never saw. `Spoke` clears the tally, which
                // is the conservative answer of the two.
                let ending = if is_error {
                    crate::core::sequencer::TurnEnding::SPOKE
                } else {
                    crate::core::sequencer::turn_ending(
                        peer_ack_pending,
                        peer_ack_final_pending,
                        pass_pending,
                        &buffer,
                    )
                };
                if is_error {
                    // Failed turn (API/permission error). The error text is already
                    // persisted per-chunk above for UI visibility, but must NOT be
                    // peer-forwarded: forwarding it bounces the error to the peer,
                    // the peer replies, and that re-triggers this failing agent — an
                    // unbounded error-spam loop (Rain on the DeepSeek gateway,
                    // 2026-05-29). Drain silently.
                    debug!(agent = %cfg.slug, "errored turn; draining buffer without router-forward");
                    consecutive_errored_turns += 1;
                    if consecutive_errored_turns >= 2 {
                        // Read before the buffer is cleared below: the error
                        // line is the turn's tail, and it is what the banner
                        // must say — a stop with no reason is the silence the
                        // repeat-net's halt already taught us not to repeat.
                        let last_line: String = buffer
                            .lines()
                            .rev()
                            .find(|l| !l.trim().is_empty())
                            .unwrap_or("unknown error")
                            .chars()
                            .take(200)
                            .collect();
                        warn!(
                            agent = %cfg.slug,
                            %last_line,
                            "back-to-back errored turns; declaring the session's halt"
                        );
                        if let Some(bridge) = &cfg.bridge {
                            let _ = bridge
                                .mark_awaiting_user(
                                    cfg.session_id.to_string(),
                                    cfg.slug.to_string(),
                                    format!(
                                        "⚠ {}'s turns are failing back-to-back \
                                         (last error: \"{last_line}\"). The session \
                                         stopped so you can steer. If the error is \
                                         about prompt/context size, this \
                                         participant's context is likely \
                                         unrecoverable — close the session and \
                                         open a fresh one.",
                                        &cfg.slug
                                    ),
                                )
                                .await;
                        }
                        // Re-arm rather than latch: the halt already stops the
                        // ring, so the next errors are a NEW incident — the
                        // user's release attempt — and deserve a fresh banner.
                        consecutive_errored_turns = 0;
                    }
                    buffer.clear();
                } else {
                    consecutive_errored_turns = 0;
                    // The turn's prose is already posted as rows by the pump
                    // above; the ring delivers those to every peer off its
                    // cursor. Nothing extra to hand anywhere — the router that
                    // used to own this second delivery path is gone (task 14),
                    // and it was already bypassed on every sequencer session.
                    buffer.clear();
                }
                // A pass gets a ROW, so declining a turn is something the user can
                // see rather than a gap in the transcript (design §1).
                //
                // **Written at the ending, not at the tool call, and BEFORE the
                // completion goes out.** Both halves are ordering decisions:
                //
                // - at the ending, because `turn_ending` may OVERRIDE the pass —
                //   a row posted when the tool fired would sit above the agent's
                //   own 900-character review claiming it had nothing to add;
                // - before the completion, because the completion is what steps
                //   the ring, and the sequencer reads the next participant's
                //   backlog straight out of storage. Send first and the two
                //   become a RACE between this insert and that read — and the
                //   losing side hands the next participant a backlog with no
                //   pass in it, so the row surfaces a round late. Awaiting the
                //   insert first is what removes the race rather than winning
                //   it; `a_pass_posts_its_row_before_the_completion_goes_out`
                //   is the pin, and it fails on the reordered form.
                //
                // Failure posts nothing and still completes the turn: a lost row
                // costs visibility, whereas a completion withheld over it would
                // freeze the ring on this participant for the rest of the
                // session. Same trade every host injection in this file makes.
                if matches!(ending, crate::core::sequencer::TurnEnding::Passed) {
                    match storage
                        .post_to_channel(
                            cfg.session_id.clone(),
                            "participant",
                            Some(&cfg.slug),
                            MessageKind::Text.as_str(),
                            PASS_NOTICE,
                            None,
                            )
                        .await
                    {
                        Ok(m) => cfg.notify_persisted(m.message_id()),
                        Err(e) => warn!(?e, agent = %cfg.slug, "persisting the pass row"),
                    }
                }
                // B5: tell the ring the turn ended. Sent for BOTH branches and
                // whether or not there was prose — the sequencer steps on the
                // completion, not on the text, so a silent turn that never
                // reported would freeze the cycle on this participant.
                // **Boot reports readiness, not a completion** (rc3 D21). The
                // epoch would be 0 — never issued — so a `TurnComplete` here is
                // dropped by the ring and nothing learns this participant is
                // ready. `boot_done` is the signal D21 §4 starts the ring on.
                if cfg.is_booting() {
                    if let (Some(done), Some(participant_id)) =
                        (&cfg.boot_done, cfg.participant_id)
                    {
                        if done.send(participant_id).await.is_err() {
                            warn!(
                                agent = %cfg.slug,
                                "boot completion DROPPED: the boot channel closed — the \
                                 session starts on its timeout instead"
                            );
                        }
                    }
                } else if let (Some(sequencer_tx), Some(participant_id)) =
                    (&cfg.sequencer_tx, cfg.participant_id)
                {
                    let epoch = turn_epoch.unwrap_or(0);
                    if sequencer_tx
                        .send(crate::core::sequencer::SequencerCommand::TurnComplete {
                            participant_id,
                            epoch,
                            ending,
                        })
                        .await
                        .is_err()
                    {
                        warn!(
                            agent = %cfg.slug,
                            "turn completion DROPPED: sequencer channel closed — the ring \
                             will not step past this participant"
                        );
                    }
                }
                // Opened by the next event, from whatever epoch is live then —
                // unless the cell has not moved, which means no turn was handed
                // out and the event is a straggler (rc3 D24).
                last_completed_epoch = turn_epoch;
                turn_epoch = None;
                // The clock for the NEXT turn's "how long until it spoke"
                // starts here — the ring hands the next turn out within
                // microseconds of this completion.
                turn_opened_at = std::time::Instant::now();
                // peer_ack is per-turn — reset after BOTH branches so an errored
                // turn (which skips the router) can't leak the flag into the next.
                peer_ack_pending = false;
                peer_ack_final_pending = false;
                pass_pending = false;
                // Turn ended → this agent is idle, UNLESS we handed off to the
                // router (which clears it after setting the peer busy, avoiding the
                // momentary `Idle` flicker that would unlock the input mid-handoff).
                {
                    if let Some(activity) = &cfg.activity {
                        activity.set_busy_slug(&cfg.slug, false);
                    }
                }
                // Batch 7: turn done → no tools can still be in flight; reset so a
                // stranded ToolUse-without-ToolResult can't wedge stall detection.
                if let Some(liveness) = &cfg.liveness {
                    liveness.reset_tools();
                }
                // Batch 3.1 Part 1: safety-clear a stranded atomic-op flag at
                // turn end (an atomic ToolUse with no matching ToolResult
                // shouldn't happen, but never strand the flag → never wedge a
                // future cancel). Guarded by our own id so this pump can't clear
                // a flag it didn't set (the flag is HANDS-only; Rain's pump
                // never holds an id).
                if atomic_tool_id.is_some() {
                    if let Some(flag) = cfg.in_atomic_tool.as_ref() {
                        flag.store(false, Ordering::Release);
                    }
                    atomic_tool_id = None;
                }
            }
            AgentEvent::Init { session_id, .. } => {
                debug!(agent = %cfg.slug, ?session_id, "init received");
                // Persist the claude-code session UUID so the next reopen of
                // this bot-hq session can resume each agent's prior context
                // via `--resume <uuid>`. Idempotent UPDATE — on a resume spawn
                // the same UUID comes back and we just overwrite with itself.
                // Stored on the PARTICIPANT row, not in one of two `sessions`
                // columns keyed by agent name (rc3 D10). The old setter could
                // only address two agents and returned `Err` for any other, so a
                // third participant's conversation was dropped and it restarted
                // blank on every respawn.
                if let (Some(claude_id), Some(pid)) = (session_id, cfg.participant_id) {
                    if let Err(e) = storage.set_participant_claude_id(pid, &claude_id).await {
                        warn!(?e, agent = %cfg.slug, "persisting claude session id");
                    }
                }
            }
            AgentEvent::Exited(msg) => {
                warn!(agent = %cfg.slug, msg = %msg, "agent exited");
                exit_msg = Some(msg.clone());
                // Trailing prose is already in the channel as rows; a peer
                // reads it off its cursor whenever it next takes a turn, so a
                // dying agent no longer needs to push a final copy anywhere.
                buffer.clear();
                // The agent is dying → force self-idle unconditionally (the
                // post-loop cleanup also clears it; idempotent).
                if let Some(activity) = &cfg.activity {
                    activity.set_busy_slug(&cfg.slug, false);
                }
                break;
            }
            AgentEvent::Health(state) => {
                // B2: relay the retry-supervisor's liveness transition to the
                // UI as a health dot. Not persisted — purely a status signal.
                if let Some(bridge) = &cfg.bridge {
                    bridge.notify_agent_health(
                        cfg.session_id.to_string(),
                        &cfg.slug,
                        state.as_str(),
                    );
                }
            }
        }
    }

    // Pump terminated (channel closed — the supervisor suppresses per-incarnation
    // Exited events, so a closed channel is the reliable "agent stopped" signal).
    // Clear its busy unconditionally so a crashed/stopped agent can't strand the
    // session `Busy` with the chat input locked.
    if let Some(activity) = &cfg.activity {
        activity.set_busy_slug(&cfg.slug, false);
    }
    // Batch 3.1 Part 1: crashed/stopped mid-atomic-tool → clear the flag so a
    // pending deferred cancel can proceed (the agent's already dead) and a
    // respawn isn't blocked. Guarded by our own id (Rain's pump never sets it).
    if atomic_tool_id.is_some() {
        if let Some(flag) = cfg.in_atomic_tool.as_ref() {
            flag.store(false, Ordering::Release);
        }
    }
    // B2: the event loop ended → the agent's supervisor returned (exhausted
    // retries / permanent error / process exit / intentional close). Flag it
    // dead so the UI dot goes red. On an intentional close the session is being
    // removed anyway, so a late "dead" is harmless.
    if let Some(bridge) = &cfg.bridge {
        bridge.notify_agent_health(
            cfg.session_id.to_string(),
            &cfg.slug,
            AgentHealth::Dead.as_str(),
        );
    }
    // **A pump that dies HOLDING a turn has to end it.** Clearing busy above is
    // half the job: the ring still has this participant as its holder, waiting
    // for a completion from a process that no longer exists, and it steps on
    // nothing else. Nothing was minted here before — the health dot went red and
    // that was the whole account — so the cycle sat parked with an empty halt
    // slot until the next user message, which is the same wedge the ring's own
    // unreachable path used to leave.
    //
    // Declared under this agent's OWN slug, not "system": the ring resolves the
    // asker to a participant and only the HOLDER declaring ends the turn in
    // flight (rc3 D35), which is exactly what this is. `mark_awaiting_user`
    // fills the halt slot and parks the ring in one call.
    if turn_epoch.is_some() {
        let closed = matches!(
            storage.get_session(&cfg.session_id).await,
            Ok(Some(s)) if s.closed_at.is_some()
        );
        if let (false, Some(bridge)) = (closed, &cfg.bridge) {
            let detail = exit_msg
                .as_deref()
                .map(|m| format!(" ({m})"))
                .unwrap_or_default();
            let _ = bridge
                .mark_awaiting_user(
                    cfg.session_id.to_string(),
                    cfg.slug.to_string(),
                    format!(
                        "{} stopped mid-turn{detail} — the turn it was holding cannot \
                         end. Send a message to respawn them and deal a fresh turn.",
                        cfg.slug
                    ),
                )
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::spawn::{ContextReport, ContextVerdict};

    /// `recv()` with a deadline.
    ///
    /// A bare `rx.recv().await` turns a regression into a HANG rather than a
    /// failure: the test waits forever for a wire the broken code never sends,
    /// and prints nothing. This batch produced two — one wedged a run for seven
    /// minutes, and one hung `cargo test` outright when a session-id mismatch
    /// made a scope check refuse every wire. Both would have been a clean
    /// failure in seconds through this.
    async fn next_wire<T>(rx: &mut tokio::sync::mpsc::Receiver<T>) -> T {
        tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("expected a wire within 2s; none arrived")
            .expect("the sender was dropped before a wire arrived")
    }

    use crate::core::ipav::IpavPhase;

    async fn setup() -> (Storage, Arc<Mutex<IpavState>>) {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        let st = Arc::new(Mutex::new(IpavState::default()));
        (s, st)
    }

    /// `slug` is the roster slug a real session carries (`"hands"` / `"eyes"`).
    ///
    /// It used to be an `Author`, from which both the slug and `edits_files`
    /// were derived — the retired two-party discriminant standing in for "which
    /// of the seeded roles". Taking the slug directly is what rc3 D10 says
    /// identity is, and `edits_files` stays a capability question rather than a
    /// name question.
    fn fast_cfg(slug: &str) -> PumpConfig {
        PumpConfig {
            session_id: "s1".into(),
            slug: slug.into(),
            edits_files: slug == "hands",
            participant_id: None,
            bridge: None,
            self_input_tx: None,
            activity: None,
            in_atomic_tool: None,
            liveness: None,
            sequencer_tx: None,
            turn_epoch: None,
            booting: None,
            boot_done: None,
        }
    }

    /// A pump wired to the ring. `participant_id` is required: the pump only
    /// reports a turn end when it knows which participant ended it.
    ///
    /// The paragraph that used to sit above this one said the forward/suppress/
    /// break decision "is tested in `core::router`" — a module task 14 deleted,
    /// so it pointed a reader at coverage that cannot be there (round 3). The
    /// decision itself moved into the ring with everything else; what this
    /// helper still pins is the pump's own half, which is emitting the right
    /// turn-end signal.
    fn cfg_with_ring(
        slug: &str,
    ) -> (
        PumpConfig,
        mpsc::Receiver<crate::core::sequencer::SequencerCommand>,
    ) {
        let (tx, rx) = mpsc::channel(16);
        let cfg = PumpConfig {
            sequencer_tx: Some(tx),
            participant_id: Some(1),
            ..fast_cfg(slug)
        };
        (cfg, rx)
    }

    /// A plain end-of-turn event.
    fn turn_end() -> AgentEvent {
        AgentEvent::TurnComplete {
            stop_reason: None,
            subtype: None,
            is_error: false,
            api_error_status: None,
            context: ContextReport::none(ContextVerdict::NoWindow),
        }
    }

    /// The epoch the next `TurnComplete` on the ring channel carries.
    async fn next_epoch(
        rx: &mut mpsc::Receiver<crate::core::sequencer::SequencerCommand>,
    ) -> u64 {
        match next_wire(rx).await {
            crate::core::sequencer::SequencerCommand::TurnComplete { epoch, .. } => epoch,
            other => panic!("expected a TurnComplete, got {other:?}"),
        }
    }

    /// Pull one `TurnComplete` off the ring channel → its [`TurnEnding`].
    ///
    /// Replaces the old `next_forward`, which read the body off a
    /// `RouterCommand::Forward`. The prose is no longer on this wire at all —
    /// it is a channel ROW, so body assertions read storage (see
    /// [`turn_bodies`]) and this returns only how the turn ended.
    fn next_turn_end(
        rx: &mut mpsc::Receiver<crate::core::sequencer::SequencerCommand>,
    ) -> Option<crate::core::sequencer::TurnEnding> {
        match rx.try_recv() {
            Ok(crate::core::sequencer::SequencerCommand::TurnComplete { ending, .. }) => {
                Some(ending)
            }
            _ => None,
        }
    }

    /// The agent-authored text rows this pump persisted, in order.
    async fn turn_bodies(storage: &Storage) -> Vec<String> {
        storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.kind == MessageKind::Text.as_str())
            .map(|m| m.content)
            .collect()
    }

    /// rc3 **P7**: the pump writes a reading for EVERY completed turn, and the
    /// unusable ones are the reason it exists.
    ///
    /// The wire this pins is the one that was missing entirely: `ContextUsage`
    /// reached the UI and was never written down, so a participant that died
    /// with `Prompt is too long` on 2026-08-12 left no record of what its meter
    /// had shown. Asserting `ContextReport` parses correctly would not catch a
    /// pump that never persists it — the parse is one half of the join and this
    /// is the other.
    #[tokio::test(flavor = "current_thread")]
    async fn every_completed_turn_records_a_context_reading() {
        let (storage, state) = setup().await;
        let (cfg, _ring_rx) = cfg_with_ring("hands");
        let slug = cfg.slug.to_string();
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        // A turn the meter can show…
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: Some("success".into()),
                is_error: false,
                api_error_status: None,
                context: ContextReport {
                    model: Some("claude-opus-5".into()),
                    used_tokens: Some(620_000),
                    reported_window: Some(1_000_000),
                    verdict: ContextVerdict::Usable,
                },
            })
            .await
            .unwrap();
        // …and one it cannot, because the provider reported no window. This is
        // the state the dead participant was in, and it must leave a row.
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: Some("success".into()),
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let history = storage
            .context_readings_for_participant("s1", &slug, 10)
            .await
            .unwrap();
        assert_eq!(
            history.iter().map(|r| r.verdict.as_str()).collect::<Vec<_>>(),
            ["usable", "no_window"],
            "both turns must leave a reading — the unusable one especially"
        );
        assert_eq!(history[0].used_tokens, Some(620_000));
        assert_eq!(history[0].reported_window, Some(1_000_000));
    }

    /// rc3 **D21** — a participant that is ORIENTING opens no turn, completes
    /// no turn, and does not speak to its peers.
    ///
    /// D21 names this the hard part and *"where this will break if rushed"*, and
    /// the failure is specific: during boot no turn has been handed out, so the
    /// epoch cell still reads its initial `0` and `last_completed_epoch` is
    /// `None`. D24's straggler guard cannot see that — `Some(0) != None` — so
    /// the pump binds epoch 0 and completes with it, and the ring discards it
    /// forever. The session would then wait for a readiness signal that was
    /// silently thrown away.
    ///
    /// Three assertions, one per thing boot must change. Note what this does
    /// NOT pin: the `!cfg.is_booting()` guard on the epoch BIND is invisible
    /// here, because the completion arm is guarded separately, so no
    /// `TurnComplete` is emitted either way. That guard is pinned by
    /// `a_participant_still_booting_when_the_ring_starts_binds_the_real_epoch`
    /// below — verified by deleting it and watching only that test redden.
    #[tokio::test(flavor = "current_thread")]
    async fn a_booting_participant_reports_readiness_instead_of_completing_a_turn() {
        let (storage, state) = setup().await;
        let (mut cfg, mut ring_rx) = cfg_with_ring("hands");
        // The cell as it actually is before the ring starts: nothing handed out.
        cfg.turn_epoch = Some(Arc::new(std::sync::atomic::AtomicU64::new(0)));
        cfg.booting = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        let (boot_tx, mut boot_rx) = mpsc::channel::<i64>(4);
        cfg.boot_done = Some(boot_tx);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx.send(AgentEvent::Text("CL loaded for bot-hq".into())).await.unwrap();
        ev_tx.send(turn_end()).await.unwrap();

        // 1. Readiness reaches the host, carrying who is ready.
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), boot_rx.recv())
                .await
                .expect("the pump never reported that boot finished"),
            Some(1),
        );
        // 2. And NOT as a turn completion. An epoch-0 completion is the
        //    discarded-forever case; the ring must not see one at all.
        assert!(
            next_turn_end(&mut ring_rx).is_none(),
            "boot must not report a turn completion — epoch 0 was never issued"
        );
        // 3. What it said while orienting is a `boot` row, so `channel_page`
        //    keeps it out of every peer's backlog while the user still sees it.
        let kinds: Vec<String> = storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.content == "CL loaded for bot-hq")
            .map(|m| m.kind)
            .collect();
        assert_eq!(kinds, vec![MessageKind::Boot.as_str().to_string()]);

        drop(ev_tx);
        let _ = task.await;
    }

    /// **A pump that dies HOLDING a turn fills the halt slot.**
    ///
    /// Clearing busy on the way out was only half the unwind: the RING still had
    /// this participant as its holder, waiting on a completion from a process
    /// that no longer exists, and it steps on nothing else. What the session
    /// showed was a red health dot and an empty halt slot — idle, unflagged, and
    /// indistinguishable from a session that had simply finished. The idle nudge
    /// could not cover it either, because for most of that window the flag still
    /// read Busy.
    ///
    /// Declared under the agent's OWN slug, not "system", because only the
    /// HOLDER declaring ends the turn in flight (rc3 D35) — the same call the
    /// error-streak halt above makes.
    #[tokio::test(flavor = "current_thread")]
    async fn a_pump_that_dies_holding_a_turn_declares_the_halt() {
        let (storage, state) = setup().await;
        let (mut cfg, _ring_rx) = cfg_with_ring("hands");
        cfg.turn_epoch = Some(Arc::new(std::sync::atomic::AtomicU64::new(4)));
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        cfg.bridge = Some(Arc::clone(&bridge));
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        // One event binds the epoch — from here the pump is holding a turn.
        ev_tx.send(AgentEvent::Text("half a thought".into())).await.unwrap();
        // And the process dies with the turn still open.
        ev_tx
            .send(AgentEvent::Exited("provider limit".into()))
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let halt = storage.session_halt("s1").await.unwrap();
        assert!(
            halt.as_ref().is_some_and(|(by, reason, _)| by == "hands"
                && reason.contains("stopped mid-turn")
                && reason.contains("provider limit")),
            "a pump that died holding a turn left the slot empty: {halt:?}"
        );
    }

    /// The other half, and the one that keeps the declaration honest: a pump
    /// that ends BETWEEN turns has nothing to unwind, and a halt there would
    /// banner every ordinary shutdown — including the close path, which kills
    /// every agent on purpose.
    #[tokio::test(flavor = "current_thread")]
    async fn a_pump_that_ends_between_turns_declares_nothing() {
        let (storage, state) = setup().await;
        let (mut cfg, mut ring_rx) = cfg_with_ring("hands");
        cfg.turn_epoch = Some(Arc::new(std::sync::atomic::AtomicU64::new(4)));
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        cfg.bridge = Some(Arc::clone(&bridge));
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx.send(AgentEvent::Text("done".into())).await.unwrap();
        ev_tx.send(turn_end()).await.unwrap();
        // The completion clears the epoch, so the pump holds nothing when it
        // stops.
        let mut ended = None;
        for _ in 0..200 {
            ended = next_turn_end(&mut ring_rx);
            if ended.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(ended.is_some(), "the turn completed");
        drop(ev_tx);
        task.await.unwrap();

        assert!(
            storage.session_halt("s1").await.unwrap().is_none(),
            "a pump with no turn in flight must not declare a halt on its way out"
        );
    }

    /// The other half: with boot OVER, the pump behaves exactly as it always
    /// did. Without this the guard above could be a deletion rather than a
    /// guard, and every turn in the session would report readiness to nobody.
    #[tokio::test(flavor = "current_thread")]
    async fn a_participant_that_has_finished_booting_completes_turns_normally() {
        let (storage, state) = setup().await;
        let (mut cfg, mut ring_rx) = cfg_with_ring("hands");
        cfg.turn_epoch = Some(Arc::new(std::sync::atomic::AtomicU64::new(7)));
        // Wired, but CLEARED — the state the session is in from turn one on.
        cfg.booting = Some(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let (boot_tx, mut boot_rx) = mpsc::channel::<i64>(4);
        cfg.boot_done = Some(boot_tx);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx.send(AgentEvent::Text("working".into())).await.unwrap();
        ev_tx.send(turn_end()).await.unwrap();
        assert_eq!(
            next_epoch(&mut ring_rx).await,
            7,
            "a cleared boot flag must leave the epoch binding untouched"
        );
        assert!(
            boot_rx.try_recv().is_err(),
            "readiness is a boot-only signal; a normal turn must not send one"
        );
        assert_eq!(turn_bodies(&storage).await, vec!["working"], "and its prose is `text`");

        drop(ev_tx);
        let _ = task.await;
    }

    /// The BOOT TIMEOUT path, which is where D21's hard part actually bites.
    ///
    /// D21 §4: the ring starts *"when every participant has finished orienting —
    /// or a timeout fires, because one slow agent must not hold the session"*.
    /// So a participant CAN still be mid-boot when the ring hands out turn one,
    /// and that is the case the epoch-bind guard exists for.
    ///
    /// Without it the pump binds `turn_epoch = Some(0)` on its first boot event.
    /// `turn_epoch` is only re-read when it is `None`, so when the real turn
    /// arrives the pump is still holding 0 — and every completion from then on
    /// carries an epoch the ring never issued and discards. That is the
    /// `s-206e8921` wedge exactly, reached through boot instead of through a
    /// straggler.
    #[tokio::test(flavor = "current_thread")]
    async fn a_participant_still_booting_when_the_ring_starts_binds_the_real_epoch() {
        let (storage, state) = setup().await;
        let (mut cfg, mut ring_rx) = cfg_with_ring("hands");
        let cell = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let booting = Arc::new(std::sync::atomic::AtomicBool::new(true));
        cfg.turn_epoch = Some(Arc::clone(&cell));
        cfg.booting = Some(Arc::clone(&booting));
        let (boot_tx, _boot_rx) = mpsc::channel::<i64>(4);
        cfg.boot_done = Some(boot_tx);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        // A slow agent: it has SAID something while orienting but has not
        // finished. No `turn_end`.
        ev_tx.send(AgentEvent::Text("still reading the CL".into())).await.unwrap();

        // **Wait for that event to be PROCESSED before the ring moves**, or the
        // test proves nothing: `send` only queues, so flipping the cell first
        // would have the pump read the new epoch when it finally got here and
        // the race this exists to catch would never happen. The same trap the
        // D24 test below documents; the persisted row is the synchronisation
        // point.
        for _ in 0..200 {
            let rows = storage.messages_for_session("s1", None).await.unwrap();
            if rows.iter().any(|m| m.content == "still reading the CL") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        // The timeout fires: boot ends and the ring hands out turn one.
        booting.store(false, std::sync::atomic::Ordering::Release);
        cell.store(1, std::sync::atomic::Ordering::Release);

        ev_tx.send(AgentEvent::Text("now working".into())).await.unwrap();
        ev_tx.send(turn_end()).await.unwrap();

        assert_eq!(
            next_epoch(&mut ring_rx).await,
            1,
            "a participant caught mid-boot by the timeout must complete on the epoch \
             it was actually handed — carrying 0 here is the s-206e8921 wedge"
        );

        drop(ev_tx);
        let _ = task.await;
    }

    /// rc3 **D24** — the wedge that killed `s-206e8921`.
    ///
    /// A pump binds its turn to the epoch cell on the first event after a
    /// completion. If that event is a STRAGGLER — output from the turn that just
    /// ended, arriving before the ring has handed out another — the cell still
    /// reads the epoch just completed, and the next real turn inherits it. Every
    /// completion from then on carries a retired number, is discarded by the
    /// sequencer's guard, and the ring can never step past a participant it is
    /// waiting on. Nothing recovers it; the session stops for good.
    ///
    /// Observed live: completed at 03:56:01 carrying epoch 9, handed epoch 11 at
    /// 03:56:28, completed again at 04:01:51 **still carrying 9**.
    ///
    /// Delete the `last_completed_epoch` guard and the last assertion here reads
    /// 9 — which is the wedge, reproduced.
    #[tokio::test(flavor = "current_thread")]
    async fn a_straggler_after_a_completed_turn_does_not_bind_the_next_one() {
        let (storage, state) = setup().await;
        let (mut cfg, mut ring_rx) = cfg_with_ring("hands");
        let cell = Arc::new(std::sync::atomic::AtomicU64::new(9));
        cfg.turn_epoch = Some(Arc::clone(&cell));
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        // Turn one, on epoch 9.
        ev_tx.send(AgentEvent::Text("working".into())).await.unwrap();
        ev_tx.send(turn_end()).await.unwrap();
        assert_eq!(
            next_epoch(&mut ring_rx).await,
            9,
            "the turn in flight completes on the epoch it was handed"
        );

        // **The straggler.** One more event, before the ring hands anything out —
        // the cell has not moved. This must NOT open a turn on epoch 9.
        //
        // **And it has to be PROCESSED before the cell moves**, or the test
        // proves nothing: `send` only queues, so storing 11 first would have the
        // pump read 11 when it eventually gets here and the race would never
        // happen. An earlier draft did exactly that and passed with the guard
        // deleted. Waiting for the row the straggler persists is the barrier —
        // the pump cannot have written it without having run the binding code
        // above it.
        ev_tx.send(AgentEvent::Text("a late word".into())).await.unwrap();
        for _ in 0..200 {
            let rows = storage.messages_for_session("s1", None).await.unwrap();
            if rows.iter().any(|m| m.content.contains("a late word")) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        // Only now does the ring hand out the next turn: the cell moves to 11.
        cell.store(11, std::sync::atomic::Ordering::Release);
        ev_tx.send(AgentEvent::Text("turn two".into())).await.unwrap();
        ev_tx.send(turn_end()).await.unwrap();

        drop(ev_tx);
        task.await.unwrap();
        assert_eq!(
            next_epoch(&mut ring_rx).await,
            11,
            "the second turn completes on the epoch the RING handed it, not on the \
             one a straggler bound it to"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn errored_turn_emits_no_forward() {
        // Regression (Rain on the DeepSeek gateway, 2026-05-29): a turn that ends
        // in an API error must not bounce to the peer: the peer replies, and that
        // re-triggers the failing agent — an unbounded error-spam loop.
        //
        // **Under the ring this assertion inverts, and that is the point.** The
        // router was told "do not forward"; the sequencer must still be told the
        // turn ENDED, or the cycle freezes on this participant forever. The loop
        // is prevented by the ending being `done: false` with no prose row to
        // wake anyone with — not by withholding the completion.
        let (storage, state) = setup().await;
        let (cfg, mut ring_rx) = cfg_with_ring("eyes");
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        let err = "API Error: 400 Failed to deserialize the JSON body into the \
                   target type: messages[17].role: unknown variant `system`, \
                   expected `user` or `assistant` at line 1 column 49275";
        ev_tx.send(AgentEvent::Text(err.into())).await.unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: Some("error_during_execution".into()),
                is_error: true,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();

        drop(ev_tx);
        task.await.unwrap();
        assert!(
            next_turn_end(&mut ring_rx).is_some(),
            "an errored turn must still report its end, or the ring never steps past it"
        );
        // Persisted for UI visibility even though not forwarded.
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].content.contains("API Error"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn back_to_back_errored_turns_declare_a_visible_halt() {
        // s-f6a441ff: a 2.9 MB paste blew both participants' context windows;
        // every dealt turn ended "Prompt is too long", the ring stepped past
        // each one (a single errored turn is survivable by design — the test
        // above), and the volley ran 11 error turns across 5 minutes before
        // the text-repeat net halted the cycle SILENTLY. Two errored turns in
        // a row from one pump = this participant cannot work: the pump fills
        // the session's halt slot so the stop has a banner, not a shrug.
        let (storage, state) = setup().await;
        let (mut cfg, mut ring_rx) = cfg_with_ring("hands");
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        cfg.bridge = Some(Arc::clone(&bridge));
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        let send_error_turn = |ev_tx: mpsc::Sender<AgentEvent>| async move {
            ev_tx
                .send(AgentEvent::Text("Prompt is too long".into()))
                .await
                .unwrap();
            ev_tx
                .send(AgentEvent::TurnComplete {
                    stop_reason: None,
                    subtype: Some("error_during_execution".into()),
                    is_error: true,
                    api_error_status: None,
                    context: ContextReport::none(ContextVerdict::NoWindow),
                })
                .await
                .unwrap();
        };

        // Turn one errors. The halt write (if any) lands BEFORE the completion
        // is reported, so once the completion is visible the absence is proof.
        send_error_turn(ev_tx.clone()).await;
        let mut first = None;
        for _ in 0..200 {
            first = next_turn_end(&mut ring_rx);
            if first.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(first.is_some(), "the first errored turn reports its end");
        assert!(
            storage.session_halt("s1").await.unwrap().is_none(),
            "ONE errored turn is survivable and must not halt the session"
        );

        // Turn two errors — the streak trips and the halt slot fills.
        send_error_turn(ev_tx.clone()).await;
        let mut second = None;
        for _ in 0..200 {
            second = next_turn_end(&mut ring_rx);
            if second.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(second.is_some(), "the second errored turn still reports its end");
        let halt = storage.session_halt("s1").await.unwrap();
        assert!(
            halt.as_ref()
                .is_some_and(|(_, reason, _)| reason.contains("Prompt is too long")
                    && reason.contains("failing back-to-back")),
            "the halt slot carries the error as the visible reason: {halt:?}"
        );
        // rc3 D35 holds here too: a halt is SESSION state, never a tray row.
        let tray = storage.tray_entries_for_session("s1").await.unwrap();
        assert!(
            !tray.iter().any(|q| q.kind == "halt"),
            "the error halt writes no tray rows: {tray:?}"
        );

        drop(ev_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn text_emits_forward_only_on_turn_complete() {
        // I/P is turn-based: text does NOT emit a Forward mid-turn; the pump emits
        // exactly one Forward on TurnComplete carrying the buffered text.
        let (storage, state) = setup().await; // default phase = Investigate
        let (cfg, mut ring_rx) = cfg_with_ring("hands");
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx.send(AgentEvent::Text("hello".into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            next_turn_end(&mut ring_rx).is_none(),
            "must not emit a Forward mid-turn (before TurnComplete)"
        );

        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let ending = next_turn_end(&mut ring_rx).expect("the turn end must reach the ring");
        assert!(
            matches!(ending, crate::core::sequencer::TurnEnding::Spoke { .. }),
            "a substantive turn is not a done-vote — the cycle must continue: {ending:?}"
        );
        // The prose is a row now; `from` is implicit in the participant the
        // completion carries, so there is no author field on this wire to check.
        let bodies = turn_bodies(&storage).await;
        assert!(bodies.iter().any(|b| b.contains("hello")));
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].author, "hands");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_phase_coalesces_into_one_forward() {
        let (storage, state) = setup().await;
        state.lock().await.current_phase = IpavPhase::Apply;

        let (cfg, mut ring_rx) = cfg_with_ring("hands");
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx.send(AgentEvent::Text("step 1".into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            next_turn_end(&mut ring_rx).is_none(),
            "no Forward mid-turn in Apply"
        );

        ev_tx.send(AgentEvent::Text("step 2".into())).await.unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: Some("end_turn".into()),
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        assert!(
            next_turn_end(&mut ring_rx).is_some(),
            "the turn end must reach the ring"
        );
        // What coalescing MEANS changed with the transport, so the assertion had
        // to move rather than be dropped. The router coalesced a turn's text into
        // one Forward so the peer was woken once; the ring gets that structurally
        // — one turn is one handover however many Text events it carried. So the
        // surviving property is that the ring stepped EXACTLY once.
        assert!(
            next_turn_end(&mut ring_rx).is_none(),
            "one turn must hand over once, however many text events it carried"
        );
        // Each Text event still gets its own row: the user reads the turn as it
        // arrives, and the peer reads the same rows off its cursor.
        let bodies = turn_bodies(&storage).await;
        assert!(bodies.iter().any(|b| b.contains("step 1")));
        assert!(bodies.iter().any(|b| b.contains("step 2")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn turn_complete_emits_forward() {
        let (storage, state) = setup().await;
        let (cfg, mut ring_rx) = cfg_with_ring("hands");
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx.send(AgentEvent::Text("quick".into())).await.unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: Some("end_turn".into()),
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        assert!(
            next_turn_end(&mut ring_rx).is_some(),
            "the turn end must reach the ring"
        );
        let bodies = turn_bodies(&storage).await;
        assert!(bodies.iter().any(|b| b.contains("quick")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_use_persists_but_emits_no_forward() {
        let (storage, state) = setup().await;
        let (cfg, mut ring_rx) = cfg_with_ring("hands");
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu1".into(),
                name: "ask_user_choice".into(),
                input: serde_json::json!({"question":"?","options":["a","b"]}),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        assert!(
            next_turn_end(&mut ring_rx).is_none(),
            "tool use alone emits no Forward"
        );
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].kind, "tool_use");
    }

    #[test]
    fn peer_ack_is_final_matrix() {
        use serde_json::json;
        // Only a peer_ack call with an explicit `final: true` counts.
        assert!(peer_ack_is_final("peer_ack", &json!({"final": true})));
        assert!(peer_ack_is_final(
            "mcp__bot-hq-signaling__peer_ack",
            &json!({"final": true})
        ));
        // Default / absent / false → the length proxy still governs.
        assert!(!peer_ack_is_final("peer_ack", &json!({})));
        assert!(!peer_ack_is_final("peer_ack", &json!({"final": false})));
        // A non-peer_ack tool can't assert finality no matter what it passes.
        assert!(!peer_ack_is_final("Bash", &json!({"final": true})));
        // Non-boolean `final` is not a truthy opt-in — don't coerce.
        assert!(!peer_ack_is_final("peer_ack", &json!({"final": "yes"})));
        assert!(!peer_ack_is_final("peer_ack", &json!({"final": 1})));
    }

    #[test]
    fn is_peer_ack_tool_matches_bare_and_prefixed() {
        // Bare alias (tests) + the real MCP-prefixed wire name both match.
        assert!(is_peer_ack_tool("peer_ack"));
        assert!(is_peer_ack_tool("mcp__bot-hq-signaling__peer_ack"));
        // Other tools + near-misses without the MCP `__` separator do NOT match.
        assert!(!is_peer_ack_tool("ask_user_choice"));
        assert!(!is_peer_ack_tool("Edit"));
        assert!(!is_peer_ack_tool("keeper_ack"));
        assert!(!is_peer_ack_tool("speer_ack"));
    }

    #[test]
    fn is_pass_turn_tool_matches_bare_and_prefixed() {
        // Bare alias (tests) + the real MCP-prefixed wire name both match.
        assert!(is_pass_turn_tool("pass_turn"));
        assert!(is_pass_turn_tool("mcp__bot-hq-signaling__pass_turn"));
        // Near-misses. `bypass_turn` is the one that matters: a `contains`
        // check would read it as a pass and silently throw the turn away.
        assert!(!is_pass_turn_tool("bypass_turn"));
        assert!(!is_pass_turn_tool("pass_turn_v2"));
        assert!(!is_pass_turn_tool("peer_ack"));
        assert!(!is_pass_turn_tool("Edit"));
    }

    /// A pass is a ROW plus a completion, and the row lands FIRST.
    ///
    /// Both halves are the slice's contract. The row is what makes the pass
    /// visible (design §1) and it carries `origin = 'participant'` (rc3
    /// decisions, locked) — asserted here through `unread_for_participant`,
    /// which is the peer's real read path, so the row is proven visible to the
    /// other participant and not merely present in a table.
    ///
    /// **The ordering is enforced by a channel with no room left**, which is
    /// what makes it an assertion rather than a hope. The sequencer channel is
    /// pre-filled to capacity, so the pump's completion send PARKS. If the row
    /// were written after the send, the pump would be parked before writing it
    /// and the poll below would time out; the row appearing while the
    /// completion is still un-enqueued is the proof it was written first.
    ///
    /// That ordering is not cosmetic: the completion is what steps the ring,
    /// and the sequencer reads the next participant's backlog straight out of
    /// storage. Written after the send, the insert races that read, and the
    /// losing side surfaces the pass a round late.
    /// **The resume chain, through the one function that joins it** (rc3 D10).
    ///
    /// An agent's claude-code conversation id used to be stored in one of two
    /// `sessions` columns picked by a `match agent { "brian" => …, "rain" => …,
    /// other => bail }`. Under role-derived slugs every write would have hit the
    /// `bail` arm and been dropped — silently, because the site only `warn`s —
    /// and every respawn would start blank with a cold cache.
    ///
    /// Pinned through the PUMP rather than on `set_participant_claude_id`,
    /// because the storage call and the `Init` handler are the two halves and it
    /// was the join that broke: the setter alone would be green with nothing
    /// calling it.
    #[tokio::test]
    async fn an_init_event_persists_the_resume_id_on_the_participants_own_row() {
        let (storage, state) = setup().await;
        storage.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
        let eyes = storage
            .participant_by_slug("s1", "eyes")
            .await
            .unwrap()
            .expect("the seeded reviewer")
            .id;
        // Not slot 0, on purpose: the old two-column writer would have put a
        // slot-1 id in the wrong column had it been keyed positionally.
        let cfg = PumpConfig { participant_id: Some(eyes), ..fast_cfg("eyes") };

        let (ev_tx, ev_rx) = mpsc::channel(4);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));
        ev_tx
            .send(AgentEvent::Init { session_id: Some("cc-uuid-42".into()) })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let roster = storage.participants_for_session("s1").await.unwrap();
        let reviewer = roster.iter().find(|p| p.id == eyes).unwrap();
        assert_eq!(
            reviewer.claude_session_id.as_deref(),
            Some("cc-uuid-42"),
            "the resume id must land on this participant's own row"
        );
        // And nobody else's — a mis-keyed write would resume the wrong
        // conversation into the wrong agent.
        assert!(
            roster.iter().filter(|p| p.id != eyes).all(|p| p.claude_session_id.is_none()),
            "the id leaked onto another participant"
        );
        // The spawn path reads exactly this to decide `--resume` vs a cold
        // start, so the round trip is what makes it load-bearing.
        assert!(
            roster.iter().any(|p| p.claude_session_id.is_some()),
            "spawn would treat this as a first spawn and lose the warm cache"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_pass_posts_its_row_before_the_completion_goes_out() {
        let (storage, state) = setup().await;
        // `create_session` seeds no roster; the participants have to exist for
        // the row to resolve to one.
        storage.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
        let hands = storage
            .participant_by_slug("s1", "hands")
            .await
            .unwrap()
            .expect("ensure_session_roster seeds brian")
            .id;
        let eyes = storage
            .participant_by_slug("s1", "eyes")
            .await
            .unwrap()
            .expect("ensure_session_roster seeds rain")
            .id;

        let (seq_tx, mut seq_rx) = mpsc::channel(1);
        // The one slot, spent. Anything the pump sends now has to wait.
        seq_tx
            .send(crate::core::sequencer::SequencerCommand::UserMessage { mentions: Vec::new() })
            .await
            .unwrap();
        let cfg = PumpConfig {
            participant_id: Some(hands),
            sequencer_tx: Some(seq_tx),
            ..fast_cfg("hands")
        };
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_pass".into(),
                name: "mcp__bot-hq-signaling__pass_turn".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();

        // Wait for the row WHILE the completion cannot yet be delivered.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let row = loop {
            let unread = storage.unread_for_participant(eyes).await.unwrap();
            if let Some(r) = unread.rows.iter().find(|r| r.content.contains("passed")) {
                break r.clone();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no pass row within 2s — the pump is parked on the full sequencer \
                 channel, which means the completion was sent before the row was written"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(row.origin, "participant", "rc3: a pass row is the participant's");
        assert_eq!(row.participant_id, Some(hands), "and attributed to the passer");
        assert_eq!(row.kind, "text");

        // Now let the completion through and read what it says.
        assert!(matches!(
            next_wire(&mut seq_rx).await,
            crate::core::sequencer::SequencerCommand::UserMessage { .. }
        ));
        match next_wire(&mut seq_rx).await {
            crate::core::sequencer::SequencerCommand::TurnComplete {
                participant_id,
                ending,
                ..
            } => {
                assert_eq!(participant_id, hands);
                assert_eq!(
                    ending,
                    crate::core::sequencer::TurnEnding::Passed,
                    "the tool has to reach the ring as a PASS, not a done vote"
                );
            }
            other => panic!("expected a TurnComplete, got {other:?}"),
        }

        drop(ev_tx);
        task.await.unwrap();
    }

    /// A turn that calls `pass_turn` and then writes a substantive review is
    /// NOT a pass: the text wins, the ring is told `Spoke`, and no pass row is
    /// posted claiming the participant had nothing to add.
    ///
    /// The row half matters as much as the ending. `Passed` is the one ending
    /// that leaves other participants' done votes standing, so a review read as
    /// a pass would both mislabel the transcript and carry a stale tally over
    /// the top of real output.
    #[tokio::test(flavor = "current_thread")]
    async fn a_substantive_turn_overrides_its_own_pass() {
        let (storage, state) = setup().await;
        storage.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
        let hands = storage
            .participant_by_slug("s1", "hands")
            .await
            .unwrap()
            .unwrap()
            .id;

        let (seq_tx, mut seq_rx) = mpsc::channel(8);
        let cfg = PumpConfig {
            participant_id: Some(hands),
            sequencer_tx: Some(seq_tx),
            ..fast_cfg("hands")
        };
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        let review = format!("BLOCKING: {}", "the retry loop never backs off. ".repeat(12));
        assert!(review.len() > 200, "the body has to clear the content-free floor");
        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_pass".into(),
                name: "pass_turn".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();
        ev_tx.send(AgentEvent::Text(review.clone())).await.unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        match next_wire(&mut seq_rx).await {
            crate::core::sequencer::SequencerCommand::TurnComplete { ending, .. } => assert_eq!(
                ending,
                crate::core::sequencer::TurnEnding::SPOKE,
                "a turn carrying a review is substantive output, pass or no pass"
            ),
            other => panic!("expected a TurnComplete, got {other:?}"),
        }
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert!(
            msgs.iter().any(|m| m.content.contains("BLOCKING")),
            "the review itself is persisted"
        );
        assert!(
            !msgs.iter().any(|m| m.content.contains("nothing to add")),
            "an overridden pass must post NO row — it would contradict the review \
             sitting next to it"
        );
    }

    /// The pass flag is per-turn. Turn 1 passes, turn 2 says something — and
    /// turn 2 must not inherit the pass.
    ///
    /// A leaked flag is not a cosmetic bug: `Passed` leaves the tally standing,
    /// so a substantive turn wearing a stale pass would carry votes cast before
    /// it into the next consensus check.
    #[tokio::test(flavor = "current_thread")]
    async fn the_pass_flag_does_not_leak_into_the_next_turn() {
        let (storage, state) = setup().await;
        storage.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
        let hands = storage
            .participant_by_slug("s1", "hands")
            .await
            .unwrap()
            .unwrap()
            .id;

        let (seq_tx, mut seq_rx) = mpsc::channel(8);
        let cfg = PumpConfig {
            participant_id: Some(hands),
            sequencer_tx: Some(seq_tx),
            ..fast_cfg("hands")
        };
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        let complete = || AgentEvent::TurnComplete {
            stop_reason: None,
            subtype: None,
            is_error: false,
            api_error_status: None,
            context: ContextReport::none(ContextVerdict::NoWindow),
        };
        // Turn 1: a bare pass.
        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_pass".into(),
                name: "pass_turn".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();
        ev_tx.send(complete()).await.unwrap();
        // Turn 2: short prose, no pass. Short on purpose — it is under the
        // content-free floor, so ONLY a leaked flag could make it a pass.
        ev_tx.send(AgentEvent::Text("on it".into())).await.unwrap();
        ev_tx.send(complete()).await.unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let endings: Vec<_> = std::iter::from_fn(|| match seq_rx.try_recv() {
            Ok(crate::core::sequencer::SequencerCommand::TurnComplete { ending, .. }) => {
                Some(ending)
            }
            _ => None,
        })
        .collect();
        assert_eq!(
            endings,
            vec![
                crate::core::sequencer::TurnEnding::Passed,
                crate::core::sequencer::TurnEnding::SPOKE
            ],
            "turn 2 called nothing, so it is ordinary output"
        );
        let passes = storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.content.contains("nothing to add"))
            .count();
        assert_eq!(passes, 1, "exactly one pass row, from turn 1");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn peer_ack_sets_flag_in_forward() {
        // peer_ack is PASSED THROUGH to the router (which suppresses the wake): the
        // pump emits a Forward with peer_ack=true. The text is still persisted.
        let (storage, state) = setup().await;
        let (cfg, mut ring_rx) = cfg_with_ring("hands");
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::Text("Agreed — nothing to add.".into()))
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_ack".into(),
                // The real wire name is MCP-prefixed.
                name: "mcp__bot-hq-signaling__peer_ack".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let ending = next_turn_end(&mut ring_rx).expect("turn end emitted");
        assert!(
            matches!(ending, crate::core::sequencer::TurnEnding::Done),
            "a content-free peer_ack turn must end the turn as a done-vote: {ending:?}"
        );
        // The agent's text is still persisted for the user.
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert!(
            msgs.iter()
                .any(|m| m.content.contains("Agreed — nothing to add.")),
            "peer_ack must still persist the agent's text"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn peer_ack_flag_is_per_turn() {
        // The peer_ack flag applies only to the turn it was called in: turn 1's
        // Forward carries peer_ack=true, turn 2's (no ack) carries peer_ack=false.
        let (storage, state) = setup().await;
        let (cfg, mut ring_rx) = cfg_with_ring("hands");
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        // Turn 1: peer_ack.
        ev_tx.send(AgentEvent::Text("acked".into())).await.unwrap();
        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_ack".into(),
                name: "peer_ack".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();

        // Turn 2: no peer_ack.
        ev_tx
            .send(AgentEvent::Text("real follow-up".into()))
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        // Turn 1 acked → a done-vote. Turn 2 did NOT ack, so the flag must have
        // been reset between turns; if it leaked, turn 2 would vote done too.
        let t1 = next_turn_end(&mut ring_rx).expect("turn 1 end");
        assert!(
            matches!(t1, crate::core::sequencer::TurnEnding::Done),
            "an acked turn votes done: {t1:?}"
        );
        let t2 = next_turn_end(&mut ring_rx).expect("turn 2 end");
        assert!(
            matches!(t2, crate::core::sequencer::TurnEnding::Spoke { .. }),
            "peer_ack must not leak into the next turn: {t2:?}"
        );
        let bodies = turn_bodies(&storage).await;
        assert!(bodies.iter().any(|b| b.contains("acked")));
        assert!(bodies.iter().any(|b| b.contains("real follow-up")));
    }

    #[test]
    fn is_atomic_command_matrix() {
        use serde_json::json;
        // Bash + atomic git ops / migrations → true.
        assert!(is_atomic_command("Bash", &json!({"command": "git commit -m x"})));
        assert!(is_atomic_command(
            "Bash",
            &json!({"command": "git push origin main"})
        ));
        assert!(is_atomic_command(
            "Bash",
            &json!({"command": "cd repo && git commit -F /tmp/m"})
        ));
        assert!(is_atomic_command("Bash", &json!({"command": "sqlx migrate run"})));
        assert!(is_atomic_command(
            "Bash",
            &json!({"command": "php artisan migrate"})
        ));
        // action_gate: the real wire name is MCP-prefixed; a bare alias also matches.
        assert!(is_atomic_command(
            "mcp__bot-hq-signaling__action_gate",
            &json!({"command": "git push"})
        ));
        assert!(is_atomic_command(
            "action_gate",
            &json!({"command": "git commit -m x"})
        ));
        // Non-atomic commands on a command surface → false.
        assert!(!is_atomic_command("Bash", &json!({"command": "git status"})));
        assert!(!is_atomic_command("Bash", &json!({"command": "ls -la"})));
        assert!(!is_atomic_command(
            "mcp__bot-hq-signaling__action_gate",
            &json!({"command": "git diff"})
        ));
        // Non-command tool surfaces → false even with a command-ish field.
        assert!(!is_atomic_command("Edit", &json!({"command": "git commit"})));
        assert!(!is_atomic_command("Read", &json!({})));
        // Missing / null command → false (no panic).
        assert!(!is_atomic_command("Bash", &json!({})));
        assert!(!is_atomic_command("Bash", &json!({"command": null})));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn atomic_tool_sets_and_clears_flag() {
        // An atomic ToolUse sets the shared flag; a NON-matching ToolResult does
        // NOT clear it (parallel-call safety); the id-matching ToolResult clears.
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let flag = Arc::new(AtomicBool::new(false));
        let cfg = PumpConfig {
            in_atomic_tool: Some(Arc::clone(&flag)),
            ..fast_cfg("hands")
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_commit".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command": "git commit -m x"}),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(flag.load(Ordering::Acquire), "atomic ToolUse sets the flag");

        ev_tx
            .send(AgentEvent::ToolResult {
                tool_use_id: "tu_other".into(),
                content: "ok".into(),
                is_error: false,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            flag.load(Ordering::Acquire),
            "a non-matching ToolResult must NOT clear the flag"
        );

        ev_tx
            .send(AgentEvent::ToolResult {
                tool_use_id: "tu_commit".into(),
                content: "ok".into(),
                is_error: false,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !flag.load(Ordering::Acquire),
            "the id-matching ToolResult clears the flag"
        );

        drop(ev_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn turn_complete_safety_clears_atomic_flag() {
        // A turn that ends with an atomic op still "in flight" (no ToolResult)
        // must not strand the flag — TurnComplete safety-clears it.
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let flag = Arc::new(AtomicBool::new(false));
        let cfg = PumpConfig {
            in_atomic_tool: Some(Arc::clone(&flag)),
            ..fast_cfg("hands")
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_push".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command": "git push"}),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(flag.load(Ordering::Acquire));

        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !flag.load(Ordering::Acquire),
            "TurnComplete safety-clears a stranded atomic flag"
        );

        drop(ev_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edit_during_investigate_self_nudges_the_editor() {
        // A3a: an executor editing in Investigate gets a one-time reminder
        // pointing it at Apply.
        //
        // **The reminder is a ROW now, not a stdin write.** It used to go into
        // this pump's own `self_input_tx` while the agent was mid-edit — which
        // it cannot read mid-generation anyway: the write opened a fresh
        // generation the ring never dealt, whose completion carried a stale
        // epoch and was discarded, and the same row then arrived a second time
        // off the cursor. Persisted, it reaches the agent at its next dealt
        // turn, which is the first moment it can act on it (advance the phase,
        // or say why the edit was intended).
        let (storage, state) = setup().await; // default phase = Investigate
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (self_tx, mut self_rx) = mpsc::channel(8);
        let cfg = PumpConfig {
            self_input_tx: Some(crate::agents::ParticipantInput::new("s1", self_tx)),
            ..fast_cfg("hands")
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu1".into(),
                name: "Edit".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let nudges: Vec<String> = storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .filter(|c| c.contains("editing files before the Apply phase"))
            .collect();
        assert_eq!(nudges.len(), 1, "the reminder is persisted once: {nudges:?}");
        assert!(nudges[0].contains("Apply"));
        assert!(
            self_rx.try_recv().is_err(),
            "the reminder must not open a generation outside the ring — it rides \
             the cursor like every other row"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edit_during_apply_does_not_nudge() {
        // A3a: editing in Apply is correct — no nudge.
        let (storage, state) = setup().await;
        state.lock().await.current_phase = IpavPhase::Apply;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (self_tx, mut self_rx) = mpsc::channel(8);
        let cfg = PumpConfig {
            self_input_tx: Some(crate::agents::ParticipantInput::new("s1", self_tx)),
            ..fast_cfg("hands")
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu1".into(),
                name: "Write".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(self_rx.try_recv().is_err(), "no nudge in Apply");

        drop(ev_tx);
        task.await.unwrap();
    }

    #[test]
    fn provider_limit_detection_matches_known_shapes() {
        // The two archive incidents, verbatim shapes.
        assert_eq!(
            detect_provider_limit(
                "You're out of usage credits. Run /usage-credits to keep using Fable 5."
            )
            .as_deref(),
            Some("You're out of usage credits. Run /usage-credits to keep using Fable 5.")
        );
        assert!(detect_provider_limit(
            "You've hit your session limit \u{b7} resets 7pm (Asia/Manila)"
        )
        .is_some());
        // Native-era provider bodies.
        assert!(detect_provider_limit("Error: 402 Insufficient Balance").is_some());
        // Ordinary prose must not trip it.
        assert_eq!(detect_provider_limit("the rate limiter test now passes"), None);
        assert_eq!(detect_provider_limit("credits to the reviewer for the catch"), None);
        // Analysis QUOTING a limit line inside a longer chunk must not trip it
        // (Rain's advisory b657bf79: the detector matched agent speech, so a
        // review discussing the incident would self-halt the session).
        let analysis = format!(
            "The archive study found the message \"You're out of usage credits\" \
             rendered as ordinary agent speech, so the session looked merely \
             quiet while the agent sat dead for hours. {}",
            "The fix classifies it into a health state instead. ".repeat(2)
        );
        assert!(analysis.len() > PROVIDER_LIMIT_MAX_CHUNK);
        assert_eq!(detect_provider_limit(&analysis), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_limit_turn_notifies_peer_once_and_halts() {
        // A quota death must produce: one peer notice (not one per retry), the
        // stalled health mark, and an awaiting-user halt row — instead of
        // rendering as ordinary speech in a merely-quiet session (3h13m dead in
        // the archive study).
        let (storage, state) = setup().await;
        let (mut cfg, mut ring_rx) = cfg_with_ring("hands");
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        cfg.bridge = Some(Arc::clone(&bridge));
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        for _ in 0..2 {
            ev_tx
                .send(AgentEvent::Text(
                    "You're out of usage credits. Run /usage-credits to continue.".into(),
                ))
                .await
                .unwrap();
            ev_tx
                .send(AgentEvent::TurnComplete {
                    stop_reason: None,
                    subtype: None,
                    is_error: false,
                    api_error_status: None,
                    context: ContextReport::none(ContextVerdict::NoWindow),
                })
                .await
                .unwrap();
        }
        // Assert health BEFORE dropping the channel: the pump's exit path
        // (channel closed = process death) legitimately overwrites health with
        // "dead", which in this test would mask the stalled mark. Poll until
        // the async limit handling lands.
        for _ in 0..100 {
            if bridge.current_agent_health("s1", "hands").as_deref() == Some("stalled") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            bridge.current_agent_health("s1", "hands").as_deref(),
            Some("stalled")
        );
        drop(ev_tx);
        task.await.unwrap();

        // Exactly ONE notice despite two limit turns (dedupe window). Counted on
        // ROWS now: the ring delivers the row off each peer's cursor, so the row
        // is both the record and the delivery, and a duplicate would be visible
        // to the user rather than only on a wire.
        let notices = storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.content.contains("hit a provider limit"))
            .count();
        assert_eq!(notices, 1, "one notice per incident, not per retry");
        // The turn still has to be reported, or the ring freezes on a
        // participant the provider has stopped answering for — the same property
        // the errored-turn test pins, on the other path that can strand a turn.
        assert!(
            next_turn_end(&mut ring_rx).is_some(),
            "a limit-stalled turn must still report its end to the ring"
        );
        // rc3 D35: a halt is SESSION state, not a tray row — the provider-limit
        // yield fills the session's one halt slot, and the tray stays empty.
        let halt = storage.session_halt("s1").await.unwrap();
        assert!(
            halt.as_ref()
                .is_some_and(|(_, reason, _)| reason.contains("Provider limit")),
            "the session's halt slot carries the provider-limit reason: {halt:?}"
        );
        let tray = storage.tray_entries_for_session("s1").await.unwrap();
        assert!(
            !tray.iter().any(|q| q.kind == "halt"),
            "nothing writes halt ROWS any more: {tray:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_provider_limit_notice_is_a_host_row_that_names_the_agent_and_quotes_the_line() {
        // B5 Task 2's remaining gap: this was the one `RouterCommand::Forward`
        // producer whose text existed nowhere but the wire — an inline `format!`
        // straight onto a peer's stdin. It now posts a row of its own.
        //
        // **Renamed in round 2.** This was
        // `..._is_a_row_and_the_forward_is_unchanged`, and the forward half went
        // with `core::router` in task 14 — the body says so itself, three
        // paragraphs down. The name kept promising a pairing the test no longer
        // checks, and the receiver it bound to check it sat unread (clippy:
        // unused variable). What the test actually pins is below: a host-owned
        // row, and the interpolation the peer will read.
        //
        // `_ring_rx` stays BOUND rather than dropped — dropping it closes the
        // channel under the pump's `sequencer_tx`, which is a different code
        // path from the one being tested.
        let (storage, state) = setup().await;
        let (cfg, _ring_rx) = cfg_with_ring("hands");
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx
            .send(AgentEvent::Text("Error: 402 Insufficient Balance".into()))
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let notice = storage
            .channel_after("s1", 0, 100)
            .await
            .unwrap()
            .rows
            .into_iter()
            .find(|m| m.content.contains("hit a provider limit"))
            .expect("the notice must have a row of its own");
        // Host-authored, so it is nobody's turn output — NOT attributed to the
        // agent it is about, whatever the wire's peer tag says.
        assert_eq!(notice.origin, "system");
        assert_eq!(notice.participant_id, None);
        // No envelope. The phase and banner the peer reads are read at FORWARD
        // time, which a hold can put long after this row was written, so writing
        // one here would record a wire the peer may never get.
        assert_eq!(notice.envelope, None);

        // The router copy this used to assert is gone with `core::router`
        // (task 14): the notice is a row, and the ring delivers rows off each
        // peer's cursor. What still matters — and is checked below — is that the
        // row's TEXT is the thing the peer will read.
        // So pin the interpolation itself, which is the part that can change
        // under both at once. The peer is told WHO stalled and WHAT the provider
        // said; `as_str()` quietly becoming a display name, or the quoted
        // `{line}` being dropped, would leave the equality above green while the
        // peer reads something else.
        let body = &notice.content;
        assert!(
            body.contains("hands"),
            "the notice must name the stalled agent: {body}"
        );
        assert!(
            body.contains("Error: 402 Insufficient Balance"),
            "the notice must quote the provider's line verbatim: {body}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_provider_limit_writes_its_notice_row_even_with_no_peer() {
        // The post sits INSIDE the `router_tx` guard, so a solo session still
        // records nothing here. Parity: there is no peer to notify, the notice
        // text is addressed to one ("do not take over their work"), and this
        // batch is a plumbing change — surfacing it to a solo user would be a
        // product decision, not a serialisation one.
        //
        // The bridge IS wired, so this also pins that skipping the notice does
        // not skip the halt: a solo user must still be told the session is
        // parked, which is the whole reason the peer notice is not what carries
        // that news.
        let (storage, state) = setup().await;
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        let cfg = PumpConfig {
            bridge: Some(Arc::clone(&bridge)),
            ..fast_cfg("hands") // no router_tx
        };
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx
            .send(AgentEvent::Text("Error: 402 Insufficient Balance".into()))
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
                context: ContextReport::none(ContextVerdict::NoWindow),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        // **This assertion inverted with task 14, deliberately.** The post used
        // to sit inside the `router_tx` guard, so a solo session hit a provider
        // limit and wrote NOTHING — the record was conflated with the delivery,
        // and with nobody to deliver to there was also nothing to see. That is
        // the exact defect rc3 exists to remove. The row is now written
        // unconditionally; whether anyone is there to read it is the ring's
        // question, not the recording's.
        let notice = storage
            .channel_after("s1", 0, 100)
            .await
            .unwrap()
            .rows
            .into_iter()
            .find(|m| m.content.contains("hit a provider limit"));
        let notice = notice.expect("a solo session must still record the limit it hit");
        assert_eq!(notice.origin, "system");
        assert_eq!(notice.participant_id, None);
    }
}