//! MCP JSON-RPC types.
//!
//! Minimal subset of [Model Context Protocol] we serve: `initialize`,
//! `tools/list`, `tools/call`, `ping`. Notifications (`notifications/*`) are
//! accepted but discarded.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// MCP protocol version we advertise.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    /// Notifications omit `id`. Requests must have one (string or number).
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    /// Optional structured error payload (JSON-RPC 2.0 §5.1 `data` member).
    /// Reserved for spec-completeness — every error today is built via `new()`
    /// with `data: None`, so it never serializes; populate it if a tool ever
    /// needs to attach machine-readable error detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const PARSE_ERROR: i32 = -32700;

    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// The Tauri `AppHandle` isn't wired yet (app still in setup, or a test
    /// bridge has none). Shared by every webview / web_search tool in the
    /// dispatcher (there was an external one too until the driver server was
    /// removed on 2026-08-17).
    pub fn app_handle_missing() -> Self {
        Self::new(Self::INTERNAL_ERROR, "Tauri AppHandle not yet initialized")
    }

    /// The "main" webview window wasn't found on the `AppHandle`.
    pub fn webview_missing() -> Self {
        Self::new(Self::INTERNAL_ERROR, "main webview not found")
    }
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: Value, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Parse an IPAV phase target from an MCP argument. Returns INVALID_PARAMS
/// with the canonical error_hint message on failure. `field` is the
/// argument name as the dispatcher exposes it on the wire (`"target"` for
/// `advance_phase`/`request_phase_advance`, `"phase"` for
/// `session_doc_search`'s optional filter) — preserved in the error string
/// for caller clarity. Single source of truth so the three call sites in
/// `jsonrpc.rs` can't drift on what they tell agents.
pub(crate) fn parse_phase_arg(
    field: &str,
    value: &str,
) -> Result<crate::core::ipav::IpavPhase, JsonRpcError> {
    crate::core::ipav::IpavPhase::parse(value).ok_or_else(|| {
        JsonRpcError::new(
            JsonRpcError::INVALID_PARAMS,
            format!(
                "unknown {field} '{value}' (expected {})",
                crate::core::ipav::IpavPhase::error_hint()
            ),
        )
    })
}

// ---- MCP-specific shapes ----

