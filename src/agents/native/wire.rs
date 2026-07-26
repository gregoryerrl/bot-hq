//! Anthropic Messages wire format ↔ [`AgentEvent`].
//!
//! Pure functions only: build a request body, parse a response into the events
//! the rest of bot-hq already consumes, and assemble the tool-result message.
//! No IO — the loop (B3) owns the HTTP and the channels, this module owns the
//! shapes.
//!
//! ## The three traps
//!
//! Encoded as tests, not comments, because they are the bugs hand-rolled loops
//! actually hit:
//!
//! 1. **Echo the assistant `content` array back byte-identical**, `thinking`
//!    blocks included. Drop or edit them and the *current* request succeeds,
//!    then the NEXT one 400s on a signature/ordering check.
//! 2. **Every `tool_result` goes in ONE user message.** Splitting them across
//!    messages silently trains the model out of making parallel tool calls.
//! 3. **Check `stop_reason == "refusal"` BEFORE touching `content`.** On a
//!    decline `content` can be empty, so code that indexes `content[0]`
//!    unconditionally panics exactly when a safety classifier fires.

use crate::agents::spawn::{AgentEvent, ContextUsage};
use serde_json::{json, Value};

/// Everything needed to build one `POST /v1/messages` body.
pub struct RequestSpec<'a> {
    pub model: &'a str,
    pub max_tokens: u32,
    /// Top-level `system` — never a `role:"system"` entry inside `messages`.
    /// That shape is precisely what `llm_proxy.rs` exists to rewrite out of
    /// claude-code's requests; the native path must not reintroduce it.
    pub system: Option<&'a str>,
    pub tools: &'a [Value],
    pub messages: &'a [Value],
}

/// A tool the model asked us to run.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// The result of running one [`ToolCall`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

/// What one response means for the loop's control flow.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnStep {
    /// Run `calls`, append their results as ONE user message, call again.
    ToolUse { calls: Vec<ToolCall> },
    /// Turn finished normally. `text` is the concatenated assistant prose.
    End { text: String },
    /// Model declined (trap 3).
    Refusal { details: Value },
    /// Output budget exhausted. On Opus 5 thinking and response text share this
    /// budget, so this usually means "raise `max_tokens`", not "long answer".
    MaxTokens,
    /// A `stop_reason` this loop does not handle. Carried rather than panicked
    /// on so the caller can surface it as an `Error` event.
    Unhandled { stop_reason: String },
}

/// A parsed response: control flow, the events to emit, and the history entry.
#[derive(Debug, Clone)]
pub struct ParsedTurn {
    pub step: TurnStep,
    /// `Text` / `ToolUse` events in content order, ready to send downstream.
    pub events: Vec<AgentEvent>,
    /// The assistant message to append to history — the FULL `content` array,
    /// byte-identical (trap 1). `None` when `content` was absent/empty, which
    /// is the refusal case.
    pub assistant_message: Option<Value>,
    /// Context occupancy, when the window is known. `None` renders as a visible
    /// gap in the UI, never as a guessed percentage.
    pub context: Option<ContextUsage>,
}

/// Build the request body.
///
/// Deliberately absent: `temperature`, `top_p`, `top_k`, and
/// `thinking.budget_tokens`. All four are hard 400s on Claude Opus 5.
pub fn build_request(spec: &RequestSpec) -> Value {
    let mut body = json!({
        "model": spec.model,
        "max_tokens": spec.max_tokens,
        "messages": spec.messages,
    });

    let map = body.as_object_mut().expect("json! built an object");
    if let Some(system) = spec.system.filter(|s| !s.is_empty()) {
        map.insert("system".into(), json!(system));
    }
    // Omit `tools` entirely when empty — some gateways reject `"tools": []`.
    if !spec.tools.is_empty() {
        map.insert("tools".into(), json!(spec.tools));
    }
    body
}

