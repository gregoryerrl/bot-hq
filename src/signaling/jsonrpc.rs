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
            // Per caller: the core tools it can use carry the always-load
            // mark, so they need no `ToolSearch` round trip (feedback #88).
            let tools = crate::signaling::protocol::tools_list_for(&caller.capabilities);
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

/// The cap `cl_write_file` enforces on a body, applied here to a
/// `content_path` BEFORE the file is read: bot-hq is one process, and reading
/// a multi-gigabyte path into it to discover it is over the cap would take
/// the UI down with the bridge.
const CONTENT_PATH_MAX_BYTES: u64 = 1_048_576;
const CL_CONTENT_PATH_ADVICE: &str = "the CL write cap is 1 MiB. CL files are high-signal study \
     notes; trim or split instead";

/// The cap on a SESSION DOC body read from a file (`session_doc_write`'s
/// `content_path`; EYES, plan point 5). Lower than the CL's: a doc is archived
/// on every replace (up to 50 versions of a phase doc), delivered whole to the
/// document pane, and kept with the session, so a 100 MB log read into one
/// would bloat the database and the archive.
const DOC_CONTENT_PATH_MAX_BYTES: u64 = 262_144;
const DOC_CONTENT_PATH_ADVICE: &str = "a session doc body read from a file is capped at 256 KiB. \
     Attach the part that matters (for a log, its tail) or split it";

/// Read a tool's body from a file on this machine (feedback #30): absolute, a
/// regular file, at most `max_bytes` by stat, UTF-8. `advice` ends the
/// over-the-cap error, naming the cap and what to do instead.
///
/// The read runs with bot-hq's own privileges, not the calling agent's: the
/// path is not confined to the working repo, and a file the agent's sandbox
/// denies its Bash tool is readable here if bot-hq can read it. Not an
/// escalation today — every participant runs as the same user, and the
/// body lands in a library it could already write with `content` — but the
/// asymmetry is the thing to revisit if agents ever run under a narrower
/// identity than the host.
async fn read_content_path(
    path: &str,
    max_bytes: u64,
    advice: &str,
) -> std::result::Result<String, String> {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return Err(format!("content_path must be an absolute path, got '{path}'"));
    }
    let meta = tokio::fs::metadata(p)
        .await
        .map_err(|e| format!("content_path '{path}': {e}"))?;
    if !meta.is_file() {
        return Err(format!("content_path '{path}' is not a regular file"));
    }
    if meta.len() > max_bytes {
        return Err(format!("content_path '{path}' is {} bytes — {advice}", meta.len()));
    }
    tokio::fs::read_to_string(p)
        .await
        .map_err(|e| format!("content_path '{path}': {e} (the body must be UTF-8 text)"))
}