#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// The sentence a gated tool's description opens with, **derived from the gate
/// itself** rather than written beside it.
///
/// These four lines used to spell the capability by hand and three of them named
/// the wrong one: `disposition_finding` and `override_reviewer_block` claimed
/// `edit_files`, `approve_finding` claimed `file_finding`, while
/// [`capability::required_for`](crate::agents::capability::required_for) gates
/// each on its own. `tools/list` is unfiltered, so every agent in every session
/// read the wrong rule — and the SAME agent's layer-2 prompt section, which is
/// generated from `required_for`, told it the right one. Two answers to one
/// question, in one context window.
///
/// Same principle as rc3 D3 (`agents::capability_prompt`): the text that states
/// a rule is produced from the data the runtime enforces, so the two cannot
/// disagree. Rename a capability slug and every description follows.
///
/// **Panics** if `tool` is not gated at all. A description that claims a
/// requirement the runtime does not enforce is the defect the reframe contract's
/// acceptance rule 2 names, and this way it cannot ship: the descriptor list is
/// a `LazyLock` and [`tests::every_gated_tool_names_the_capability_its_gate_reads`]
/// forces it, so a mis-registered tool fails the suite rather than a user's call.
///
/// Leaks one string per gated tool, once per process, because
/// [`ToolDescriptor::description`] is `&'static str` and this runs inside that
/// `LazyLock`. Four allocations, for a sentence that cannot go stale.
fn gated_by(tool: &'static str, rest: &str) -> &'static str {
    let cap = crate::agents::capability::required_for(tool).unwrap_or_else(|| {
        panic!("`{tool}` describes itself as gated, but `required_for` gates no such tool")
    });
    Box::leak(format!("Requires the `{}` capability. {rest}", cap.slug()).into_boxed_str())
}

/// Hand-built JSON Schemas for our tools. We don't pull in `schemars`.
pub fn tool_descriptors() -> &'static [ToolDescriptor] {
    use std::sync::LazyLock;
    static TOOLS: LazyLock<Vec<ToolDescriptor>> = LazyLock::new(|| {
        vec![
        ToolDescriptor {
            name: "ask_user_choice",
            description: gated_by("ask_user_choice", "Ask the user to pick one option from a list. Returns IMMEDIATELY with a parked acknowledgment (status=parked plus a choice_id) — it does NOT block and it halts NOTHING: the question sits in the user's tray while the session keeps working, and the pick arrives later as a user row you read at your next dealt turn — an idle session (unless paused) is dealt one the moment the user answers, a working session reads it at its next boundary (often alongside the user's next send). After calling it, carry on with whatever else is workable (or pass if nothing is) — don't guess the answer, poll, or re-ask; if you must rephrase, withdraw the old question first. Use this whenever a decision belongs to the user."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The question to show the user." },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Concrete options the user can pick."
                    }
                },
                "required": ["question", "options"]
            }),
        },
        ToolDescriptor {
            name: "mark_awaiting_user",
            description: gated_by("mark_awaiting_user", "Declare the session's HALT: the ring stops where it stands, the session's single halt slot is filled with your reason (a later declaration replaces it), your own in-flight generation is interrupted so the declaration is true, and the user gets the floor. `reason` is the recap the user reads above their input box — say where things stand and what resumes it (a wake time, a command pasted verbatim, a tray answer). Cleared by the user's next message. Use it when the next move is genuinely the user's; a question that blocks nothing is `ask_user_choice`. REFUSED (no halt declared, an error returned) when `reason` names a peer — the words `peer`, `eyes`, `hands` or a legacy agent name, word-boundary matched — because waiting on a peer is not waiting on the user; when a legitimate user-wait reason must mention a role, declare it through `halt`, which carries no such filter."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": { "type": "string", "description": "Short reason why we're waiting on the user." }
                },
                "required": ["reason"]
            }),
        },
        ToolDescriptor {
            name: "peer_ack",
            description: "Say you have converged — you agree with your peer, or have nothing substantive to add — and END the round rather than bouncing another acknowledgment that costs a full turn. Your text is still saved to the chat and the user still reads it; what the ack changes is how your TURN ENDS.\n\nA content-free ack ends the turn as DONE: a vote toward consensus, which is what stops the session cycling and hands the floor back to the user once everyone has voted. A LONG turn (over ~200 characters) is not treated as an ack, because a long turn is usually a review or a correction and those must never be silently downgraded to agreement.\n\nThat length rule is a proxy, and it misfires on the most common way a round actually ends: \"I agree, and here is the one reason why\" runs past the limit. When your turn IS your closing statement, pass `final: true` to say so explicitly — the vote then does not depend on length. Nothing is lost either way: `final` changes the VOTE, not the record.\n\nDo NOT pass `final: true` on a turn that carries a new finding, a correction, or anything your peer still needs to act on — that is a vote for finishing, cast on a turn that says the opposite.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "final": {
                        "type": "boolean",
                        "description": "True when this turn is your closing statement — cast the done vote regardless of length. Omit (or false) for the default length-based behaviour."
                    }
                }
            }),
        },
        ToolDescriptor {
            name: "pass_turn",
            description: "Decline this turn. Use it when the turn has come round to you and you genuinely have nothing to add — no finding, no correction, no next step of your own. Your pass is recorded in the chat so the user can see you were asked and chose to stay quiet; the turn then moves on.\n\nThis is NOT the same as saying you are finished. Being finished (peer_ack, or simply having nothing left to do) counts toward the session settling; a pass counts toward nothing and deliberately cannot end the session on its own. Reach for it when the work is someone else's this round — the reviewer with nothing yet to review, the executor waiting on a decision — and reach for the finished verbs instead when you actually believe the work is done.\n\nCalling it costs nothing and no argument is needed. If you write substantive text in the same turn (more than ~200 characters), that text wins: the turn counts as a real turn and the pass is ignored, because a pass has to mean silence to mean anything. So do not use it as a preface to a point you are about to make. ONE pass per turn: a second call in the same turn is refused (the count clears when the ring deals you a new turn) — end the turn instead.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDescriptor {
            name: "halt",
            description: gated_by("halt", "Yield control back to the user and unlock the chat input. Use when the participants have converged or you've finished the current slice and the next move is genuinely the user's — it ends the agent-to-agent loop cleanly. Sets the session to awaiting-user (the input unlocks even mid-turn, since awaiting outranks busy) and stops the ring until the user's next message. Like mark_awaiting_user but framed as a yield rather than a specific pending question — and without its peer-word filter; pass an optional reason, shown in the halt banner above the input (not the tray)."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": { "type": "string", "description": "Optional short note on why you're yielding — it fills the session's halt slot and shows in the halt banner above the input (a halt is not a tray row)." }
                }
            }),
        },
        ToolDescriptor {
            name: "advance_phase",
            description: "Cast your VOTE to move the IPAV phase (rc3 D37 — no user gate; with more than one active participant it is not a solo move either): the phase advances only when every active participant has voted for the same target at the same state of the work — a solo roster's own vote IS that consensus and moves the chip at once; until then the tool answers NOT ADVANCED with the tally, and you are still in the current phase. Call it whenever your work crosses a boundary — investigation done -> Plan, plan stated -> Apply, mutation done -> Verify — and expect your peers to vote on their turns. The state of the work is fingerprinted over the session's phase-TAGGED documents (untagged scratch docs do not count), so WRITE your phase doc first, then vote (a doc write after your vote orphans it, and everyone re-votes on changed work); a pass retracts your vote. On consensus the chip moves and every participant reads a [PHASE: X] transition notice. Use exact phase names: Investigate, Plan, Apply, Verify.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "enum": ["Investigate", "Plan", "Apply", "Verify"],
                        "description": "The phase you're moving to."
                    }
                },
                "required": ["target"]
            }),
        },
        ToolDescriptor {
            name: "web_search",
            description: "Search the web and get back structured results (title, url, snippet). Runs INSIDE bot-hq via a headless browser, so it works regardless of which model backs this agent — unlike the built-in WebSearch, which is inert on third-party model gateways (e.g. DeepSeek). Use it to look things up online (docs, issues, current info); optionally read a result URL afterward with WebFetch.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query." },
                    "num_results": { "type": "integer", "minimum": 1, "description": "Optional cap on how many results to return." },
                    "engine": { "type": "string", "enum": ["google", "startpage", "bing", "auto"], "description": "Optional preferred engine to try FIRST (still falls back to the others). Default cascade order: Google → Startpage → Bing. Use 'auto' or omit for the default." }
                },
                "required": ["query"]
            }),
        },
        ToolDescriptor {
            name: "request_phase_advance",
            description: "OPT-IN gated phase advance — use ONLY when you want to pause for explicit user acknowledgment before crossing a boundary. Most phase transitions go through `advance_phase` (the participants' vote, no user gate). Reserve this for irreversible / destructive Apply work (force-push, prod writes, large rewrites). Adds a chat message and fills the session's halt slot; the ring stops until the user picks a phase in the session header OR replies in chat (implicit decline).",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "enum": ["Investigate", "Plan", "Apply", "Verify"],
                        "description": "The phase you want to move to."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why this transition needs explicit user ack (e.g., 'about to force-push to main')."
                    }
                },
                "required": ["target", "reason"]
            }),
        },
        ToolDescriptor {
            name: "file_feedback",
            description: "File an issue or idea about BOT-HQ ITSELF — the tool you are running inside — not about the repo this session is working on. Use it when bot-hq gets in your way (a gate that renders unreadably, a round-trip that wasted the user's time, a tool whose contract surprised you) or when you think of something that would make the workflow better. It writes to a queue a later bot-hq session works through; it does NOT interrupt the user and does NOT surface mid-session, so it never costs the current task anything. Any participant may call it. Returns the reference you can quote back (e.g. 'filed as feedback #12'). Do NOT use it for bugs in the project you are working on — those belong in that project's tracker.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["issue", "idea"],
                        "description": "'issue' = something is broken or annoying; 'idea' = something could be better."
                    },
                    "title": {
                        "type": "string",
                        "description": "One line naming the problem or suggestion, specific enough to triage without opening it."
                    },
                    "body": {
                        "type": "string",
                        "description": "What happened and why it mattered. Cite concrete evidence — the tool call, the file:line, the wasted minutes — the way you would in a bug report."
                    }
                },
                "required": ["kind", "title", "body"]
            }),
        },
        ToolDescriptor {
            name: "request_approval",
            description: gated_by("request_approval", "Request user approval for a policy-gated action (push_gate, force_push, per_action). PARKS and returns IMMEDIATELY with a parked acknowledgment carrying a choice_id — it does NOT block waiting for the answer, but the park LATCHES the ring: finish your turn normally, then nobody is dealt another until the user answers. The user's pick arrives later as an out-of-band message, so do not re-issue on no answer yet; call `gate_status` with the choice_id if you need to know whether it resolved. The outcome is written to violations.jsonl. Call this BEFORE running the action (e.g., before a prod query). For a Tool-Gate-blocked Bash command, use `action_gate` instead."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        // Derived from the enum's own serde names, so the
                        // schema and `parse_violation_kind` cannot disagree.
                        "enum": crate::policy::ViolationKind::REQUESTABLE
                            .iter()
                            .map(|k| k.wire_name())
                            .collect::<Vec<_>>(),
                        "description": "Which policy class triggered this request."
                    },
                    "action": {
                        "type": "string",
                        "description": "The concrete action being requested (e.g., 'git push origin main')."
                    },
                    "question": {
                        "type": "string",
                        "description": "Prompt shown to the user. Be specific about WHAT and WHY."
                    },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 2,
                        "description": "Options. Convention: include at least one starting with 'Approve' and one 'Deny'."
                    },
                    "detail": {
                        "type": "string",
                        "description": "Optional context for the violations log (e.g., 'forbidden word: bot-hq')."
                    }
                },
                "required": ["kind", "action", "question", "options"]
            }),
        },
        ToolDescriptor {
            name: "action_gate",
            description: gated_by("action_gate", "Route a Bash command that the bot-hq Tool Gate blocked (the PreToolUse hook told you to call this). An `auto_allow`/unmatched command runs immediately and you get its output back. A `gate` command PARKS for the user's approval and this call returns AT ONCE with `gate_id` — it does not block, so there is nothing to time out — but the park LATCHES the ring: finish your turn, then nobody is dealt another until the user answers. On approve bot-hq executes the command in your working repo and the stdout/stderr/exit code arrives as an out-of-band message; on reject you get a rejection notice (any text beyond 'Reject' is the user's reasoning — read it). NEVER re-issue a parked command or assume it ran; call gate_status(gate_id) to check. Re-parking an identical command while one is pending returns the existing gate instead of stacking a duplicate prompt. Prefer `--body-file /tmp/x.md` over inline heredocs for long bodies."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The exact Bash command the gate blocked (e.g., 'gh issue comment 41 --body-file /tmp/x'). bot-hq runs it verbatim in your working repo on approval."
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDescriptor {
            name: "gate_status",
            description: "Current state of a parked action_gate command by its gate_id: pending (still awaiting the user — do not re-issue), approved (executed; output was delivered as an out-of-band message), or rejected (not run; includes the user's answer text). A `request_approval` choice_id works too — those rows carry no command, so the answer is the user's pick and nothing ran on bot-hq's side. Read-only, callable by any participant. Use this instead of guessing whether a gated command ran.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "gate_id": {
                        "type": "string",
                        "description": "The gate_id returned by action_gate when it parked the command."
                    }
                },
                "required": ["gate_id"]
            }),
        },
        ToolDescriptor {
            name: "check_commit_message",
            description: "Pre-commit grep against project policy.forbidden_in_commits. Returns 'ok' if clean, or 'forbidden_word: <word>' if the message contains a disallowed phrase. Always call this BEFORE `git commit`. If the result is anything other than 'ok', rewrite the message — do not bypass.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The proposed commit message (and optionally the diff content) to scan."
                    }
                },
                "required": ["message"]
            }),
        },
        ToolDescriptor {
            name: "eyes_flag",
            description: gated_by("eyes_flag", "File a review finding on this session — usually during Verify. `severity='blocking'` records a finding that GATES `git commit` until a participant holding `disposition_finding` resolves it (the mechanical sign-off gate, mirroring the commit-message gate — review-completion becomes enforced, not just socially expected); `severity='advisory'` is a nit that NEVER blocks. Returns the finding id — an OPEN finding with an identical `summary` is a re-raise: its existing id comes back, no duplicate row is filed, and its raise count grows only if another participant has spoken since (change the summary to file a genuinely new finding). Use `blocking` for a real bug / correctness or safety issue you want fixed before ship; do NOT over-use it for style nits (that trains the executor to ignore the gate). This is how a finding STICKS instead of relying on someone reading chat."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "severity": {
                        "type": "string",
                        "enum": ["blocking", "advisory"],
                        "description": "blocking = gates commit until dispositioned; advisory = never gates."
                    },
                    "summary": { "type": "string", "description": "What the finding is — concise + actionable (the bug and its impact)." },
                    "code_ref": { "type": "string", "description": "Optional file:line or symbol the finding points at." }
                },
                "required": ["severity", "summary"]
            }),
        },
        ToolDescriptor {
            name: "disposition_finding",
            description: gated_by("disposition_finding", "Resolve a `blocking` review finding so it stops gating `git commit`. `status='fixed'` (you fixed it — `reason` should reference the fix: commit/line/test) or `status='rebutted'` (you disagree — `reason` must justify why). A rebuttal does NOT need the finder's agreement (so it can't deadlock); it is recorded on the finding with your reason and clears the banner — the user sees it as your tool call in the transcript, so write the reason for them. `reason` is REQUIRED for both. Call this for each open blocking finding before committing; see what's open with `check_open_findings`."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "finding_id": { "type": "string", "description": "The finding id from eyes_flag / check_open_findings." },
                    "status": {
                        "type": "string",
                        "enum": ["fixed", "rebutted"],
                        "description": "fixed = resolved in code; rebutted = you disagree (with reason)."
                    },
                    "reason": { "type": "string", "description": "Required. The fix reference (fixed) or the justification (rebutted). Surfaced to the user." }
                },
                "required": ["finding_id", "status", "reason"]
            }),
        },
        ToolDescriptor {
            name: "check_open_findings",
            description: "Check whether any EYES `blocking` findings are still unresolved for this session. Returns 'ok' if clear, or 'blocked: <N> unresolved blocking finding(s)' plus the list (each with its finding id). ALWAYS call this BEFORE `git commit`: if it's not 'ok', resolve each finding via `disposition_finding` (fix or rebut) first. The pre-commit / pre-push git hooks enforce this mechanically too, but checking here lets you resolve cleanly instead of hitting a blocked commit.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDescriptor {
            name: "override_reviewer_block",
            description: gated_by("override_reviewer_block", "REQUEST an override of a 'reviewer down' commit block when this session's reviewer is Stalled/Dead and you've confirmed the change is safe to ship unreviewed. It does NOT lift the block itself: it parks an Approve/Reject decision for the user (returns at once with the gate id) and the block lifts only on their Approve — the user decides, you do not. `reason` is REQUIRED and is shown to them beside the decision (the fail-closed escape valve). An approved override auto-clears when the reviewer recovers — and a request still parked when it recovers is voided. Only needed when check_open_findings returns 'blocked: reviewer down'."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": { "type": "string", "description": "Required. Why it's safe to commit without a live reviewer. Logged + surfaced." }
                },
                "required": ["reason"]
            }),
        },
        ToolDescriptor {
            name: "approve_finding",
            description: gated_by("approve_finding", "Confirm that an escalated finding the executor marked fixed is genuinely resolved — clears the escalation's 'awaiting confirmation' signal. Use this AFTER a finding you re-raised has been dispositioned, once you've verified the fix is real. This does NOT gate commits (the disposition already cleared the commit gate); it's the sign-off that closes the loop. If you DON'T agree the fix is adequate, do NOT approve — re-file it with `eyes_flag` instead (a fresh open finding re-blocks the commit)."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "finding_id": { "type": "string", "description": "The finding id (from eyes_flag / check_open_findings) to confirm as resolved." }
                },
                "required": ["finding_id"]
            }),
        },
        ToolDescriptor {
            name: "close_session",
            description: gated_by("close_session", "Close the session this agent is running in. Kills every participant's subprocess and marks the session row closed (or archived). Use this when the user asks you to close the session and the conversation has reached a natural stopping point. The FIRST call may answer with a Context-Library staleness report, and the next with a close-out learnings nudge (when adherence nudges are on and this session wrote no CL delta) instead of closing — up to two pre-answers; act on each (write the delta, or decide there is none), then call again; the call that closes is fire-and-forget — your subprocess is terminated shortly after it returns."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "archive": {
                        "type": "boolean",
                        "default": false,
                        "description": "If true, mark the session archived (hidden from dashboard) rather than just closed."
                    }
                },
                "required": []
            }),
        },
        ToolDescriptor {
            name: "list_my_pending_questions",
            description: "List the questions YOU (this agent) have currently parked for the user in this session: ask_user_choice prompts and request_approval / action_gate gates that haven't been resolved yet. (A halt is not a tray row — `mark_awaiting_user` fills the session's single halt slot, which the user's next message clears.) **Call this BEFORE issuing a new `ask_user_choice` to avoid duplicate retries** — if you already have a pending one on the same topic, supersede or withdraw it first. Returns a JSON array of { choice_id, kind, prompt, options, asked_at, supersedes_id }.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDescriptor {
            name: "withdraw_question",
            description: "Abandon a question YOU previously parked for the user (you figured it out, the context changed). Removes the prompt from the user's questions tray + the dashboard counter. If you want to REPLACE the question with a rephrased version, prefer `supersede_question` over withdraw+ask — the former is one tool call AND links the old row to the new via `supersedes_id` so the history is traceable.\n\nYours only: a question another participant parked is not yours to clear out of the user's tray.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "choice_id": {
                        "type": "string",
                        "description": "The choice_id from `list_my_pending_questions`."
                    }
                },
                "required": ["choice_id"]
            }),
        },
        ToolDescriptor {
            name: "supersede_question",
            description: gated_by("supersede_question", "Replace a stale question you parked for the user with a rephrased version. The old row gets status='superseded' (drops from the tray); the new row links to it via `supersedes_id` so the history is traceable. Same non-blocking semantics as `ask_user_choice` — parks the new question and returns a parked acknowledgment immediately; the user's pick on it arrives out-of-band.\n\nNote: `ask_user_choice` and `request_approval` auto-supersede only a pending row from this agent with the IDENTICAL prompt (a re-ask after a timeout — distinct questions accumulate in the tray). Use `supersede_question` to replace a SPECIFIC stale row with different wording without disturbing the others."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "stale_choice_id": {
                        "type": "string",
                        "description": "The choice_id of the question being replaced (from `list_my_pending_questions`)."
                    },
                    "question": {
                        "type": "string",
                        "description": "The rephrased question."
                    },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Concrete options the user can pick on the new question."
                    }
                },
                "required": ["stale_choice_id", "question", "options"]
            }),
        },
        ToolDescriptor {
            name: "session_doc_write",
            description: "Upsert a per-session scratch document (plan, investigation findings, notes — any free-form text). Isolated to THIS session; does NOT appear in cl_index_search and won't pollute the CL. **Tag with `phase` (one of `investigate`/`plan`/`apply`/`verify`) and the doc is keyed BY PHASE — exactly ONE rewritable doc per phase. Writing that phase again (even under a different slug) overwrites the single doc; if you found new info, rewrite the whole doc, never create a `-v2`.** Phase-tagged docs surface in the matching IPAV document tab and are retrievable via `session_doc_search(phase=...)`. Untagged docs are keyed by `slug` (many allowed) and are CUSTOM documents: each surfaces as its own tab beside I/P/A/V, named by its slug — for a document the IPAV set does not cover (a task checklist, an issue write-up), when the user asks or the work needs one; they stay out of phase-filtered searches. Two mechanics to know: a caller that files review findings (a reviewer) has its phase-tagged write redirected to the co-located `<phase>-eyes` doc, so the executor's doc is never overwritten by a review; and every phase REPLACE archives the previous body as an untagged `<slug>@<n>` doc (capped), which an unfiltered `session_doc_search` will list. Promote to CL by writing the body to a CL path with Write/Bash + cl_rescan(project) ONLY when the user asks.\n\n`mode: \"append\"` adds to the existing body under a timestamped separator instead of replacing it. Use it when a phase ships in SLICES: rewriting the whole doc per slice is what leaves it stale, and the phase key means you cannot open a second one. An append supersedes nothing, so nothing is archived. Default is `replace`.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "description": "Identifier for UNTAGGED scratch docs (many allowed per session). For phase-tagged writes the doc is keyed by `phase`, so slug is only a human label — use the phase name (e.g. 'plan')."
                    },
                    "body": {
                        "type": "string",
                        "description": "The document body, free-form markdown / text."
                    },
                    "phase": {
                        "type": "string",
                        "enum": ["investigate", "plan", "apply", "verify"],
                        "description": "Optional. Tags the doc AND keys it: exactly one rewritable doc per phase. Surfaces in the matching IPAV tab and is retrievable via session_doc_search(phase=...)."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "append"],
                        "description": "'replace' (default) overwrites the whole body. 'append' adds to it under a timestamped separator — use it when a phase ships in several slices, so the doc stays current instead of going stale between rewrites."
                    }
                },
                "required": ["slug", "body"]
            }),
        },
        ToolDescriptor {
            name: "session_doc_search",
            description: "List this session's scratch documents. Returns lightweight rows {id, slug, body, phase, created_at, updated_at} ordered newest-first. Use BEFORE session_doc_read to find what's worth opening. **Use the `phase` filter to pull prior-phase context (e.g., in Apply: `session_doc_search(phase=\"plan\")` finds the plan to implement; in Verify: `session_doc_search(phase=\"apply\")` finds the apply summary).** Prefer this over scrolling chat history.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter over slug + body."
                    },
                    "phase": {
                        "type": "string",
                        "enum": ["investigate", "plan", "apply", "verify"],
                        "description": "Optional. Filter results to docs tagged with this IPAV phase. Use for cross-phase context retrieval."
                    }
                },
                "required": []
            }),
        },
        ToolDescriptor {
            name: "session_doc_read",
            description: "Fetch one session-scratch document by slug. Returns {id, slug, body, created_at, updated_at} or null when the slug isn't in this session.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "description": "Slug from session_doc_search."
                    }
                },
                "required": ["slug"]
            }),
        },
        ToolDescriptor {
            name: "cl_index_search",
            description: "Search the Context Library (CL) index for relevant files BEFORE reading any CL file. The index returns lightweight {project, file_path, abs_path, description, tags, updated_at} rows — `abs_path` is the resolved on-disk location (use it, never construct a path from `project` + `file_path`); read descriptions to decide which files are worth opening. This saves context vs eagerly reading everything. The `_globals` project holds cross-project system files (eod.md, tasks.md, general-rules.md, …) and its rows are ALWAYS included alongside a project-scoped search (check the `project` field on each row); for project-scoped work pass your session's working project name. Optional `query` does a case-insensitive substring match across file_path/description/tags.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name. Use the session's project for project-scoped files, '_globals' for system-wide rules, or omit to search across all projects."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter. Matches file_path, description, and tags."
                    }
                },
                "required": []
            }),
        },
        ToolDescriptor {
            name: "cl_retrieve",
            description: "Pull the most relevant Context Library CONTENT for a query as ranked atom BODIES inline, under a token budget — instead of the cl_index_search -> eyeball descriptions -> Read the whole file loop. Each CL file is split into heading-delimited atoms; this returns the best BM25 matches (conventions/decisions win ties, newest first) formatted as `## file_path > heading_path` blocks. Cross-project `_globals` files (eod.md, tasks.md, …) are ALWAYS searched alongside the project you pass — their blocks are prefixed `[_globals]` — so never conclude a global file doesn't exist from a project-scoped miss. Use it when you need the actual CL content on a topic, not just which files exist.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project to search. Use the session's project for project-scoped CL, '_globals' for system-wide rules."
                    },
                    "query": {
                        "type": "string",
                        "description": "Free-text query, matched against atom heading_path + body (FTS5/BM25). Operators and punctuation are treated as literal words, so arbitrary text is safe."
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional: restrict retrieval to these file paths (relative to the project root, as in cl_index_search results)."
                    },
                    "budget_tokens": {
                        "type": "integer",
                        "description": "Optional soft cap on total body tokens returned (~chars/4 estimate). Defaults to 3000. The top atom is always returned even if it alone exceeds the budget."
                    }
                },
                "required": ["project", "query"]
            }),
        },
        ToolDescriptor {
            name: "cl_stale_refs",
            description: "Report Context Library claims that name code which is GONE — a symbol, flag or file the project's repo no longer contains. The CL is what bot-hq has over plain claude-code, and it is maintained by bot-hq sessions, so drift compounds silently: an audit once found a whole learning describing a connector deleted that same day, written in present tense. Run it during CL maintenance, before trusting notes that cite code. It REPORTS ONLY and never edits: `decisions.md` and `issues.md` are append-only history that legitimately names dead code, a learning explaining a deletion has to name what was deleted, and lines marked as retirements are skipped for that reason. Verify each hit against the repo yourself; writing nothing is a correct outcome.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project whose CL to check. It must have a registered repo — that repo is what the claims are checked against."
                    }
                },
                "required": ["project"]
            }),
        },
        ToolDescriptor {
            name: "cl_write_file",
            description: gated_by("cl_write_file", "Write a project-scoped Context Library file. Default mode replaces the ENTIRE file; mode:\"append\" adds content to the end instead (no read-modify-write needed). Direct write: missing parent folders are created, the write is atomic, the index rescans automatically, and every write is snapshotted into the library's local git history. Replacing a file with empty or >50%-smaller content is refused unless confirm_shrink:true (accidental-truncation guard — pass it when a prune is intentional). Also lifts the session's close-out learnings gate. bot-hq-owned _globals system files (custom-instructions.md, custom-general-rules.md, anything under _globals/agents/) are refused; so is a file the library marks agent-invisible; bodies over 1 MiB are refused. A successful write also pushes the library to its private remote, detached and fail-open."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project the file belongs to (e.g. 'bot-hq'). Use '_globals' for cross-project files like eod.md."
                    },
                    "file_path": {
                        "type": "string",
                        "description": "CL-relative path within the project (e.g. 'notes.md', 'plans/2026-07-21-handoff.md')."
                    },
                    "content": {
                        "type": "string",
                        "description": "The body to write. mode \"replace\": the full new file. mode \"append\": just the addition — it lands at the end of the existing file after a blank line."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "append"],
                        "description": "\"replace\" (default) overwrites the whole file; \"append\" adds to the end. Prefer append for learnings deltas."
                    },
                    "confirm_shrink": {
                        "type": "boolean",
                        "description": "Required true to replace an existing file with empty or >50%-smaller content. Confirms the data loss is intentional."
                    }
                },
                "required": ["project", "file_path", "content"]
            }),
        },
        ToolDescriptor {
            name: "cl_register_read",
            description: "Record that this agent read a CL file (a row in cl_reads). An audit trail only — nothing in the UI reads it back yet, so it changes nothing you see. Optional; a single awaited row insert (an unknown path is a no-op; only a real DB failure surfaces as an error).",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name the file belongs to."
                    },
                    "file_path": {
                        "type": "string",
                        "description": "Path relative to the project root (from cl_index_search results)."
                    }
                },
                "required": ["project", "file_path"]
            }),
        },
        ToolDescriptor {
            name: "cl_rescan",
            description: "Diff a project's filesystem against the index. Auto-registers new text files (.md/.yaml/.yml/.txt/.toml/.json; description = first H1 or first 80 chars), refreshes updated_at where mtime has moved, and removes orphan index rows for files that no longer exist. Use after creating CL files via Bash so the index stays in sync. Cheap to call.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name to rescan. Required."
                    }
                },
                "required": ["project"]
            }),
        },
        ToolDescriptor {
            name: "cl_folder_search",
            description: "Parallel to cl_index_search but for FOLDERS (directories) instead of files. Returns {project, folder_path, description, tags, updated_at} rows so agents can decide whether a folder is worth exploring without reading every file inside. `folder_path = \"\"` means the project root itself (the project-level description). Filter by project and/or substring query just like cl_index_search.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name. Use the session's project for project-scoped folders ('_globals' rows are always included alongside it), '_globals' alone for system-wide directories under the bot-hq root, or omit to search across all projects."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter over folder_path/description/tags."
                    }
                },
                "required": []
            }),
        },
        ToolDescriptor {
            name: "cl_register_folder_description",
            description: gated_by("cl_register_folder_description", "Upsert a description for a CL folder. Mirrors cl_register_read's role for files but writes a stored description instead of an audit row. A participant without it is denied and reviews via cl_folder_search instead. `folder_path = \"\"` writes the project-root description."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project the folder belongs to (use '_globals' for the bot-hq root tree)."
                    },
                    "folder_path": {
                        "type": "string",
                        "description": "Folder path relative to the project's CL root. Empty string targets the project root itself."
                    },
                    "description": {
                        "type": "string",
                        "description": "One-sentence description. Saved as-is into cl_folders.description."
                    },
                    "tags": {
                        "type": "string",
                        "description": "Optional comma-separated search hooks (e.g. 'PR-382, schema-v3')."
                    }
                },
                "required": ["project", "folder_path", "description"]
            }),
        },
        // Session terminal (Terminal subtab) — agents drive the session's PTY
        // where the user can watch. core/terminal.rs + bridge/terminal_tools.rs.
        ToolDescriptor {
            name: "terminal_exec",
            description: gated_by("terminal_exec", "Run ONE shell command in this session's Terminal subtab (the PTY the user watches — visible evidence, unlike your Bash tool). BLOCKING by default: types the command, waits for output to settle (~0.7s quiet or wait_ms cap, default 10s), and returns the captured output with a timed_out note when capped. Pass block:false for long-running processes (servers, watchers) and read later with terminal_read. The shell runs in the session's working repo. Commands matching a gated Tool-Gate keyword are refused — route those through action_gate. Settle is a quiet-window heuristic, not prompt detection: interactive/TUI commands may return early."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "One shell command (single line, no trailing newline)."
                    },
                    "wait_ms": {
                        "type": "integer",
                        "description": "Max total wait for output-settle, ms. Default 10000, clamped to [700, 120000]."
                    },
                    "block": {
                        "type": "boolean",
                        "description": "Default true. false = fire-and-forget (returns immediately; use terminal_read later)."
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDescriptor {
            name: "terminal_read",
            description: "Read the tail of this session's Terminal-subtab scrollback as plain text (default 100 lines, max 500) — works even after the shell exited. Paste it into chat or an IPAV session doc as fenced code when presenting terminal results as evidence.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "lines": {
                        "type": "integer",
                        "description": "How many trailing lines to return. Default 100, max 500."
                    }
                }
            }),
        },
        // Webview automation — agents test the bot-hq UI on their own.
        // These mirrored external_jsonrpc.rs's equivalents until the external
        // driver was deleted (2026-08-17); this is the only copy now.
        ToolDescriptor {
            name: "webview_screenshot",
            description: "Capture the bot-hq main window to a PNG under `<data_dir>/.local/screenshots/<ts>.png`. Returns `{path}`. Open the file with your built-in Read tool (which supports PNG). Use this AS YOUR EYES on what the user sees. macOS Screen Recording permission required; in a headless/agent environment the capture may not render.",
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        },
        ToolDescriptor {
            name: "webview_click",
            description: "Synthesize a click on the first DOM element matching the CSS selector in the bot-hq webview. Fire-and-forget — verify with a follow-up `webview_screenshot`. Examples: `a[href=\"/sessions/abc\"]`, `button[title^=\"Capture\"]`, `[data-testid=foo]`.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for the target element." }
                },
                "required": ["selector"]
            }),
        },
        ToolDescriptor {
            name: "webview_type",
            description: "Set the value of an input/textarea matched by the CSS selector + dispatch input/change events the React-friendly way (via the prototype value setter so React state actually updates). Element is also focused.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for an input or textarea." },
                    "text": { "type": "string", "description": "Text to set." }
                },
                "required": ["selector", "text"]
            }),
        },
        ToolDescriptor {
            name: "webview_scroll",
            description: "Scroll an element (if selector given) or the page. `y` is the destination scrollTop / scrollY in pixels.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "Optional CSS selector. Omit for window scroll." },
                    "y": { "type": "integer", "description": "Destination in pixels." }
                },
                "required": ["y"]
            }),
        },
        ToolDescriptor {
            name: "webview_press_key",
            description: "Synthesize keydown + keypress + keyup on a target element (or document.activeElement if no selector). Key names follow DOM KeyboardEvent.key: `Enter`, `Tab`, `Escape`, `ArrowDown`, `a`, etc.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "DOM key name." },
                    "selector": { "type": "string", "description": "Optional CSS selector for the target element." }
                },
                "required": ["key"]
            }),
        },
    ]
    });
    &TOOLS
}