/// Parse a successful (2xx) response body.
///
/// `window` comes from the [`ProviderProfile`](super::profile::ProviderProfile);
/// `None` there means no `ContextUsage` is produced at all.
pub fn parse_turn(resp: &Value, model: &str, window: Option<u64>) -> ParsedTurn {
    let stop_reason = resp
        .get("stop_reason")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let context = usage_to_context(resp.get("usage"), model, window);

    // TRAP 3 — before `content` is touched.
    if stop_reason == "refusal" {
        return ParsedTurn {
            step: TurnStep::Refusal {
                details: resp.get("stop_details").cloned().unwrap_or(Value::Null),
            },
            events: Vec::new(),
            assistant_message: None,
            context,
        };
    }

    let content = resp
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut events = Vec::new();
    let mut calls = Vec::new();
    let mut text = String::new();

    for block in &content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                    events.push(AgentEvent::Text(t.to_string()));
                }
            }
            Some("tool_use") => {
                let call = ToolCall {
                    id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    input: block.get("input").cloned().unwrap_or(Value::Null),
                };
                events.push(AgentEvent::ToolUse {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    input: call.input.clone(),
                });
                calls.push(call);
            }
            // `thinking` and any future block type: not surfaced as an event,
            // but MUST survive into `assistant_message` untouched (trap 1).
            _ => {}
        }
    }

    // TRAP 1 — the full array, unmodified, straight from the response.
    let assistant_message = (!content.is_empty()).then(|| json!({
        "role": "assistant",
        "content": content,
    }));

    let step = match stop_reason {
        // A `tool_use` stop with no `tool_use` block is a protocol violation.
        // Reporting it as `ToolUse { calls: [] }` would have the loop post
        // `{"role":"user","content":[]}` on the next hop, which is itself a
        // 400 — so the real fault would surface as a confusing downstream
        // error instead of here, where it happened.
        "tool_use" if calls.is_empty() => TurnStep::Unhandled {
            stop_reason: "tool_use (no tool_use block present)".into(),
        },
        "tool_use" => TurnStep::ToolUse { calls },
        "end_turn" | "stop_sequence" => TurnStep::End { text },
        "max_tokens" => TurnStep::MaxTokens,
        other => TurnStep::Unhandled {
            stop_reason: other.to_string(),
        },
    };

    ParsedTurn {
        step,
        events,
        assistant_message,
        context,
    }
}

/// TRAP 2 — assemble ALL tool results into ONE user message.
pub fn tool_results_message(outcomes: &[ToolOutcome]) -> Value {
    let blocks: Vec<Value> = outcomes
        .iter()
        .map(|o| {
            let mut b = json!({
                "type": "tool_result",
                "tool_use_id": o.tool_use_id,
                "content": o.content,
            });
            if o.is_error {
                b.as_object_mut()
                    .expect("json! built an object")
                    .insert("is_error".into(), json!(true));
            }
            b
        })
        .collect();

    json!({ "role": "user", "content": blocks })
}

/// Context occupancy from a response `usage` object.
///
/// Numerator matches the claude-code path exactly
/// (`input + cache_read_input + cache_creation_input`, see [`ContextUsage`]):
/// cached tokens change what a token *costs*, not whether it *occupies the
/// window*. Omitting them under-reports by orders of magnitude.
///
/// Returns `None` when the window is unknown or zero — no division by zero, and
/// no guessed percentage.
///
/// Also returns `None` when `usage` carries **none** of the three fields, rather
/// than reporting a confident `0`. A gateway that spells them differently would
/// otherwise render as a flat 0% forever, which is exactly the failure the
/// claude-code path guards against ("a known-wrong value is worse than none",
/// `events.rs`). One field present is enough — a turn really can have zero cache
/// tokens.
pub fn usage_to_context(
    usage: Option<&Value>,
    model: &str,
    window: Option<u64>,
) -> Option<ContextUsage> {
    const FIELDS: [&str; 3] = [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ];

    let window = window.filter(|w| *w > 0)?;
    let usage = usage?;

    let mut seen_any = false;
    let mut used_tokens = 0u64;
    for key in FIELDS {
        if let Some(n) = usage.get(key).and_then(Value::as_u64) {
            seen_any = true;
            used_tokens += n;
        }
    }
    if !seen_any {
        return None;
    }

    Some(ContextUsage {
        model: model.to_string(),
        used_tokens,
        context_window: window,
    })
}

