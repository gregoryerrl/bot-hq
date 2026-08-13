//! Pure-function JSON-RPC dispatch for our MCP-subset endpoint.
//!
//! Separated from the HTTP layer so we can unit-test method handling without
//! standing up hyper.

use crate::policy::{ViolationKind, ViolationOutcome};
use crate::signaling::bridge::{ApprovalContext, SignalingBridge};
use crate::signaling::protocol::*;
use crate::signaling::response::{internal_err_no_prefix, ok_response, result_json};
use crate::signaling::tool_args::{arg_opt_str, arg_required_str, arg_required_str_array};
use serde_json::{json, Value};
use std::sync::Arc;

/// Identity of the (session, agent) pair making the call. Comes from the
/// URL path the agent's mcp-config points at.
///
/// `capabilities` is resolved from `session_participants` by
/// [`resolve_caller_capabilities`] before dispatch, so [`call_tool`] stays a
/// pure function of its arguments — the reason this module exists apart from
/// the HTTP layer. It is the ONLY thing the tool gate consults: nothing below
/// compares an agent name.
#[derive(Debug, Clone)]
pub struct CallerIdentity {
    pub session_id: String,
    pub agent: String,
    pub capabilities: crate::agents::ResolvedCapabilities,
}

/// Read one caller's invite-time capability snapshot out of the roster.
///
/// Called once per RPC by the HTTP layer. Every failure resolves to
/// [`ResolvedCapabilities::Unreadable`], which denies every GATED tool and
/// leaves ungated ones alone — see that type's docs for why a gate degrades in
/// the opposite direction from the prompt layer.
///
/// The reasons are short fixed strings rather than formatted errors because
/// they are quoted into the refusal an agent reads; a sqlx error rendered into
/// an agent's transcript is noise it cannot act on, while "the session roster
/// could not be read" is something it can report.
pub async fn resolve_caller_capabilities(
    bridge: &SignalingBridge,
    session_id: &str,
    agent: &str,
) -> crate::agents::ResolvedCapabilities {
    use crate::agents::{CapabilitySet, ResolvedCapabilities};

    let Some(storage) = bridge.storage_handle().await else {
        return ResolvedCapabilities::Unreadable {
            reason: "bot-hq's storage is not wired up yet",
        };
    };
    let row = match storage.participant_by_slug(session_id, agent).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            tracing::warn!(
                %session_id,
                %agent,
                "no participant row for this caller; every gated tool is refused"
            );
            return ResolvedCapabilities::Unreadable {
                reason: "you are not on this session's roster",
            };
        }
        Err(e) => {
            tracing::warn!(%session_id, %agent, ?e, "reading the caller's participant row failed");
            return ResolvedCapabilities::Unreadable {
                reason: "the session roster could not be read",
            };
        }
    };
    match CapabilitySet::from_json(&row.capabilities) {
        Some(set) => ResolvedCapabilities::Known(set),
        None => {
            tracing::warn!(
                %session_id,
                %agent,
                capabilities = %row.capabilities,
                "capabilities column is not a JSON array of slugs; every gated tool is refused"
            );
            ResolvedCapabilities::Unreadable {
                reason: "your capability set did not decode",
            }
        }
    }
}

/// Dispatch one JSON-RPC request. Returns the response value (which the HTTP
/// layer wraps in `JsonRpcResponse::ok` / `err`).
///
/// Notifications (no id) return `Ok(None)` — caller writes a 202 with no body.
pub async fn dispatch(
    req: JsonRpcRequest,
    caller: &CallerIdentity,
    bridge: &Arc<SignalingBridge>,
) -> Result<Option<JsonRpcResponse>, JsonRpcError> {
    let id = match req.id.clone() {
        Some(v) => v,
        None => {
            // notification — execute (if relevant) and drop the response.
            return Ok(None);
        }
    };

    match req.method.as_str() {
        "initialize" => Ok(Some(JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "serverInfo": {
                    "name": "bot-hq-signaling",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "tools": { "listChanged": false }
                }
            }),
        ))),
        "ping" => Ok(Some(JsonRpcResponse::ok(id, json!({})))),
        "tools/list" => {
            let tools = tool_descriptors();
            Ok(Some(JsonRpcResponse::ok(id, json!({ "tools": tools }))))
        }
        "tools/call" => {
            let params = req
                .params
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing params"))?;
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing tool name")
                })?
                .to_string();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            let result = call_tool(&name, args, caller, bridge).await?;
            Ok(Some(JsonRpcResponse::ok(
                id,
                serde_json::to_value(result).unwrap_or(json!(null)),
            )))
        }
        _ => Err(JsonRpcError::new(
            JsonRpcError::METHOD_NOT_FOUND,
            format!("unknown method {}", req.method),
        )),
    }
}

/// Tools whose decision the capability gate deliberately does NOT take yet.
///
/// rc3 is a reframe: the SOURCE of the gate moves from an agent name to a
/// capability set, and the decision itself must not move
/// (`docs/plans/2026-08-12-rc3-reframe-contract.md`, rule 1). Exactly one tool
/// would decide differently under the capability model than under the name gate
/// it replaces:
///
/// **It is EMPTY as of rc3 D16**, and that is the end state this list was
/// written for rather than a gap in it.
///
/// `close_session` was its one entry. The reframe shipped with the pre-rc3
/// answer held — any agent could close, EYES included — because routing it
/// through capabilities would newly REFUSE it for EYES, and a behaviour change
/// is not a reframe. The user has since taken that change on its own merits:
/// *"close session tick on role capabilities. if no agents are ticked, then user
/// must be the one to manually click the close button if they want to close."*
///
/// So `close_session` now gates on `Capability::CloseSession` like every other
/// tool, read from the participant's invite-time snapshot of its role's ticks.
/// Two consequences were decided rather than discovered:
///
///   * **a roster where nobody holds it is LEGAL**, not an error — it means the
///     session ends when the user says so. The UI Close button
///     (`tauri_cmd::sessions::close_session`) calls `CoreAppState::close_session`
///     directly and has never touched this gate, which is what makes that
///     configuration usable rather than a session nobody can end;
///   * **the seeded `eyes` role does not hold it**, so a HANDS + EYES session
///     behaves as pre-rc3 did, and a session of EYES alone can no longer close
///     itself. That is the intended change and the reason it was held for a
///     decision (CL issues #5: a reviewer closed a session with unwritten CL
///     learnings still pending).
///
/// Adding an entry here reopens a gap, so
/// `parity::the_parity_hold_is_exactly_the_known_divergence` asserts this list
/// is empty. The enforcement table in `agents::capability_prompt`'s module doc
/// records that a held tool is not enforced.
const PARITY_HOLD: &[&str] = &[];

/// Is `tool`'s allow/deny decision routed through the caller's capability set?
///
/// False for a tool on [`PARITY_HOLD`], which keeps the pre-rc3 answer.
///
/// `pub(crate)` because it is also the answer to "may this tool's DESCRIPTION
/// say it needs a capability": `protocol`'s gate-line sweep asks it, so a held
/// tool cannot advertise an enforcement it does not have, and un-holding one
/// makes the sweep demand the line.
pub(crate) fn capability_gated(tool: &str) -> bool {
    !PARITY_HOLD.contains(&tool) && crate::agents::capability::required_for(tool).is_some()
}

/// Parse + validate the optional `phase` arg shared by session_doc_write and
/// session_doc_search. Returns Ok(None) when absent; Err with INVALID_PARAMS
/// when present but unparseable. Routed through `IpavPhase::parse` (the single
/// source of truth, shared with `advance_phase`) and normalized to the canonical
/// lowercase `tag()` — so the same phase string can't be valid for one phase
/// tool and rejected by another (the old `VALID_PHASES` drift), and any accepted
/// casing/chip stores as a consistent tag the IPAV tabs can match.
fn parse_optional_phase(args: &Value) -> Result<Option<String>, JsonRpcError> {
    let raw = args.get("phase").and_then(Value::as_str);
    match raw {
        None => Ok(None),
        Some(p) => match crate::core::ipav::IpavPhase::parse(p) {
            Some(phase) => Ok(Some(phase.tag().to_string())),
            None => Err(JsonRpcError::new(
                JsonRpcError::INVALID_PARAMS,
                format!(
                    "phase must be one of {}, got {:?}",
                    crate::core::ipav::IpavPhase::error_hint(),
                    p
                ),
            )),
        },
    }
}

/// Append a warning when the agent yields on top of a halt the user has not
/// answered yet. `prior` is the earlier halt's prompt.
///
/// Warn, never refuse. The bridge cannot tell "I made real progress and am
/// yielding again" from "I am restating the same state", and refusing the
/// second case would strand an agent with no way to hand control back. Putting
/// the discipline in the ack keeps the escape hatch open while making the
/// treadmill visible at the moment it happens.
fn with_repeat_halt_note(base: &str, prior: Option<&str>) -> String {
    let Some(prior) = prior else {
        return base.to_string();
    };
    let mut quoted = prior.replace('\n', " ");
    if quoted.chars().count() > 120 {
        quoted = quoted.chars().take(117).collect::<String>() + "...";
    }
    format!(
        "{base}\n\nNOTE — you already had an unanswered halt parked here: \"{quoted}\". \
         The user has not replied since, so this second yield parks another row \
         without moving anything. A halt blocks the session as hard as a question and \
         is governed by the same test: if anything in your queue is still workable, \
         work it instead of yielding; if you are genuinely blocked, stay silent and \
         wait rather than re-announcing the same state."
    )
}

/// Case-insensitive word-boundary scan for a peer/agent name in an
/// awaiting-user reason. Returns the offending word for the error message.
fn peer_shaped_reason(reason: &str) -> Option<&'static str> {
    // Heuristic VOCABULARY, not an identity check — nothing is keyed on a
    // participant being called any of these. `eyes` / `hands` are the role slugs
    // an agent writes today; the two person names stay because a user may still
    // name a MODEL that way (rc3 D10 leaves display names theirs), and a reason
    // that mentions one is peer-shaped either way. Word-boundary matched below,
    // so `constraint` does not trip `rain`.
    const PEER_WORDS: &[&str] = &["rain", "brian", "peer", "eyes", "hands"];
    let lower = reason.to_lowercase();
    PEER_WORDS.iter().copied().find(|w| {
        lower.match_indices(w).any(|(i, _)| {
            let before_ok = i == 0
                || !lower[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric());
            let after = i + w.len();
            let after_ok = after >= lower.len()
                || !lower[after..].chars().next().is_some_and(|c| c.is_alphanumeric());
            before_ok && after_ok
        })
    })
}