/// What an MCP server returns from `tools/call`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallResult {
    pub content: Vec<ToolContentBlock>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContentBlock {
    Text { text: String },
}

impl ToolCallResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContentBlock::Text { text: s.into() }],
            is_error: false,
        }
    }
    pub fn error(s: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContentBlock::Text { text: s.into() }],
            is_error: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The canonical docs list every registered tool** (audit M-F10/M-T1).
    ///
    /// `ARCHITECTURE.md` said "Internal tools (36)" against a registry of 40,
    /// and `README.md`'s table was missing five — `pass_turn`, `file_feedback`,
    /// `gate_status`, `cl_stale_refs`, `cl_retrieve` — which is four tools'
    /// worth of drift plus one that was never added. Nothing compared either
    /// list to the code, so both were wrong and neither knew it.
    ///
    /// Asserted from the REGISTRY outward, not the other way round: a doc naming
    /// a tool that no longer exists is a different (and less costly) problem
    /// than a tool an agent can call and no human-facing doc mentions.
    #[test]
    fn every_registered_tool_is_documented() {
        let arch = include_str!("../../ARCHITECTURE.md");
        let readme = include_str!("../../README.md");
        // **LISTED, not merely mentioned** — the distinction is the whole test.
        // A bare backtick-name needle is satisfied by any prose mention, and 30
        // of the 40 tools are named more than once in at least one of these
        // docs, so their table row could be deleted with the test still green
        // (measured by EYES on `e300995`, by deleting `action_gate`'s row). The
        // needles below are the shapes a READER scans:
        //   * README's table row starts `| ` + backtick-name;
        //   * ARCHITECTURE's list is one paragraph, so the name must appear
        //     inside THAT paragraph rather than anywhere in the file.
        let arch_list = arch
            .split("**Internal tools (")
            .nth(1)
            .expect("ARCHITECTURE.md has an internal-tools list")
            .split("\n\n")
            .next()
            .expect("a split always yields a first part");
        let mut missing: Vec<String> = Vec::new();
        for d in tool_descriptors() {
            if !arch_list.contains(&format!("`{}`", d.name)) {
                missing.push(format!("ARCHITECTURE.md list: {}", d.name));
            }
            if !readme.contains(&format!("| `{}", d.name)) {
                missing.push(format!("README.md table: {}", d.name));
            }
        }
        assert!(
            missing.is_empty(),
            "tools an agent can call that the canonical docs do not LIST — a \
             prose mention elsewhere does not count, because a reader looking \
             for the tool surface reads the table and the list: {missing:?}"
        );
        // And the COUNT in ARCHITECTURE's heading, which is the part that read
        // as authoritative while being wrong.
        let n = tool_descriptors().len();
        assert!(
            arch.contains(&format!("**Internal tools ({n})**")),
            "ARCHITECTURE.md's tool count is not {n} — the registry moved and \
             the heading did not"
        );
    }

    /// `request_approval`'s `kind` enum is DERIVED from `ViolationKind::REQUESTABLE`
    /// (round 8) — every listed name parses back through the serde parser the
    /// handler uses, and the two audit-only kinds are absent. A hand-written
    /// list had to be kept in lockstep with the enum by eye.
    #[test]
    fn request_approval_kind_enum_is_the_requestable_kinds() {
        let d = tool_descriptors()
            .iter()
            .find(|d| d.name == "request_approval")
            .expect("request_approval is registered");
        let listed: Vec<String> = d.input_schema["properties"]["kind"]["enum"]
            .as_array()
            .expect("kind has an enum")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let expected: Vec<String> = crate::policy::ViolationKind::REQUESTABLE
            .iter()
            .map(|k| k.wire_name())
            .collect();
        assert_eq!(listed, expected);
        for name in &listed {
            assert!(
                serde_json::from_value::<crate::policy::ViolationKind>(serde_json::Value::String(
                    name.clone()
                ))
                .is_ok(),
                "{name} must parse as a ViolationKind"
            );
        }
        assert!(!listed.iter().any(|n| n == "policy_mutation" || n == "findings"));
    }

    /// The three descriptions an agent's stop discipline turns on must say what
    /// the tools DO (rc3 D35 / D37). Round 7 found `ask_user_choice` telling the
    /// agent "the session stays halted until then… stop and wait" — the exact
    /// opposite of the prompt layer, and an instruction that manufactures the
    /// silent stall the idle watchdog exists to catch — while `mark_awaiting_user`
    /// called itself "non-blocking… badge is set" and `advance_phase` promised to
    /// move the chip a vote had gated since that morning. Two answers in one
    /// context window is a defect whichever one the model picks.
    #[test]
    fn stop_and_phase_tool_descriptions_match_the_shipped_semantics() {
        let desc = |name: &str| {
            tool_descriptors()
                .iter()
                .find(|d| d.name == name)
                .unwrap_or_else(|| panic!("{name} must be registered"))
                .description
                .to_string()
        };
        let ask = desc("ask_user_choice");
        assert!(ask.contains("halts NOTHING"), "a question stops nothing: {ask}");
        assert!(ask.contains("carry on"), "the agent keeps working: {ask}");
        assert!(!ask.contains("stays halted"), "the pre-D35 claim must not come back: {ask}");
        assert!(!ask.contains("stop and wait"), "no stop-and-wait: {ask}");
        let halt = desc("mark_awaiting_user");
        assert!(halt.contains("HALT"), "it is the halt: {halt}");
        assert!(halt.contains("ring stops"), "and says the ring stops: {halt}");
        assert!(!halt.contains("non-blocking"), "a halt is not a badge: {halt}");
        let adv = desc("advance_phase");
        assert!(adv.contains("VOTE"), "advance_phase is a vote (D37): {adv}");
        assert!(adv.contains("NOT ADVANCED"), "and names the answer it gives: {adv}");
        assert!(adv.contains("phase doc first, then vote"), "doc-then-vote ordering: {adv}");
        assert!(!adv.contains("chip yourself"), "the self-move claim must not come back: {adv}");
        let req = desc("request_phase_advance");
        assert!(!req.contains("self-advance"), "no self-advance framing: {req}");
        // Round 8, second pass — the same defect class, four more tools.
        assert!(
            ask.contains("next dealt turn") && ask.contains("idle session (unless paused) is dealt one"),
            "the pick is read at the next dealt turn, at once when idle: {ask}"
        );
        assert!(
            !ask.contains("batched with the user's next send"),
            "an idle session does not wait for the user's next send: {ask}"
        );
        assert!(
            adv.contains("solo roster's own vote IS that consensus"),
            "a solo roster moves the chip on its own vote: {adv}"
        );
        let ovr = desc("override_reviewer_block");
        assert!(ovr.contains("REQUEST an override"), "it requests, it does not override: {ovr}");
        assert!(ovr.contains("lifts only on their Approve"), "the user decides: {ovr}");
        assert!(
            ovr.contains("An approved override auto-clears when the reviewer recovers"),
            "the recovery clear (bridge/mod.rs set_agent_health) is stated: {ovr}"
        );
        let pend = desc("list_my_pending_questions");
        assert!(!pend.contains("mark_awaiting_user halts"), "no halt rows exist since D35: {pend}");
        assert!(pend.contains("A halt is not a tray row"), "and it says where the halt lives: {pend}");
        let close = desc("close_session");
        assert!(close.contains("The FIRST call may answer"), "the first call may not close: {close}");
        assert!(!close.contains(". Fire-and-forget"), "the bare fire-and-forget promise is gone: {close}");
        let docw = desc("session_doc_write");
        assert!(docw.contains("`<phase>-eyes`"), "the reviewer redirect is described: {docw}");
        assert!(docw.contains("`<slug>@<n>`"), "the archives are described: {docw}");
    }

    /// **Nothing an agent reads names a person (rc3 D10).**
    ///
    /// The name-removal commit claimed this and it was not true: six tool
    /// descriptions still said `EYES-only (rain)` / `HANDS-only (brian)`, and
    /// tool descriptions are read by every agent on every session, so they are
    /// among the most-read text in the product. Sweeping the WHOLE descriptor
    /// list — rather than the six that were wrong — is what stops the seventh.
    ///
    /// **It used to cover two lists.** The external driver had its own toolset,
    /// and the first version of this sweep skipped it — so `create_session` kept
    /// shipping agent-named parameters to a driver reading them exactly as a
    /// spawned agent does. One list is all that remains to cover.
    ///
    /// **It now runs with NO exemptions at all.** The only ones it ever had were
    /// `external_jsonrpc::wire`'s frozen identifiers — keys an external driver
    /// sent or the server returned — and the external driver was removed when
    /// the user demoted it to a future plugin. Deleting a published wire deleted
    /// the reason its agent-named keys had to be tolerated, so this sweep got
    /// strictly stronger by losing a caller.
    ///
    /// Word-boundary matched: `constraint` contains `rain`, and a substring
    /// check would fail on prose that is perfectly fine.
    ///
    /// **This never ran until round 2.** Its `#[test]` was written above the
    /// PREVIOUS test's doc comment, so Rust merged both doc blocks and both
    /// attributes onto `every_registered_tool_is_documented` and left this
    /// function with neither — an ordinary private fn nothing called. The
    /// compiler said so twice, in every build: `duplicated attribute` on the
    /// pair, and `function is never used` here. Two warnings nobody read, for a
    /// guard written to stop a regression that had already happened once.
    #[test]
    fn no_tool_description_an_agent_reads_names_an_agent() {
        for d in tool_descriptors().iter() {
            let prose =
                format!("{} {} {}", d.name, d.description, d.input_schema).to_lowercase();
            for word in prose.split(|c: char| !c.is_alphanumeric()) {
                assert!(
                    word != "brian" && word != "rain",
                    "tool `{}` still names an agent: {}",
                    d.name,
                    d.description
                );
            }
            // **And no phrase that assumes there are exactly two of them.** The
            // name sweep above passed while six descriptions still said "the
            // duo", "both agents", "either agent" and "peer-forwarding" (audit
            // C1-2) — an N-participant session reads those as instructions about
            // a session shape it does not have, and "peer-forward" names a
            // router deleted in `3adfc68`. Word-matching brian/rain cannot see
            // any of it, which is why this second sweep exists.
            for phrase in [
                "the duo",
                "duo's",
                "both agents",
                "either agent",
                "peer-forward",
                "peer forwarding",
            ] {
                assert!(
                    !prose.contains(phrase),
                    "tool `{}` says `{phrase}` — that is a two-participant \
                     assumption (or deleted-router vocabulary) in prose every \
                     agent reads: {}",
                    d.name,
                    d.description
                );
            }
        }
    }

    /// **The gate line names the capability the gate actually reads.**
    ///
    /// The first version of this test carried its own table of (tool,
    /// capability) pairs and THREE OF FOUR were wrong — `disposition_finding`
    /// and `override_reviewer_block` were pinned to `edit_files`,
    /// `approve_finding` to `file_finding` — so it certified the drift it was
    /// written to catch. `tools/list` is unfiltered, so every agent in every
    /// session read those lines, while the same agent's layer-2 prompt section
    /// (generated from `required_for`) told it the opposite.
    ///
    /// Nothing here spells a capability. The expectation is computed from
    /// `capability::required_for` — the map `gated_by` renders from and
    /// `jsonrpc::call_tool` gates on — so the only way to pass is to agree with
    /// the runtime. Three rules, all derived:
    ///
    /// 1. an ENFORCED gated tool states its capability;
    /// 2. no descriptor names a capability that is not its gate;
    /// 3. a tool the gate does not enforce (none today — `jsonrpc::PARITY_HOLD` is empty) claims
    ///    nothing — reframe-contract rule 2: nothing ships an enforcement that
    ///    is not wired.
    #[test]
    fn every_gated_tool_names_the_capability_its_gate_reads() {
        use crate::agents::capability::{required_for, Capability};
        let mut enforced = 0;
        for d in tool_descriptors() {
            let claimed: Vec<&str> = Capability::ALL
                .into_iter()
                .filter(|c| d.description.contains(&format!("`{}` capability", c.slug())))
                .map(|c| c.slug())
                .collect();
            let gate =
                required_for(d.name).filter(|_| crate::signaling::jsonrpc::capability_gated(d.name));
            match gate {
                Some(cap) => {
                    enforced += 1;
                    assert_eq!(
                        claimed,
                        vec![cap.slug()],
                        "`{}` is gated on `{}`; its description claims {:?}",
                        d.name,
                        cap.slug(),
                        claimed
                    );
                }
                None => assert!(
                    claimed.is_empty(),
                    "`{}` claims {:?} but nothing enforces it — a described gate that \
                     does not exist is worse than no line at all",
                    d.name,
                    claimed
                ),
            }
        }
        assert!(
            enforced >= 14,
            "only {enforced} enforced gated tools were checked — the descriptor list \
             or the gate map shrank without this test noticing"
        );
    }
}