/// An RFC3339 timestamp cut to whole seconds in UTC (`2026-09-06T04:40:01Z`).
///
/// `cl_index.updated_at` holds two shapes — `Z`/millis from `now_utc()` and
/// `+00:00`/nanoseconds from a rescan's disk mtime — and a staleness read
/// needs neither the fraction nor the offset. Anything that does not parse is
/// passed through untouched rather than dropped: a strange timestamp is still
/// a timestamp.
pub(crate) fn whole_seconds(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| {
            t.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_else(|_| ts.to_string())
}

/// Parse + validate the optional `phase` arg shared by session_doc_write and
/// session_doc_search. Returns Ok(None) when absent; Err with INVALID_PARAMS
/// when present but unparseable. Routed through `IpavPhase::parse` (the single
/// source of truth, shared with `advance_phase`) and normalized to the canonical
/// lowercase `tag()` — so the same phase string can't be valid for one phase
/// tool and rejected by another (the old `VALID_PHASES` drift), and any accepted
/// casing/chip stores as a consistent tag the IPAV tabs can match.
fn parse_optional_phase(args: &Value) -> Result<Option<String>, JsonRpcError> {
    // One parser (`protocol::parse_phase_arg`), one error text — this used to
    // re-implement the parse with its own wording (round 8, T2-6).
    args.get("phase")
        .and_then(Value::as_str)
        .map(|p| super::protocol::parse_phase_arg("phase", p).map(|ph| ph.tag().to_string()))
        .transpose()
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

/// `wake_after_secs` on `mark_awaiting_user` / `halt` (round 12 — a TEMPORARY
/// halt): absent = an ordinary halt; present = a wake in 10…3600 s, else
/// INVALID_PARAMS — an out-of-range wait is a typo, not a request.
fn parse_wake_after(args: &Value) -> Result<Option<std::time::Duration>, JsonRpcError> {
    let Some(v) = args.get("wake_after_secs") else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let secs = v.as_u64().ok_or_else(|| {
        JsonRpcError::new(
            JsonRpcError::INVALID_PARAMS,
            "wake_after_secs must be an integer number of seconds (10..=3600)",
        )
    })?;
    if !(10..=3600).contains(&secs) {
        return Err(JsonRpcError::new(
            JsonRpcError::INVALID_PARAMS,
            "wake_after_secs must be between 10 and 3600 (an hour); re-declare when the wait continues",
        ));
    }
    Ok(Some(std::time::Duration::from_secs(secs)))
}

/// Does a halt `reason` read as waiting on a PEER rather than on the user?
///
/// The shape it refuses (the `s-96fda118` deadlock): both agents marked
/// themselves awaiting-user over work each thought was the OTHER's, and the
/// session sat dead 100 minutes. Round 12 narrowed it after it refused
/// legitimate halts four times out of five real hits — "Two things in your
/// **hands**." (s-1c29c521, 00:20:01Z), "EYES F1–F5 folded in, no rebuttals",
/// "EYES awake and passing" — because the old vocabulary was the lowercase
/// English nouns `eyes`/`hands` (and the retired names) word-matched anywhere.
///
/// Now two conditions, both in ONE sentence: a peer TOKEN — a role name as
/// the roster renders it (`HANDS`, `EYES`), the lowercase slug when the word
/// before it is not a possessive or a quantity ("waiting on hands" yes, "in
/// your hands" / "more eyes on it" no — EYES F14), the summons form
/// `@hands`/`@eyes`, or the word `peer(s)` — AND a WAIT shape
/// (`wait`/`awaiting`, `pending`, `until`, `blocked`, `handed`/`hand off`,
/// `need(s)`, `to review/answer/respond/confirm/verify/reply/sign`). Word
/// boundaries are `[A-Za-z0-9_]` (the same rule as `policy::contains_word`,
/// so `eyes_flag` is one word). Returns the token that tripped it, for the
/// refusal text. Heuristic vocabulary, not an identity check — nothing is
/// keyed on a participant being called any of these.
fn peer_shaped_reason(reason: &str, roster: &[(String, String)]) -> Option<String> {
    // The vocabulary is the LIVE roster (1.0.0 Batch 6, M6 — promoted by the
    // config-line review): the deadlock this guards against (s-96fda118, 100
    // minutes) is a property of ANY multi-participant session, but the old
    // constant list knew only HANDS/EYES — for a user who names their roles
    // PILOT/NAVIGATOR (exactly the population the neutral-default work
    // creates) the guard never fired. Each roster entry contributes its
    // display name (matched as written), its slug (lowercase, behind the
    // possessive/quantity filter below), and the summons form `@slug`. The
    // caller passes the session's enabled participants; an unreadable roster
    // falls back to the seeded pair so the guard never silently disarms.
    const WAIT_SHAPES: &[&str] = &[
        "wait",
        "pending",
        "until",
        "blocked",
        "handed",
        "hand off",
        "need",
        "to review",
        "to answer",
        "to respond",
        "to confirm",
        "to verify",
        "to reply",
        "to sign",
    ];
    fn is_word_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }
    /// `needle` occurs in `hay` on word boundaries (`[A-Za-z0-9_]`), matched
    /// exactly as given (the caller decides the case).
    fn has_word(hay: &str, needle: &str) -> bool {
        hay.match_indices(needle).any(|(i, _)| {
            let before_ok = i == 0 || !hay[..i].chars().next_back().is_some_and(is_word_char);
            let after = i + needle.len();
            let after_ok = after >= hay.len() || !hay[after..].chars().next().is_some_and(is_word_char);
            before_ok && after_ok
        })
    }
    /// The lowercase slugs count too — "waiting on hands to finish" is the
    /// deadlock shape in lowercase (EYES F14: the uppercase-only rule bought
    /// precision by giving up that recall) — but NOT when the word before
    /// them is a possessive or a quantity, which is what separates the
    /// English noun ("in your hands", "needs more eyes on it") from the role.
    fn lowercase_slug(lower: &str, slug: &str) -> bool {
        const NOT_A_ROLE_BEFORE: &[&str] = &[
            "your", "my", "our", "their", "his", "her", "its", "the", "more", "extra",
            "fresh", "many", "two", "four", "all", "both", "own", "some", "no", "of",
        ];
        lower.match_indices(slug).any(|(i, _)| {
            let before_ok = i == 0 || !lower[..i].chars().next_back().is_some_and(is_word_char);
            let after = i + slug.len();
            let after_ok = after >= lower.len() || !lower[after..].chars().next().is_some_and(is_word_char);
            if !(before_ok && after_ok) {
                return false;
            }
            // The last real WORD before the slug — trimmed of every non-word
            // char, not only whitespace, so "all-hands" / "four-eyes" see
            // `all` / `four` and not the empty string `rsplit` yields after a
            // trailing hyphen (EYES, round 12).
            let prev_word = lower[..i]
                .trim_end_matches(|c: char| !is_word_char(c))
                .rsplit(|c: char| !is_word_char(c))
                .next()
                .unwrap_or("");
            !NOT_A_ROLE_BEFORE.contains(&prev_word)
        })
    }
    for sentence in reason.split(['.', '!', '?', '\n', ';']) {
        let lower = sentence.to_lowercase();
        let token: Option<String> = roster
            .iter()
            .find_map(|(display, slug)| {
                let at_form = format!("@{slug}");
                (has_word(sentence, display)
                    || has_word(&lower, &at_form)
                    || lowercase_slug(&lower, slug))
                .then(|| slug.clone())
            })
            .or_else(|| {
                (has_word(&lower, "peer") || has_word(&lower, "peers"))
                    .then(|| "peer".to_string())
            });
        let Some(token) = token else { continue };
        if WAIT_SHAPES.iter().any(|w| lower.contains(w)) {
            return Some(token);
        }
    }
    None
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
            // Host-authored (`origin = 'system'`, NULL participant, 0044),
            // exactly as the capped halt posts — the refusal is the host's
            // account, not the caller's turn output.
            if crate::core::post_system_notice(
                &storage,
                Some(bridge),
                caller.session_id.as_str(),
                crate::storage::MessageKind::SystemNotice,
                body,
                None,
            )
            .await
            .is_none()
            {
                tracing::warn!(
                    session_id = %caller.session_id,
                    agent = %caller.agent,
                    tool = %name,
                    "a capability refusal was not recorded in the channel"
                );
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

/// The `session_doc_write` reply: `{id, slug}`, plus a `note` when the body
/// was stored with `redacted` secrets replaced by markers (F10).
/// What a reviewer is told when its own co-doc write withdrew its vote.
const VOTE_WITHDRAWN_NOTE: &str = "your own phase vote was withdrawn by this write: review notes \
     changed after you voted. Cast it again with advance_phase if it still stands";

/// Where a session-doc write or edit lands for one caller.
enum DocTarget {
    /// The reviewer's co-located `<phase>-eyes` doc of this phase.
    Codoc(String),
    /// The doc the caller named (keyed by `phase` when one was passed).
    Own,
}

/// Decide where `caller`'s write or edit of `slug` lands.
///
/// A REVIEWER-shaped caller's write belongs to its co-doc when it carries a
/// `phase`, or when the slug itself names a phase doc or a `<phase>-eyes` doc.
/// EYES' advisory `9a1602f1`: the redirect used to look at the `phase`
/// ARGUMENT alone, so a reviewer's untagged write to `plan` (or to
/// `plan-eyes`) took the executor's path and landed in the executor's doc.
///
/// The mirror: a co-doc is the reviewer's. Any other caller's untagged write
/// to a `<phase>-eyes` slug is refused — it would replace the review.
fn session_doc_target(
    caller: &CallerIdentity,
    slug: &str,
    explicit_phase: Option<&str>,
) -> Result<DocTarget, JsonRpcError> {
    if caller.capabilities.reviewer_shaped() {
        let phase = explicit_phase
            .or_else(|| crate::storage::reviewer_codoc_phase(slug))
            .or_else(|| crate::storage::phase_doc_slug(slug));
        return Ok(match phase {
            Some(p) => DocTarget::Codoc(p.to_string()),
            None => DocTarget::Own,
        });
    }
    if explicit_phase.is_none() {
        if let Some(of_phase) = crate::storage::reviewer_codoc_phase(slug) {
            return Err(JsonRpcError::new(
                JsonRpcError::INVALID_PARAMS,
                format!(
                    "`{slug}` holds the reviewer's notes on the {of_phase} phase; a write from \
                     you would replace the review. Answer it in your own phase doc or in the \
                     channel."
                ),
            ));
        }
    }
    Ok(DocTarget::Own)
}

/// A handoff doc is put back into ITS owner's context after a compaction
/// (`agents::handoff`), so a peer's write or edit would arrive there as the
/// owner's own notes. Only the owner changes it; the user edits it in its tab.
/// `handoff-<anything that is not a participant>` is an ordinary custom doc,
/// and a phase-tagged write is keyed by its phase, not by this slug.
async fn refuse_a_peers_handoff_doc(
    bridge: &Arc<SignalingBridge>,
    caller: &CallerIdentity,
    slug: &str,
    explicit_phase: Option<&str>,
) -> Result<(), JsonRpcError> {
    if explicit_phase.is_some() {
        return Ok(());
    }
    let Some(owner) = crate::agents::handoff::participant_of(slug) else {
        return Ok(());
    };
    if owner == caller.agent || !bridge.is_session_participant(&caller.session_id, owner).await {
        return Ok(());
    }
    Err(JsonRpcError::new(
        JsonRpcError::INVALID_PARAMS,
        format!(
            "`{slug}` is {owner}'s handoff doc: bot-hq puts it back into {owner}'s context after \
             a compaction, so only {owner} writes it (the user can edit it in its tab). Yours is \
             `{}`.",
            crate::agents::handoff::doc_slug(&caller.agent)
        ),
    ))
}

/// `expect_occurrences` for the two exact-edit tools: absent or null is 1,
/// anything but a positive integer is refused.
fn parse_expect_occurrences(args: &Value) -> Result<usize, JsonRpcError> {
    match args.get("expect_occurrences") {
        None | Some(Value::Null) => Ok(1),
        Some(v) => match v.as_u64() {
            Some(n) if n >= 1 => Ok(n as usize),
            _ => Err(JsonRpcError::new(
                JsonRpcError::INVALID_PARAMS,
                "expect_occurrences must be a positive integer",
            )),
        },
    }
}

fn doc_write_reply(id: i64, slug: &str, redacted: usize, extra: Option<&str>) -> String {
    let mut notes: Vec<String> = Vec::new();
    if redacted > 0 {
        notes.push(format!("stored{}", crate::policy::secret_scan::redaction_note(redacted)));
    }
    if let Some(extra) = extra {
        notes.push(extra.to_string());
    }
    if notes.is_empty() {
        return json!({"id": id, "slug": slug}).to_string();
    }
    json!({"id": id, "slug": slug, "note": notes.join("; ")}).to_string()
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
            // A peer-shaped reason is a category error that stalls the session:
            // in the archive study both agents marked themselves awaiting-user
            // over work each thought was the OTHER's, and the session sat dead
            // 100 minutes until the user shouted. Waiting on a peer is not
            // waiting on the user — refuse and tell the agent what to do
            // instead. Word-boundary match so e.g. "restrained" can't trip it.
            let roster_tokens = bridge.roster_peer_tokens(&caller.session_id).await;
            if let Some(hit) = peer_shaped_reason(&reason, &roster_tokens) {
                return Ok(ToolCallResult::error(format!(
                    "reason names your peer ('{hit}') — mark_awaiting_user is for \
                     waiting on the USER, and halting on a peer stalls the session: \
                     a halt stops the ring, so the peer you are waiting for never \
                     gets the turn to answer. If the work needs your peer, post it \
                     in the channel — they read it when the ring hands them their \
                     next turn; if they aren't responding, do the work yourself or \
                     ask the user a concrete question via ask_user_choice."
                )));
            }
            // Round 12: a TEMPORARY halt when `wake_after_secs` is given — the
            // session wakes the declarer when the countdown ends.
            let wake_after = parse_wake_after(&args)?;
            let prior = match wake_after {
                Some(d) => {
                    bridge
                        .mark_temporary_halt(caller.session_id.clone(), caller.agent.clone(), reason, d)
                        .await
                }
                None => {
                    bridge
                        .mark_awaiting_user(caller.session_id.clone(), caller.agent.clone(), reason)
                        .await
                }
            };
            let ack = match wake_after {
                Some(d) => format!(
                    "ok — TEMPORARY halt: the session wakes you in {}s (a turn dealt to you with a \
                     system row); re-declare if the wait continues",
                    d.as_secs()
                ),
                None => "ok".to_string(),
            };
            Ok(ToolCallResult::text(with_repeat_halt_note(&ack, prior.as_deref())))
        }
        "peer_ack" => {
            // The effect is realized in the PUMP, which is the only place that
            // sees a whole turn: it observes THIS ToolUse and, at turn end,
            // `sequencer::turn_ending` turns it into the turn's ending — `Done`
            // (a consensus vote) when the turn is content-free or `final`, and a
            // plain `Spoke` when it is not — the ack simply does not count, and
            // no row records that (the tool text used to promise one).
            // Nothing to do bridge-side; the call just needs to succeed.
            Ok(ToolCallResult::text(
                "peer_ack noted — it becomes a DONE vote when this turn is \
                 content-free, OR when you passed `final: true` (the deliberate \
                 override: final counts even on a substantive turn). Without \
                 `final`, a turn carrying substantive text (>200 chars) ends as \
                 an ordinary spoken turn and the ack does not count: reviews and \
                 corrections must never be silently downgraded to agreement. A \
                 counted ack still settles nothing alone — the session settles \
                 when every active participant is done.",
            ))
        }
        "pass_turn" => {
            // Realized in the participant's pump, exactly like `peer_ack` above: the pump
            // observes THIS ToolUse and `sequencer::turn_ending` turns it into a
            // `TurnEnding::Passed` at the flush (pump.rs::pump_agent). Whether the
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
            let wake_after = parse_wake_after(&args)?;
            let prior = match wake_after {
                Some(d) => {
                    bridge
                        .mark_temporary_halt(caller.session_id.clone(), caller.agent.clone(), reason, d)
                        .await
                }
                None => {
                    bridge
                        .mark_awaiting_user(caller.session_id.clone(), caller.agent.clone(), reason)
                        .await
                }
            };
            let ack = match wake_after {
                Some(d) => format!(
                    "halted — TEMPORARY: the session wakes you in {}s (a turn dealt to you with a \
                     system row); input unlocked meanwhile.",
                    d.as_secs()
                ),
                None => "halted — yielded to the user; input unlocked.".to_string(),
            };
            Ok(ToolCallResult::text(with_repeat_halt_note(&ack, prior.as_deref())))
        }
        "advance_phase" => {
            let target = arg_required_str(&args, "target")?;
            // Canonicalised before it reaches the message: the tool accepts chip
            // forms, and "the session is now in A" is not a sentence an agent
            // should have to decode. `parse_phase_arg` already rejects anything
            // unparseable, so the unwrap cannot fire.
            let target = parse_phase_arg("target", &target)?.name().to_string();
            // **The phase is a VOTE now**, so the answer is whatever actually
            // happened — not the literal "phase advanced" this returned
            // unconditionally. An agent told it advanced writes the next phase's
            // document and starts mutating; if the tally was not complete, it is
            // doing that while the session is still in the previous phase and
            // the reviewer has not voted.
            // The CURRENT phase, so the refusal can name where the session
            // actually is. `deliver_oob` already reads it through the same
            // accessor; a dead or headless session answers `None`, and naming
            // the phase "unknown" is honest there rather than guessing.
            let current = bridge
                .current_session_phase(&caller.session_id)
                .await
                .map(|p| p.name().to_string())
                .unwrap_or_else(|| "its current phase".to_string());
            let outcome = bridge
                .agent_advance_phase(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    target,
                )
                .await;
            Ok(ToolCallResult::text(outcome.message(&current)))
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
            // Deliberately NOT capability-gated (nor was it in the pre-rc3,
            // deleted HANDS_ONLY_TOOLS): filing is not a repo
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
                command: None,
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
            // Scoped to the caller's session (round 11): the row carries the
            // user's answer and the exact command.
            let msg = bridge
                .gate_status_for(&gate_id, Some(&caller.session_id))
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(msg))
        }
        "action_gate" => {
            let command = arg_required_str(&args, "command")?;
            // Round 12: the agent may force the park — a command that must not
            // run unapproved regardless of the Tool Gate keyword list.
            let require_approval = args
                .get("require_approval")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let output = bridge
                .action_gate(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    command,
                    require_approval,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(output))
        }
        "close_session" => {
            let archive = args
                .get("archive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            // A3b (adherence), kept as the ONE refusal in the close path (EYES
            // P6 — the rung with teeth): the FIRST close with no CL learnings
            // delta this session nudges to persist it; the retry proceeds. The
            // staleness sweep and the open-advisory list no longer cost a turn
            // each — they ride the close card's recap below (0081, F5).
            if bridge.should_nudge_close(&caller.session_id).await {
                return Ok(ToolCallResult::text(
                    "Before closing: persist this session's bounded learnings delta via \
                     cl_write_file (read the project's notes.md, append your ~5 one-liners \
                     under ## Learnings, and write the FULL updated body), so the next \
                     session doesn't re-discover what this one learned. Then call \
                     close_session again. (If there's genuinely nothing to persist, just \
                     call close_session again.)",
                ));
            }
            // 0081 (F4): an AGENT's close needs the user's word given SINCE the
            // session was last (re)opened — a `close` card answered Approve at or
            // after COALESCE(reopened_at, created_at). Two closes in week 35 had
            // none: one re-used a "reclose" given before a reopen, the other
            // fired 90 s after "before you close, draft…". The REOPENED notice
            // said pre-close instructions are void; both agents read it and
            // closed anyway — so this is mechanical now. The UI Close button is
            // a different path (tauri_cmd) and stays ungated, as is who holds
            // `CloseSession` (rc3 D16).
            let approved = {
                let storage = bridge.storage_handle().await;
                match storage {
                    Some(storage) => storage
                        .close_approved_since_open(&caller.session_id)
                        .await
                        .map_err(internal_err_no_prefix)?,
                    // No storage wired (test bridges built via ::new): nothing
                    // to check against, close as before.
                    None => true,
                }
            };
            if approved {
                bridge.request_session_close(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    archive,
                );
                return Ok(ToolCallResult::text(
                    "session close requested — your subprocess will be terminated shortly",
                ));
            }
            let recap = bridge.close_recap(&caller.session_id).await;
            let card = bridge
                .park_close_card(&caller.session_id, &caller.agent, &recap.card)
                .await
                .map_err(internal_err_no_prefix)?;
            let detail = if recap.detail.is_empty() {
                String::new()
            } else {
                format!("\n\nDetail behind the recap (yours to act on while the card is up):\n{}", recap.detail)
            };
            Ok(ToolCallResult::text(format!(
                "close PARKED for the user's approval (card {card}) — the session closes on \
                 their Approve; on Reject a system row tells you to keep working, and their \
                 answer text is the next instruction. No further close_session call is \
                 needed. Recap on the card:\n{}{detail}",
                recap.card
            )))
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
        // `eyes_flag` is the pre-1.0 name, kept as an ALIAS (Batch 6 M7c):
        // live sessions spawned before the rename carry prompts and role
        // prose that say `eyes_flag`, and a reviewer mid-session must not
        // lose its one enforcement tool to a rename. One implementation,
        // two accepted names; the registry advertises only `flag_finding`.
        "flag_finding" | "eyes_flag" => {
            let severity_str = arg_required_str(&args, "severity")?;
            let severity = crate::storage::FindingSeverity::parse(&severity_str).ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    format!("unknown severity '{severity_str}' (expected 'blocking' or 'advisory')"),
                )
            })?;
            let summary = arg_required_str(&args, "summary")?;
            let code_ref = arg_opt_str(&args, "code_ref");
            let gate_id = arg_opt_str(&args, "gate_id");
            let uid = bridge
                .eyes_flag_for_gate(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    severity,
                    summary,
                    code_ref,
                    gate_id,
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
                .disposition_finding(caller.session_id.clone(), finding_id, status, reason, caller.agent.clone())
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
            let result = bridge
                .override_reviewer_block(&caller.session_id, &caller.agent, &reason)
                .await;
            Ok(ToolCallResult::text(result))
        }
        "approve_finding" => {
            let finding_id = arg_required_str(&args, "finding_id")?;
            let result = bridge
                .approve_finding(caller.session_id.clone(), finding_id)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(result))
        }
        "list_my_pending_questions" => {
            let rows = bridge
                .list_questions_for_session(&caller.session_id)
                .await
                .map_err(internal_err_no_prefix)?;
            // A QUEUED outward publish (0080) is the caller's too — parked on
            // its behalf, waiting for the reviewer's read — and it is exactly
            // the row an agent may want back (feedback #83: a command nobody
            // wanted any more stayed queued, summoned the reviewer and
            // prompted the user). The user's tray never shows a queued row;
            // its issuer's own list does.
            let queued = bridge
                .list_queued_gates_for_session(&caller.session_id)
                .await
                .map_err(internal_err_no_prefix)?;
            // Filter to this agent's still-open questions and shape into the
            // documented contract. `status` tells a queued gate from a parked
            // one.
            let mine: Vec<Value> = rows
                .iter()
                .filter(|r| r.status == "pending")
                .chain(queued.iter())
                .filter(|r| r.agent == caller.agent)
                .map(|r| {
                    json!({
                        "choice_id": r.choice_id,
                        "kind": r.kind,
                        "status": r.status,
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
            use crate::signaling::bridge::Withdrawal;
            let text = match bridge.withdraw_question_for(&choice_id, Some(&caller.agent)).await {
                Withdrawal::Withdrawn => "withdrawn",
                Withdrawal::NotPending => "no-op: choice_id was not pending",
                // Round 9: this used to read "not pending" — the row IS pending,
                // it is just not this caller's to clear.
                Withdrawal::NotYours => {
                    "no-op: that question was parked by another participant — it is \
                     still pending and not yours to withdraw"
                }
                Withdrawal::Unverifiable => {
                    "no-op: the question's owner could not be read (storage error), so it \
                     was NOT withdrawn — it may still be pending; retry, or leave it for \
                     the user"
                }
            };
            Ok(ToolCallResult::text(text))
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
            // `body` inline, or `content_path`: a body that already exists on
            // disk (feedback #71 / #77) — a test log the reviewer should be
            // able to read, a generated table. Exactly one of the two.
            let body = match (
                args.get("body").and_then(Value::as_str),
                args.get("content_path").and_then(Value::as_str),
            ) {
                (Some(_), Some(_)) => {
                    return Err(JsonRpcError::new(
                        JsonRpcError::INVALID_PARAMS,
                        "pass either `body` or `content_path`, not both",
                    ))
                }
                (None, None) => {
                    return Err(JsonRpcError::new(
                        JsonRpcError::INVALID_PARAMS,
                        "missing `body` (or `content_path` to read the body from a file)",
                    ))
                }
                (Some(b), None) => b.to_string(),
                (None, Some(path)) => read_content_path(path, DOC_CONTENT_PATH_MAX_BYTES, DOC_CONTENT_PATH_ADVICE)
                    .await
                    .map_err(|e| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, e))?,
            };
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
            // executor's single per-phase doc: its write goes to a co-located,
            // attributed `<phase>-eyes` doc (same phase tag → same IPAV tab).
            // Untagged reviewer scratch writes fall through to the normal path.
            //
            // **rc3 D10: the reviewer is whoever holds `file_finding`, not
            // whoever is called `rain`.** **WS2 (2026-08-27): re-keyed to
            // REVIEWER-SHAPED** — `FileFinding && !EditFiles` — so an EXECUTOR
            // granted `file_finding` (the reverse review channel) keeps
            // authoring the primary phase docs. See
            // `ResolvedCapabilities::reviewer_shaped` for the full argument,
            // and [`session_doc_target`] for where each write lands.
            //
            // F10: the bridge redacts the body before it is stored; the reply
            // says so, so an agent reading its doc back is not surprised by a
            // marker where it wrote a secret.
            let redacted = crate::policy::secret_scan::find_secrets(&body).len();
            refuse_a_peers_handoff_doc(bridge, caller, &slug, phase.as_deref()).await?;
            match session_doc_target(caller, &slug, phase.as_deref())? {
                DocTarget::Codoc(p) => {
                    let written = bridge
                        .session_doc_write_eyes(&caller.session_id, &p, &body, &caller.agent, append)
                        .await
                        .map_err(internal_err_no_prefix)?;
                    let note = written.vote_withdrawn.then_some(VOTE_WITHDRAWN_NOTE);
                    Ok(ToolCallResult::text(doc_write_reply(written.id, &written.slug, redacted, note)))
                }
                DocTarget::Own => {
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
                    Ok(ToolCallResult::text(doc_write_reply(id, &slug, redacted, None)))
                }
            }
        }
        "session_doc_edit" => {
            let slug = arg_required_str(&args, "slug")?;
            let old_string = arg_required_str(&args, "old_string")?;
            let new_string = arg_required_str(&args, "new_string")?;
            let expect = parse_expect_occurrences(&args)?;
            refuse_a_peers_handoff_doc(bridge, caller, &slug, None).await?;
            // The same routing as a write: a reviewer's edit of a phase doc's
            // name is an edit of ITS co-doc, and a co-doc is not the
            // executor's to edit.
            let (target, codoc) = match session_doc_target(caller, &slug, None)? {
                DocTarget::Codoc(phase) => (format!("{phase}-eyes"), true),
                DocTarget::Own => (slug.clone(), false),
            };
            let redacted = crate::policy::secret_scan::find_secrets(&new_string).len();
            let done = bridge
                .session_doc_edit(&caller.session_id, &target, &old_string, &new_string, expect)
                .await
                .map_err(|e| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, format!("{e:#}")))?;
            let vote_withdrawn = if codoc {
                bridge
                    .after_session_codoc_write(&caller.session_id, &caller.agent, &done.slug)
                    .await
            } else {
                false
            };
            let mut reply = json!({
                "id": done.id,
                "slug": done.slug,
                "occurrences": done.occurrences,
                "bytes_before": done.bytes_before,
                "bytes_after": done.bytes_after,
            });
            if let Some(archive) = &done.archived_as {
                reply["previous_body"] = json!(archive);
            }
            let mut notes: Vec<String> = Vec::new();
            if redacted > 0 {
                notes.push(format!("stored{}", crate::policy::secret_scan::redaction_note(redacted)));
            }
            if vote_withdrawn {
                notes.push(VOTE_WITHDRAWN_NOTE.to_string());
            }
            if !notes.is_empty() {
                reply["note"] = json!(notes.join("; "));
            }
            Ok(ToolCallResult::text(reply.to_string()))
        }
        "session_doc_search" => {
            let query = args.get("query").and_then(Value::as_str);
            let phase = parse_optional_phase(&args)?;
            // Archived versions (`slug@<n>`) stay out of a search unless asked
            // for — one bare search would otherwise return up to ten old full
            // bodies per rewritten custom doc (feedback #37, plan review M5).
            let include_archives = args.get("include_archives").and_then(Value::as_bool).unwrap_or(false)
                || query.is_some_and(|q| q.contains('@'));
            let rows = bridge
                .session_doc_search(&caller.session_id, query, phase.as_deref())
                .await
                .map_err(internal_err_no_prefix)?;
            // **Bodies, within a budget** (feedback #75). The rows are newest
            // first; each doc's whole body rides while the reply stays under
            // `DOC_REPLY_BUDGET_BYTES`, and a doc that does not fit is listed
            // with its size and outline instead, so the caller reads the part
            // it needs. Every row says how big the doc is.
            let budget = crate::signaling::bridge::DOC_REPLY_BUDGET_BYTES;
            let mut spent = 0usize;
            let trimmed: Vec<Value> = rows
                .into_iter()
                .filter(|d| include_archives || !crate::signaling::bridge::is_archive_slug(&d.slug))
                .map(|d| {
                    let bytes = d.body.len();
                    let mut row = json!({
                        "id": d.id,
                        "slug": d.slug,
                        "phase": d.phase,
                        "bytes": bytes,
                        "lines": d.body.lines().count(),
                        "created_at": d.created_at,
                        "updated_at": d.updated_at,
                    });
                    if spent + bytes <= budget {
                        spent += bytes;
                        row["body"] = json!(d.body);
                    } else {
                        row["headings"] = json!(crate::signaling::bridge::doc_outline(&d.body));
                        row["note"] = json!(format!(
                            "body left out: it does not fit this reply's {budget}-byte budget. Read \
                             it with session_doc_read(slug: \"{}\") plus `heading`, `lines` or `grep`.",
                            d.slug
                        ));
                    }
                    row
                })
                .collect();
            Ok(result_json(&trimmed, "[]"))
        }
        "session_doc_read" => {
            let slug = arg_required_str(&args, "slug")?;
            let grep = args.get("grep").and_then(Value::as_str);
            let lines = args.get("lines").and_then(Value::as_str);
            let heading = args.get("heading").and_then(Value::as_str);
            if heading.is_some() && lines.is_some() {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    "pass `heading` or `lines`, not both — a heading already names a line range",
                ));
            }
            let row = bridge
                .session_doc_read(&caller.session_id, &slug)
                .await
                .map_err(internal_err_no_prefix)?;
            let Some(d) = row else {
                return Ok(ToolCallResult::text("null".to_string()));
            };
            let meta = json!({
                "id": d.id,
                "slug": d.slug,
                "created_at": d.created_at,
                "updated_at": d.updated_at,
            });
            let with = |meta: Value, extra: Value| -> String {
                let mut out = meta;
                if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
                    o.extend(e.clone());
                }
                out.to_string()
            };
            // `heading`: one section, as the line range it spans (feedback
            // #75 / #77 — the latest design in a 130 KB plan was found by
            // slicing character offsets by hand).
            let section = match heading {
                None => None,
                Some(needle) => match crate::signaling::bridge::doc_section(&d.body, needle) {
                    Some(found) => Some(found),
                    None => {
                        let known: Vec<String> = crate::signaling::bridge::doc_headings(&d.body)
                            .into_iter()
                            .take(40)
                            .map(|h| h.text)
                            .collect();
                        return Err(JsonRpcError::new(
                            JsonRpcError::INVALID_PARAMS,
                            format!(
                                "no heading in `{}` contains {needle:?}. Its headings: {}",
                                d.slug,
                                if known.is_empty() { "(none)".to_string() } else { known.join(" | ") }
                            ),
                        ));
                    }
                },
            };
            let range = section.as_ref().map(|(_, from, to)| format!("{from}-{to}"));
            let lines = range.as_deref().or(lines);
            if grep.is_some() || lines.is_some() {
                // `grep` / `lines` / `heading`: a selective view, not the whole
                // body (feedback #37).
                let mut excerpt = crate::signaling::bridge::doc_excerpt(&d.body, grep, lines)
                    .map_err(|e| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, e.to_string()))?;
                if let Some((h, _, _)) = &section {
                    excerpt["heading"] = json!(h.text);
                }
                return Ok(ToolCallResult::text(with(meta, excerpt)));
            }
            // The whole body — unless it would overflow the reply (feedback
            // #77: a 121 KB plan spilled to a one-line file). Then the outline,
            // and the three ways to read a part.
            let budget = crate::signaling::bridge::DOC_REPLY_BUDGET_BYTES;
            if d.body.len() > budget {
                let outline = json!({
                    "bytes": d.body.len(),
                    "total_lines": d.body.lines().count(),
                    "headings": crate::signaling::bridge::doc_outline(&d.body),
                    "note": format!(
                        "the body is {} bytes, over this reply's {budget}-byte budget, so it is not \
                         returned whole. Read a section with `heading`, a range with `lines` \
                         (\"1-400\", or \"400-\" to the end), or search it with `grep`.",
                        d.body.len()
                    ),
                });
                return Ok(ToolCallResult::text(with(meta, outline)));
            }
            Ok(ToolCallResult::text(with(meta, json!({ "body": d.body }))))
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
            // The boot call every participant makes, so each row is kept as
            // small as it can be without losing a decision signal (F17, week
            // 35: ~490 bytes a row, 25–28 KB a call, 14 of 14 boots). Shape:
            // `{project, file_path, description, bytes, updated_at, tags?,
            // abs_path?}`.
            //
            // `abs_path` is the RESOLVED on-disk location — agents were joining
            // `_globals` into the path themselves and constructing
            // `<library>/_globals/<file>`, which does not exist (root-level
            // files live directly under the library root). It rides a row only
            // when the file is NOT at the derivable default
            // `<library>/projects/<project>/<file_path>`: every `_globals` row,
            // and any project with a custom `projects.cl_path`. The tool
            // description states that rule. Kept inline here rather than shared
            // with `tauri_cmd/cl.rs`, whose always-present `abs_path` the
            // @-mention list filters on.
            //
            // `bytes` is a stat at call time — `cl_index` carries no size and a
            // stat is always current — so a reader can weigh a whole-file
            // `Read` before paying for it; a row whose file is gone omits it.
            let default_root = bridge
                .data_dir()
                .map(|d| crate::paths::Paths::for_data_dir(d.clone()));
            let mut roots: std::collections::HashMap<String, Option<std::path::PathBuf>> =
                std::collections::HashMap::new();
            let mut trimmed: Vec<serde_json::Value> = Vec::with_capacity(rows.len());
            for r in rows {
                if !roots.contains_key(&r.project_id) {
                    let root = bridge.cl_project_root(&r.project_id).await;
                    roots.insert(r.project_id.clone(), root);
                }
                let root = roots.get(&r.project_id).and_then(|root| root.as_ref());
                let resolved = root.map(|root| root.join(&r.file_path));
                let derivable = default_root
                    .as_ref()
                    .map(|p| p.project_dir(&r.project_id).join(&r.file_path));
                let abs_path = match (&resolved, &derivable) {
                    (Some(real), Some(default)) if real == default => None,
                    (Some(real), _) => Some(real.display().to_string()),
                    (None, _) => None,
                };
                let bytes = match &resolved {
                    Some(p) => tokio::fs::metadata(p).await.ok().map(|m| m.len()),
                    None => None,
                };
                let mut row = serde_json::json!({
                    "project": r.project_id,
                    "file_path": r.file_path,
                    // 160 rather than the primer's `CL_PRIMER_DESC_MAX` (100):
                    // the primer is a fixed prompt block, this is the per-call
                    // payload where the description IS the decision signal —
                    // the cap only guards against a fat project's H1s.
                    "description": crate::core::session::truncate_chars(&r.description, 160),
                    "updated_at": whole_seconds(&r.updated_at),
                });
                if let Some(b) = bytes {
                    row["bytes"] = serde_json::json!(b);
                }
                if let Some(tags) = r.tags.as_deref().filter(|t| !t.trim().is_empty()) {
                    row["tags"] = serde_json::json!(tags);
                }
                if let Some(p) = abs_path {
                    row["abs_path"] = serde_json::json!(p);
                }
                trimmed.push(row);
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
            // `content` inline; or `content_path`: a body the agent built on
            // disk (feedback #30 — a 610-line file re-emitted seven times to
            // change a few lines each); or `session_doc`: one of THIS session's
            // documents, copied byte for byte (feedback #78 — promoting two
            // docs at a close meant re-typing 20k characters of TeX by hand,
            // and nothing could check the copy). Exactly one of the three.
            let sources = (
                args.get("content").and_then(Value::as_str),
                args.get("content_path").and_then(Value::as_str),
                args.get("session_doc").and_then(Value::as_str),
            );
            let content = match sources {
                (Some(c), None, None) => c.to_string(),
                (None, Some(p), None) => {
                    match read_content_path(p, CONTENT_PATH_MAX_BYTES, CL_CONTENT_PATH_ADVICE).await {
                        Ok(c) => c,
                        Err(e) => return Ok(ToolCallResult::error(e)),
                    }
                }
                (None, None, Some(slug)) => {
                    match bridge
                        .session_doc_read(&caller.session_id, slug)
                        .await
                        .map_err(internal_err_no_prefix)?
                    {
                        Some(doc) => doc.body,
                        None => {
                            return Ok(ToolCallResult::error(format!(
                                "session_doc `{slug}` is not a document of this session — \
                                 session_doc_search lists them"
                            )))
                        }
                    }
                }
                (None, None, None) => {
                    return Ok(ToolCallResult::error(
                        "missing `content` (or `content_path` to read the body from a file, or \
                         `session_doc` to copy one of this session's documents)"
                            .to_string(),
                    ))
                }
                _ => {
                    return Ok(ToolCallResult::error(
                        "pass exactly one of `content`, `content_path` and `session_doc`"
                            .to_string(),
                    ))
                }
            };
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
            // Feedback #64: lets a replace drop lines another writer added
            // since this session last wrote the whole file.
            let confirm_overwrite = args
                .get("confirm_overwrite")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let msg = bridge
                .cl_write_file_with(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    project,
                    file_path,
                    content,
                    append,
                    confirm_shrink,
                    confirm_overwrite,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(msg))
        }
        "cl_edit_file" => {
            let project = arg_required_str(&args, "project")?;
            let file_path = arg_required_str(&args, "file_path")?;
            let old_string = arg_required_str(&args, "old_string")?;
            let new_string = arg_required_str(&args, "new_string")?;
            let expect = match args.get("expect_occurrences") {
                None | Some(Value::Null) => 1,
                Some(v) => match v.as_u64() {
                    Some(n) if n >= 1 => n as usize,
                    _ => {
                        return Ok(ToolCallResult::error(
                            "expect_occurrences must be a positive integer".to_string(),
                        ))
                    }
                },
            };
            let confirm_shrink = args
                .get("confirm_shrink")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let msg = bridge
                .cl_edit_file(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    project,
                    file_path,
                    old_string,
                    new_string,
                    expect,
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
            // F10: an agent's folder description (and tags) is redacted HERE,
            // at the agent's tool: the bridge function below is shared with the
            // Context Library tab (`tauri_cmd::cl::cl_set_folder_description`),
            // which keeps what the user types as written.
            let description = crate::policy::secret_scan::redact_string(arg_required_str(&args, "description")?);
            let tags = arg_opt_str(&args, "tags").map(crate::policy::secret_scan::redact_string);
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
            // `capture_main_window` sleeps 150 ms and waits on a `screencapture`
            // child — blocking work, on a 2-worker runtime; off the reactor.
            let handle = handle.clone();
            let data_dir = data_dir.to_path_buf();
            let path = tokio::task::spawn_blocking(move || {
                crate::tauri_cmd::screenshot::capture_main_window(&handle, &data_dir)
            })
            .await
            .map_err(|e| {
                JsonRpcError::new(
                    JsonRpcError::INTERNAL_ERROR,
                    format!("screenshot task failed: {e}"),
                )
            })?
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
    /// The seeded-pair fixture for `peer_shaped_reason` — the roster shape
    /// every pre-Batch-6 assertion was written against. Batch 6 made the
    /// vocabulary roster-derived; these tests pin that the HANDS/EYES world
    /// behaves exactly as before, and the roster test below pins the new
    /// recall (a renamed roster fires too).
    fn peer_shaped_reason_pair(reason: &str) -> Option<String> {
        super::peer_shaped_reason(
            reason,
            &[
                ("HANDS".to_string(), "hands".to_string()),
                ("EYES".to_string(), "eyes".to_string()),
            ],
        )
    }

    /// **A renamed roster fires the guard** (Batch 6 M6 — the promotion's
    /// whole point): the deadlock is a property of any multi-participant
    /// session, and the neutral-default work creates exactly the population
    /// whose roles are NOT named HANDS/EYES.
    #[test]
    fn the_peer_guard_speaks_the_rosters_own_names() {
        let roster = vec![
            ("PILOT".to_string(), "pilot".to_string()),
            ("NAVIGATOR".to_string(), "navigator".to_string()),
        ];
        assert_eq!(
            super::peer_shaped_reason("blocked until PILOT answers", &roster),
            Some("pilot".to_string())
        );
        assert_eq!(
            super::peer_shaped_reason("waiting on @navigator to review", &roster),
            Some("navigator".to_string())
        );
        // The English-noun filter still applies to the renamed slug.
        assert_eq!(
            super::peer_shaped_reason("the autopilot needs your sign-off", &roster),
            None
        );
        // And a HANDS mention means nothing to a roster that has no HANDS.
        assert_eq!(
            super::peer_shaped_reason("blocked until HANDS answers", &roster),
            None
        );
    }

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
            agent: "hands".into(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::preset_hands(),
            ),
        }
    }

    /// An EYES caller. See [`caller`] for why the preset is trustworthy here.
    fn eyes_caller() -> CallerIdentity {
        CallerIdentity {
            session_id: "s1".into(),
            agent: "eyes".into(),
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

    /// The wire for feedback #88: `tools/list` answers per CALLER, with the
    /// always-load mark on the core tools that caller can use, so claude-code
    /// keeps them in context without a `ToolSearch` round trip. Swapping the
    /// handler back to the bare `tool_descriptors()` turns this red.
    #[tokio::test]
    async fn tools_list_marks_the_callers_core_tools_always_loaded() {
        let bridge = SignalingBridge::new();
        let marked = |who: CallerIdentity| {
            let bridge = &bridge;
            async move {
                let res = dispatch(req("tools/list", json!({}), 1), &who, bridge).await.unwrap().unwrap();
                let v = serde_json::to_value(&res).unwrap();
                v["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|t| t["_meta"]["anthropic/alwaysLoad"] == json!(true))
                    .map(|t| t["name"].as_str().unwrap().to_string())
                    .collect::<Vec<_>>()
            }
        };
        let hands = marked(caller()).await;
        let eyes = marked(eyes_caller()).await;
        for core in ["pass_turn", "session_doc_write", "cl_retrieve", "gate_status"] {
            assert!(hands.iter().any(|n| n == core) && eyes.iter().any(|n| n == core), "{core}");
        }
        // What only one of them can use is marked only for that one.
        assert!(hands.iter().any(|n| n == "action_gate") && !eyes.iter().any(|n| n == "action_gate"));
        assert!(hands.iter().any(|n| n == "ask_user_choice") && !eyes.iter().any(|n| n == "ask_user_choice"));
        assert!(eyes.iter().any(|n| n == "flag_finding") && !hands.iter().any(|n| n == "flag_finding"));
        // The rest of the registry stays deferred for both.
        for deferred in ["webview_click", "cl_rescan", "close_session", "file_feedback"] {
            assert!(!hands.iter().any(|n| n == deferred) && !eyes.iter().any(|n| n == deferred), "{deferred}");
        }
    }

    /// **WS3 (2026-08-27): open advisories are surfaced once at close.** 92% of
    /// advisories (174/189 lifetime) died undispositioned with their sessions
    /// before this — filed, never seen again. Since 0081 the surfacing rides
    /// the close card's recap (a count the user reads, the uids in the
    /// executor's detail) instead of costing its own refusal turn; a clean
    /// session's card carries no such line.
    #[tokio::test]
    async fn close_surfaces_open_advisories_once_then_proceeds() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage
            .insert_finding(
                "s1", "uid-a1", "eyes",
                crate::storage::FindingSeverity::Advisory,
                "the draft overstates the backlog claim", None,
            )
            .await
            .unwrap();
        storage
            .insert_finding(
                "s1", "uid-a2", "eyes",
                crate::storage::FindingSeverity::Advisory,
                "provenance missing on the force_all paragraph", None,
            )
            .await
            .unwrap();
        // A blocking finding must NOT appear in the advisory listing — it has
        // its own mechanical gate.
        storage
            .insert_finding(
                "s1", "uid-b1", "eyes",
                crate::storage::FindingSeverity::Blocking,
                "wrong constant", None,
            )
            .await
            .unwrap();

        // 0081: the delta nudge is the one refusal; the advisory listing now
        // rides the close CARD's recap (a count on the card, the uids in the
        // executor's detail) instead of costing its own turn.
        let first = close_text(&bridge, 1).await;
        assert!(first.contains("learnings delta"), "the delta nudge comes first; got: {first}");
        let second = close_text(&bridge, 2).await;
        assert!(second.contains("close PARKED"), "got: {second}");
        assert!(
            second.contains("2 open advisory finding(s)")
                && second.contains("uid-a1")
                && second.contains("uid-a2"),
            "the card's recap counts the open advisories and the detail names them; got: {second}"
        );
        assert!(
            !second.contains("uid-b1"),
            "blocking findings are not advisories and have their own gate; got: {second}"
        );
        let card = pending_close_card(&storage).await.expect("card parked");
        assert!(card.prompt.contains("2 open advisory finding(s)"), "the user reads the count on the card");

        // A session with NO open advisories gets a recap without the line.
        storage.create_session("s2", "t", None).await.unwrap();
        let clean_caller = CallerIdentity {
            session_id: "s2".into(),
            agent: "hands".into(),
            capabilities: caller().capabilities,
        };
        for n in [3, 4] {
            let _ = dispatch(
                req("tools/call", json!({"name": "close_session", "arguments": {}}), n),
                &clean_caller,
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
        }
        let clean_card = storage
            .tray_entries_for_session("s2")
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.status == "pending" && r.kind == "close")
            .expect("the clean session's card parked on the second call");
        assert!(
            !clean_card.prompt.contains("advisory finding"),
            "no open advisories → no line; got: {}",
            clean_card.prompt
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
                assert_eq!(agent, "hands");
                assert!(!archive);
            }
            other => panic!("expected SessionCloseRequest, got {other:?}"),
        }
    }

    /// Dispatch `close_session` for the default caller and return the tool text.
    async fn close_text(bridge: &std::sync::Arc<SignalingBridge>, id: i64) -> String {
        let res = dispatch(
            req("tools/call", json!({"name": "close_session", "arguments": {}}), id),
            &caller(),
            bridge,
        )
        .await
        .unwrap()
        .unwrap();
        serde_json::to_value(&res).unwrap()["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// The one pending `close` card in s1, if any.
    async fn pending_close_card(storage: &crate::storage::Storage) -> Option<crate::storage::SessionTrayEntry> {
        storage
            .tray_entries_for_session("s1")
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.status == "pending" && r.kind == "close")
    }

    /// Approve a close card the way the user's click does, at `answered_at = now`.
    async fn approve_close_card(storage: &crate::storage::Storage, choice_id: &str) {
        storage
            .insert_tray_entry(
                "s1",
                choice_id,
                "hands",
                crate::storage::QuestionKind::Close,
                "Close this session?",
                Some(&["Approve".to_string(), "Reject".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(storage.answer_tray_entry(choice_id, "Approve").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn close_session_nudges_for_cl_delta_then_parks_a_close_card() {
        // A3b kept (EYES P6): with storage wired and no CL write this session,
        // the FIRST close returns the learnings nudge and requests nothing. The
        // SECOND — 0081 — parks a close CARD instead of closing: no user Approve
        // exists since the session opened. The card's Approve closes.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s1", "close", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;
        let mut sub = bridge.subscribe();

        let first = close_text(&bridge, 1).await;
        assert!(first.contains("learnings"), "first close must nudge for the learnings delta");
        assert!(sub.try_recv().is_err(), "nudged close must NOT request session close");

        let second = close_text(&bridge, 2).await;
        assert!(second.contains("close PARKED"), "got: {second}");
        assert!(second.contains("CL learnings delta: NOT written"), "got: {second}");
        assert!(!second.contains("close requested"));
        let card = pending_close_card(&storage).await.expect("a pending close card");
        assert!(card.prompt.contains("Close this session?"));
        // A gate: it seeds the latch and takes the gate slot.
        assert_eq!(storage.pending_gate_ids("s1").await.unwrap(), vec![card.choice_id.clone()]);
        // Nothing but the card's PendingChoice event went out — no close request.
        let mut close_requests = 0;
        while let Ok(ev) = sub.try_recv() {
            if matches!(ev, SignalingEvent::SessionCloseRequest { .. }) {
                close_requests += 1;
            }
        }
        assert_eq!(close_requests, 0, "parking the card must not close");

        // A third call while the card is up returns the SAME card, no second one.
        let third = close_text(&bridge, 3).await;
        assert!(third.contains(&card.choice_id), "got: {third}");
        assert_eq!(
            storage.tray_entries_for_session("s1").await.unwrap().iter().filter(|r| r.kind == "close").count(),
            1
        );

        // The user's Approve on the card closes the session — the same event the
        // direct close emits, so D15's epilogue path is unchanged.
        bridge.resolve_choice(&card.choice_id, "Approve".into()).await.unwrap();
        let mut saw_close = false;
        while let Ok(ev) = sub.try_recv() {
            if let SignalingEvent::SessionCloseRequest { session_id, agent, archive } = ev {
                assert_eq!(session_id, "s1");
                assert_eq!(agent, "hands");
                assert!(!archive);
                saw_close = true;
            }
        }
        assert!(saw_close, "Approve on the close card must request the close");
    }

    #[tokio::test]
    async fn an_approve_since_open_lets_an_agent_close_through_without_a_card() {
        // P3's regression case too: `reopened_at` is NULL on a never-reopened
        // session, and COALESCE makes the bound `created_at` — a scalar max()
        // would have made it NULL and parked a card on every close.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s1", "close", None).await.unwrap();
        assert!(storage.get_session("s1").await.unwrap().unwrap().reopened_at.is_none());
        // A CL write this session lifts the delta nudge; the card is the only
        // thing left between the agent and the close.
        bridge.set_storage(storage.clone()).await;
        approve_close_card(&storage, "card-1").await;
        // Skip the delta nudge for this test by taking it once.
        let _ = close_text(&bridge, 1).await;
        let mut sub = bridge.subscribe();
        let text = close_text(&bridge, 2).await;
        assert!(text.contains("close requested"), "got: {text}");
        assert!(matches!(sub.recv().await.unwrap(), SignalingEvent::SessionCloseRequest { .. }));
        assert!(pending_close_card(&storage).await.is_none());
    }

    #[tokio::test]
    async fn an_approve_given_before_a_reopen_does_not_count() {
        // s-34a1b88e 08:56, mechanised: the user approved a close, reopened the
        // session, and the agent re-used the old approval. The bound moves to
        // `reopened_at`, so the stale Approve is void and a fresh card parks.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s1", "close", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;
        approve_close_card(&storage, "card-old").await;
        // The close happened, then the user reopened — strictly after the Approve.
        storage.close_session("s1", false).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        assert!(storage.reopen_session("s1").await.unwrap());
        let reopened = storage.get_session("s1").await.unwrap().unwrap();
        let reopened_at = reopened.reopened_at.clone().expect("reopen stamps reopened_at");
        let old = storage.get_tray_entry("card-old").await.unwrap().unwrap();
        assert!(old.answered_at.unwrap() < reopened_at, "fixture: the Approve predates the reopen");
        assert!(!storage.close_approved_since_open("s1").await.unwrap());

        let _ = close_text(&bridge, 1).await; // the delta nudge
        let mut sub = bridge.subscribe();
        let text = close_text(&bridge, 2).await;
        assert!(text.contains("close PARKED"), "a pre-reopen Approve must not close: {text}");
        let mut close_requests = 0;
        while let Ok(ev) = sub.try_recv() {
            if matches!(ev, SignalingEvent::SessionCloseRequest { .. }) {
                close_requests += 1;
            }
        }
        assert_eq!(close_requests, 0, "the stale Approve must not close the session");
        assert!(pending_close_card(&storage).await.is_some());
    }

    #[tokio::test]
    async fn the_staleness_sweep_rides_the_close_card_recap_instead_of_costing_a_turn() {
        // #31's sweep and WS3's advisory list used to be two refusals before the
        // delta nudge — three turns to close. Since 0081 they are lines on the
        // close card's recap (F5: the sweep demoted to a report line) and the
        // executor gets the hit list in the same tool result.
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("library/projects/bot-hq");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("conventions.md"), "The duo maintains this.\n").unwrap();
        std::fs::write(proj.join("vision.md"), "The `duo` is the core of it.\n").unwrap();
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
        bridge.set_storage(storage.clone()).await;
        let mut sub = bridge.subscribe();

        bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "vision.md".to_string(),
                "The harness is the core of it, restated at a similar length.".to_string(),
                false,
                false,
            )
            .await
            .unwrap();

        // The CL write lifted the delta nudge, so the FIRST close parks the card.
        let text = close_text(&bridge, 1).await;
        assert!(text.contains("close PARKED"), "got: {text}");
        assert!(text.contains("Staleness sweep"), "the card recap names the sweep: {text}");
        assert!(text.contains("conventions.md:1"), "the executor gets the hit list: {text}");
        assert!(text.contains("CL learnings delta: written"), "got: {text}");
        let mut close_requests = 0;
        while let Ok(ev) = sub.try_recv() {
            if matches!(ev, SignalingEvent::SessionCloseRequest { .. }) {
                close_requests += 1;
            }
        }
        assert_eq!(close_requests, 0, "the card parks; nothing closes yet");
        let card = pending_close_card(&storage).await.expect("card parked");
        assert!(card.prompt.contains("Staleness sweep"), "the user reads the sweep line on the card");
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
    async fn eyes_is_refused_a_tool_its_capabilities_do_not_grant() {
        let bridge = SignalingBridge::new();
        for tool in &[
            "mark_awaiting_user",
            "ask_user_choice",
            "request_approval",
            "action_gate",
            "halt",
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
                &eyes_caller(),
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
            let v = serde_json::to_value(&res).unwrap();
            assert_eq!(
                v["result"]["isError"],
                json!(true),
                "tool {tool} should return is_error=true for eyes"
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
        storage.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();

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
    async fn hands_rejected_from_eyes_only_eyes_flag() {
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
    async fn eyes_rejected_from_disposition_finding() {
        // disposition_finding needs a capability EYES does not hold (pre-rc3 it
        // sat in the deleted HANDS_ONLY_TOOLS) — EYES is rejected.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "disposition_finding", "arguments": {"finding_id": "f1", "status": "fixed", "reason": "x"}}),
                1,
            ),
            &eyes_caller(),
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
    async fn hands_rejected_from_approve_finding() {
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
            &eyes_caller(),
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

    /// **The peer-word refusal lives on `mark_awaiting_user` and NOT on `halt`
    /// — pinned at the wire** (round 9, review advisory 60b84947). Both
    /// descriptors now state it: `mark_awaiting_user` refuses a reason naming a
    /// peer and names `halt` as the path when a legitimate user-wait must
    /// mention a role. `peer_shaped_reason` was tested in isolation and neither
    /// dispatch test reached the branch, so the refusal could be deleted — or
    /// added to `halt`, which round 9 nearly did (a refused halt parks nothing:
    /// an undeclared stall) — with a green suite while two descriptors said
    /// otherwise. Same class as E1: helper pinned, wire not.
    #[tokio::test]
    async fn the_peer_word_refusal_guards_mark_awaiting_user_and_not_halt() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let reason = "waiting on the user to approve the override of EYES' blocking finding";
        // mark_awaiting_user: an error, no halt declared.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "mark_awaiting_user", "arguments": {"reason": reason}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true), "must be refused: {v}");
        assert!(
            v["result"]["content"][0]["text"].as_str().unwrap_or("").contains("names your peer"),
            "the refusal names the reason: {v}"
        );
        assert!(
            sub.try_recv().is_err(),
            "a refused mark_awaiting_user must not declare a halt"
        );
        // halt: the same reason declares the halt.
        let res = dispatch(
            req("tools/call", json!({"name": "halt", "arguments": {"reason": reason}}), 2),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(
            v["result"]["content"][0]["text"].as_str().unwrap_or("").contains("halted"),
            "halt carries no peer-word filter: {v}"
        );
        let ev = sub.recv().await.unwrap();
        assert!(matches!(ev, SignalingEvent::AwaitingUser { reason: r, .. } if r == reason));
    }

    /// Round 12 Q2 — a TEMPORARY halt at the wire: `wake_after_secs` inside
    /// 10…3600 declares the halt with a wake instant (the ack says so, and the
    /// `AwaitingUser` event still fires — the banner is the same slot); outside
    /// the range, or not an integer, it is INVALID_PARAMS and NO halt is
    /// declared — an out-of-range wait is a typo, and a halt declared on a typo
    /// would wedge the session on the wrong clock. Both tools take it.
    #[tokio::test]
    async fn wake_after_secs_makes_a_temporary_halt_and_an_out_of_range_value_declares_nothing() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        for (tool, secs) in [("mark_awaiting_user", 10u64), ("halt", 3600)] {
            let res = dispatch(
                req(
                    "tools/call",
                    json!({"name": tool, "arguments": {"reason": "CI on PR #531", "wake_after_secs": secs}}),
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
            assert!(
                text.contains("TEMPORARY") && text.contains(&format!("{secs}s")),
                "{tool}: the ack names the temporary halt and its wait: {v}"
            );
            let ev = sub.recv().await.unwrap();
            assert!(
                matches!(ev, SignalingEvent::AwaitingUser { reason, .. } if reason == "CI on PR #531"),
                "{tool}: a temporary halt is still a declared halt"
            );
        }
        for (tool, bad) in [
            ("mark_awaiting_user", json!(5)),
            ("halt", json!(7200)),
            ("mark_awaiting_user", json!("soon")),
        ] {
            let err = dispatch(
                req(
                    "tools/call",
                    json!({"name": tool, "arguments": {"reason": "CI", "wake_after_secs": bad}}),
                    2,
                ),
                &caller(),
                &bridge,
            )
            .await
            .unwrap_err();
            assert_eq!(err.code, JsonRpcError::INVALID_PARAMS, "{tool} {bad}");
            assert!(err.message.contains("wake_after_secs"), "{tool}: {}", err.message);
            assert!(sub.try_recv().is_err(), "{tool}: a refused wait declares no halt");
        }
        // `null` is "no wait": an ordinary halt.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "halt", "arguments": {"reason": "plain", "wake_after_secs": null}}),
                3,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(
            !v["result"]["content"][0]["text"].as_str().unwrap_or("").contains("TEMPORARY"),
            "null is an ordinary halt: {v}"
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

    /// **The agent-facing `cl_index_search` row is the slim shape** (F17, week
    /// 35). Nothing pinned it before this: the Tauri twin and the plugin
    /// dispatch each have tests, the payload every participant's boot call
    /// reads had none. Each assertion goes red when its conditional inverts —
    /// `abs_path` back on every row, `bytes` missing, tags as `null`, a
    /// nanosecond timestamp, an uncapped description.
    #[tokio::test]
    async fn cl_index_search_rows_are_slim_and_carry_abs_path_only_off_the_default_root() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("library");
        std::fs::create_dir_all(lib.join("projects/bot-hq")).unwrap();
        std::fs::write(lib.join("projects/bot-hq/notes.md"), "0123456789").unwrap();
        std::fs::write(lib.join("eod.md"), "eod body").unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("custom.md"), "custom root").unwrap();

        let bridge = SignalingBridge::new_with(None, Some(tmp.path().to_path_buf()));
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        storage
            .upsert_project("custom", "custom", None, None, Some(elsewhere.to_str().unwrap()))
            .await
            .unwrap();
        let long = "a".repeat(200);
        storage
            .upsert_cl_index("bot-hq", "notes.md", &long, None)
            .await
            .unwrap();
        storage
            .upsert_cl_index("bot-hq", "gone.md", "deleted on disk", None)
            .await
            .unwrap();
        storage
            .upsert_cl_index("_globals", "eod.md", "EOD", Some("eod, daily"))
            .await
            .unwrap();
        storage
            .upsert_cl_index("custom", "custom.md", "custom", None)
            .await
            .unwrap();
        bridge.set_storage(storage).await;

        let rows = |v: &Value| -> Vec<Value> {
            let text = v["result"]["content"][0]["text"].as_str().unwrap().to_string();
            serde_json::from_str(&text).unwrap()
        };
        let find = |rows: &[Value], project: &str, file: &str| -> Value {
            rows.iter()
                .find(|r| r["project"] == project && r["file_path"] == file)
                .cloned()
                .unwrap_or_else(|| panic!("no row for {project}/{file}"))
        };

        let res = dispatch(
            req("tools/call", json!({"name": "cl_index_search", "arguments": {"project": "bot-hq"}}), 1),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let rows_bot = rows(&serde_json::to_value(&res).unwrap());

        // A project row at the default root: derivable, so no `abs_path`;
        // `bytes` is the on-disk size; null tags are absent, not `null`.
        let notes = find(&rows_bot, "bot-hq", "notes.md");
        assert!(notes.get("abs_path").is_none(), "default-root row carries no abs_path: {notes}");
        assert_eq!(notes["bytes"], json!(10));
        assert!(notes.get("tags").is_none(), "null tags are omitted: {notes}");
        // The description cap is a char count with an ellipsis, not a byte slice.
        let desc = notes["description"].as_str().unwrap();
        assert_eq!(desc.chars().count(), 161, "160 chars + `…`: {desc}");
        assert!(desc.ends_with('…'));
        // Whole seconds, UTC `Z`, no fraction — `now_utc()` wrote millis.
        let ts = notes["updated_at"].as_str().unwrap();
        assert!(
            !ts.contains('.') && ts.ends_with('Z') && ts.len() == 20,
            "updated_at is trimmed to whole seconds: {ts}"
        );
        // A row whose file is gone says so by omission.
        let gone = find(&rows_bot, "bot-hq", "gone.md");
        assert!(gone.get("bytes").is_none(), "missing file ⇒ no bytes: {gone}");

        // A `_globals` row rides along and keeps `abs_path` — the root-level
        // location is exactly the one agents used to construct wrongly.
        let eod = find(&rows_bot, "_globals", "eod.md");
        assert_eq!(eod["abs_path"], json!(lib.join("eod.md").display().to_string()));
        assert_eq!(eod["bytes"], json!(8));
        assert_eq!(eod["tags"], json!("eod, daily"));

        // A project with a custom library path is NOT derivable, so it keeps it.
        let res = dispatch(
            req("tools/call", json!({"name": "cl_index_search", "arguments": {"project": "custom"}}), 2),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let rows_custom = rows(&serde_json::to_value(&res).unwrap());
        let custom = find(&rows_custom, "custom", "custom.md");
        assert_eq!(
            custom["abs_path"],
            json!(elsewhere.join("custom.md").display().to_string()),
            "custom cl_path root keeps abs_path"
        );
        assert_eq!(custom["bytes"], json!(11));
    }

    #[test]
    fn whole_seconds_trims_both_stored_timestamp_shapes_and_passes_junk_through() {
        assert_eq!(super::whole_seconds("2026-08-24T22:26:38.439732628+00:00"), "2026-08-24T22:26:38Z");
        assert_eq!(super::whole_seconds("2026-09-06T04:40:01.284Z"), "2026-09-06T04:40:01Z");
        assert_eq!(super::whole_seconds("2026-09-06T12:40:01+08:00"), "2026-09-06T04:40:01Z");
        assert_eq!(super::whole_seconds("not a time"), "not a time");
    }

    #[tokio::test]
    async fn cl_write_file_dispatch_writes_for_the_writer_and_denies_the_reviewer() {
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
                    "content": "eyes-authored"
                }}),
                1,
            ),
            &eyes_caller(),
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

    /// Feedback #64, through the dispatch arm: a replace that drops another
    /// session's lines is refused as a tool error naming them, and the same
    /// call with `confirm_overwrite: true` goes through — the argument reaches
    /// the guard.
    #[tokio::test]
    async fn cl_write_file_dispatch_carries_confirm_overwrite_to_the_replace_guard() {
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
        bridge
            .cl_write_file("s1".into(), "hands".into(), "bot-hq".into(), "eod.md".into(), "- mine\n".into(), false, false)
            .await
            .unwrap();
        bridge
            .cl_write_file("s-b".into(), "hands".into(), "bot-hq".into(), "eod.md".into(), "- theirs\n".into(), true, false)
            .await
            .unwrap();
        let replace = |confirm: Option<bool>, id: i64| {
            let mut args = json!({"project": "bot-hq", "file_path": "eod.md", "content": "- mine, redrafted\n"});
            if let Some(c) = confirm {
                args["confirm_overwrite"] = json!(c);
            }
            req("tools/call", json!({"name": "cl_write_file", "arguments": args}), id)
        };
        // A bridge refusal comes back as a JSON-RPC error carrying its text.
        let Err(refusal) = dispatch(replace(None, 1), &caller(), &bridge).await else {
            panic!("the replace guard refuses a drop the caller did not confirm");
        };
        assert!(refusal.message.contains("\"- theirs\" — hands, s-b"), "got: {refusal:?}");

        let res = dispatch(replace(Some(true), 2), &caller(), &bridge).await.unwrap().unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("replaced"), "got: {text}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("library/projects/bot-hq/eod.md")).unwrap(),
            "- mine, redrafted\n"
        );
    }

    /// `cl_edit_file` is gated exactly like `cl_write_file` (the user's call,
    /// tray df02d2a8): the reviewer is refused by name of the capability, the
    /// executor's edit lands through the dispatch arm.
    #[tokio::test]
    async fn cl_edit_file_dispatch_edits_for_the_writer_and_denies_the_reviewer() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("library/projects/bot-hq");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("notes.md"), "- the share is unreadable\n").unwrap();
        let log = crate::policy::ViolationsLog::new(tmp.path());
        let bridge = SignalingBridge::with_policy(log, tmp.path().to_path_buf());
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        storage.create_session("s1", "CL edit", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;

        let args = json!({
            "project": "bot-hq",
            "file_path": "notes.md",
            "old_string": "unreadable",
            "new_string": "readable since 09-01"
        });
        let res = dispatch(
            req("tools/call", json!({"name": "cl_edit_file", "arguments": args}), 1),
            &eyes_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));
        assert!(
            v["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("needs the `write_context_library` capability"),
            "got: {v}"
        );
        assert_eq!(
            std::fs::read_to_string(proj.join("notes.md")).unwrap(),
            "- the share is unreadable\n"
        );

        let res = dispatch(
            req("tools/call", json!({"name": "cl_edit_file", "arguments": args}), 2),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("edited 'notes.md'"), "got: {text}");
        assert_eq!(
            std::fs::read_to_string(proj.join("notes.md")).unwrap(),
            "- the share is readable since 09-01\n"
        );

        // A non-positive expectation is refused at the dispatch boundary.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_edit_file", "arguments": {
                    "project": "bot-hq", "file_path": "notes.md",
                    "old_string": "a", "new_string": "b", "expect_occurrences": 0
                }}),
                3,
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
            .unwrap()
            .contains("positive integer"));
    }

    /// `content_path` hands `cl_write_file` a body built on disk (feedback
    /// #30): exactly one of `content` / `content_path`, an absolute path, and
    /// the 1 MiB cap is applied by stat BEFORE the file is read.
    #[tokio::test]
    async fn cl_write_file_dispatch_reads_a_content_path_under_the_cap() {
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
        bridge.set_storage(storage).await;

        let built = tmp.path().join("built.md");
        std::fs::write(&built, "# built on disk\nbody\n").unwrap();
        let call = |args: Value, id: i64| {
            req("tools/call", json!({"name": "cl_write_file", "arguments": args}), id)
        };
        let text_of = |res: JsonRpcResponse| -> (bool, String) {
            let v = serde_json::to_value(&res).unwrap();
            (
                v["result"]["isError"] == json!(true),
                v["result"]["content"][0]["text"].as_str().unwrap().to_string(),
            )
        };

        // The happy path: the body is the file's text.
        let res = dispatch(
            call(
                json!({"project": "bot-hq", "file_path": "notes.md",
                       "content_path": built.to_str().unwrap()}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let (is_err, text) = text_of(res);
        assert!(!is_err && text.starts_with("created"), "got: {text}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("library/projects/bot-hq/notes.md")).unwrap(),
            "# built on disk\nbody\n"
        );

        // Both, neither, and a relative path are refused.
        for (args, needle) in [
            (
                json!({"project": "bot-hq", "file_path": "notes.md",
                       "content": "x", "content_path": built.to_str().unwrap()}),
                // Three sources since `session_doc` (feedback #78): the
                // refusal names all three.
                "exactly one of `content`, `content_path` and `session_doc`",
            ),
            (json!({"project": "bot-hq", "file_path": "notes.md"}), "missing `content`"),
            (
                json!({"project": "bot-hq", "file_path": "notes.md", "content_path": "built.md"}),
                "absolute path",
            ),
        ] {
            let res = dispatch(call(args, 2), &caller(), &bridge).await.unwrap().unwrap();
            let (is_err, text) = text_of(res);
            assert!(is_err && text.contains(needle), "got: {text}");
        }

        // Over the cap: refused by stat, so the reply names the size and the
        // body was never read (a multi-GB path would otherwise be loaded into
        // the one process that also runs the UI).
        let huge = tmp.path().join("huge.md");
        let f = std::fs::File::create(&huge).unwrap();
        f.set_len(2 * 1024 * 1024).unwrap();
        let res = dispatch(
            call(
                json!({"project": "bot-hq", "file_path": "notes.md",
                       "content_path": huge.to_str().unwrap()}),
                3,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let (is_err, text) = text_of(res);
        assert!(is_err && text.contains("2097152 bytes") && text.contains("1 MiB"), "got: {text}");
    }

    #[tokio::test]
    async fn peer_ack_allowed_for_either_agent() {
        // peer_ack is NOT role-gated — both HANDS and EYES converge via it. (The
        // real suppression happens in the duo pump; here we just assert the
        // dispatch accepts the call from either agent.)
        let bridge = SignalingBridge::new();
        for c in [caller(), eyes_caller()] {
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
        for c in [caller(), eyes_caller()] {
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
        for c in [caller(), eyes_caller()] {
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
        // The contract changed with the vote: an unconditional "phase advanced"
        // was true only while an advance always happened. Here the bridge has no
        // storage, so it keeps the old fire-and-forget path and DOES advance.
        assert_eq!(
            v["result"]["content"][0]["text"],
            "ADVANCED — the session is now in Apply."
        );
        let ev = sub.recv().await.unwrap();
        match ev {
            SignalingEvent::AgentAdvancePhase { target, agent, .. } => {
                assert_eq!(target, "Apply");
                assert_eq!(agent, "hands");
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
    async fn eyes_can_self_advance_phase() {
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
            &eyes_caller(),
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
    async fn eyes_can_call_request_phase_advance() {
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
            &eyes_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(
            v["result"]["isError"],
            json!(false),
            "eyes should be allowed to call request_phase_advance"
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
        // The contract changed with the vote: an unconditional "phase advanced"
        // was true only while an advance always happened. Here the bridge has no
        // storage, so it keeps the old fire-and-forget path and DOES advance.
        assert_eq!(
            v["result"]["content"][0]["text"],
            "ADVANCED — the session is now in Apply."
        );
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

    /// Feedback #37: a bare `session_doc_search` hides archived versions; the
    /// flag (or an `@` in the query) brings them back; `session_doc_read`'s
    /// `grep` returns only matching lines.
    #[tokio::test]
    async fn doc_search_hides_archives_and_read_greps() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        let call = |name: &str, args: Value| {
            let bridge = &bridge;
            let r = req("tools/call", json!({ "name": name, "arguments": args }), 1);
            async move {
                let out = dispatch(r, &caller(), bridge).await.unwrap().unwrap();
                let v = serde_json::to_value(&out).unwrap();
                v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()
            }
        };
        call("session_doc_write", json!({"slug": "eod", "body": "Dug into it.\nsecond"})).await;
        call("session_doc_write", json!({"slug": "eod", "body": "Confirmed it.\nsecond"})).await;
        let bare = call("session_doc_search", json!({})).await;
        assert!(bare.contains("\"eod\"") && !bare.contains("eod@1"), "got: {bare}");
        let all = call("session_doc_search", json!({"include_archives": true})).await;
        assert!(all.contains("eod@1"), "got: {all}");
        let at = call("session_doc_search", json!({"query": "eod@"})).await;
        assert!(at.contains("eod@1"), "got: {at}");
        let hit = call("session_doc_read", json!({"slug": "eod@1", "grep": "dug"})).await;
        assert!(hit.contains("\"line\":1") && !hit.contains("second"), "got: {hit}");
    }

    /// One tool call through the real handler, as `who`. `Ok` is the reply text.
    async fn tool(
        bridge: &Arc<SignalingBridge>,
        who: CallerIdentity,
        name: &str,
        args: Value,
    ) -> Result<String, JsonRpcError> {
        let r = req("tools/call", json!({ "name": name, "arguments": args }), 1);
        let out = dispatch(r, &who, bridge).await?.expect("a response");
        let v = serde_json::to_value(&out).unwrap();
        Ok(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
    }

    async fn two_role_session() -> (Arc<SignalingBridge>, crate::storage::Storage) {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        storage
            .ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS)
            .await
            .unwrap();
        (bridge, storage)
    }

    async fn doc_body(storage: &crate::storage::Storage, slug: &str) -> Option<String> {
        storage.session_document_by_slug("s1", slug).await.unwrap().map(|d| d.body)
    }

    /// EYES' advisory `9a1602f1` (s-d43b3630), through the real handler: a
    /// reviewer's write that names a phase doc or its own co-doc WITHOUT a
    /// `phase` argument used to land in the executor's phase doc while the
    /// reply named `<phase>-eyes`. Every such write now lands in the co-doc and
    /// says so; the executor's doc is untouched, and an executor's untagged
    /// write to a co-doc is refused. The round-13 case — an executor appending
    /// to its own phase doc without `phase` — still keeps the tag.
    #[tokio::test]
    async fn a_reviewers_untagged_write_never_lands_in_the_executors_phase_doc() {
        let (bridge, storage) = two_role_session().await;
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "plan", "phase": "plan", "body": "THE PLAN"}))
            .await
            .unwrap();
        tool(&bridge, eyes_caller(), "session_doc_write", json!({"slug": "plan-eyes", "phase": "plan", "body": "review one"}))
            .await
            .unwrap();

        // The natural call: extend one's own co-doc by its slug, no `phase`.
        let reply = tool(
            &bridge,
            eyes_caller(),
            "session_doc_write",
            json!({"slug": "plan-eyes", "mode": "append", "body": "review two"}),
        )
        .await
        .unwrap();
        assert!(reply.contains("\"slug\":\"plan-eyes\""), "the reply names where it wrote: {reply}");
        assert_eq!(doc_body(&storage, "plan").await.as_deref(), Some("THE PLAN"), "the executor's doc is untouched");
        let review = doc_body(&storage, "plan-eyes").await.unwrap();
        assert!(review.contains("review one") && review.contains("review two"), "both slices: {review}");

        // The same with the phase doc's own name, in replace mode.
        let reply = tool(&bridge, eyes_caller(), "session_doc_write", json!({"slug": "plan", "body": "review three"}))
            .await
            .unwrap();
        assert!(reply.contains("\"slug\":\"plan-eyes\""), "redirected, and it says so: {reply}");
        assert_eq!(doc_body(&storage, "plan").await.as_deref(), Some("THE PLAN"));
        assert!(doc_body(&storage, "plan-eyes").await.unwrap().contains("review three"));

        // A reviewer's scratch doc whose name is only a parser shorthand stays scratch.
        tool(&bridge, eyes_caller(), "session_doc_write", json!({"slug": "a", "body": "scratch"})).await.unwrap();
        assert_eq!(doc_body(&storage, "a").await.as_deref(), Some("scratch"));
        assert!(doc_body(&storage, "apply-eyes").await.is_none());

        // The mirror: the co-doc is the reviewer's.
        let refused = tool(&bridge, caller(), "session_doc_write", json!({"slug": "plan-eyes", "body": "mine now"}))
            .await
            .expect_err("an executor's untagged write to a co-doc is refused");
        assert!(refused.message.contains("holds the reviewer's notes on the plan phase"), "{}", refused.message);
        assert!(doc_body(&storage, "plan-eyes").await.unwrap().contains("review three"));

        // Round 13 stays fixed: an untagged append to one's own phase doc keeps the tag.
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "plan", "mode": "append", "body": "slice two"}))
            .await
            .unwrap();
        let plan = storage.session_document_by_slug("s1", "plan").await.unwrap().unwrap();
        assert_eq!(plan.phase.as_deref(), Some("plan"));
        assert!(plan.body.starts_with("THE PLAN") && plan.body.contains("slice two"));
    }

    /// Feedback #70 / #95, end to end: review notes are not the work. The
    /// executor votes; the reviewer records notes through the real handler; the
    /// reviewer's vote then completes the tally WITHOUT the executor voting
    /// again. Counting the co-doc in `phase_artifact_fingerprint` again turns
    /// this red.
    #[tokio::test]
    async fn a_reviewers_notes_leave_the_executors_phase_vote_standing() {
        let (bridge, _storage) = two_role_session().await;
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "investigate", "phase": "investigate", "body": "findings"}))
            .await
            .unwrap();
        let first = tool(&bridge, caller(), "advance_phase", json!({"target": "Plan"})).await.unwrap();
        assert!(first.starts_with("NOT ADVANCED") && first.contains("1 of 2"), "{first}");

        tool(&bridge, eyes_caller(), "session_doc_write", json!({"slug": "review", "phase": "investigate", "body": "two points"}))
            .await
            .unwrap();
        let second = tool(&bridge, eyes_caller(), "advance_phase", json!({"target": "Plan"})).await.unwrap();
        assert!(second.starts_with("ADVANCED"), "the executor's vote still counted: {second}");
    }

    /// …and what replaces the invalidation: the WRITER's own vote goes with its
    /// write, so a reviewer that records a new objection after voting has to
    /// vote again, and the reply tells it so. The executor's vote is not
    /// touched. Deleting the `retract_phase_votes` call in
    /// `session_doc_write_eyes` turns this red.
    #[tokio::test]
    async fn a_reviewers_own_vote_is_withdrawn_by_its_later_notes() {
        let (bridge, storage) = two_role_session().await;
        let hands = storage.participant_by_slug("s1", "hands").await.unwrap().unwrap();
        let voted = tool(&bridge, eyes_caller(), "advance_phase", json!({"target": "Plan"})).await.unwrap();
        assert!(voted.contains("1 of 2"), "{voted}");

        let reply = tool(&bridge, eyes_caller(), "session_doc_write", json!({"slug": "x", "phase": "investigate", "body": "wait: a new objection"}))
            .await
            .unwrap();
        assert!(reply.contains("your own phase vote was withdrawn"), "the reviewer is told: {reply}");

        // The executor's vote alone does not complete a tally the reviewer left.
        let out = tool(&bridge, caller(), "advance_phase", json!({"target": "Plan"})).await.unwrap();
        assert!(out.starts_with("NOT ADVANCED") && out.contains("1 of 2"), "{out}");
        // A write with no standing vote says nothing about votes, and the
        // executor's ballot is still there for the reviewer's fresh one.
        let reply = tool(&bridge, eyes_caller(), "session_doc_write", json!({"slug": "x", "phase": "investigate", "mode": "append", "body": "resolved"}))
            .await
            .unwrap();
        assert!(!reply.contains("withdrawn"), "{reply}");
        let _ = hands;
        let done = tool(&bridge, eyes_caller(), "advance_phase", json!({"target": "Plan"})).await.unwrap();
        assert!(done.starts_with("ADVANCED"), "{done}");
    }

    /// EYES' plan point 8: the "review notes landed" row is one per co-doc per
    /// run of the writer's turn — five appended slices post one row — and a
    /// fresh one after anybody else has spoken.
    #[tokio::test]
    async fn review_notes_are_announced_once_per_codoc_per_run() {
        let (bridge, storage) = two_role_session().await;
        let rows = |storage: crate::storage::Storage| async move {
            storage
                .messages_for_session("s1", None)
                .await
                .unwrap()
                .into_iter()
                .filter(|m| m.kind == "system_notice" && m.content.contains("wrote review notes to"))
                .map(|m| m.content)
                .collect::<Vec<_>>()
        };
        let note = |body: &'static str, phase: &'static str| {
            let bridge = &bridge;
            async move {
                tool(bridge, eyes_caller(), "session_doc_write", json!({"slug": "r", "phase": phase, "mode": "append", "body": body}))
                    .await
                    .unwrap();
            }
        };
        note("slice 1", "plan").await;
        storage.post_to_channel("s1", "participant", Some("eyes"), "text", "Reviewing.", None).await.unwrap();
        note("slice 2", "plan").await;
        note("slice 3", "plan").await;
        let seen = rows(storage.clone()).await;
        assert_eq!(seen.len(), 1, "one row for three slices of one run: {seen:?}");
        assert!(seen[0].contains("wrote review notes to `plan-eyes`") && seen[0].contains("phase votes stand"), "{}", seen[0]);

        // Another co-doc in the same run gets its own row.
        note("apply notes", "apply").await;
        assert_eq!(rows(storage.clone()).await.len(), 2);

        // The executor speaks; the reviewer's next slice is a new run.
        storage.post_to_channel("s1", "participant", Some("hands"), "text", "Folded in.", None).await.unwrap();
        note("slice 4", "plan").await;
        assert_eq!(rows(storage.clone()).await.len(), 3, "a fresh row after someone else spoke");
    }

    fn parsed(reply: &str) -> Value {
        serde_json::from_str(reply).unwrap_or_else(|e| panic!("a JSON reply ({e}): {reply}"))
    }

    /// Feedback #75: a bare `session_doc_search` returned every doc's whole
    /// body — 476,583 characters in one session — and overflowed the tool
    /// result. Bodies now ride within a budget, newest first; a doc that does
    /// not fit is listed with its size and its outline, and every row says how
    /// big the doc is.
    #[tokio::test]
    async fn session_doc_search_returns_bodies_within_a_budget() {
        let (bridge, _storage) = two_role_session().await;
        let big: String = std::iter::once("# Apply\n".to_string())
            .chain((1..=900).map(|i| format!("## Slice {i}\nwhat slice {i} changed, in a sentence long enough to count\n")))
            .collect();
        assert!(big.len() > crate::signaling::bridge::DOC_REPLY_BUDGET_BYTES);
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "plan", "phase": "plan", "body": "# Plan\nthe plan"})).await.unwrap();
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "apply", "phase": "apply", "body": big})).await.unwrap();
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "checklist", "body": "- [ ] one"})).await.unwrap();

        let reply = tool(&bridge, caller(), "session_doc_search", json!({})).await.unwrap();
        assert!(reply.len() < 2 * crate::signaling::bridge::DOC_REPLY_BUDGET_BYTES, "the reply is bounded: {} bytes", reply.len());
        let rows = parsed(&reply);
        let row = |slug: &str| rows.as_array().unwrap().iter().find(|r| r["slug"] == slug).unwrap().clone();
        for small in ["plan", "checklist"] {
            assert!(row(small)["body"].is_string(), "{small} rides whole");
            assert!(row(small).get("note").is_none());
        }
        assert_eq!(row("plan")["bytes"], json!("# Plan\nthe plan".len()));
        assert_eq!(row("plan")["lines"], json!(2));
        let left_out = row("apply");
        assert!(left_out.get("body").is_none(), "the doc over the budget carries no body");
        assert_eq!(left_out["bytes"], json!(big.len()));
        assert_eq!(left_out["headings"][0], json!({"line": 1, "level": 1, "text": "Apply"}));
        assert_eq!(left_out["headings"][1]["text"], json!("Slice 1"));
        assert!(left_out["note"].as_str().unwrap().contains("session_doc_read(slug: \"apply\")"), "{left_out}");
        // The phase filter is the same rule: the prose that says "search by
        // phase to pull the plan" still gets the plan's body.
        let by_phase = parsed(&tool(&bridge, caller(), "session_doc_search", json!({"phase": "plan"})).await.unwrap());
        assert_eq!(by_phase[0]["body"], json!("# Plan\nthe plan"));
    }

    /// Feedback #75 / #77: one section by its heading, a line range, or — for a
    /// body over the budget with no selector — the outline in place of a spill.
    #[tokio::test]
    async fn session_doc_read_returns_a_section_a_range_or_the_outline() {
        let (bridge, _storage) = two_role_session().await;
        let big: String = std::iter::once("# Plan\nintro\n".to_string())
            .chain((1..=900).map(|i| format!("## Group {i}\ndesign of group {i}, in a sentence long enough to count\n")))
            .collect();
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "plan", "phase": "plan", "body": big})).await.unwrap();

        let section = parsed(&tool(&bridge, caller(), "session_doc_read", json!({"slug": "plan", "heading": "group 7"})).await.unwrap());
        assert_eq!(section["heading"], json!("Group 7"));
        assert_eq!(section["lines"], json!("15-16"));
        assert_eq!(section["body"], json!("## Group 7\ndesign of group 7, in a sentence long enough to count"));

        // grep inside a section.
        let hits = parsed(&tool(&bridge, caller(), "session_doc_read", json!({"slug": "plan", "heading": "Group 12", "grep": "design"})).await.unwrap());
        assert_eq!(hits["matches"].as_array().unwrap().len(), 1);
        assert_eq!(hits["matches"][0]["line"], json!(26));

        let unknown = tool(&bridge, caller(), "session_doc_read", json!({"slug": "plan", "heading": "nonesuch"}))
            .await
            .expect_err("an unknown heading is an error, not an empty body");
        assert!(unknown.message.contains("no heading in `plan` contains \"nonesuch\"") && unknown.message.contains("Group 1"), "{}", unknown.message);
        let both = tool(&bridge, caller(), "session_doc_read", json!({"slug": "plan", "heading": "Group 7", "lines": "1-3"}))
            .await
            .expect_err("heading and lines together are refused");
        assert!(both.message.contains("not both"), "{}", both.message);

        // No selector on a body over the budget: the outline, not a spill.
        let outline = parsed(&tool(&bridge, caller(), "session_doc_read", json!({"slug": "plan"})).await.unwrap());
        assert!(outline.get("body").is_none(), "no whole body over the budget");
        assert_eq!(outline["bytes"], json!(big.len()));
        assert_eq!(outline["total_lines"], json!(1802));
        assert_eq!(outline["headings"][0]["text"], json!("Plan"));
        assert!(outline["note"].as_str().unwrap().contains("`heading`"), "{outline}");

        // A range larger than the budget is cut on a line, and says where to go on.
        let cut = parsed(&tool(&bridge, caller(), "session_doc_read", json!({"slug": "plan", "lines": "1-"})).await.unwrap());
        assert!(cut["body"].as_str().unwrap().len() <= crate::signaling::bridge::DOC_REPLY_BUDGET_BYTES);
        let shown_to: usize = cut["lines"].as_str().unwrap().split('-').nth(1).unwrap().parse().unwrap();
        assert!(shown_to < 1802);
        assert!(cut["note"].as_str().unwrap().contains(&format!("Continue with lines: \"{}-\"", shown_to + 1)), "{cut}");

        // A small doc is still returned whole, as before.
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "notes", "body": "short"})).await.unwrap();
        assert_eq!(parsed(&tool(&bridge, caller(), "session_doc_read", json!({"slug": "notes"})).await.unwrap())["body"], json!("short"));
    }

    /// Feedback #58 / #77: a passage of an existing doc is corrected IN PLACE.
    /// The count is asserted, the phase tag survives, the body as it was is
    /// archived (EYES, plan point 4), and the same ownership rules as a write
    /// hold: a reviewer edits its co-doc, an executor cannot, and nobody edits
    /// a peer's handoff doc.
    #[tokio::test]
    async fn session_doc_edit_corrects_a_passage_in_place_and_archives_the_old_body() {
        let bridge = SignalingBridge::new();
        let handoffs = tempfile::tempdir().unwrap();
        let storage = crate::storage::Storage::memory().await.unwrap().with_handoff_dir(handoffs.path().to_path_buf());
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        storage.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();

        tool(&bridge, caller(), "session_doc_write", json!({"slug": "plan", "phase": "plan", "body": "E2 covers six topics.\nE3 is next.\nE2 again."}))
            .await
            .unwrap();
        let reply = parsed(&tool(&bridge, caller(), "session_doc_edit", json!({"slug": "plan", "old_string": "six topics", "new_string": "four topics"})).await.unwrap());
        assert_eq!(reply["slug"], json!("plan"));
        assert_eq!(reply["occurrences"], json!(1));
        assert_eq!(reply["previous_body"], json!("plan@1"), "the body as it was is archived");
        let plan = storage.session_document_by_slug("s1", "plan").await.unwrap().unwrap();
        assert_eq!(plan.body, "E2 covers four topics.\nE3 is next.\nE2 again.");
        assert_eq!(plan.phase.as_deref(), Some("plan"), "the phase tag survives an edit");
        assert_eq!(doc_body(&storage, "plan@1").await.as_deref(), Some("E2 covers six topics.\nE3 is next.\nE2 again."));

        // The count is an assertion: two matches for an expected one changes nothing.
        let two = tool(&bridge, caller(), "session_doc_edit", json!({"slug": "plan", "old_string": "E2", "new_string": "E5"}))
            .await
            .expect_err("a non-unique anchor is refused");
        assert!(two.message.contains("found 2 occurrence(s) of old_string in `plan`, expected 1"), "{}", two.message);
        assert_eq!(doc_body(&storage, "plan").await.unwrap(), "E2 covers four topics.\nE3 is next.\nE2 again.");
        tool(&bridge, caller(), "session_doc_edit", json!({"slug": "plan", "old_string": "E2", "new_string": "E5", "expect_occurrences": 2}))
            .await
            .unwrap();
        assert_eq!(doc_body(&storage, "plan").await.unwrap(), "E5 covers four topics.\nE3 is next.\nE5 again.");

        // Refusals that change nothing.
        for (args, needle) in [
            (json!({"slug": "nope", "old_string": "a", "new_string": "b"}), "no session doc `nope`"),
            (json!({"slug": "plan@1", "old_string": "six", "new_string": "6"}), "archived version"),
            (json!({"slug": "plan", "old_string": "", "new_string": "b"}), "old_string is empty"),
            (json!({"slug": "plan", "old_string": "E3", "new_string": "E3"}), "identical"),
            (json!({"slug": "plan", "old_string": "E3", "new_string": "E4", "expect_occurrences": 0}), "positive integer"),
        ] {
            let refused = tool(&bridge, caller(), "session_doc_edit", args.clone()).await.expect_err("refused");
            assert!(refused.message.contains(needle), "{args}: {}", refused.message);
        }

        // A reviewer's edit of the phase's name is an edit of ITS co-doc, and
        // counts as a co-doc write: its own vote goes with it.
        tool(&bridge, eyes_caller(), "session_doc_write", json!({"slug": "r", "phase": "plan", "body": "point 1: the table is wrong"}))
            .await
            .unwrap();
        tool(&bridge, eyes_caller(), "advance_phase", json!({"target": "Plan"})).await.unwrap();
        let reply = parsed(&tool(&bridge, eyes_caller(), "session_doc_edit", json!({"slug": "plan", "old_string": "is wrong", "new_string": "was fixed"})).await.unwrap());
        assert_eq!(reply["slug"], json!("plan-eyes"));
        assert!(reply["note"].as_str().unwrap().contains("your own phase vote was withdrawn"), "{reply}");
        assert!(doc_body(&storage, "plan-eyes").await.unwrap().contains("the table was fixed"));
        assert_eq!(doc_body(&storage, "plan").await.unwrap(), "E5 covers four topics.\nE3 is next.\nE5 again.", "not the executor's doc");
        let refused = tool(&bridge, caller(), "session_doc_edit", json!({"slug": "plan-eyes", "old_string": "point 1", "new_string": "x"}))
            .await
            .expect_err("an executor does not edit the review");
        assert!(refused.message.contains("holds the reviewer's notes"), "{}", refused.message);

        // Handoff docs: the owner's edit reaches its file (the edit passes the
        // store's one choke point); a peer's edit is refused.
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "handoff-hands", "body": "Standing order: gate every read."}))
            .await
            .unwrap();
        tool(&bridge, caller(), "session_doc_edit", json!({"slug": "handoff-hands", "old_string": "gate every read", "new_string": "gate every production read"}))
            .await
            .unwrap();
        let file = crate::agents::handoff::file_path(handoffs.path(), "s1", "hands").unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("gate every production read"), "the file follows an edit");
        let refused = tool(&bridge, eyes_caller(), "session_doc_edit", json!({"slug": "handoff-hands", "old_string": "Standing", "new_string": "Lifted"}))
            .await
            .expect_err("a peer does not edit another's handoff doc");
        assert!(refused.message.contains("is hands's handoff doc"), "{}", refused.message);
    }

    /// Feedback #71 / #77: a doc's body can come from a file — a test log the
    /// reviewer should be able to read, without the model re-emitting it —
    /// within a cap (EYES, plan point 5), and exactly one of `body` and
    /// `content_path`.
    #[tokio::test]
    async fn session_doc_write_takes_its_body_from_a_file() {
        let (bridge, storage) = two_role_session().await;
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("suite.log");
        std::fs::write(&log, "test result: ok. 1642 passed; 0 failed\n").unwrap();
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "evidence-suite", "content_path": log.to_str().unwrap()}))
            .await
            .unwrap();
        assert_eq!(doc_body(&storage, "evidence-suite").await.as_deref(), Some("test result: ok. 1642 passed; 0 failed\n"));
        // The reviewer reads it like any doc.
        let read = parsed(&tool(&bridge, eyes_caller(), "session_doc_read", json!({"slug": "evidence-suite"})).await.unwrap());
        assert_eq!(read["body"], json!("test result: ok. 1642 passed; 0 failed\n"));

        let too_big = tmp.path().join("huge.log");
        std::fs::write(&too_big, "x".repeat(300_000)).unwrap();
        for (args, needle) in [
            (json!({"slug": "e", "body": "x", "content_path": log.to_str().unwrap()}), "not both"),
            (json!({"slug": "e"}), "missing `body`"),
            (json!({"slug": "e", "content_path": "relative/suite.log"}), "absolute path"),
            (json!({"slug": "e", "content_path": tmp.path().join("absent.log").to_str().unwrap()}), "absent.log"),
            (json!({"slug": "e", "content_path": too_big.to_str().unwrap()}), "capped at 256 KiB"),
        ] {
            let refused = tool(&bridge, caller(), "session_doc_write", args.clone()).await.expect_err("refused");
            assert!(refused.message.contains(needle), "{args}: {}", refused.message);
        }
        assert!(doc_body(&storage, "e").await.is_none(), "a refused write stores nothing");
    }

    /// Feedback #78: a session doc is promoted to the Context Library byte for
    /// byte — `cl_write_file(session_doc: slug)` — instead of being re-typed.
    /// Exactly one body source, and an unknown slug says so.
    #[tokio::test]
    async fn cl_write_file_copies_a_session_doc_byte_for_byte() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("library/projects/bot-hq");
        std::fs::create_dir_all(&proj).unwrap();
        let log = crate::policy::ViolationsLog::new(tmp.path());
        let bridge = SignalingBridge::with_policy(log, tmp.path().to_path_buf());
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.upsert_project("bot-hq", "bot-hq", None, None, None).await.unwrap();
        storage.create_session("s1", "promote", None).await.unwrap();
        storage.ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
        bridge.set_storage(storage.clone()).await;

        // TeX-heavy text is where hand copies lose backslashes.
        let body = "## Key questions\n\\begin{align}\n  A\\mathbf{x} &= \\lambda\\mathbf{x} \\\\\n\\end{align}\n— approved 2026-10-02\n";
        tool(&bridge, eyes_caller(), "session_doc_write", json!({"slug": "eyes-handoff", "body": body})).await.unwrap();

        let reply = tool(
            &bridge,
            caller(),
            "cl_write_file",
            json!({"project": "bot-hq", "file_path": "plans/eyes-handoff.md", "session_doc": "eyes-handoff"}),
        )
        .await
        .unwrap();
        assert!(reply.starts_with("created 'plans/eyes-handoff.md'"), "{reply}");
        assert_eq!(std::fs::read_to_string(proj.join("plans/eyes-handoff.md")).unwrap(), body);

        let unknown = tool(&bridge, caller(), "cl_write_file", json!({"project": "bot-hq", "file_path": "x.md", "session_doc": "nope"}))
            .await
            .unwrap();
        assert!(unknown.contains("`nope` is not a document of this session"), "{unknown}");
        let two = tool(&bridge, caller(), "cl_write_file", json!({"project": "bot-hq", "file_path": "x.md", "content": "a", "session_doc": "eyes-handoff"}))
            .await
            .unwrap();
        assert!(two.contains("exactly one of `content`, `content_path` and `session_doc`"), "{two}");
        assert!(!proj.join("x.md").exists());
    }

    /// EYES' advisory `d2ced691`: a handoff doc too long to put back whole ends
    /// with "read the rest: session_doc_read(slug, lines: \"N-\")" — and that
    /// open-ended range used to be refused, so the first step a compacted
    /// participant was told to take returned an error. The test makes the
    /// EXACT call the rendered file names, through the real handler, and gets
    /// the rest of the doc from the first line that was not shown whole.
    #[tokio::test]
    async fn the_call_a_cut_handoff_docs_footer_names_returns_the_rest() {
        let bridge = SignalingBridge::new();
        let handoffs = tempfile::tempdir().unwrap();
        let storage = crate::storage::Storage::memory()
            .await
            .unwrap()
            .with_handoff_dir(handoffs.path().to_path_buf());
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        storage
            .ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS)
            .await
            .unwrap();
        let body: String = (1..=400)
            .map(|i| format!("line {i}: — standing instruction number {i} —\n"))
            .collect();
        tool(&bridge, caller(), "session_doc_write", json!({"slug": "handoff-hands", "body": body}))
            .await
            .unwrap();

        let file = crate::agents::handoff::file_path(handoffs.path(), "s1", "hands").unwrap();
        let rendered = std::fs::read_to_string(&file).unwrap();
        let footer = rendered.rsplit("[bot-hq]").next().unwrap();
        assert!(footer.contains("Read the rest: session_doc_read(slug: \"handoff-hands\", lines: \""), "{footer}");
        let from = footer.find("lines: \"").unwrap() + "lines: \"".len();
        let lines_arg = &footer[from..from + footer[from..].find('"').unwrap()];
        assert!(lines_arg.ends_with('-'), "the footer names an open-ended range: {lines_arg:?}");
        let resume: usize = lines_arg.trim_end_matches('-').parse().unwrap();

        let reply = tool(
            &bridge,
            caller(),
            "session_doc_read",
            json!({"slug": "handoff-hands", "lines": lines_arg}),
        )
        .await
        .expect("the call the footer names must work");
        let v: Value = serde_json::from_str(&reply).unwrap();
        let rest = v["body"].as_str().expect("an excerpt body");
        assert!(rest.starts_with(&format!("line {resume}: ")), "starts at the resume line: {}", &rest[..60]);
        assert!(rest.ends_with("number 400 —"), "and runs to the end of the doc");
        assert_eq!(v["lines"], json!(format!("{resume}-400")));
        // Nothing falls between the file and the call: every earlier line is in the file.
        assert!(rendered.contains(&format!("line {}: — standing instruction number {} —\n", resume - 1, resume - 1)));
    }

    /// Feedback #83, through the real handlers: a QUEUED outward publish is in
    /// its issuer's `list_my_pending_questions` (marked `queued` — the user's
    /// tray never shows it) and `withdraw_question` takes it back. It used to
    /// be absent from the list and answer "was not pending", so a command
    /// nobody wanted any more went on to summon the reviewer and prompt the
    /// user. Another participant neither sees nor withdraws it.
    #[tokio::test]
    async fn a_queued_gate_is_listed_for_its_issuer_and_withdrawn_by_it() {
        let (bridge, storage) = two_role_session().await;
        sqlx::query(
            "INSERT INTO session_tray (session_id, choice_id, agent, kind, prompt, options_json, status, asked_at, command_text) \
             VALUES ('s1', '23e69f96-0000-4000-8000-000000000001', 'hands', 'approval', 'Run gated command?', \
                     '[\"Approve\",\"Reject\"]', 'queued', '2026-10-03T00:00:00Z', 'gh api repos/o/r/issues/comments/1')",
        )
        .execute(storage.pool())
        .await
        .unwrap();

        let mine = parsed(&tool(&bridge, caller(), "list_my_pending_questions", json!({})).await.unwrap());
        assert_eq!(mine.as_array().unwrap().len(), 1, "{mine}");
        assert_eq!(mine[0]["status"], json!("queued"));
        assert_eq!(mine[0]["choice_id"], json!("23e69f96-0000-4000-8000-000000000001"));
        let theirs = parsed(&tool(&bridge, eyes_caller(), "list_my_pending_questions", json!({})).await.unwrap());
        assert_eq!(theirs, json!([]), "a peer's queued gate is not in the reviewer's list");

        let refused = tool(&bridge, eyes_caller(), "withdraw_question", json!({"choice_id": "23e69f96-0000-4000-8000-000000000001"}))
            .await
            .unwrap();
        assert!(refused.contains("parked by another participant"), "{refused}");
        let done = tool(&bridge, caller(), "withdraw_question", json!({"choice_id": "23e69f96-0000-4000-8000-000000000001"}))
            .await
            .unwrap();
        assert_eq!(done, "withdrawn");
        assert_eq!(parsed(&tool(&bridge, caller(), "list_my_pending_questions", json!({})).await.unwrap()), json!([]));
        // The short id, as a peer would quote it, says what happened to it.
        let status = tool(&bridge, eyes_caller(), "gate_status", json!({"gate_id": "23e69f96"})).await.unwrap();
        assert!(status.starts_with("withdrawn —") && status.contains("issued it"), "{status}");
    }

    /// A handoff doc is its owner's: bot-hq puts `handoff-<slug>` back into
    /// THAT participant's context after a compaction, so a peer's write would
    /// arrive there as the owner's own notes (EYES, plan point 12). The owner's
    /// write lands and is mirrored to its file; a slug that names no
    /// participant is an ordinary custom doc.
    #[tokio::test]
    async fn only_its_owner_writes_a_participants_handoff_doc() {
        let bridge = SignalingBridge::new();
        let handoffs = tempfile::tempdir().unwrap();
        let storage = crate::storage::Storage::memory()
            .await
            .unwrap()
            .with_handoff_dir(handoffs.path().to_path_buf());
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        storage
            .ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS)
            .await
            .unwrap();
        let write = |who: CallerIdentity, slug: &'static str| {
            let bridge = &bridge;
            let r = req(
                "tools/call",
                json!({ "name": "session_doc_write", "arguments": {"slug": slug, "body": "notes"} }),
                1,
            );
            async move { dispatch(r, &who, bridge).await.map(|_| ()) }
        };

        write(caller(), "handoff-hands").await.expect("the owner's write lands");
        let file = crate::agents::handoff::file_path(handoffs.path(), "s1", "hands").unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("notes"), "and reaches its file");

        let refused = write(eyes_caller(), "handoff-hands").await.expect_err("a peer's write is refused");
        assert!(
            refused.message.contains("is hands's handoff doc")
                && refused.message.contains("Yours is `handoff-eyes`"),
            "the refusal names the owner and the caller's own slug: {}",
            refused.message
        );
        write(eyes_caller(), "handoff-eyes").await.expect("its own lands");
        // `after-compact` is no participant: an ordinary custom doc, as before.
        write(eyes_caller(), "handoff-after-compact").await.expect("not a handoff doc");
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
            err.message.contains("unknown phase") && err.message.contains("expected"),
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

        // **WS2 (2026-08-27): an EXECUTOR granted `file_finding` is still the
        // phase-doc AUTHOR.** The redirect keys on reviewer-SHAPED
        // (`FileFinding && !EditFiles`), not bare `FileFinding` — under the
        // bare predicate this caller's every phase write would silently reroute
        // to `plan-eyes` and the primary phase docs would never be written.
        let filing_executor = CallerIdentity {
            session_id: "s1".into(),
            agent: "eyes".into(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::from_slugs(&["edit_files", "file_finding"]),
            ),
        };
        assert!(
            !filing_executor.capabilities.reviewer_shaped(),
            "holding both capabilities is executor-shaped, or this case proves nothing"
        );
        let exec = write(filing_executor, "the plan, revised").await;
        assert!(
            exec.contains("\"slug\":\"plan\""),
            "an executor that may also file findings writes the phase doc itself; got: {exec}"
        );
        let plan = bridge
            .session_doc_read("s1", "plan")
            .await
            .unwrap()
            .expect("the phase doc");
        assert_eq!(plan.body, "the plan, revised", "the author's write landed on the primary doc");
    }

    /// **`mode:"append"` must survive the reviewer redirect** (round 9). The
    /// reviewer arm called `session_doc_write_eyes` with no `append` at all, so a
    /// reviewer's append — the descriptor sells the mode to every caller — was
    /// archived-and-replaced: its earlier findings destroyed by its own second
    /// slice. Wire-level: through `dispatch`, as the reviewer, twice.
    #[tokio::test]
    async fn a_reviewers_append_reaches_the_review_doc_as_an_append() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        let reviewer = CallerIdentity {
            session_id: "s1".into(),
            agent: "eyes".into(),
            capabilities: crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::preset_eyes(),
            ),
        };
        for (body, mode) in [("E1 first slice", "replace"), ("E2 second slice", "append")] {
            dispatch(
                req(
                    "tools/call",
                    json!({
                        "name": "session_doc_write",
                        "arguments": {"slug": "investigate", "body": body, "phase": "investigate", "mode": mode}
                    }),
                    1,
                ),
                &reviewer,
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
        }
        let doc = bridge
            .session_doc_read("s1", "investigate-eyes")
            .await
            .unwrap()
            .expect("the review doc");
        assert!(doc.body.contains("E1 first slice"), "the append replaced the first slice: {}", doc.body);
        assert!(doc.body.contains("E2 second slice"), "{}", doc.body);
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
            &eyes_caller(),
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
    async fn eyes_untagged_doc_write_allowed() {
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
                    "arguments": {"slug": "eyes-scratch", "body": "my notes"}
                }),
                1,
            ),
            &eyes_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_ne!(
            v["result"]["isError"],
            json!(true),
            "eyes's untagged scratch doc must be allowed"
        );
        let read = bridge
            .session_doc_read("s1", "eyes-scratch")
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
            err.message.contains("unknown phase") && err.message.contains("expected"),
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
        // The s-96fda118 deadlock reason shape: parking on the peer.
        assert_eq!(
            peer_shaped_reason_pair("Handed the six refusal probes to EYES — they can only run natively"),
            Some("eyes".to_string())
        );
        assert_eq!(peer_shaped_reason_pair("waiting for my peer to review"), Some("peer".to_string()));
        assert_eq!(peer_shaped_reason_pair("EYES review pending"), Some("eyes".to_string()));
        assert_eq!(peer_shaped_reason_pair("blocked until @hands answers"), Some("hands".to_string()));
        assert_eq!(peer_shaped_reason_pair("Peers need to confirm the plan first"), Some("peer".to_string()));
        // Word boundaries: substrings inside real words must not trip it.
        assert_eq!(peer_shaped_reason_pair("waiting for the rainbow deploy window"), None);
        assert_eq!(peer_shaped_reason_pair("user must restrain the migration"), None);
        assert_eq!(peer_shaped_reason_pair("need the user's tracker token"), None);
        assert_eq!(peer_shaped_reason_pair(""), None);
    }

    /// Round 12: the guard refused legitimate user-waits that merely MENTIONED
    /// a role, or used `hands`/`eyes` as the English nouns they are — four of
    /// the five real hits in the archive. A peer token without a wait shape,
    /// or a wait shape with only lowercase English, is not a peer-wait.
    #[test]
    fn peer_guard_needs_a_role_token_and_a_wait_shape_in_one_sentence() {
        // s-1c29c521 00:20:01Z — refused, then rephrased: English noun.
        assert_eq!(peer_shaped_reason_pair("All work that doesn't need you is done. Two things in your hands."), None);
        assert_eq!(peer_shaped_reason_pair("needs your eyes on the design before I continue"), None);
        assert_eq!(peer_shaped_reason_pair("rain check on the deploy until you say"), None);
        // s-cf106858 / s-0d063183 — descriptive mentions, no wait on the peer.
        assert_eq!(peer_shaped_reason_pair("Plan complete and reviewed (EYES F1–F5 folded in, no rebuttals)"), None);
        assert_eq!(peer_shaped_reason_pair("Conversational turn — floor is yours; EYES awake and passing, no work queued."), None);
        // A snake_case identifier is one word.
        assert_eq!(peer_shaped_reason_pair("waiting for you to approve the eyes_flag finding"), None);
        // The wait shape in ANOTHER sentence does not reach across.
        assert_eq!(peer_shaped_reason_pair("EYES reviewed the plan. Waiting for your console read."), None);
        // Uppercase is how a participant writes a peer; the lowercase slug
        // counts too (F14 — the deadlock shape in lowercase) unless a
        // possessive or a quantity precedes it, which is the English noun.
        assert_eq!(peer_shaped_reason_pair("waiting on hands to finish"), Some("hands".to_string()));
        assert_eq!(peer_shaped_reason_pair("waiting on HANDS to finish"), Some("hands".to_string()));
        assert_eq!(peer_shaped_reason_pair("blocked until eyes answers"), Some("eyes".to_string()));
        assert_eq!(peer_shaped_reason_pair("need more eyes on it before I continue"), None);
        assert_eq!(peer_shaped_reason_pair("the work is in my hands now; waiting for your console read"), None);
        assert_eq!(peer_shaped_reason_pair("all hands on deck until the deploy lands"), None);
        // Hyphenated: the word before the slug is still the word, not the hyphen.
        assert_eq!(peer_shaped_reason_pair("all-hands meeting until Friday"), None);
        assert_eq!(peer_shaped_reason_pair("a four-eyes check is pending on your side"), None);
    }

    /// F10 (plan C4h, the user's pick `c2ca371d`): what an AGENT writes to the
    /// Context Library through its tools is redacted — a replace, a
    /// `content_path` body, an append (only the new text), an edit (only the
    /// replacement) and a folder description — while what the user typed into
    /// the file stays as written, and the Context Library tab's own folder
    /// editor (the shared bridge function) keeps the user's text.
    #[tokio::test]
    async fn an_agents_cl_writes_are_redacted_and_the_users_text_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let cl = tmp.path().join("library/projects/bot-hq");
        std::fs::create_dir_all(&cl).unwrap();
        let token = format!("{}{}", "ghp_", "1234567890abcdefghijABCDEF");
        let marker = "[redacted: a GitHub access token]";
        // What the user typed into a CL file, as the Library tab writes it.
        let typed = format!("user token: {token}\n");
        std::fs::write(cl.join("notes.md"), &typed).unwrap();
        let body_file = tmp.path().join("body.md");
        std::fs::write(&body_file, format!("from a path: {token}\n")).unwrap();

        let bridge = SignalingBridge::new_with(None, Some(tmp.path().to_path_buf()));
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.upsert_project("bot-hq", "bot-hq", None, None, None).await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;
        let call = |name: &str, arguments: Value| {
            let bridge = bridge.clone();
            let request = req("tools/call", json!({"name": name, "arguments": arguments}), 1);
            async move {
                let res = dispatch(request, &caller(), &bridge).await.unwrap().unwrap();
                let v = serde_json::to_value(&res).unwrap();
                assert!(v["result"]["isError"] != json!(true), "{v}");
            }
        };

        call("cl_write_file", json!({"project": "bot-hq", "file_path": "new.md", "content": format!("key {token}\n")})).await;
        assert_eq!(std::fs::read_to_string(cl.join("new.md")).unwrap(), format!("key {marker}\n"));

        call("cl_write_file", json!({"project": "bot-hq", "file_path": "path.md", "content_path": body_file.to_str().unwrap()})).await;
        assert_eq!(std::fs::read_to_string(cl.join("path.md")).unwrap(), format!("from a path: {marker}\n"));

        call("cl_write_file", json!({"project": "bot-hq", "file_path": "notes.md", "mode": "append", "content": format!("agent adds {token}\n")})).await;
        let notes = std::fs::read_to_string(cl.join("notes.md")).unwrap();
        assert!(notes.starts_with(&typed), "the user's text stays as written: {notes}");
        assert!(notes.contains(&format!("agent adds {marker}")), "{notes}");

        // An edit matches the file RAW and redacts only its replacement — so
        // carrying the user's secret through an edit rewrites it (EYES P6).
        call("cl_edit_file", json!({"project": "bot-hq", "file_path": "notes.md", "old_string": format!("user token: {token}"), "new_string": format!("user token: {token} (rotated)")})).await;
        let notes = std::fs::read_to_string(cl.join("notes.md")).unwrap();
        assert!(notes.starts_with(&format!("user token: {marker} (rotated)\n")), "{notes}");
        assert!(!notes.contains(&token), "{notes}");

        call("cl_register_folder_description", json!({"project": "bot-hq", "folder_path": "", "description": format!("keys live in {token}"), "tags": token.clone()})).await;
        let folders = storage.cl_folder_search(Some("bot-hq"), None).await.unwrap();
        assert_eq!(folders[0].description, format!("keys live in {marker}"));
        assert_eq!(folders[0].tags.as_deref(), Some(marker));

        // The Library tab's folder editor calls the bridge function directly:
        // what the user types there is kept.
        bridge
            .cl_register_folder_description("bot-hq", "", &format!("my key {token}"), None)
            .await
            .unwrap();
        let folders = storage.cl_folder_search(Some("bot-hq"), None).await.unwrap();
        assert_eq!(folders[0].description, format!("my key {token}"));
    }

    /// F10: a write that stored an agent's secret as a marker SAYS so in its
    /// reply — a CL write and a session doc — and a clean write does not. An
    /// edit whose `old_string` quotes a secret the file holds only as its
    /// marker is refused with that reason instead of "check the exact text",
    /// which the agent did.
    #[tokio::test]
    async fn a_redacting_write_says_so_and_an_edit_quoting_the_secret_names_the_marker() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("library/projects/bot-hq")).unwrap();
        let bridge = SignalingBridge::new_with(None, Some(tmp.path().to_path_buf()));
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.upsert_project("bot-hq", "bot-hq", None, None, None).await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        bridge.set_storage(storage).await;
        let token = format!("{}{}", "ghp_", "1234567890abcdefghijABCDEF");
        let call = |name: &str, arguments: Value| {
            let bridge = bridge.clone();
            let request = req("tools/call", json!({"name": name, "arguments": arguments}), 1);
            async move {
                // A refusal surfaces either as an error RESULT or as a JSON-RPC
                // error (`internal_err_no_prefix`); both are "refused".
                match dispatch(request, &caller(), &bridge).await {
                    Ok(res) => {
                        let v = serde_json::to_value(res.unwrap()).unwrap();
                        (
                            v["result"]["isError"] == json!(true),
                            v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(),
                        )
                    }
                    Err(e) => (true, e.message),
                }
            }
        };
        let note = "secret-shaped string(s) in it were stored as `[redacted: …]` markers";

        let body = format!("key {token}\n{}\n", "a line of context that the edit keeps. ".repeat(4));
        let (err, reply) = call("cl_write_file", json!({"project": "bot-hq", "file_path": "a.md", "content": body})).await;
        assert!(!err && reply.contains(&format!("1 {note}")), "{reply}");
        let (err, reply) = call("cl_write_file", json!({"project": "bot-hq", "file_path": "b.md", "content": "clean\n"})).await;
        assert!(!err && !reply.contains("redacted"), "{reply}");

        let (err, reply) = call("cl_edit_file", json!({"project": "bot-hq", "file_path": "a.md", "old_string": format!("key {token}"), "new_string": "key rotated"})).await;
        assert!(err, "the raw secret is not in the file: {reply}");
        assert!(reply.contains("holds its `[redacted: …]` marker"), "{reply}");
        let (err, reply) = call("cl_edit_file", json!({"project": "bot-hq", "file_path": "a.md", "old_string": "key [redacted: a GitHub access token]", "new_string": "key rotated"})).await;
        assert!(!err && !reply.contains("redacted"), "matching the marker works: {reply}");

        let (err, reply) = call("session_doc_write", json!({"slug": "notes", "body": format!("got {token}")})).await;
        assert!(!err, "{reply}");
        let v: Value = serde_json::from_str(&reply).unwrap();
        assert!(v["note"].as_str().is_some_and(|n| n.contains(note)), "{reply}");
        let (_, reply) = call("session_doc_write", json!({"slug": "clean", "body": "nothing here"})).await;
        let v: Value = serde_json::from_str(&reply).unwrap();
        assert!(v.get("note").is_none(), "{reply}");
    }
}