/// The refusal a gated tool returns when the caller's set does not admit it.
///
/// Built from `capability_prompt::phrasing(cap).deny` — the SAME sentence the
/// agent's prompt already listed under "You may not". That is not decoration:
/// layer 2 tells the agent it and the gate are never describing different
/// grants, and reusing one string for both is part of how that stops being a
/// claim. It also carries the "instead" clause each denial already has, so the
/// refusal keeps the actionable half of the three name-based messages it
/// replaces — a bare "denied" would have lost it.
fn gate_refusal(name: &str, caller: &CallerIdentity) -> String {
    let Some(cap) = crate::agents::capability::required_for(name) else {
        // Unreachable via `call_tool` (the gate runs only when `capability_gated`
        // is true, which implies a mapping). Kept total rather than panicking.
        return format!("tool '{name}' is not available to you");
    };
    match caller.capabilities.unreadable_reason() {
        Some(reason) => format!(
            "tool '{name}' needs the `{}` capability, and bot-hq could not read your \
             capability set — {reason}. Every gated tool is refused until that is fixed; \
             report this rather than working around it.",
            cap.slug()
        ),
        None => format!(
            "tool '{name}' needs the `{}` capability, which this session did not grant you. \
             You may not {}.",
            cap.slug(),
            crate::agents::capability_prompt::phrasing(cap).deny
        ),
    }
}

/// The row a refused tool call leaves behind (rc3 **P2**).
///
/// Pure, so the sentence is assertable without a database, and one line — the
/// `system_notice` lane's sizing, the same the capped halt (D7) accepted.
///
/// It names the three things a reader needs and nothing else: WHO called, WHAT
/// they called, and WHICH capability was missing. The participant is named by
/// the display rule (`ROLE · Model`), never by the slug, which is an internal
/// key.
fn refusal_notice(who: &str, tool: &str, cap_slug: &str, unreadable: Option<&str>) -> String {
    match unreadable {
        Some(reason) => format!(
            "[System: {who} called `{tool}`, which needs the `{cap_slug}` capability, and its \
             capability set could not be read — {reason}. The call was refused; every gated \
             tool stays refused until that is fixed.]"
        ),
        None => format!(
            "[System: {who} called `{tool}`, which needs the `{cap_slug}` capability this \
             session did not grant it. The call was refused and nothing ran.]"
        ),
    }
}

/// Refuse a gated tool call **and** record it in the session channel (rc3 P2).
///
/// **One function for both halves, and that is the point.** Before this, a
/// refusal was told to the caller and to nobody else, so a gate that was
/// silently OPEN and a gate that was simply never exercised looked identical —
/// capability enforcement was decorative for weeks and no session would have
/// shown it. Posting the row from a second call at the gate would be one
/// deletable line; producing the refusal and the record together means any path
/// that refuses a gated tool leaves a record by construction.
///
/// **It records, it does not block.** No halt, no awaiting flag, no gate: the
/// caller gets exactly the refusal it got before, and the row is a record. A
/// failed write is warned about and swallowed — losing the account of a refusal
/// must not also change what the agent is told.
async fn refuse_gated_tool(
    name: &str,
    caller: &CallerIdentity,
    bridge: &SignalingBridge,
) -> ToolCallResult {
    let refusal = gate_refusal(name, caller);
    // Only a mapped tool can reach the gate (`capability_gated` implies a
    // mapping), so this is the same `None` arm `gate_refusal` calls unreachable.
    if let Some(cap) = crate::agents::capability::required_for(name) {
        if let Some(storage) = bridge.storage_handle().await {
            // The display rule, not the slug: `ROLE · Model`, resolved live.
            // A caller with no roster row at all — one of the ways the set goes
            // unreadable — has no name to resolve, and the rule's own last
            // resort is the slug.
            let who = match storage
                .participant_by_slug(&caller.session_id, &caller.agent)
                .await
            {
                Ok(Some(p)) => storage.display_name_of(&p).await,
                _ => caller.agent.clone(),
            };
            let body = refusal_notice(
                &who,
                name,
                cap.slug(),
                caller.capabilities.unreadable_reason(),
            );
            match storage
                .post_to_channel(
                    caller.session_id.as_str(),
                    // Host-authored, so `origin = 'system'` with a NULL
                    // participant (0044), exactly as the capped halt posts —
                    // the refusal is the host's account, not the caller's turn
                    // output.
                    "system",
                    None,
                    crate::storage::MessageKind::SystemNotice.as_str(),
                    body,
                    None,
                )
                .await
            {
                Ok(row) => bridge.notify_message_persisted(
                    Arc::from(caller.session_id.as_str()),
                    row.message_id(),
                ),
                Err(e) => tracing::warn!(
                    session_id = %caller.session_id,
                    agent = %caller.agent,
                    tool = %name,
                    ?e,
                    "a capability refusal was not recorded in the channel"
                ),
            }
        } else {
            tracing::warn!(
                session_id = %caller.session_id,
                tool = %name,
                "no storage wired; a capability refusal went unrecorded"
            );
        }
    }
    ToolCallResult::error(refusal)
}