/// The `TurnComplete` that ends a SUCCESSFUL turn.
pub fn turn_complete_ok(stop_reason: &str, context: Option<ContextUsage>) -> AgentEvent {
    AgentEvent::TurnComplete {
        stop_reason: Some(stop_reason.to_string()),
        subtype: Some("success".into()),
        is_error: false,
        api_error_status: None,
        context,
    }
}

/// The `TurnComplete` that ends a FAILED turn.
///
/// `status` is the upstream HTTP status when the failure was an API error; the
/// retry supervisor reads exactly this field through `is_transient_api_error` to
/// decide auto-resume vs. surface. A failed turn's buffered text must never be
/// peer-forwarded — hence `is_error: true` is non-negotiable here.
pub fn turn_complete_err(status: Option<u16>, subtype: &str) -> AgentEvent {
    AgentEvent::TurnComplete {
        stop_reason: None,
        subtype: Some(subtype.to_string()),
        is_error: true,
        api_error_status: status,
        context: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp_with_thinking_and_tool() -> Value {
        json!({
            "stop_reason": "tool_use",
            "content": [
                { "type": "thinking", "thinking": "hmm", "signature": "sig-abc" },
                { "type": "text", "text": "Let me check." },
                { "type": "tool_use", "id": "tu_1", "name": "read_file",
                  "input": { "path": "Cargo.toml" } }
            ],
            "usage": { "input_tokens": 10 }
        })
    }

    // ---- trap 1 ----------------------------------------------------------

    #[test]
    fn trap1_assistant_message_echoes_content_byte_identical() {
        let resp = resp_with_thinking_and_tool();
        let parsed = parse_turn(&resp, "m", None);
        let msg = parsed.assistant_message.expect("tool_use turn has content");

        assert_eq!(msg["role"], "assistant");
        // Not "equivalent" — identical to what the server sent.
        assert_eq!(msg["content"], resp["content"]);
    }

    #[test]
    fn trap1_thinking_block_survives_with_its_signature() {
        let parsed = parse_turn(&resp_with_thinking_and_tool(), "m", None);
        let msg = parsed.assistant_message.unwrap();
        let thinking = &msg["content"][0];

        assert_eq!(thinking["type"], "thinking");
        // The signature is what the next request is validated against; losing
        // it 400s the FOLLOWING call, not this one.
        assert_eq!(thinking["signature"], "sig-abc");
    }

    #[test]
    fn trap1_unknown_block_types_are_preserved_too() {
        let resp = json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "some_future_block", "payload": { "a": 1 } }]
        });
        let parsed = parse_turn(&resp, "m", None);
        assert_eq!(
            parsed.assistant_message.unwrap()["content"],
            resp["content"]
        );
    }

    // ---- trap 2 ----------------------------------------------------------

    #[test]
    fn trap2_all_tool_results_land_in_one_user_message() {
        let msg = tool_results_message(&[
            ToolOutcome {
                tool_use_id: "tu_1".into(),
                content: "ok".into(),
                is_error: false,
            },
            ToolOutcome {
                tool_use_id: "tu_2".into(),
                content: "boom".into(),
                is_error: true,
            },
        ]);

        assert_eq!(msg["role"], "user");
        let blocks = msg["content"].as_array().expect("content is an array");
        assert_eq!(blocks.len(), 2, "both results must share ONE user message");
        assert_eq!(blocks[0]["tool_use_id"], "tu_1");
        assert_eq!(blocks[1]["tool_use_id"], "tu_2");
    }

    #[test]
    fn trap2_is_error_is_set_only_on_failures() {
        let msg = tool_results_message(&[
            ToolOutcome {
                tool_use_id: "tu_1".into(),
                content: "ok".into(),
                is_error: false,
            },
            ToolOutcome {
                tool_use_id: "tu_2".into(),
                content: "boom".into(),
                is_error: true,
            },
        ]);
        let blocks = msg["content"].as_array().unwrap();
        assert!(blocks[0].get("is_error").is_none());
        assert_eq!(blocks[1]["is_error"], true);
    }

    // ---- trap 3 ----------------------------------------------------------

    #[test]
    fn trap3_refusal_with_empty_content_does_not_panic() {
        let resp = json!({
            "stop_reason": "refusal",
            "content": [],
            "stop_details": { "reason": "classifier" }
        });
        let parsed = parse_turn(&resp, "m", None);
        assert!(matches!(parsed.step, TurnStep::Refusal { .. }));
        assert!(parsed.assistant_message.is_none());
    }

    #[test]
    fn trap3_refusal_with_absent_content_does_not_panic() {
        let parsed = parse_turn(&json!({ "stop_reason": "refusal" }), "m", None);
        match parsed.step {
            TurnStep::Refusal { details } => assert_eq!(details, Value::Null),
            other => panic!("expected Refusal, got {other:?}"),
        }
    }

    // ---- events ----------------------------------------------------------

    #[test]
    fn events_follow_content_order_and_skip_thinking() {
        let parsed = parse_turn(&resp_with_thinking_and_tool(), "m", None);
        assert_eq!(parsed.events.len(), 2, "thinking is not a downstream event");
        assert!(matches!(&parsed.events[0], AgentEvent::Text(t) if t == "Let me check."));
        match &parsed.events[1] {
            AgentEvent::ToolUse { id, name, .. } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "read_file");
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_step_carries_the_calls() {
        let parsed = parse_turn(&resp_with_thinking_and_tool(), "m", None);
        match parsed.step {
            TurnStep::ToolUse { calls } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].input["path"], "Cargo.toml");
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn end_turn_concatenates_text_blocks() {
        let resp = json!({
            "stop_reason": "end_turn",
            "content": [
                { "type": "text", "text": "one " },
                { "type": "text", "text": "two" }
            ]
        });
        match parse_turn(&resp, "m", None).step {
            TurnStep::End { text } => assert_eq!(text, "one two"),
            other => panic!("expected End, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_stop_with_no_tool_use_block_is_flagged_here_not_downstream() {
        // Letting this through as `ToolUse { calls: [] }` makes the loop post
        // an empty tool_results message, so the 400 lands a hop away from the
        // actual fault.
        let resp = json!({
            "stop_reason": "tool_use",
            "content": [{ "type": "text", "text": "no tool here" }]
        });
        match parse_turn(&resp, "m", None).step {
            TurnStep::Unhandled { stop_reason } => assert!(stop_reason.contains("tool_use")),
            other => panic!("expected Unhandled, got {other:?}"),
        }
    }

    #[test]
    fn unknown_stop_reason_is_carried_not_panicked_on() {
        let resp = json!({ "stop_reason": "pause_turn", "content": [] });
        match parse_turn(&resp, "m", None).step {
            TurnStep::Unhandled { stop_reason } => assert_eq!(stop_reason, "pause_turn"),
            other => panic!("expected Unhandled, got {other:?}"),
        }
    }

    // ---- request building ------------------------------------------------

    #[test]
    fn request_puts_system_at_top_level_never_in_messages() {
        let msgs = vec![json!({ "role": "user", "content": "hi" })];
        let body = build_request(&RequestSpec {
            model: "m",
            max_tokens: 100,
            system: Some("you are rain"),
            tools: &[],
            messages: &msgs,
        });

        assert_eq!(body["system"], "you are rain");
        // The exact shape `llm_proxy.rs` exists to rewrite away.
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["role"].as_str())
            .collect();
        assert!(!roles.contains(&"system"));
    }

    #[test]
    fn request_omits_empty_tools_and_absent_system() {
        let msgs = vec![json!({ "role": "user", "content": "hi" })];
        let body = build_request(&RequestSpec {
            model: "m",
            max_tokens: 100,
            system: None,
            tools: &[],
            messages: &msgs,
        });
        assert!(body.get("tools").is_none(), "empty tools 400s some gateways");
        assert!(body.get("system").is_none());
    }

    #[test]
    fn request_omits_the_four_fields_that_400_on_opus_5() {
        let msgs = vec![json!({ "role": "user", "content": "hi" })];
        let body = build_request(&RequestSpec {
            model: "claude-opus-5",
            max_tokens: 100,
            system: None,
            tools: &[],
            messages: &msgs,
        });
        for banned in ["temperature", "top_p", "top_k", "thinking"] {
            assert!(body.get(banned).is_none(), "{banned} is a hard 400 on Opus 5");
        }
    }

    // ---- context accounting ---------------------------------------------

    #[test]
    fn context_includes_cache_tokens_in_the_numerator() {
        let usage = json!({
            "input_tokens": 2,
            "cache_read_input_tokens": 20_000,
            "cache_creation_input_tokens": 3_955,
            "output_tokens": 500
        });
        let ctx = usage_to_context(Some(&usage), "m", Some(200_000)).unwrap();
        // Excluding cache reads would report 2 instead of 23,957.
        assert_eq!(ctx.used_tokens, 23_957);
        assert_eq!(ctx.context_window, 200_000);
    }

    #[test]
    fn context_excludes_output_tokens() {
        let usage = json!({ "input_tokens": 100, "output_tokens": 900 });
        let ctx = usage_to_context(Some(&usage), "m", Some(1_000)).unwrap();
        assert_eq!(ctx.used_tokens, 100);
    }

    #[test]
    fn unknown_window_yields_no_context_rather_than_a_guess() {
        let usage = json!({ "input_tokens": 100 });
        assert!(usage_to_context(Some(&usage), "m", None).is_none());
        assert!(usage_to_context(Some(&usage), "m", Some(0)).is_none());
    }

    #[test]
    fn absent_usage_yields_no_context() {
        assert!(usage_to_context(None, "m", Some(200_000)).is_none());
    }

    #[test]
    fn usage_with_no_recognised_fields_yields_no_context_rather_than_zero() {
        // A gateway spelling these differently would otherwise pin the meter at
        // a confident 0% forever — a wrong number, not a visible gap.
        let usage = json!({ "inputTokens": 5_000, "outputTokens": 10 });
        assert!(usage_to_context(Some(&usage), "m", Some(200_000)).is_none());
    }

    #[test]
    fn one_recognised_field_is_enough_since_cache_tokens_can_be_zero() {
        let usage = json!({ "input_tokens": 42 });
        let ctx = usage_to_context(Some(&usage), "m", Some(200_000)).unwrap();
        assert_eq!(ctx.used_tokens, 42);
    }

    // ---- TurnComplete contract ------------------------------------------

    #[test]
    fn failed_turn_carries_the_status_the_supervisor_classifies_on() {
        match turn_complete_err(Some(529), "api_error") {
            AgentEvent::TurnComplete {
                is_error,
                api_error_status,
                ..
            } => {
                assert!(is_error, "a failed turn must never be peer-forwarded");
                assert_eq!(api_error_status, Some(529));
                assert!(crate::agents::spawn::is_transient_api_error(529));
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    #[test]
    fn successful_turn_reports_no_error_status() {
        match turn_complete_ok("end_turn", None) {
            AgentEvent::TurnComplete {
                is_error,
                api_error_status,
                stop_reason,
                ..
            } => {
                assert!(!is_error);
                assert_eq!(api_error_status, None);
                assert_eq!(stop_reason.as_deref(), Some("end_turn"));
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }
}