async fn call_tool(
    name: &str,
    args: Value,
    caller: &CallerIdentity,
    bridge: &Arc<SignalingBridge>,
) -> Result<ToolCallResult, JsonRpcError> {
    // Liveness ground truth: any tool call proves the agent is there. The
    // reviewer commit gate consults this to overrule a stale Stalled verdict.
    bridge.note_agent_rpc(&caller.session_id, &caller.agent);
    // THE TOOL GATE. One check, reading the caller's capability snapshot — no
    // agent name appears in it. Which capability a tool needs lives in
    // `capability::required_for`, the same map the prompt's layer 2 is generated
    // from, so the section that tells an agent what it may do and the gate that
    // enforces it are the same data.
    //
    // Ungated tools never reach the roster at all: `capability_gated` is false
    // for them, so a call that was never gated cannot be affected by a roster
    // that will not read.
    if capability_gated(name) && !caller.capabilities.allows_tool(name) {
        // Refusing and recording are one call (rc3 P2) — see
        // `refuse_gated_tool` for why they cannot be separated.
        return Ok(refuse_gated_tool(name, caller, bridge).await);
    }
    match name {
        "ask_user_choice" => {
            let question = arg_required_str(&args, "question")?;
            let options = arg_required_str_array(&args, "options")?;
            if options.is_empty() {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    "options must be a non-empty array of strings",
                ));
            }
            // ask_user_choice is non-blocking: this returns a parked ack
            // (`{"status":"parked","choice_id"}`) immediately, NOT the pick. The
            // user's choice arrives later as an out-of-band user message.
            let parked = bridge
                .ask_user_choice(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    question,
                    options,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(parked))
        }
        "mark_awaiting_user" => {
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // A peer-shaped reason is a category error that deadlocks the duo:
            // in the archive study both agents marked themselves awaiting-user
            // over work each thought was the OTHER's, and the session sat dead
            // 100 minutes until the user shouted. Waiting on a peer is not
            // waiting on the user — refuse and tell the agent what to do
            // instead. Word-boundary match so e.g. "restrained" can't trip it.
            if let Some(hit) = peer_shaped_reason(&reason) {
                return Ok(ToolCallResult::error(format!(
                    "reason names your peer ('{hit}') — mark_awaiting_user is for \
                     waiting on the USER, and parking on a peer deadlocks the duo. \
                     If the work needs your peer, message them (your turn output is \
                     forwarded automatically); if they aren't responding, do the \
                     work yourself or ask the user a concrete question via \
                     ask_user_choice."
                )));
            }
            let prior = bridge
                .mark_awaiting_user(caller.session_id.clone(), caller.agent.clone(), reason)
                .await;
            Ok(ToolCallResult::text(with_repeat_halt_note(
                "ok",
                prior.as_deref(),
            )))
        }
        "peer_ack" => {
            // The effect is realized in the duo pump: it observes THIS ToolUse
            // event and suppresses the turn's peer-forward (duo.rs::pump_agent).
            // Nothing to do bridge-side — the call just needs to succeed so the
            // agent's turn proceeds. Either agent may call it.
            Ok(ToolCallResult::text(
                "peer_ack noted — suppressed only if this turn is content-free. If \
                 the turn carries substantive text (>200 chars) it is still \
                 forwarded to your peer, tagged as an overridden ack: reviews and \
                 corrections must never be silently discarded.",
            ))
        }
        "pass_turn" => {
            // Realized in the duo pump, exactly like `peer_ack` above: the pump
            // observes THIS ToolUse and `sequencer::turn_ending` turns it into a
            // `TurnEnding::Passed` at the flush (duo.rs::pump_agent). Whether the
            // pass STANDS depends on text the agent may not have written yet, so
            // this handler does not decide that.
            //
            // Ungated: every participant that can hold a turn can decline one.
            //
            // **What it DOES decide is repetition** (rc3 D25). One turn carries
            // at most one pass; the first already recorded the whole of what a
            // pass says, so a second is incoherent rather than merely redundant.
            // Answering it with the same cheerful acknowledgment is what let a
            // participant call this 141 times in eight minutes in `s-a4e9a1b4`,
            // at one real model call each.
            let n = bridge.record_pass(&caller.session_id, &caller.agent);
            if n > 1 {
                tracing::warn!(
                    session_id = %caller.session_id,
                    agent = %caller.agent,
                    passes = n,
                    "pass_turn called again in a turn that already passed; refusing"
                );
                return Ok(ToolCallResult::error(format!(
                    "your pass is ALREADY recorded for this turn — this is call {n}. \
                     Calling it again cannot change anything, and repeating it burns a \
                     model call per attempt. STOP CALLING TOOLS AND END YOUR TURN: \
                     write nothing further and let the turn close. The ring hands you \
                     the next one when it comes round. If you believe you are stuck in \
                     a loop, you are — end the turn."
                )));
            }
            Ok(ToolCallResult::text(
                "pass noted — your turn is recorded as a pass and moves on. It counts \
                 toward nothing: a session settles when its participants say they are \
                 FINISHED, and a pass is not that. If this turn also carries substantive \
                 text, the text wins and the pass is ignored.",
            ))
        }
        "halt" => {
            // Yield to the user: reuse mark_awaiting_user's machinery (set the
            // awaiting flag + Halt tray row + AwaitingUser event). `awaiting`
            // outranks `busy` in SessionActivity::derive, so the input unlocks
            // immediately — no busy-flag poking needed. HANDS-only (gated above).
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Agent yielded — your move.")
                .to_string();
            let prior = bridge
                .mark_awaiting_user(caller.session_id.clone(), caller.agent.clone(), reason)
                .await;
            Ok(ToolCallResult::text(with_repeat_halt_note(
                "halted — yielded to the user; input unlocked.",
                prior.as_deref(),
            )))
        }
        "declare_working" => {
            let reason = arg_required_str(&args, "reason")?;
            // Upper bound is 3600, not 1800: a HANDS agent orchestrating
            // subagents cannot re-declare mid-wait, because nothing wakes it
            // until the subagent returns. Measured on 2026-08-07 during the B5
            // batch — a single implementer ran 2652s, so the old 1800s ceiling
            // guaranteed the declaration expired before the work finished, and
            // the watchdog nudged into a genuinely-working session. One such
            // nudge killed a subagent mid-task.
            //
            // The TTL still exists and still matters: a dead background task
            // surfaces as the nudge within one poll of expiry. This widens the
            // ceiling to cover observed durations with headroom; it does not
            // make a declaration permanent.
            let secs = args
                .get("expected_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(600.0)
                .clamp(30.0, 3600.0);
            let ttl = std::time::Duration::from_secs_f64(secs);
            match bridge
                .declare_working(&caller.session_id, &reason, ttl)
                .await
            {
                Some(applied) => Ok(ToolCallResult::text(format!(
                    "working declared for {}s — the idle watchdog holds and the WORKING \
                     badge shows your reason. It EXPIRES: re-declare on each wake while \
                     the background work continues, or finish with a proper park/halt/\
                     close-ask. Cleared automatically by the user's next message.",
                    applied.as_secs()
                ))),
                None => Ok(ToolCallResult::text(
                    "declare_working unavailable — session not registered (still \
                     spawning or already closing); proceed without it.",
                )),
            }
        }
        "advance_phase" => {
            let target = arg_required_str(&args, "target")?;
            parse_phase_arg("target", &target)?;
            bridge.agent_advance_phase(caller.session_id.clone(), caller.agent.clone(), target);
            Ok(ToolCallResult::text("phase advanced"))
        }
        "web_search" => {
            let query = arg_required_str(&args, "query")?;
            let num_results = args.get("num_results").and_then(Value::as_u64).map(|n| n as usize);
            let engine = args.get("engine").and_then(Value::as_str).map(str::to_string);
            let app = bridge
                .app_handle()
                .ok_or_else(JsonRpcError::app_handle_missing)?
                .clone();
            match crate::signaling::web_search::run_search(app, &query, num_results, engine).await {
                Ok(hits) => Ok(result_json(&hits, "[]")),
                Err(e) => Ok(ToolCallResult::error(e)),
            }
        }
        "request_phase_advance" => {
            let target = arg_required_str(&args, "target")?;
            parse_phase_arg("target", &target)?;
            let reason = arg_required_str(&args, "reason")?;
            bridge
                .request_phase_advance(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    target,
                    reason,
                )
                .await;
            Ok(ToolCallResult::text(
                "request submitted — awaiting user. They will advance the phase chip or reply.",
            ))
        }
        "file_feedback" => {
            // Deliberately NOT in HANDS_ONLY_TOOLS: filing is not a repo
            // mutation and never reaches the user mid-session, and EYES hits
            // bot-hq friction as often as HANDS does.
            let kind = arg_required_str(&args, "kind")?;
            let title = arg_required_str(&args, "title")?;
            let body = arg_required_str(&args, "body")?;
            let id = bridge
                .file_feedback(&caller.session_id, &caller.agent, &kind, &title, &body)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(format!(
                "filed as feedback #{id} ({kind}). It's queued for a bot-hq session to work — \
                 nothing further is needed from you, and the user was not interrupted."
            )))
        }
        "request_approval" => {
            let kind_str = args
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing kind"))?;
            let kind = parse_violation_kind(kind_str).ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    format!("unknown kind '{kind_str}'"),
                )
            })?;
            let action = arg_required_str(&args, "action")?;
            let question = arg_required_str(&args, "question")?;
            let options = arg_required_str_array(&args, "options")?;
            if options.len() < 2 {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    "options must have at least 2 entries",
                ));
            }
            let detail = arg_opt_str(&args, "detail");
            let ctx = ApprovalContext {
                kind,
                action,
                detail,
            };
            // PARKED, not blocking: the blocking twin is reserved for the
            // pre-push hook, which needs a synchronous bool for its exit code.
            // An agent that blocks here times out at ~60s mid-decision and
            // can't tell queued from failed.
            let parked = bridge
                .request_approval_parked(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    question,
                    options,
                    ctx,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(parked))
        }
        "gate_status" => {
            let gate_id = arg_required_str(&args, "gate_id")?;
            let msg = bridge
                .gate_status(&gate_id)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(msg))
        }
        "action_gate" => {
            let command = arg_required_str(&args, "command")?;
            let output = bridge
                .action_gate(caller.session_id.clone(), caller.agent.clone(), command)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(output))
        }
        "close_session" => {
            let archive = args
                .get("archive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            // #31 close-out staleness sweep, checked BEFORE (and independently
            // of) the delta nudge: an agent that DID write the CL gets no delta
            // nudge, and that is exactly the agent whose rewrite may have left
            // other files citing a retired concept. Fires at most once and never
            // blocks — the next close_session proceeds either way.
            if let Some(report) = bridge.staleness_sweep(&caller.session_id).await {
                return Ok(ToolCallResult::text(report));
            }
            // A3b (adherence): soft-gate the FIRST close with no CL learnings
            // delta this session — nudge to persist the delta, then close on
            // the retry. The UI force-close path (tauri_cmd) is separate + ungated.
            if bridge.should_nudge_close(&caller.session_id).await {
                Ok(ToolCallResult::text(
                    "Before closing: persist this session's bounded learnings delta via \
                     cl_write_file (read the project's notes.md, append your ~5 one-liners \
                     under ## Learnings, and write the FULL updated body), so the next \
                     session doesn't re-discover what this one learned. Then call \
                     close_session again. (If there's genuinely nothing to persist, just \
                     call close_session again and it will close.)",
                ))
            } else {
                bridge.request_session_close(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    archive,
                );
                Ok(ToolCallResult::text(
                    "session close requested — your subprocess will be terminated shortly",
                ))
            }
        }
        "check_commit_message" => {
            let message = arg_required_str(&args, "message")?;
            // Audit the policy files BEFORE resolving — if the agent has
            // quietly modified policy.yaml to remove forbidden words,
            // PolicyMutation gets logged and the user sees it post-hoc.
            // v1 is audit-only; the check below still uses the new content.
            if let Err(err) = bridge
                .audit_policy_files_for_session(&caller.session_id, &caller.agent)
                .await
            {
                tracing::warn!(%err, session_id = %caller.session_id, "policy-file audit failed");
            }
            let policy = bridge
                .resolve_policy_for(&caller.session_id)
                .await
                .map_err(internal_err_no_prefix)?;
            match policy.first_forbidden_word(&message) {
                None => Ok(ToolCallResult::text("ok")),
                Some(word) => {
                    // Best-effort log: the user didn't decide anything, but
                    // bot-hq DID block (the agent will see the error and
                    // hopefully rewrite). Record as Denied so the audit
                    // trail captures the catch.
                    if let Some(log) = bridge.violations_log() {
                        if let Err(err) = log
                            .record(
                                caller.session_id.clone(),
                                caller.agent.clone(),
                                ViolationKind::CommitGrep,
                                "git commit".to_string(),
                                ViolationOutcome::Denied,
                                Some(format!("forbidden word '{word}' in proposed message")),
                            )
                            .await
                        {
                            // The block still lands (the agent sees the error
                            // either way) — but a hole in the audit trail must
                            // not be invisible.
                            tracing::warn!(%err, session_id = %caller.session_id, "violation-log write failed");
                        }
                    }
                    Ok(ToolCallResult::text(format!("forbidden_word: {word}")))
                }
            }
        }
        "eyes_flag" => {
            let severity_str = arg_required_str(&args, "severity")?;
            let severity = crate::storage::FindingSeverity::parse(&severity_str).ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    format!("unknown severity '{severity_str}' (expected 'blocking' or 'advisory')"),
                )
            })?;
            let summary = arg_required_str(&args, "summary")?;
            let code_ref = arg_opt_str(&args, "code_ref");
            let uid = bridge
                .eyes_flag(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    severity,
                    summary,
                    code_ref,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(format!("finding filed: {uid}")))
        }
        "disposition_finding" => {
            let finding_id = arg_required_str(&args, "finding_id")?;
            let status_str = arg_required_str(&args, "status")?;
            // Agent dispositions are fixed | rebutted only; `open` isn't a
            // resolution (and there is no agent-driven "stale" disposition).
            let status = crate::storage::FindingStatus::parse(&status_str)
                .filter(|s| {
                    matches!(
                        s,
                        crate::storage::FindingStatus::Fixed
                            | crate::storage::FindingStatus::Rebutted
                    )
                })
                .ok_or_else(|| {
                    JsonRpcError::new(
                        JsonRpcError::INVALID_PARAMS,
                        format!("status must be 'fixed' or 'rebutted', got '{status_str}'"),
                    )
                })?;
            let reason = arg_required_str(&args, "reason")?;
            let result = bridge
                .disposition_finding(finding_id, status, reason, caller.agent.clone())
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(result))
        }
        "check_open_findings" => {
            let result = bridge
                .check_open_findings(&caller.session_id)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(result))
        }
        "override_reviewer_block" => {
            let reason = arg_required_str(&args, "reason")?;
            let result = bridge.override_reviewer_block(&caller.session_id, &reason);
            Ok(ToolCallResult::text(result))
        }
        "approve_finding" => {
            let finding_id = arg_required_str(&args, "finding_id")?;
            let result = bridge
                .approve_finding(finding_id)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(result))
        }
        "list_my_pending_questions" => {
            let rows = bridge
                .list_questions_for_session(&caller.session_id)
                .await
                .map_err(internal_err_no_prefix)?;
            // Filter to this agent's still-pending questions and shape into
            // the documented contract.
            let mine: Vec<Value> = rows
                .iter()
                .filter(|r| r.agent == caller.agent && r.status == "pending")
                .map(|r| {
                    json!({
                        "choice_id": r.choice_id,
                        "kind": r.kind,
                        "prompt": r.prompt,
                        "options": r.options(),
                        "asked_at": r.asked_at,
                        "supersedes_id": r.supersedes_id,
                    })
                })
                .collect();
            Ok(result_json(&mine, "[]"))
        }
        "withdraw_question" => {
            let choice_id = arg_required_str(&args, "choice_id")?;
            let was_pending = bridge.withdraw_question(&choice_id).await;
            Ok(ToolCallResult::text(if was_pending {
                "withdrawn"
            } else {
                "no-op: choice_id was not pending"
            }))
        }
        "supersede_question" => {
            let stale_choice_id = arg_required_str(&args, "stale_choice_id")?;
            let question = arg_required_str(&args, "question")?;
            let options = arg_required_str_array(&args, "options")?;
            if options.is_empty() {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    "options must have at least 1 entry",
                ));
            }
            // Non-blocking, like ask_user_choice: returns a parked ack, not the
            // pick — the user's choice on the new question arrives out-of-band.
            let parked = bridge
                .supersede_question_with_new(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    stale_choice_id,
                    question,
                    options,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(parked))
        }
        "terminal_exec" => {
            let command = arg_required_str(&args, "command")?;
            let wait_ms = args.get("wait_ms").and_then(Value::as_u64);
            let block = args.get("block").and_then(Value::as_bool);
            let output = bridge
                .terminal_exec(caller.session_id.clone(), command, wait_ms, block)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(output))
        }
        "terminal_read" => {
            let lines = args.get("lines").and_then(Value::as_u64);
            let output = bridge
                .terminal_read(caller.session_id.clone(), lines)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(output))
        }
        "session_doc_write" => {
            let slug = arg_required_str(&args, "slug")?;
            let body = arg_required_str(&args, "body")?;
            let phase = parse_optional_phase(&args)?;
            // Default "replace" keeps every existing caller unchanged; an
            // unrecognised mode is refused rather than silently replacing, since
            // a caller that meant to append would otherwise destroy the doc.
            let append = match args.get("mode").and_then(Value::as_str) {
                None | Some("replace") => false,
                Some("append") => true,
                Some(other) => {
                    return Err(JsonRpcError::new(
                        JsonRpcError::INVALID_PARAMS,
                        format!("unknown mode '{other}' — expected 'replace' or 'append'"),
                    ))
                }
            };
            // A reviewer contributing to a phase doc must not overwrite the
            // executor's single per-phase doc. Route a phase-tagged REVIEWER
            // write to a co-located, attributed `<phase>-review` doc (same phase
            // tag → same IPAV tab). Untagged reviewer scratch writes fall
            // through to the normal overwrite path.
            //
            // **rc3 D10: the reviewer is whoever holds `file_finding`, not
            // whoever is called `rain`.** This arm used to read
            // `caller.agent.as_str() == "rain"`, which under role-derived slugs
            // matches no participant — so every phase-tagged write took the
            // fallback arm and clobbered the other participant's phase doc,
            // silently, while the EYES prompt and migration 0049 both kept
            // promising the co-located behaviour. Same capability the commit
            // gate's reviewer registry is built from (`core::session`), so the
            // two cannot disagree about who a reviewer is.
            match (
                caller
                    .capabilities
                    .grants(crate::agents::Capability::FileFinding),
                phase.as_deref(),
            ) {
                (true, Some(p)) => {
                    let (id, eyes_slug) = bridge
                        .session_doc_write_eyes(&caller.session_id, p, &body, &caller.agent)
                        .await
                        .map_err(internal_err_no_prefix)?;
                    Ok(ToolCallResult::text(
                        json!({"id": id, "slug": eyes_slug}).to_string(),
                    ))
                }
                _ => {
                    let id = bridge
                        .session_doc_write(
                            &caller.session_id,
                            &slug,
                            &body,
                            phase.as_deref(),
                            append,
                        )
                        .await
                        .map_err(internal_err_no_prefix)?;
                    Ok(ToolCallResult::text(
                        json!({"id": id, "slug": slug}).to_string(),
                    ))
                }
            }
        }
        "session_doc_search" => {
            let query = args.get("query").and_then(Value::as_str);
            let phase = parse_optional_phase(&args)?;
            let rows = bridge
                .session_doc_search(&caller.session_id, query, phase.as_deref())
                .await
                .map_err(internal_err_no_prefix)?;
            let trimmed: Vec<Value> = rows
                .into_iter()
                .map(|d| {
                    json!({
                        "id": d.id,
                        "slug": d.slug,
                        "body": d.body,
                        "phase": d.phase,
                        "created_at": d.created_at,
                        "updated_at": d.updated_at,
                    })
                })
                .collect();
            Ok(result_json(&trimmed, "[]"))
        }
        "session_doc_read" => {
            let slug = arg_required_str(&args, "slug")?;
            let row = bridge
                .session_doc_read(&caller.session_id, &slug)
                .await
                .map_err(internal_err_no_prefix)?;
            match row {
                Some(d) => Ok(ToolCallResult::text(
                    json!({
                        "id": d.id,
                        "slug": d.slug,
                        "body": d.body,
                        "created_at": d.created_at,
                        "updated_at": d.updated_at,
                    })
                    .to_string(),
                )),
                None => Ok(ToolCallResult::text("null".to_string())),
            }
        }
        "cl_index_search" => {
            let project = args.get("project").and_then(Value::as_str);
            let query = args.get("query").and_then(Value::as_str);
            let mut rows = bridge
                .cl_index_search_agent(project, query)
                .await
                .map_err(internal_err_no_prefix)?;
            // Project-scoped searches also list `_globals` rows (the `project`
            // field distinguishes them) — same always-reachable contract as
            // cl_retrieve. `None` already spans every project. The _agent
            // variant keeps user-hidden files out of both scopes.
            if let Some(p) = project {
                if p != crate::storage::Project::GLOBALS {
                    let globals = bridge
                        .cl_index_search_agent(Some(crate::storage::Project::GLOBALS), query)
                        .await
                        .map_err(internal_err_no_prefix)?;
                    rows.extend(globals);
                }
            }
            // Strip noisy fields; agents care about file_path, description,
            // tags, updated_at. `abs_path` is the RESOLVED on-disk location —
            // agents were joining `_globals` into the path themselves and
            // constructing `<library>/_globals/<file>`, which does not exist
            // (root-level files live directly under the library root).
            let mut roots: std::collections::HashMap<String, Option<std::path::PathBuf>> =
                std::collections::HashMap::new();
            let mut trimmed: Vec<serde_json::Value> = Vec::with_capacity(rows.len());
            for r in rows {
                if !roots.contains_key(&r.project_id) {
                    let root = bridge.cl_project_root(&r.project_id).await;
                    roots.insert(r.project_id.clone(), root);
                }
                let abs_path = roots
                    .get(&r.project_id)
                    .and_then(|root| root.as_ref())
                    .map(|root| root.join(&r.file_path).display().to_string());
                trimmed.push(serde_json::json!({
                    "project": r.project_id,
                    "file_path": r.file_path,
                    "abs_path": abs_path,
                    "description": r.description,
                    "tags": r.tags,
                    "updated_at": r.updated_at,
                }));
            }
            Ok(result_json(&trimmed, "[]"))
        }
        "cl_retrieve" => {
            let project = arg_required_str(&args, "project")?;
            let query = arg_required_str(&args, "query")?;
            let paths: Option<Vec<String>> = args
                .get("paths")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            let budget = args
                .get("budget_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(3000);
            // `_globals` always rides along (2026-08-05): cross-project files
            // like eod.md must be reachable from a project-scoped query —
            // agents were inventing their own eod.md in repos when the real
            // one couldn't rank in.
            let atoms = bridge
                .cl_retrieve(&project, &query, paths.as_deref(), budget, true)
                .await
                .map_err(internal_err_no_prefix)?;
            // Stage-4b measurement: log this retrieval (best-effort; never fails
            // the call). `caller` carries the session/agent context here.
            bridge
                .log_retrieval_event(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    &project,
                    &query,
                    &atoms,
                    budget,
                )
                .await;
            // Inline the atom bodies as readable `## file > heading` blocks — the
            // whole point is to hand the agent the CONTENT, not a TOC.
            let text = if atoms.is_empty() {
                // Failure-mode #5 (CL brief): an empty retrieval must never read
                // as "no constraints exist" — the fact may simply rank below the
                // match threshold or use different words.
                format!(
                    "(no matching CL atoms for: {query} — this does NOT mean no \
                     conventions/constraints exist; rephrase the query or check \
                     cl_index_search.)"
                )
            } else {
                let mut out = String::new();
                for atom in &atoms {
                    // Two flag flavors (issues.md #23): code-drift (repo-backed,
                    // hash mismatch) vs age (repo-less fallback) — worded
                    // differently so the reader knows whether drift was DETECTED
                    // or the claim is merely old and unverifiable.
                    let flag = if atom.stale {
                        match atom.stale_age_days {
                            Some(d) => format!(
                                "⚠ possibly stale (no repo to verify against; last updated {d}d ago) — date-check before trusting.\n"
                            ),
                            None => "⚠ possibly stale (cited code changed since indexed) — verify against the source.\n".to_string(),
                        }
                    } else {
                        String::new()
                    };
                    // Cross-scope rows announce their origin so `[_globals]
                    // eod.md` can't be mistaken for a project file (display
                    // only — nothing parses rendered headings).
                    let scope = if atom.project_id != project {
                        format!("[{}] ", atom.project_id)
                    } else {
                        String::new()
                    };
                    out.push_str(&format!(
                        "## {}{} > {}\n{}{}\n\n",
                        scope, atom.file_path, atom.heading_path, flag, atom.body
                    ));
                }
                out.trim_end().to_string()
            };
            Ok(ToolCallResult::text(text))
        }
        "cl_write_file" => {
            let project = arg_required_str(&args, "project")?;
            let file_path = arg_required_str(&args, "file_path")?;
            let content = arg_required_str(&args, "content")?;
            let append = match args.get("mode").and_then(Value::as_str) {
                None | Some("replace") => false,
                Some("append") => true,
                Some(other) => {
                    return Ok(ToolCallResult::error(format!(
                        "invalid mode '{other}' — use \"replace\" (default) or \"append\""
                    )))
                }
            };
            let confirm_shrink = args
                .get("confirm_shrink")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let msg = bridge
                .cl_write_file(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    project,
                    file_path,
                    content,
                    append,
                    confirm_shrink,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(msg))
        }
        "cl_stale_refs" => {
            // Report only (rc3 P4). Ungated like the other CL READS — it writes
            // nothing, and a maintenance session that cannot see the drift is
            // the state this exists to end.
            let project = arg_required_str(&args, "project")?;
            let report = bridge
                .cl_stale_refs(&project)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(report))
        }
        "cl_register_read" => {
            let project = arg_required_str(&args, "project")?;
            let file_path = arg_required_str(&args, "file_path")?;
            // Awaited audit insert (cheap single-row write). Unknown paths
            // no-op inside the bridge; only real DB failures surface as errors.
            bridge
                .cl_register_read(
                    &caller.agent,
                    Some(&caller.session_id),
                    &project,
                    &file_path,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text("recorded"))
        }
        "cl_folder_search" => {
            let project = args.get("project").and_then(Value::as_str);
            let query = args.get("query").and_then(Value::as_str);
            let mut rows = bridge
                .cl_folder_search(project, query)
                .await
                .map_err(internal_err_no_prefix)?;
            // Same `_globals` union as cl_index_search, folder flavor.
            if let Some(p) = project {
                if p != crate::storage::Project::GLOBALS {
                    let globals = bridge
                        .cl_folder_search(Some(crate::storage::Project::GLOBALS), query)
                        .await
                        .map_err(internal_err_no_prefix)?;
                    rows.extend(globals);
                }
            }
            let trimmed: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "project": r.project_id,
                        "folder_path": r.folder_path,
                        "description": r.description,
                        "tags": r.tags,
                        "updated_at": r.updated_at,
                    })
                })
                .collect();
            Ok(result_json(&trimmed, "[]"))
        }
        "cl_register_folder_description" => {
            let project = arg_required_str(&args, "project")?;
            let folder_path = arg_required_str(&args, "folder_path")?;
            let description = arg_required_str(&args, "description")?;
            let tags = arg_opt_str(&args, "tags");
            bridge
                .cl_register_folder_description(
                    &project,
                    &folder_path,
                    &description,
                    tags.as_deref(),
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text("ok"))
        }
        "cl_rescan" => {
            let project = arg_required_str(&args, "project")?;
            let report = bridge
                .cl_rescan(&project)
                .await
                .map_err(internal_err_no_prefix)?;
            // A3b: a cl_rescan is the proxy for "the agent touched the CL" — it
            // lifts the close-delta gate so a later close_session won't nudge.
            bridge.mark_cl_rescan(&caller.session_id).await;
            Ok(result_json(&report, "{}"))
        }
        "webview_screenshot" => {
            let handle = bridge
                .app_handle()
                .ok_or_else(JsonRpcError::app_handle_missing)?;
            let data_dir = bridge.data_dir().ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INTERNAL_ERROR,
                    "bridge data_dir not configured (test bridge?)".to_string(),
                )
            })?;
            let path = crate::tauri_cmd::screenshot::capture_main_window(handle, data_dir)
                .map_err(internal_err_no_prefix)?;
            Ok(result_json(
                &json!({ "path": path.display().to_string() }),
                "{}",
            ))
        }
        other => match super::webview_js::webview_tool_js(other, &args)? {
            Some(js) => {
                eval_in_webview(bridge, &js)?;
                Ok(ok_response())
            }
            None => Err(JsonRpcError::new(
                JsonRpcError::METHOD_NOT_FOUND,
                format!("unknown tool {other}"),
            )),
        },
    }
}

fn eval_in_webview(bridge: &Arc<SignalingBridge>, js: &str) -> Result<(), JsonRpcError> {
    use tauri::Manager;
    let handle = bridge
        .app_handle()
        .ok_or_else(JsonRpcError::app_handle_missing)?;
    let window = handle
        .get_webview_window("main")
        .ok_or_else(JsonRpcError::webview_missing)?;
    window.eval(js).map_err(internal_err_no_prefix)?;
    Ok(())
}

fn parse_violation_kind(s: &str) -> Option<ViolationKind> {
    // Parse through serde so the wire names can't drift from `ViolationKind`'s
    // own `#[serde(rename_all = "snake_case")]` derive (a hand-written match
    // had to be kept in lockstep with the enum). Unknown string → None.
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signaling::bridge::SignalingEvent;

    /// A HANDS caller, carrying the grants a real spawn resolves for one.
    ///
    /// The presets are not a stand-in for the database: `parity::
    /// the_seeded_roster_resolves_to_the_presets` reads a migrated database
    /// through `resolve_caller_capabilities` and asserts it produces exactly
    /// these, so a seed that drifted from the presets fails there rather than
    /// leaving every test in this module quietly asserting against fiction.
    fn caller() -> CallerIdentity {
        CallerIdentity {
            session_id: "s1".into(),
            agent: "brian".into(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::preset_hands(),
            ),
        }
    }

    /// An EYES caller. See [`caller`] for why the preset is trustworthy here.
    fn rain_caller() -> CallerIdentity {
        CallerIdentity {
            session_id: "s1".into(),
            agent: "rain".into(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::preset_eyes(),
            ),
        }
    }

    fn req(method: &str, params: Value, id: i64) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: method.into(),
            params: Some(params),
        }
    }

    #[tokio::test]
    async fn initialize_returns_capabilities() {
        let bridge = SignalingBridge::new();
        let res = dispatch(req("initialize", json!({}), 1), &caller(), &bridge)
            .await
            .unwrap()
            .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(v["result"]["serverInfo"]["name"], "bot-hq-signaling");
    }

    #[tokio::test]
    async fn tools_list_returns_all_tools() {
        let bridge = SignalingBridge::new();
        let res = dispatch(req("tools/list", json!({}), 1), &caller(), &bridge)
            .await
            .unwrap()
            .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let tools = v["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"ask_user_choice"));
        assert!(names.contains(&"mark_awaiting_user"));
        assert!(names.contains(&"peer_ack"));
        assert!(names.contains(&"halt"));
        assert!(names.contains(&"declare_working"));
        assert!(names.contains(&"request_approval"));
        assert!(names.contains(&"action_gate"));
        assert!(names.contains(&"check_commit_message"));
        assert!(names.contains(&"close_session"));
        assert!(names.contains(&"list_my_pending_questions"));
        assert!(names.contains(&"withdraw_question"));
        assert!(names.contains(&"cl_write_file"));
        assert!(names.contains(&"terminal_exec"));
        assert!(names.contains(&"terminal_read"));
        assert_eq!(
            tools.len(),
            names.iter().collect::<std::collections::HashSet<_>>().len(),
            "tool names should be unique"
        );
    }

    #[tokio::test]
    async fn close_session_emits_event() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "close_session", "arguments": {}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("close requested"));
        let ev = sub.recv().await.unwrap();
        match ev {
            SignalingEvent::SessionCloseRequest {
                session_id,
                agent,
                archive,
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(agent, "brian");
                assert!(!archive);
            }
            other => panic!("expected SessionCloseRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn close_session_nudges_for_cl_delta_then_closes() {
        // A3b: with storage wired (adherence on by default) and no cl_rescan this
        // session, the FIRST close_session returns a write-then-prune nudge and
        // does NOT request close; the SECOND closes.
        let bridge = SignalingBridge::new();
        bridge
            .set_storage(crate::storage::Storage::memory().await.unwrap())
            .await;
        let mut sub = bridge.subscribe();

        let first = dispatch(
            req(
                "tools/call",
                json!({"name": "close_session", "arguments": {}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&first).unwrap();
        assert!(
            v["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("learnings"),
            "first close must nudge for the learnings delta"
        );
        assert!(
            sub.try_recv().is_err(),
            "nudged close must NOT request session close"
        );

        let second = dispatch(
            req(
                "tools/call",
                json!({"name": "close_session", "arguments": {}}),
                2,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v2 = serde_json::to_value(&second).unwrap();
        assert!(v2["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("close requested"));
        match sub.recv().await.unwrap() {
            SignalingEvent::SessionCloseRequest { .. } => {}
            other => panic!("expected SessionCloseRequest on retry, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn close_session_surfaces_the_staleness_sweep_before_the_delta_nudge() {
        // #31: an agent that wrote the CL clears the delta nudge — and is exactly
        // the one whose rewrite can strand a retired concept elsewhere. The sweep
        // therefore runs first and independently; it fires once, then the close
        // proceeds (advisory, never a hold).
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("library/projects/bot-hq");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("conventions.md"), "The duo maintains this.\n").unwrap();
        std::fs::write(proj.join("vision.md"), "The duo is the core of it.\n").unwrap();
        let bridge = SignalingBridge::with_policy(
            crate::policy::ViolationsLog::new(tmp.path()),
            tmp.path().to_path_buf(),
        );
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        storage.create_session("s1", "sweep", None).await.unwrap();
        bridge.set_storage(storage).await;
        let mut sub = bridge.subscribe();
        let caller = caller();

        bridge
            .cl_write_file(
                "s1".to_string(),
                "brian".to_string(),
                "bot-hq".to_string(),
                "vision.md".to_string(),
                "The harness is the core of it, restated at a similar length.".to_string(),
                false,
                false,
            )
            .await
            .unwrap();

        let first = dispatch(
            req("tools/call", json!({"name": "close_session", "arguments": {}}), 1),
            &caller,
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let text = serde_json::to_value(&first).unwrap()["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("staleness sweep"), "got: {text}");
        assert!(text.contains("conventions.md:1"), "got: {text}");
        assert!(
            sub.try_recv().is_err(),
            "the sweep must not request the close"
        );

        let second = dispatch(
            req("tools/call", json!({"name": "close_session", "arguments": {}}), 2),
            &caller,
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        // The CL write already lifted the delta gate, so the retry closes.
        assert!(serde_json::to_value(&second).unwrap()["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("close requested"));
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let bridge = SignalingBridge::new();
        let err = dispatch(req("garbage", json!({}), 1), &caller(), &bridge)
            .await
            .unwrap_err();
        assert_eq!(err.code, JsonRpcError::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn rain_rejected_from_hands_only_tools() {
        let bridge = SignalingBridge::new();
        for tool in &[
            "mark_awaiting_user",
            "ask_user_choice",
            "request_approval",
            "action_gate",
            "halt",
            "declare_working",
            "terminal_exec",
        ] {
            let res = dispatch(
                req(
                    "tools/call",
                    json!({
                        "name": tool,
                        "arguments": {
                            "reason": "x",
                            "question": "?",
                            "options": ["a", "b"],
                            "kind": "push_gate",
                            "action": "y",
                        }
                    }),
                    1,
                ),
                &rain_caller(),
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
            let v = serde_json::to_value(&res).unwrap();
            assert_eq!(
                v["result"]["isError"],
                json!(true),
                "tool {tool} should return is_error=true for rain"
            );
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            // The refusal names the CAPABILITY it needed, not the role that has
            // it: what an agent can act on is "you were not granted this", and a
            // role name is not something it can check.
            assert!(
                text.contains("which this session did not grant you"),
                "tool {tool} should explain the missing grant, got: {text}"
            );
        }
    }

    /// rc3 **P2**: a refused tool call leaves a row, not just a return value.
    ///
    /// The defect: the gate told the caller and nobody else, so a gate that was
    /// silently OPEN and a gate that was never exercised looked identical from
    /// inside a session — capability enforcement was decorative for weeks and
    /// nothing would have shown it. Asserting the returned refusal alone
    /// reproduces exactly that blind spot, so this asserts the ROW.
    ///
    /// Driven through `dispatch`, not through `refuse_gated_tool`, so it covers
    /// the gate actually taking that path.
    #[tokio::test]
    async fn a_refused_tool_call_is_recorded_in_the_channel() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        // A real roster, so the row can name the participant by the display
        // rule rather than by its slug.
        storage.ensure_session_roster("s1", false).await.unwrap();

        let hands = CallerIdentity {
            session_id: "s1".into(),
            agent: "hands".into(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::preset_hands(),
            ),
        };
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "eyes_flag", "arguments": {"severity": "blocking", "summary": "x"}}),
                1,
            ),
            &hands,
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        // The caller is still told, unchanged — P2 adds a record, it does not
        // replace the refusal.
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));

        let rows = storage.messages_for_session("s1", None).await.unwrap();
        let notices: Vec<&crate::storage::Message> = rows
            .iter()
            .filter(|m| m.kind == crate::storage::MessageKind::SystemNotice.as_str())
            .collect();
        assert_eq!(notices.len(), 1, "a refusal should leave exactly one row");
        let body = notices[0].content.as_str();
        // WHO, WHAT, and WHICH capability — the three facts that make a wrong
        // refusal something you watch happen instead of infer.
        assert!(body.contains("HANDS"), "the row must name the participant: {body}");
        assert!(body.contains("`eyes_flag`"), "the row must name the tool: {body}");
        assert!(
            body.contains("`file_finding`"),
            "the row must name the capability it lacked: {body}"
        );
        // The participant is named by the display rule; the slug is an internal
        // key and must not be printed.
        assert!(!body.contains("`hands`"), "the row printed a slug: {body}");
        // A record, never a gate: nothing is parked on the user's tray, so the
        // session is not waiting on anyone because a tool was refused.
        assert!(
            !storage.has_pending_tray("s1").await.unwrap(),
            "a refusal must not park anything"
        );
    }

    #[tokio::test]
    async fn brian_rejected_from_eyes_only_eyes_flag() {
        // eyes_flag is the inverse gate: EYES-only, so HANDS (brian) is rejected.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "eyes_flag", "arguments": {"severity": "blocking", "summary": "x"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("needs the `file_finding` capability"), "got: {text}");
        assert!(text.contains("which this session did not grant you"), "got: {text}");
    }

    #[tokio::test]
    async fn rain_rejected_from_disposition_finding() {
        // disposition_finding joins HANDS_ONLY_TOOLS — EYES (rain) is rejected.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "disposition_finding", "arguments": {"finding_id": "f1", "status": "fixed", "reason": "x"}}),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("needs the `disposition_finding` capability"),
            "got: {text}"
        );
        assert!(text.contains("which this session did not grant you"), "got: {text}");
    }

    #[tokio::test]
    async fn disposition_finding_rejects_non_disposition_status() {
        // `stale`/`open` are not agent dispositions — only fixed|rebutted.
        let bridge = SignalingBridge::new();
        let err = dispatch(
            req(
                "tools/call",
                json!({"name": "disposition_finding", "arguments": {"finding_id": "f1", "status": "stale", "reason": "x"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(err.message.contains("fixed' or 'rebutted"), "msg: {}", err.message);
    }

    #[tokio::test]
    async fn brian_rejected_from_approve_finding() {
        // approve_finding is EYES-only — only the reviewer who raised a finding
        // can sign off its fix; HANDS can't self-approve.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "approve_finding", "arguments": {"finding_id": "f1"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("needs the `approve_finding` capability"));
    }

    #[tokio::test]
    async fn findings_gate_round_trip_via_dispatch() {
        // The full gate, end-to-end through dispatch: rain files blocking →
        // check_open_findings blocks → brian dispositions → check returns ok.
        // This is the s-3cb39c76 scenario in miniature.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();

        let filed = dispatch(
            req(
                "tools/call",
                json!({"name": "eyes_flag", "arguments": {"severity": "blocking", "summary": "NPE on null id", "code_ref": "job.rs:42"}}),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&filed).unwrap();
        assert_eq!(v["result"]["isError"], json!(false));
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let uid = text.trim_start_matches("finding filed: ").to_string();
        assert!(!uid.is_empty(), "expected a finding uid, got: {text}");

        let blocked = dispatch(
            req("tools/call", json!({"name": "check_open_findings", "arguments": {}}), 1),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&blocked).unwrap();
        assert!(
            v["result"]["content"][0]["text"].as_str().unwrap().starts_with("blocked: 1"),
            "commit-time check must block while the finding is open"
        );

        dispatch(
            req(
                "tools/call",
                json!({"name": "disposition_finding", "arguments": {"finding_id": uid, "status": "fixed", "reason": "fixed in abc123"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();

        let ok = dispatch(
            req("tools/call", json!({"name": "check_open_findings", "arguments": {}}), 1),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "ok", "gate clears after disposition");
    }

    #[tokio::test]
    async fn mark_awaiting_user_dispatch_works() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "mark_awaiting_user", "arguments": {"reason": "wait"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("ok"));
        let ev = sub.recv().await.unwrap();
        assert!(matches!(ev, SignalingEvent::AwaitingUser { reason, .. } if reason == "wait"));
    }

    #[tokio::test]
    async fn halt_dispatch_sets_awaiting() {
        // halt yields to the user: it routes through mark_awaiting_user's
        // machinery, so it emits AwaitingUser carrying the defaulted reason.
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req("tools/call", json!({"name": "halt", "arguments": {}}), 1),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("halted"));
        let ev = sub.recv().await.unwrap();
        assert!(
            matches!(ev, SignalingEvent::AwaitingUser { reason, .. } if reason.contains("yielded")),
            "halt should emit AwaitingUser with the default 'yielded' reason"
        );
    }

    /// rc3 **P4**: the staleness report is reachable as a tool, and it reports
    /// rather than edits.
    ///
    /// The wire, again: the detector is unit-tested in `cl_staleness`, and a
    /// missing `call_tool` arm would leave every one of those tests green while
    /// no session could ever run it — which is indistinguishable from the drift
    /// the item exists to end.
    #[tokio::test]
    async fn cl_stale_refs_dispatch_reports_missing_code_without_editing() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/live.rs"), "fn resolve_spawn_roster() {}").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
            vec!["add", "-A"],
            vec!["commit", "-q", "-m", "seed"],
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(&args)
                .output()
                .unwrap();
        }
        let cl = tmp.path().join("library/projects/p");
        std::fs::create_dir_all(&cl).unwrap();
        let cl_body = "The spawn calls `resolve_spawn_roster`, then `may_run_native`.\n";
        std::fs::write(cl.join("notes.md"), cl_body).unwrap();

        let bridge = SignalingBridge::new_with(None, Some(tmp.path().to_path_buf()));
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .upsert_project("p", "p", Some(repo.to_str().unwrap()), None, None)
            .await
            .unwrap();
        bridge.set_storage(storage).await;

        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_stale_refs", "arguments": {"project": "p"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("may_run_native"), "the dead symbol: {text}");
        assert!(!text.contains("resolve_spawn_roster"), "a live symbol: {text}");
        assert!(text.contains("notes.md:1"), "names file and line: {text}");
        assert!(text.contains("not a work order"), "carries the D15 caveat: {text}");
        // Report only: the CL file is byte-identical afterwards.
        assert_eq!(
            std::fs::read_to_string(cl.join("notes.md")).unwrap(),
            cl_body,
            "the report must never edit the library"
        );
    }

    #[tokio::test]
    async fn cl_retrieve_dispatch_inlines_bodies_and_handles_no_match() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .replace_atoms_for_file(
                "p",
                "notes.md",
                &[crate::storage::Atom {
                    heading_path: "Gotchas".into(),
                    body: "the migration is immutable".into(),
                    code_hash: None,
                }],
                "t",
            )
            .await
            .unwrap();
        bridge.set_storage(storage).await;

        // A real query inlines the matching atom body under a `## file > heading`.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_retrieve", "arguments": {"project": "p", "query": "migration"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("## notes.md > Gotchas"), "header present: {text}");
        assert!(text.contains("the migration is immutable"), "body inlined: {text}");

        // A term-less query returns a friendly no-match string, not an error.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_retrieve", "arguments": {"project": "p", "query": "***"}}),
                2,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no matching CL atoms"));
    }

    #[tokio::test]
    async fn cl_retrieve_dispatch_logs_a_retrieval_event() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .replace_atoms_for_file(
                "p",
                "notes.md",
                &[crate::storage::Atom {
                    heading_path: "Gotchas".into(),
                    body: "the migration is immutable".into(),
                    code_hash: None,
                }],
                "t",
            )
            .await
            .unwrap();
        // Storage is Clone (shared pool) — keep a probe to read the log after dispatch.
        let probe = storage.clone();
        bridge.set_storage(storage).await;

        dispatch(
            req(
                "tools/call",
                json!({"name": "cl_retrieve", "arguments": {"project": "p", "query": "migration"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();

        // The Stage-4b hook wrote exactly one retrieval_events row for session "s1".
        let stats = probe.retrieval_stats(Some("p"), None).await.unwrap();
        assert_eq!(stats.event_count, 1, "one retrieval logged");
        assert_eq!(stats.distinct_sessions, 1);
        assert_eq!(stats.total_atoms, 1, "the one returned atom was recorded");
        assert!(stats.total_tokens > 0, "token estimate recorded: {}", stats.total_tokens);
        assert_eq!(stats.empty_returns, 0);
    }

    #[tokio::test]
    async fn cl_write_file_dispatch_writes_for_brian_and_denies_rain() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("library/projects/bot-hq")).unwrap();
        let log = crate::policy::ViolationsLog::new(tmp.path());
        let bridge = SignalingBridge::with_policy(log, tmp.path().to_path_buf());
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        storage.create_session("s1", "CL write", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;

        // Rain is EYES — CL content writes are denied at dispatch.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_write_file", "arguments": {
                    "project": "bot-hq",
                    "file_path": "notes.md",
                    "content": "rain-authored"
                }}),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("needs the `write_context_library` capability"),
            "got: {text}"
        );
        assert!(!tmp.path().join("library/projects/bot-hq/notes.md").exists());

        // Brian writes directly; the response names the outcome.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_write_file", "arguments": {
                    "project": "bot-hq",
                    "file_path": "notes.md",
                    "content": "a direct learning"
                }}),
                2,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("created"), "got: {text}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("library/projects/bot-hq/notes.md")).unwrap(),
            "a direct learning"
        );
    }

    #[tokio::test]
    async fn peer_ack_allowed_for_either_agent() {
        // peer_ack is NOT role-gated — both HANDS and EYES converge via it. (The
        // real suppression happens in the duo pump; here we just assert the
        // dispatch accepts the call from either agent.)
        let bridge = SignalingBridge::new();
        for c in [caller(), rain_caller()] {
            let agent = c.agent.clone();
            let res = dispatch(
                req("tools/call", json!({"name": "peer_ack", "arguments": {}}), 1),
                &c,
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
            let v = serde_json::to_value(&res).unwrap();
            assert_eq!(
                v["result"]["isError"],
                json!(false),
                "peer_ack must be allowed for {agent}"
            );
        }
    }

    #[tokio::test]
    async fn pass_turn_is_ungated_and_needs_no_arguments() {
        // Ungated by design: every participant that can hold a turn can decline
        // one. Gate it and a role is pushed back onto the two endings the pass
        // exists to replace — a false done vote, or filler.
        //
        // The empty `arguments` is the second half. The pass carries no
        // parameters, and an agent reaching for it mid-turn must not be able to
        // fail the call by omitting something.
        let bridge = SignalingBridge::new();
        for c in [caller(), rain_caller()] {
            let agent = c.agent.clone();
            let res = dispatch(
                req("tools/call", json!({"name": "pass_turn", "arguments": {}}), 1),
                &c,
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
            let v = serde_json::to_value(&res).unwrap();
            assert_eq!(
                v["result"]["isError"],
                json!(false),
                "pass_turn must be allowed for {agent}"
            );
        }
    }

    /// rc3 **D25**: the SECOND pass in one turn is refused.
    ///
    /// A pass is the turn ending, so a turn carries at most one. The second is
    /// incoherent rather than merely redundant — the first already recorded the
    /// whole of what a pass says — and answering it with the same cheerful
    /// acknowledgment is what let a participant call it 141 times in eight
    /// minutes in `s-a4e9a1b4`, one real model call each.
    ///
    /// **The round cap cannot substitute for this.** The cap counts LAPS of the
    /// ring, so a participant looping inside ONE turn — which is what a turn that
    /// never ends produces — spends model calls while the counter that is meant
    /// to bound it stays at zero. The only thing that stopped the live one was
    /// the user watching the screen.
    #[tokio::test]
    async fn a_second_pass_in_one_turn_is_refused() {
        let bridge = SignalingBridge::new();
        let c = caller();
        let pass = || {
            dispatch(
                req("tools/call", json!({"name": "pass_turn", "arguments": {}}), 1),
                &c,
                &bridge,
            )
        };
        let first = serde_json::to_value(pass().await.unwrap().unwrap()).unwrap();
        assert_eq!(first["result"]["isError"], json!(false), "the first pass stands");

        let second = serde_json::to_value(pass().await.unwrap().unwrap()).unwrap();
        assert_eq!(
            second["result"]["isError"],
            json!(true),
            "a turn carries at most one pass"
        );
        let text = second["result"]["content"][0]["text"].as_str().unwrap_or_default();
        assert!(
            text.contains("ALREADY recorded"),
            "the refusal has to say WHY, or the agent reads it as a transient \
             failure and retries: {text}"
        );

        // A third is refused too, and the count keeps rising — the message names
        // which attempt this is, so a looping agent reads an escalating number
        // rather than the same sentence forever.
        let third = serde_json::to_value(pass().await.unwrap().unwrap()).unwrap();
        let text3 = third["result"]["content"][0]["text"].as_str().unwrap_or_default();
        assert!(text3.contains("call 3"), "the attempt count is visible: {text3}");
    }

    /// The counter is per PARTICIPANT and per TURN, not per session.
    #[tokio::test]
    async fn one_participants_pass_does_not_spend_anothers() {
        let bridge = SignalingBridge::new();
        for c in [caller(), rain_caller()] {
            let v = serde_json::to_value(
                dispatch(
                    req("tools/call", json!({"name": "pass_turn", "arguments": {}}), 1),
                    &c,
                    &bridge,
                )
                .await
                .unwrap()
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                v["result"]["isError"],
                json!(false),
                "{} passes for the first time in its own turn",
                c.agent
            );
        }
        // And a new turn restores it — which is what the ring calls at handover.
        let c = caller();
        bridge.clear_passes(&c.session_id, &c.agent);
        let v = serde_json::to_value(
            dispatch(
                req("tools/call", json!({"name": "pass_turn", "arguments": {}}), 1),
                &c,
                &bridge,
            )
            .await
            .unwrap()
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            v["result"]["isError"],
            json!(false),
            "a fresh turn carries a fresh pass"
        );
    }

    #[tokio::test]
    async fn pass_turn_is_advertised_in_the_tool_list() {
        // The pump can only observe a tool the agent can SEE. Nothing else in
        // this slice fails if the descriptor is missing — the flag would simply
        // never be set — so the registry entry needs its own pin.
        let names: Vec<&str> = super::super::protocol::tool_descriptors()
            .iter()
            .map(|d| d.name)
            .collect();
        assert!(names.contains(&"pass_turn"), "got: {names:?}");
    }

    #[tokio::test]
    async fn advance_phase_self_dispatch_emits_event() {
        // Self-advance path: agent moves the chip without user gate. Bridge
        // fires AgentAdvancePhase; AppState's subscriber routes to
        // core.advance_phase. We only assert the event here.
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "advance_phase",
                    "arguments": {"target": "Apply"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "phase advanced");
        let ev = sub.recv().await.unwrap();
        match ev {
            SignalingEvent::AgentAdvancePhase { target, agent, .. } => {
                assert_eq!(target, "Apply");
                assert_eq!(agent, "brian");
            }
            other => panic!("expected AgentAdvancePhase, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn advance_phase_self_rejects_bogus_target() {
        let bridge = SignalingBridge::new();
        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "advance_phase",
                    "arguments": {"target": "Wander"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(err.message.contains("unknown target"));
    }

    #[tokio::test]
    async fn rain_can_self_advance_phase() {
        // Self-advance is not HANDS-only — either agent can move the chip.
        // The user retains override via the dashboard chip click.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "advance_phase",
                    "arguments": {"target": "Plan"}
                }),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(false));
    }

    #[tokio::test]
    async fn request_phase_advance_dispatch_emits_event() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_phase_advance",
                    "arguments": {"target": "Apply", "reason": "plan done"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("awaiting user"));
        let ev = sub.recv().await.unwrap();
        match ev {
            SignalingEvent::AwaitingUser { reason, .. } => {
                assert!(
                    reason.contains("PHASE REQUEST -> Apply"),
                    "reason: {reason}"
                );
                assert!(reason.contains("plan done"), "reason: {reason}");
            }
            other => panic!("expected AwaitingUser, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn request_phase_advance_rejects_bogus_target() {
        let bridge = SignalingBridge::new();
        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_phase_advance",
                    "arguments": {"target": "Coffee", "reason": "x"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(err.message.contains("unknown target"));
    }

    #[tokio::test]
    async fn rain_can_call_request_phase_advance() {
        // Phase requests are not HANDS-only — Rain (EYES) should also be able
        // to ask the user to back off to Investigate when Brian is about to
        // mutate without a plan.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_phase_advance",
                    "arguments": {"target": "Investigate", "reason": "need to reassess"}
                }),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(
            v["result"]["isError"],
            json!(false),
            "rain should be allowed to call request_phase_advance"
        );
    }

    #[tokio::test]
    async fn request_phase_advance_accepts_chip_form() {
        // F12 regression guard: chip-form targets (I/P/A/V) must reach the
        // bridge — same leniency `advance_phase` already had. Previously
        // request_phase_advance used a hardcoded matches!() against full
        // names only and returned INVALID_PARAMS for "A".
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_phase_advance",
                    "arguments": {"target": "A", "reason": "ready to mutate"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(false));
    }

    #[tokio::test]
    async fn advance_phase_self_accepts_chip_form() {
        // Parity with request_phase_advance_accepts_chip_form — both paths
        // route through IpavPhase::parse so chip form should work here too.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "advance_phase",
                    "arguments": {"target": "A"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "phase advanced");
    }

    #[tokio::test]
    async fn ask_user_choice_dispatches_parked_ack() {
        // ask_user_choice is non-blocking at the dispatch layer too: the tool
        // call returns `{status:"parked", choice_id}` immediately, NOT the pick.
        // (No spawn needed — it doesn't wait on the user.)
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "ask_user_choice",
                    "arguments": {"question": "?", "options": ["a", "b"]}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"status\":\"parked\""), "text: {text}");
        assert!(text.contains("choice_id"), "text: {text}");
    }

    #[tokio::test]
    async fn notification_returns_no_response() {
        let bridge = SignalingBridge::new();
        let mut r = req("ping", json!({}), 1);
        r.id = None;
        let out = dispatch(r, &caller(), &bridge).await.unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn session_doc_write_then_read_round_trip() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        let write_res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "plan-v1", "body": "the plan body"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&write_res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("plan-v1"), "write returned: {text}");

        let read_res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_read",
                    "arguments": {"slug": "plan-v1"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&read_res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("\"body\":\"the plan body\""),
            "read returned: {text}"
        );
    }

    #[tokio::test]
    async fn session_doc_read_unknown_slug_returns_null() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_read",
                    "arguments": {"slug": "nope"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "null");
    }

    #[tokio::test]
    async fn session_doc_write_with_phase_then_search_by_phase() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        // Two writes under phase="plan" (even with different slugs) collapse to
        // ONE rewritable doc keyed by phase — the latest body wins. A different
        // phase keeps its own doc.
        for (slug, body, phase) in [
            ("plan-v1", "first", "plan"),
            ("plan-v2", "second", "plan"),
            ("find-1", "x", "investigate"),
        ] {
            dispatch(
                req(
                    "tools/call",
                    json!({
                        "name": "session_doc_write",
                        "arguments": {"slug": slug, "body": body, "phase": phase}
                    }),
                    1,
                ),
                &caller(),
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
        }

        // Search filtered by phase="plan" returns the single consolidated doc.
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_search",
                    "arguments": {"phase": "plan"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let rows: Vec<Value> = serde_json::from_str(text).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "phase docs collapse to one per phase, got: {text}"
        );
        assert_eq!(rows[0]["phase"], "plan");
        assert_eq!(
            rows[0]["slug"], "plan",
            "phase-tagged doc is keyed by phase name"
        );
        assert_eq!(rows[0]["body"], "second", "latest write wins");
    }

    #[tokio::test]
    async fn session_doc_write_rejects_invalid_phase() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "doc", "body": "x", "phase": "garbage"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .expect_err("invalid phase enum should return Err(JsonRpcError)");
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(
            err.message.contains("phase must be one of"),
            "msg: {}",
            err.message
        );
    }

    /// **The phase-doc router keys on the CAPABILITY, never on a slug.**
    ///
    /// This is the fifth fail-quiet name check of rc3 D10 and the only one that
    /// destroyed data: the arm read `caller.agent.as_str() == "rain"`, no
    /// participant is called that any more, so every phase-tagged review write
    /// fell through and OVERWROTE the executor's doc for that phase — silently,
    /// while migration 0049's EYES prose kept promising the co-located
    /// `<phase>-eyes` doc.
    ///
    /// Both callers below carry the SAME role-derived slug. The only difference
    /// between them is `file_finding`, so nothing but the capability can be
    /// producing the split — a router that went back to matching a name would
    /// route both the same way and fail here.
    #[tokio::test]
    async fn the_phase_doc_router_splits_on_file_finding_not_on_the_slug() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        // Same slug, opposite grants.
        let reviewer = CallerIdentity {
            session_id: "s1".into(),
            agent: "eyes".into(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::preset_eyes(),
            ),
        };
        let non_reviewer = CallerIdentity {
            session_id: "s1".into(),
            agent: "eyes".into(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::preset_hands(),
            ),
        };
        assert!(
            reviewer
                .capabilities
                .grants(crate::agents::Capability::FileFinding)
                && !non_reviewer
                    .capabilities
                    .grants(crate::agents::Capability::FileFinding),
            "the presets must differ on file_finding or this test proves nothing"
        );

        let write = |who: CallerIdentity, body: &'static str| {
            let bridge = bridge.clone();
            async move {
                let res = dispatch(
                    req(
                        "tools/call",
                        json!({
                            "name": "session_doc_write",
                            "arguments": {"slug": "plan", "body": body, "phase": "plan"}
                        }),
                        1,
                    ),
                    &who,
                    &bridge,
                )
                .await
                .unwrap()
                .unwrap();
                let v = serde_json::to_value(&res).unwrap();
                v["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string()
            }
        };

        // The non-reviewer owns the phase doc itself.
        let plain = write(non_reviewer, "the plan").await;
        assert!(
            plain.contains("\"slug\":\"plan\""),
            "a caller without file_finding writes the phase doc itself; got: {plain}"
        );
        // The reviewer is diverted to the co-located doc.
        let routed = write(reviewer, "the review").await;
        assert!(
            routed.contains("plan-eyes"),
            "a caller holding file_finding must be routed to <phase>-eyes; got: {routed}"
        );

        // And the executor's doc is intact — the clobber this arm exists to stop.
        let plan = bridge
            .session_doc_read("s1", "plan")
            .await
            .unwrap()
            .expect("the phase doc");
        assert_eq!(
            plan.body, "the plan",
            "the reviewer's write must not have overwritten the phase doc"
        );
    }

    /// Both docs land under the SAME phase tag, which is what puts them in one
    /// IPAV tab. The router split itself is pinned by
    /// `the_phase_doc_router_splits_on_file_finding_not_on_the_slug`; this one
    /// covers what the split is FOR.
    #[tokio::test]
    async fn a_reviewers_phase_write_co_locates_instead_of_clobbering() {
        // The executor authors `plan`; the reviewer's phase-tagged write lands
        // in a co-located `plan-eyes` doc (same phase tag → same IPAV tab).
        // Both persist; the executor's body is untouched.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        // The executor authors the plan doc.
        dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "plan", "body": "the plan", "phase": "plan"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();

        // The reviewer contributes — must NOT error, and must land in `plan-eyes`.
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "plan", "body": "the review", "phase": "plan"}
                }),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_ne!(
            v["result"]["isError"],
            json!(true),
            "the reviewer's phase-tagged write must be accepted"
        );
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("plan-eyes"),
            "the reviewer's write should report the co-located slug, got: {text}"
        );

        // Both docs render under the Plan tab; the executor's body is not clobbered.
        let docs = bridge
            .session_doc_search("s1", None, Some("plan"))
            .await
            .unwrap();
        assert_eq!(docs.len(), 2, "the plan doc + plan-eyes both persist");
        let plan = docs
            .iter()
            .find(|d| d.slug == "plan")
            .expect("the executor's plan doc");
        assert_eq!(plan.body, "the plan", "the executor's doc must be untouched");
        let review = docs
            .iter()
            .find(|d| d.slug == "plan-eyes")
            .expect("the co-located review doc");
        assert!(review.body.contains("### Review findings"));
        assert!(review.body.contains("the review"));
    }

    #[tokio::test]
    async fn rain_untagged_doc_write_allowed() {
        // The gate is narrow: Rain may still keep her own UNTAGGED scratch doc
        // (EYES_ROLE explicitly permits this). Only the phase-tagged form is
        // HANDS-only.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "rain-scratch", "body": "my notes"}
                }),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_ne!(
            v["result"]["isError"],
            json!(true),
            "rain's untagged scratch doc must be allowed"
        );
        let read = bridge
            .session_doc_read("s1", "rain-scratch")
            .await
            .unwrap();
        assert!(read.is_some(), "untagged scratch doc should persist");
    }

    #[tokio::test]
    async fn session_doc_search_rejects_invalid_phase() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_search",
                    "arguments": {"phase": "garbage"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .expect_err("invalid phase enum should return Err(JsonRpcError)");
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(
            err.message.contains("phase must be one of"),
            "msg: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn check_commit_message_no_policy_returns_ok() {
        // Default bridge has no data_dir → policy resolves to default → ok.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "check_commit_message",
                    "arguments": {"message": "anything with Acme inside"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "ok");
    }

    #[tokio::test]
    async fn check_commit_message_finds_forbidden_word() {
        let tmp = tempfile::tempdir().unwrap();
        // Write a project policy and register the session.
        std::fs::create_dir_all(tmp.path().join("library/projects/foo")).unwrap();
        std::fs::write(
            tmp.path().join("library/projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - bot-hq\n  - Acme\n",
        )
        .unwrap();
        let log = crate::policy::ViolationsLog::new(tmp.path());
        let bridge = SignalingBridge::with_policy(log.clone(), tmp.path().to_path_buf());
        bridge
            .register_session("s1".into(), Some("foo".into()))
            .await;

        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "check_commit_message",
                    "arguments": {"message": "fix: pass bot-hq tests"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("forbidden_word:"), "got: {text}");
        assert!(text.contains("bot-hq"));

        // Violation logged.
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, crate::policy::ViolationKind::CommitGrep);
        assert_eq!(recs[0].outcome, crate::policy::ViolationOutcome::Denied);
    }

    #[tokio::test]
    async fn request_approval_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let log = crate::policy::ViolationsLog::new(tmp.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let mut sub = bridge.subscribe();
        // The AGENT path parks: dispatch returns before the user has picked, so
        // there is nothing to await and nothing to time out. (The blocking twin
        // is the pre-push hook's, covered in bridge::tray's tests.)
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_approval",
                    "arguments": {
                        "kind": "push_gate",
                        "action": "git push origin main",
                        "question": "Approve push to main?",
                        "options": ["Approve once", "Deny"],
                        "detail": "first push to this branch"
                    }
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let ack: serde_json::Value =
            serde_json::from_str(text).expect("parked ack is JSON, not a bare pick");
        assert_eq!(ack["status"], "parked", "agent path must not block: {text}");

        let ev = sub.recv().await.unwrap();
        let pending = match ev {
            SignalingEvent::PendingChoice(p) => {
                assert!(p.approval.is_some());
                p
            }
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        assert_eq!(
            ack["choice_id"].as_str().unwrap(),
            pending.choice_id,
            "the parked ack must name the row the user will answer"
        );
        bridge
            .resolve_choice(&pending.choice_id, "Approve once".into())
            .await
            .unwrap();
        // Parking must not cost the violation record — it is written at resolve.
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, crate::policy::ViolationKind::PushGate);
        assert_eq!(recs[0].outcome, crate::policy::ViolationOutcome::Approved);
    }

    #[tokio::test]
    async fn request_approval_rejects_unknown_kind() {
        let bridge = SignalingBridge::new();
        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_approval",
                    "arguments": {
                        "kind": "bogus_kind",
                        "action": "x",
                        "question": "?",
                        "options": ["a", "b"]
                    }
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(err.message.contains("unknown kind"));
    }

    #[test]
    fn peer_shaped_reasons_are_detected_with_word_boundaries() {
        use super::peer_shaped_reason;
        // The s-96fda118 deadlock reason shape: parking on the peer.
        assert_eq!(
            peer_shaped_reason("Handed the six refusal probes to Rain — they can only run natively"),
            Some("rain")
        );
        assert_eq!(peer_shaped_reason("waiting for my peer to review"), Some("peer"));
        assert_eq!(peer_shaped_reason("EYES review pending"), Some("eyes"));
        // Word boundaries: substrings inside real words must not trip it.
        assert_eq!(peer_shaped_reason("waiting for the rainbow deploy window"), None);
        assert_eq!(peer_shaped_reason("user must restrain the migration"), None);
        assert_eq!(peer_shaped_reason("need the user's Clockify token"), None);
        assert_eq!(peer_shaped_reason(""), None);
    }

}