//! Stdout/stderr readers for an agent subprocess. Parses one stream-json
//! event per line; translates wire events into high-level `AgentEvent`.

use crate::agents::protocol::*;
use crate::agents::spawn::{AgentEvent, ContextReport, ContextVerdict};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Pump stdout-like stream until EOF; send translated events to `tx`.
/// Generic over the reader type so this is testable with `tokio::io::duplex`.
pub async fn pump_events<R: AsyncRead + Unpin>(reader: R, tx: mpsc::Sender<AgentEvent>) {
    let buf = BufReader::new(reader);
    let mut lines = buf.lines();
    // Per-turn carry: the most recent `assistant` event's point-in-time usage.
    // Lives out here because the correct context numerator arrives on a
    // DIFFERENT event than the one that reports the turn is over.
    let mut last_assistant_usage: Option<serde_json::Value> = None;
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<StreamEvent>(trimmed) {
                    Ok(ev) => {
                        for app_ev in translate(ev, &mut last_assistant_usage) {
                            if tx.send(app_ev).await.is_err() {
                                return; // receiver dropped, peer is gone
                            }
                        }
                    }
                    Err(err) => {
                        warn!(
                            error = %err,
                            line = %short_line(trimmed),
                            "stream-json parse error"
                        );
                    }
                }
            }
            Ok(None) => return,
            Err(err) => {
                warn!(error = %err, "stdout read error");
                return;
            }
        }
    }
}

pub async fn pump_stderr<R: AsyncRead + Unpin>(reader: R, agent_name: String) {
    let buf = BufReader::new(reader);
    let mut lines = buf.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // The CLI announces a model id its catalog does not know on stderr
        // (`[claude-code:unrecognized_model] {"model":…}`, 2.1.251). For a
        // non-Claude id it still prints this with the row's window applied
        // (CLAUDE_CODE_MAX_CONTEXT_TOKENS, measured 2.1.291), so the line
        // alone does not mean 200k; the pump's window notice compares the
        // numbers. Worth a WARN, not a DEBUG: it is the first sign that the
        // CLI is on its own catalog for this id.
        if line.contains("unrecognized_model") {
            warn!(target: "agent_stderr", agent = %agent_name, msg = %line, "claude CLI does not recognise the model id — a non-Claude id runs at the model row's Context window (nothing set = the CLI default); a Claude id it does not know yet needs the row's CLI settings (modelOverrides)");
        } else {
            debug!(target: "agent_stderr", agent = %agent_name, msg = %line);
        }
    }
}

/// The first 160 CHARS of an unparseable line for the log — by chars, not
/// bytes: model stdout is arbitrary text and a byte-offset slice inside a
/// multibyte char panics the pump (round 9).
fn short_line(s: &str) -> String {
    let cut = crate::text::floor_char_boundary(s, 160);
    if cut == s.len() {
        s.to_string()
    } else {
        format!("{}…", &s[..cut])
    }
}

/// The model name claude-code gives the messages it writes itself.
pub const SYNTHETIC_MODEL: &str = "<synthetic>";

/// The `hook_name` claude-code reports for a `SessionStart` hook that ran
/// because of a compaction (probed on CLI 2.1.284, s-d43b3630).
pub const COMPACT_HOOK_NAME: &str = "SessionStart:compact";

/// Translate a wire `StreamEvent` to zero or more `AgentEvent`s.
/// `assistant` events with multiple content blocks fan out to multiple events.
///
/// `last_assistant_usage` is per-turn carry owned by the caller: `assistant`
/// events write their point-in-time usage into it, and the `result` event reads
/// it back as the context numerator. The state exists because the correct
/// reading and the turn-completion signal arrive on different events — see
/// `AssistantMessage::usage` for why `result.usage` cannot be used instead.
pub fn translate(
    ev: StreamEvent,
    last_assistant_usage: &mut Option<serde_json::Value>,
) -> Vec<AgentEvent> {
    match ev {
        StreamEvent::System(sys) => match sys {
            SystemEvent::Init {
                session_id,
                api_key_source,
                ..
            } => {
                vec![AgentEvent::Init {
                    session_id,
                    api_key_source,
                }]
            }
            SystemEvent::BackgroundTasksChanged { tasks } => {
                vec![AgentEvent::BackgroundTasks { running: tasks.len() }]
            }
            SystemEvent::CompactBoundary { compact_metadata } => {
                let m = compact_metadata.unwrap_or_default();
                vec![AgentEvent::Compacted {
                    trigger: m.trigger,
                    pre_tokens: m.pre_tokens,
                    post_tokens: m.post_tokens,
                }]
            }
            // Only bot-hq's OWN post-compaction hook: the user's plugins run
            // `SessionStart:compact` hooks under the same name, so the marker
            // the handoff file opens with is what tells them apart.
            SystemEvent::HookResponse { hook_name, output, .. }
                if hook_name.as_deref() == Some(COMPACT_HOOK_NAME) =>
            {
                output
                    .as_deref()
                    .and_then(crate::agents::handoff::injected_doc)
                    .map(|has_doc| vec![AgentEvent::HandoffInjected { has_doc }])
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        },
        // **A helper's messages are not the participant's** (feedback #47).
        // claude-code streams a background or foreground subagent's assistant
        // messages and tool results on the PARENT's stdout, tagged with the
        // Agent call that launched it. Translated, they became the parent's
        // chat rows (a helper's "I'll start by reading the input file." under
        // HANDS' name), read as the parent opening a turn — which, while a peer
        // held the ring, let D3 stop the parent and so cancel every helper it
        // had running — and overwrote the parent's context reading with the
        // helper's. The helper's work reaches the parent through the Agent
        // call's own result; nothing here is the parent's to post.
        StreamEvent::Assistant(asst) if asst.parent_tool_use_id.is_some() => Vec::new(),
        StreamEvent::User(u) if u.parent_tool_use_id.is_some() => Vec::new(),
        StreamEvent::Assistant(asst) => {
            // Overwrite rather than accumulate: we want the LAST call's prompt
            // size, which is the turn's final context, not a running total.
            if let Some(u) = asst.message.usage.clone() {
                *last_assistant_usage = Some(u);
            }
            // claude-code marks its OWN messages — an API error, a usage limit —
            // with the model `<synthetic>` (its binary tests
            // `isApiErrorMessage && message.model === "<synthetic>"`, probed on
            // CLI 2.1.281). That text is not the participant speaking
            // (feedback #17); a participant's own prose that merely starts
            // "API Error:" stays its speech.
            let synthetic = asst.message.model.as_deref() == Some(SYNTHETIC_MODEL);
            asst.message
            .content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } if synthetic => Some(AgentEvent::Notice(text)),
                ContentBlock::Text { text } => Some(AgentEvent::Text(text)),
                ContentBlock::ToolUse { id, name, input } => Some(AgentEvent::ToolUse {
                    id,
                    name,
                    input,
                }),
                ContentBlock::Thinking { .. } => None,
                ContentBlock::Other => None,
            })
            .collect()
        }
        StreamEvent::User(u) => match u.message.content {
            UserContent::Blocks(blocks) => blocks
                .into_iter()
                .filter_map(|b| match b {
                    UserContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        let content = match content {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        };
                        Some(AgentEvent::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        })
                    }
                    UserContentBlock::Other => None,
                })
                .collect(),
            UserContent::Text(_) => Vec::new(),
        },
        StreamEvent::Result(r) => {
            // A turn failed if claude-code set the explicit error flag OR an
            // upstream API status is populated (e.g. the DeepSeek 400). Both
            // are absent/false on success. We deliberately do NOT infer
            // failure from a non-`success` subtype alone — unknown-but-benign
            // subtypes shouldn't wrongly suppress a legit turn's forward.
            let is_error = r.is_error || r.api_error_status.is_some();
            let api_error_status = extract_api_status(&r.api_error_status);
            // `take()` — clear the carry at turn end so a turn that produces no
            // assistant usage (errored, or a provider that omits it) cannot
            // silently inherit the PREVIOUS turn's reading and report it as
            // current. Deliberately NOT falling back to `r.usage`: that number
            // is a per-turn sum, and a known-wrong value is worse than none.
            // A `result` with no `modelUsage` map at all reports no window —
            // the same fact as a map whose entries carry none, and recorded as
            // the same verdict (rc3 P7).
            let context = match r.model_usage.as_ref() {
                Some(mu) => parse_context_usage(last_assistant_usage.take().as_ref(), mu),
                None => ContextReport::none(ContextVerdict::NoWindow),
            };
            vec![AgentEvent::TurnComplete {
                stop_reason: r.stop_reason,
                subtype: r.subtype,
                is_error,
                api_error_status,
                context,
            }]
        }
        StreamEvent::RateLimit(_) => Vec::new(),
        StreamEvent::Unknown => Vec::new(),
    }
}

/// Extract context-window occupancy from a `result` event.
///
/// # THREE token objects, and only one is the numerator
///
/// The stream carries three near-identical-looking token counts. Two of them
/// are aggregates that look plausible and are wrong. Both were shipped as the
/// numerator before being caught:
///
/// | Object | Meaning | Usable as numerator? |
/// |---|---|---|
/// | `result.modelUsage[m]` | **Cumulative for the whole session** | No — unbounded growth |
/// | `result.usage` | **Sum of every API call in the turn** | No — scales with tool calls |
/// | `assistant.message.usage` | **This API call's prompt size** | **Yes** |
///
/// Measured, session-level (3 turns, one API call each):
///
/// ```text
/// turn 1:  usage=23,823   modelUsage=23,823   (identical — first turn)
/// turn 2:  usage=25,248   modelUsage=49,071   (= 23,823 + 25,248)
/// ```
///
/// Measured, turn-level (one turn, three API calls):
///
/// ```text
/// assistant#1 usage=33,917
/// assistant#2 usage=33,917
/// assistant#3 usage=34,216   <- the real current context
/// result      usage=68,133   <- 33,917 + 34,216, not a prompt size
/// ```
///
/// **Single-call, single-turn fixtures cannot distinguish any of the three** —
/// they are identical on turn 1 of a one-call turn. That is precisely why the
/// first two bugs passed every test. Fixtures here are deliberately
/// multi-call and multi-turn shaped.
///
/// This function therefore takes the numerator as an argument: the caller
/// (`translate`) carries the last `assistant` usage forward. `modelUsage` is
/// still required for `contextWindow`, which appears **nowhere else** — not in
/// `usage`, not in the on-disk transcripts.
///
/// Returns `None` whenever the denominator is unavailable, so the UI shows a
/// gap rather than a fabricated percentage.
///
/// **Multi-entry rule:** a turn that dispatched a subagent on another model
/// carries several keys. We pick the entry with the largest *cumulative* usage
/// — the primary conversation dominates its subagents — and take its window.
/// Cumulative totals are the right signal for *which model is primary* even
/// though they are the wrong signal for *how full it is*.
fn parse_context_usage(
    usage: Option<&serde_json::Value>,
    model_usage: &serde_json::Value,
) -> ContextReport {
    /// Cumulative per-model fields (camelCase) — used ONLY to identify the
    /// primary model, never as the numerator.
    const MODEL_FIELDS: [&str; 3] = [
        "inputTokens",
        "cacheReadInputTokens",
        "cacheCreationInputTokens",
    ];
    /// Point-in-time fields on the top-level `usage` object (snake_case).
    const USAGE_FIELDS: [&str; 3] = [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ];

    fn sum(obj: &serde_json::Value, fields: &[&str]) -> u64 {
        fields
            .iter()
            .map(|k| obj.get(*k).and_then(serde_json::Value::as_u64).unwrap_or(0))
            .sum()
    }

    // Denominator + which model it belongs to.
    let Some(entries) = model_usage.as_object() else {
        return ContextReport::none(ContextVerdict::NoWindow);
    };
    let window = entries
        .iter()
        .filter_map(|(model, entry)| {
            // A zero window is as unusable as an absent one — filtering here is
            // also what keeps `fraction()` from dividing by zero.
            let window = entry
                .get("contextWindow")
                .and_then(serde_json::Value::as_u64)
                .filter(|w| *w > 0)?;
            Some((model.clone(), window, sum(entry, &MODEL_FIELDS)))
        })
        .max_by_key(|(_, _, cumulative)| *cumulative)
        .map(|(model, window, _)| (model, window));
    // No entry reported a usable window. Recorded rather than dropped: this is
    // the state a participant dies in when its gateway sends no `contextWindow`
    // — no meter, so no warning was possible (rc3 P7).
    let Some((model, context_window)) = window else {
        return ContextReport::none(ContextVerdict::NoWindow);
    };

    // Numerator: current prompt size, from the point-in-time object.
    let Some(usage) = usage else {
        return ContextReport {
            model: Some(model),
            used_tokens: None,
            reported_window: Some(context_window),
            verdict: ContextVerdict::NoUsage,
        };
    };
    let used_tokens = sum(usage, &USAGE_FIELDS);

    // A prompt cannot exceed the window it was accepted into. When `used`
    // overshoots, the provider's reported window is wrong — not the agent's
    // occupancy — and dividing by it produces a confident 100% that means
    // nothing. Observed on Rain's DeepSeek gateway: a 219,531-token prompt
    // served without error against a window it reportedly exceeds.
    //
    // The 5% band is deliberate rather than an exact `<=`. Zero tolerance would
    // hide the meter the moment a genuine reading grazes 100% — precisely when
    // it is most worth showing — and a one-token accounting skew would flicker
    // it on and off. Small overshoot clamps to 100% for display; gross
    // overshoot means the denominator is untrustworthy, so we show nothing.
    const MAX_PLAUSIBLE_OVERSHOOT: f64 = 1.05;
    if used_tokens as f64 > context_window as f64 * MAX_PLAUSIBLE_OVERSHOOT {
        // `warn` (not `debug`) on purpose: this is the diagnostic that reveals
        // what a misbehaving provider actually reports, and it must survive the
        // default `info` filter to be readable when running from a terminal.
        warn!(
            model = %model,
            used_tokens,
            context_window,
            ratio = used_tokens as f64 / context_window as f64,
            "implausible context reading — provider's contextWindow looks wrong; suppressing meter"
        );
        // Suppressed for the METER, kept for the RECORD: both operands are
        // exactly what the provider reported, and they are the evidence for
        // whether that provider's window can be trusted at all.
        return ContextReport {
            model: Some(model),
            used_tokens: Some(used_tokens),
            reported_window: Some(context_window),
            verdict: ContextVerdict::ImplausibleWindow,
        };
    }

    debug!(model = %model, used_tokens, context_window, "context usage");

    ContextReport {
        model: Some(model),
        used_tokens: Some(used_tokens),
        reported_window: Some(context_window),
        verdict: ContextVerdict::Usable,
    }
}

/// Coerce the wire `api_error_status` — which arrives as a JSON number, or
/// occasionally a string — into a `u16` HTTP status. `None` when absent or
/// unparseable. Fed to `spawn::is_transient_api_error` by the retry supervisor.
fn extract_api_status(v: &Option<serde_json::Value>) -> Option<u16> {
    match v.as_ref()? {
        serde_json::Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
        serde_json::Value::String(s) => s.trim().parse::<u16>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn pump_events_emits_text() {
        let (read, mut write) = tokio::io::duplex(4096);
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_events(read, tx));
        write
            .write_all(
                br#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hello"}]}}
"#,
            )
            .await
            .unwrap();
        let ev = rx.recv().await.unwrap();
        match ev {
            AgentEvent::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected text, got {other:?}"),
        }
        drop(write);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn pump_events_emits_tool_use_and_turn_complete() {
        let (read, mut write) = tokio::io::duplex(4096);
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_events(read, tx));
        write
            .write_all(
                br#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"tu1","name":"ask_user_choice","input":{"q":"?"}}]}}
{"type":"result","stop_reason":"end_turn","subtype":"success"}
"#,
            )
            .await
            .unwrap();
        match rx.recv().await.unwrap() {
            AgentEvent::ToolUse { name, .. } => assert_eq!(name, "ask_user_choice"),
            other => panic!("expected tool_use, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            AgentEvent::TurnComplete { stop_reason, .. } => {
                assert_eq!(stop_reason.as_deref(), Some("end_turn"))
            }
            other => panic!("expected turn_complete, got {other:?}"),
        }
        drop(write);
        task.await.unwrap();
    }

    #[test]
    fn error_result_translates_to_errored_turn_complete() {
        // The real Rain/DeepSeek 400: a failed turn arrives as a `result`
        // with is_error:true + a populated api_error_status. translate() must
        // set TurnComplete.is_error so the pump counts the errored turn (and,
        // router-era, suppressed the peer forward that otherwise volleyed).
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"api_error_status":400,"stop_reason":null}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev, &mut None).as_slice() {
            [AgentEvent::TurnComplete { is_error, .. }] => {
                assert!(*is_error, "error result must mark TurnComplete.is_error")
            }
            other => panic!("expected one errored TurnComplete, got {other:?}"),
        }
    }

    #[test]
    fn api_error_status_alone_marks_errored_turn() {
        // Defensive: a populated api_error_status is itself a failure signal,
        // even if the is_error flag is absent from the payload.
        let line = r#"{"type":"result","api_error_status":429}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev, &mut None).as_slice() {
            [AgentEvent::TurnComplete { is_error, .. }] => assert!(*is_error),
            other => panic!("expected one errored TurnComplete, got {other:?}"),
        }
    }

    #[test]
    fn success_result_translates_to_clean_turn_complete() {
        // Regression guard: a normal successful turn must NOT be marked errored
        // (else the pump would wrongly suppress forwarding legit work).
        let line = r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn"}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev, &mut None).as_slice() {
            [AgentEvent::TurnComplete { is_error, .. }] => {
                assert!(!*is_error, "success result must not be marked errored")
            }
            other => panic!("expected one clean TurnComplete, got {other:?}"),
        }
    }

    #[test]
    fn overloaded_result_propagates_api_status() {
        // The 2026-06-01 strand: claude-code surfaces an Anthropic 529 as a
        // result with api_error_status. translate() must carry the numeric
        // status through so the retry supervisor can classify it transient.
        let line = r#"{"type":"result","is_error":true,"api_error_status":529,"stop_reason":null}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev, &mut None).as_slice() {
            [AgentEvent::TurnComplete {
                is_error,
                api_error_status,
                ..
            }] => {
                assert!(*is_error);
                assert_eq!(*api_error_status, Some(529));
            }
            other => panic!("expected errored TurnComplete with status, got {other:?}"),
        }
    }

    #[test]
    fn string_api_status_is_coerced() {
        // Defensive: some gateways stringify the status.
        let line = r#"{"type":"result","is_error":true,"api_error_status":"503"}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev, &mut None).as_slice() {
            [AgentEvent::TurnComplete { api_error_status, .. }] => {
                assert_eq!(*api_error_status, Some(503))
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    #[test]
    fn success_result_has_no_api_status() {
        let line = r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn"}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev, &mut None).as_slice() {
            [AgentEvent::TurnComplete {
                api_error_status, ..
            }] => assert_eq!(*api_error_status, None),
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    /// Feedback #17(3): claude-code's own messages carry the model
    /// `<synthetic>`; their text is a Notice, not the participant's prose. A
    /// real model's text that merely starts "API Error:" stays Text.
    /// `init` carries `apiKeySource` (B4): parsed by its camelCase wire name
    /// and carried on the event; absent on an older CLI → `None`.
    #[test]
    fn init_carries_the_api_key_source() {
        let parse = |line: &str| -> StreamEvent { serde_json::from_str(line).unwrap() };
        let mut carry = None;
        let ev = parse(
            r#"{"type":"system","subtype":"init","session_id":"cc-1","model":"claude-fable-5-1","apiKeySource":"none"}"#,
        );
        assert!(matches!(
            translate(ev, &mut carry).as_slice(),
            [AgentEvent::Init { session_id: Some(id), api_key_source: Some(src) }]
                if id == "cc-1" && src == "none"
        ));
        let ev = parse(r#"{"type":"system","subtype":"init","session_id":"cc-2"}"#);
        assert!(matches!(
            translate(ev, &mut carry).as_slice(),
            [AgentEvent::Init { session_id: Some(_), api_key_source: None }]
        ));
    }

    #[test]
    fn synthetic_messages_become_notices_and_real_prose_stays_text() {
        let mut carry = None;
        let parse = |line: &str| -> StreamEvent { serde_json::from_str(line).unwrap() };
        let synthetic = parse(
            r#"{"type":"assistant","message":{"id":"m1","model":"<synthetic>","content":[{"type":"text","text":"API Error: Connection refused"}]}}"#,
        );
        assert!(matches!(
            translate(synthetic, &mut carry).as_slice(),
            [AgentEvent::Notice(t)] if t == "API Error: Connection refused"
        ));
        let quoting = parse(
            r#"{"type":"assistant","message":{"id":"m2","model":"claude-opus-5-5","content":[{"type":"text","text":"API Error: that is what the log said"}]}}"#,
        );
        assert!(matches!(translate(quoting, &mut carry).as_slice(), [AgentEvent::Text(_)]));
    }

    /// A HELPER's messages — claude-code streams a subagent's assistant
    /// messages and tool results on the parent's stdout, tagged with the Agent
    /// call that launched it — are not the participant's: no text, no tool row,
    /// and no overwrite of the parent's context reading. Shapes as probed on CLI
    /// 2.1.281 (s-5482dfff); the parent's own events carry
    /// `"parent_tool_use_id":null`.
    #[test]
    fn a_helpers_messages_are_not_the_participants() {
        let parse = |line: &str| -> StreamEvent { serde_json::from_str(line).unwrap() };
        let mut carry = None;
        let own = parse(
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"id":"m1","content":[{"type":"text","text":"LAUNCHED"}],"usage":{"input_tokens":1000}}}"#,
        );
        assert!(matches!(translate(own, &mut carry).as_slice(), [AgentEvent::Text(t)] if t == "LAUNCHED"));
        assert_eq!(carry, Some(serde_json::json!({"input_tokens": 1000})));
        let helper_call = parse(
            r#"{"type":"assistant","parent_tool_use_id":"toolu_01PY","subagent_type":"general-purpose","message":{"id":"m2","content":[{"type":"text","text":"I'll start by reading the input file."},{"type":"tool_use","id":"t9","name":"Bash","input":{"command":"echo hello-from-helper"}}],"usage":{"input_tokens":12825}}}"#,
        );
        assert!(translate(helper_call, &mut carry).is_empty(), "a helper's text and call are not the parent's");
        assert_eq!(
            carry,
            Some(serde_json::json!({"input_tokens": 1000})),
            "the parent's context reading is not the helper's"
        );
        let helper_result = parse(
            r#"{"type":"user","parent_tool_use_id":"toolu_01PY","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t9","content":"hello-from-helper"}]}}"#,
        );
        assert!(translate(helper_result, &mut carry).is_empty());
        let own_result = parse(
            r#"{"type":"user","parent_tool_use_id":null,"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01PY","content":"Async agent launched successfully."}]}}"#,
        );
        assert!(matches!(translate(own_result, &mut carry).as_slice(), [AgentEvent::ToolResult { .. }]));
    }

    /// claude-code's own count of the process's running background tasks, sent
    /// whenever the set changes, becomes `BackgroundTasks` — shapes as probed.
    #[test]
    fn background_tasks_changed_reports_the_running_count() {
        let parse = |line: &str| -> StreamEvent { serde_json::from_str(line).unwrap() };
        let started = parse(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"a360","task_type":"local_agent","description":"probe"}],"uuid":"u1","session_id":"s"}"#,
        );
        assert!(matches!(translate(started, &mut None).as_slice(), [AgentEvent::BackgroundTasks { running: 1 }]));
        let ended = parse(r#"{"type":"system","subtype":"background_tasks_changed","tasks":[],"uuid":"u2","session_id":"s"}"#);
        assert!(matches!(translate(ended, &mut None).as_slice(), [AgentEvent::BackgroundTasks { running: 0 }]));
    }

    /// A compaction, as claude-code 2.1.284 writes it to stdout (s-d43b3630),
    /// becomes `Compacted` with claude-code's own token counts.
    #[test]
    fn a_compact_boundary_becomes_compacted_with_its_token_counts() {
        let parse = |line: &str| -> StreamEvent { serde_json::from_str(line).unwrap() };
        let boundary = parse(
            r#"{"type":"system","subtype":"compact_boundary","session_id":"s","uuid":"u","compact_metadata":{"trigger":"auto","pre_tokens":970716,"post_tokens":17629,"cumulative_dropped_tokens":953087,"duration_ms":70752},"logical_parent_uuid":"p"}"#,
        );
        match translate(boundary, &mut None).as_slice() {
            [AgentEvent::Compacted { trigger, pre_tokens, post_tokens }] => {
                assert_eq!(trigger.as_deref(), Some("auto"));
                assert_eq!((*pre_tokens, *post_tokens), (Some(970716), Some(17629)));
            }
            other => panic!("expected one Compacted, got {other:?}"),
        }
        let bare = parse(r#"{"type":"system","subtype":"compact_boundary"}"#);
        assert!(matches!(
            translate(bare, &mut None).as_slice(),
            [AgentEvent::Compacted { trigger: None, pre_tokens: None, post_tokens: None }]
        ));
    }

    /// Of the `SessionStart:compact` hook responses on the stream, only the one
    /// that printed a rendered handoff file is bot-hq's; a plugin's hook under
    /// the same name, and bot-hq's file under another hook name, are not.
    #[test]
    fn only_bot_hqs_own_post_compaction_hook_reports_a_handoff_injection() {
        let response = |name: &str, output: &str| -> StreamEvent {
            serde_json::from_value(serde_json::json!({
                "type": "system", "subtype": "hook_response", "hook_name": name,
                "hook_event": "SessionStart", "output": output, "exit_code": 0, "outcome": "success",
            }))
            .unwrap()
        };
        let doc = crate::agents::handoff::render("s-1", "hands", Some(("keep gating reads", "2026-10-03T05:00:00Z")));
        let no_doc = crate::agents::handoff::render("s-1", "hands", None);
        assert!(matches!(
            translate(response(COMPACT_HOOK_NAME, &doc), &mut None).as_slice(),
            [AgentEvent::HandoffInjected { has_doc: true }]
        ));
        assert!(matches!(
            translate(response(COMPACT_HOOK_NAME, &no_doc), &mut None).as_slice(),
            [AgentEvent::HandoffInjected { has_doc: false }]
        ));
        assert!(translate(response(COMPACT_HOOK_NAME, "{\"hookSpecificOutput\":{}}"), &mut None).is_empty());
        assert!(translate(response("SessionStart:startup", &doc), &mut None).is_empty());
    }

    #[tokio::test]
    async fn parse_errors_dont_abort_stream() {
        let (read, mut write) = tokio::io::duplex(4096);
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_events(read, tx));
        write.write_all(b"not json\n").await.unwrap();
        write
            .write_all(
                br#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"ok"}]}}
"#,
            )
            .await
            .unwrap();
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, AgentEvent::Text(t) if t == "ok"));
        drop(write);
        task.await.unwrap();
    }

    // ---- context-window occupancy -----------------------------------------

    /// Point-in-time `usage`, snake_case. The NUMERATOR.
    fn usage(input: u64, cache_read: u64, cache_create: u64) -> serde_json::Value {
        serde_json::json!({
            "input_tokens": input,
            "cache_read_input_tokens": cache_read,
            "cache_creation_input_tokens": cache_create,
            "output_tokens": 7,
        })
    }

    /// The shape observed live from claude-code v2.x. Pins the field names —
    /// they are camelCase on this map even though the sibling `usage` object
    /// uses snake_case, which is exactly the kind of thing a refactor breaks.
    #[test]
    fn parses_first_party_context_usage() {
        let mu = serde_json::json!({
            "claude-opus-5": {
                "inputTokens": 2,
                "outputTokens": 4,
                "cacheReadInputTokens": 11631,
                "cacheCreationInputTokens": 12324,
                "contextWindow": 1_000_000u64,
                "canonicalModel": "claude-opus-5",
                "provider": "firstParty"
            }
        });
        let u = usage(2, 11_631, 12_324);
        let c = parse_context_usage(Some(&u), &mu).usable().expect("usable entry");
        assert_eq!(c.model, "claude-opus-5");
        // Cache fields count: they occupy the window even when cheap to send.
        assert_eq!(c.used_tokens, 2 + 11_631 + 12_324);
        assert_eq!(c.context_window, 1_000_000);
        assert!((c.fraction() - 0.023_957).abs() < 1e-6);
    }

    /// REGRESSION — the bug that shipped and pinned every long-lived agent at
    /// 100%. `modelUsage` accumulates across the whole session while `usage`
    /// reports only the current turn, so on turn N the two diverge badly.
    ///
    /// Numbers below are the real measured shape: a session whose cumulative
    /// total has passed the window, but whose live prompt is only ~62% of it.
    /// Reading the numerator from `modelUsage` yields >100%; reading it from
    /// `usage` yields the truth.
    ///
    /// A single-turn fixture CANNOT catch this — the two objects are identical
    /// on turn 1 — which is exactly why the original tests all passed.
    #[test]
    fn numerator_comes_from_point_in_time_usage_not_cumulative_model_usage() {
        let mu = serde_json::json!({
            "claude-opus-5": {
                // Cumulative across many turns — already past the window.
                "inputTokens": 4_000,
                "cacheReadInputTokens": 3_000_000u64,
                "cacheCreationInputTokens": 250_000u64,
                "contextWindow": 1_000_000u64,
            }
        });
        // The live prompt for THIS turn: 619,856 tokens = 62%.
        let u = usage(2, 616_968, 2_886);

        let c = parse_context_usage(Some(&u), &mu).usable().expect("usable entry");
        assert_eq!(c.used_tokens, 619_856, "numerator must be point-in-time");
        assert_eq!(c.context_window, 1_000_000);
        assert!(
            c.fraction() < 1.0,
            "a partially-full session must not saturate: got {}",
            c.fraction()
        );
        // 61.9856% floors to 61 — the UI deliberately floors rather than
        // rounds, so 99.6% never displays as a "100%" wall it hasn't hit.
        assert_eq!((c.fraction() * 100.0).floor() as u64, 61);
    }

    /// Helper: a `modelUsage` map with one entry and a given window.
    fn model_usage_with_window(window: u64) -> serde_json::Value {
        serde_json::json!({
            "some-model": {
                "inputTokens": 1, "cacheReadInputTokens": 1,
                "cacheCreationInputTokens": 1, "contextWindow": window,
            }
        })
    }

    /// A genuinely full window must still REPORT. Suppressing at exactly 100%
    /// would hide the meter at the one moment it matters most.
    #[test]
    fn used_equal_to_window_is_reported() {
        let mu = model_usage_with_window(1_000);
        let u = usage(0, 1_000, 0);
        let c = parse_context_usage(Some(&u), &mu).usable().expect("100% is a valid reading");
        assert_eq!(c.used_tokens, 1_000);
        assert_eq!((c.fraction() * 100.0).floor() as u64, 100);
    }

    /// Small overshoot is plausible accounting skew, not bad data. Tolerating
    /// it keeps the meter from flickering on and off around the boundary; the
    /// UI clamps the display to 100%.
    #[test]
    fn small_overshoot_is_tolerated() {
        let mu = model_usage_with_window(1_000);
        let u = usage(0, 1_030, 0); // 3% over
        let c = parse_context_usage(Some(&u), &mu).usable().expect("3% overshoot is skew, not corruption");
        assert_eq!(c.used_tokens, 1_030);
    }

    /// REGRESSION — Rain's real DeepSeek shape. A 219,531-token prompt served
    /// without error against a reported 131,072 window: the prompt was
    /// accepted, so the WINDOW is wrong, not the occupancy. Dividing anyway
    /// produced a confident "100%" that meant nothing.
    #[test]
    fn gross_overshoot_suppressed_as_bad_provider_data() {
        let mu = model_usage_with_window(131_072);
        let u = usage(139, 219_392, 0); // 219,531 — ~167% of the reported window
        assert!(
            parse_context_usage(Some(&u), &mu).usable().is_none(),
            "an impossible ratio must suppress, not render 100%"
        );
    }

    /// `modelUsage` present but `usage` absent: we have a denominator and no
    /// numerator, so report nothing rather than 0%.
    #[test]
    fn missing_usage_yields_none() {
        let mu = serde_json::json!({
            "claude-opus-5": { "inputTokens": 5, "contextWindow": 1_000_000u64 }
        });
        assert!(parse_context_usage(None, &mu).usable().is_none());
    }

    /// A provider that reports tokens but no window yields NOTHING rather than
    /// a partial figure — the UI must show a gap, never a guessed percentage.
    /// This is the expected path for gateway models (Rain on DeepSeek).
    #[test]
    fn missing_context_window_yields_none() {
        let mu = serde_json::json!({
            "deepseek-v4-pro": { "inputTokens": 759, "cacheReadInputTokens": 191_872 }
        });
        assert!(parse_context_usage(Some(&usage(1,1,1)), &mu).usable().is_none());
    }

    /// Zero window is as unusable as an absent one, and filtering it here is
    /// what keeps `fraction()` from dividing by zero.
    #[test]
    fn zero_context_window_yields_none() {
        let mu = serde_json::json!({ "weird-model": { "inputTokens": 10, "contextWindow": 0 } });
        assert!(parse_context_usage(Some(&usage(1,1,1)), &mu).usable().is_none());
    }

    /// A turn that dispatched a subagent on another model carries several keys.
    /// We report the primary conversation — the entry with the most used
    /// tokens — not whichever key happened to serialize first.
    #[test]
    fn multi_model_picks_largest_consumer() {
        let mu = serde_json::json!({
            "claude-haiku-4-5": { "inputTokens": 500, "contextWindow": 200_000u64 },
            "claude-opus-5":    { "inputTokens": 90_000, "contextWindow": 1_000_000u64 },
        });
        // The DENOMINATOR must come from the selected entry — opus's 1M, not
        // haiku's 200K. Picking the wrong entry would quietly rescale every
        // reading by 5x, which is exactly the class of bug that looks fine.
        let u = usage(10, 20, 30);
        let c = parse_context_usage(Some(&u), &mu).usable().expect("usable entry");
        assert_eq!(c.model, "claude-opus-5");
        assert_eq!(c.context_window, 1_000_000);
        // ...while the numerator stays independent of the map entirely.
        assert_eq!(c.used_tokens, 60);
    }

    /// Mixed map: entries lacking a window are skipped, not allowed to win the
    /// max-by-tokens comparison and suppress a perfectly good sibling.
    #[test]
    fn windowless_entry_doesnt_mask_usable_one() {
        let mu = serde_json::json!({
            "gateway-model": { "inputTokens": 900_000 },
            "claude-opus-5": { "inputTokens": 1_000, "contextWindow": 1_000_000u64 },
        });
        let c = parse_context_usage(Some(&usage(1,1,1)), &mu).usable().expect("usable entry");
        assert_eq!(c.model, "claude-opus-5");
    }

    /// Distinct from the single-entry case: this drains `filter_map` to empty
    /// across SEVERAL entries, so `max_by_key` sees an empty iterator. The
    /// realistic shape for a duo where every agent runs on a gateway provider.
    #[test]
    fn multiple_windowless_entries_yield_none() {
        let mu = serde_json::json!({
            "deepseek-v4-pro": { "inputTokens": 759, "cacheReadInputTokens": 191_872 },
            "some-other-gateway-model": { "inputTokens": 4_000 },
        });
        assert!(parse_context_usage(Some(&usage(1,1,1)), &mu).usable().is_none());
    }

    #[test]
    fn empty_or_non_object_model_usage_yields_none() {
        assert!(parse_context_usage(Some(&usage(1,1,1)), &serde_json::json!({})).usable().is_none());
        assert!(parse_context_usage(Some(&usage(1,1,1)), &serde_json::json!("nonsense")).usable().is_none());
        assert!(parse_context_usage(Some(&usage(1,1,1)), &serde_json::Value::Null).usable().is_none());
    }

    /// End-to-end: a `result` line carrying `modelUsage` surfaces occupancy on
    /// `TurnComplete`.
    #[test]
    fn translate_carries_context_usage() {
        // A real turn in order: the assistant call supplies the point-in-time
        // numerator, the result supplies only the window. `result.usage` is
        // deliberately set to a DIFFERENT (larger) value here — if the numerator
        // ever regresses to reading it, this assertion catches it.
        let mut carry = None;
        let asst = r#"{"type":"assistant","message":{"id":"m","content":[],
            "usage":{"input_tokens":1,"cache_read_input_tokens":9,"cache_creation_input_tokens":0}}}"#;
        translate(serde_json::from_str(asst).unwrap(), &mut carry);

        let line = r#"{"type":"result","stop_reason":"end_turn","subtype":"success",
            "usage":{"input_tokens":500,"cache_read_input_tokens":500,"cache_creation_input_tokens":0},
            "modelUsage":{"claude-opus-5":{"inputTokens":50,"cacheReadInputTokens":900,
            "cacheCreationInputTokens":0,"contextWindow":1000}}}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev, &mut carry).as_slice() {
            [AgentEvent::TurnComplete { context, .. }] => {
                let c = context.usable().expect("context present");
                assert_eq!(c.used_tokens, 10);
                assert_eq!(c.context_window, 1000);
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    /// REGRESSION — the second numerator bug. `result.usage` is the SUM of
    /// every API call in the turn, so a 3-tool-call turn roughly triples it.
    /// The real current context is the LAST assistant call's prompt size.
    ///
    /// Numbers are from the live probe that found it:
    ///   assistant#1 33,917 · assistant#2 33,917 · assistant#3 34,216
    ///   result 68,133  (= 33,917 + 34,216)
    ///
    /// Reading `result.usage` gives 68,133 — nearly double the truth, and on a
    /// long agentic turn enough to trip the plausibility guard and blank the
    /// meter entirely.
    #[test]
    fn numerator_uses_last_assistant_not_result_sum() {
        let mut carry = None;

        let asst = |sum: u64| {
            format!(
                r#"{{"type":"assistant","message":{{"id":"m","content":[{{"type":"text","text":"x"}}],
                   "usage":{{"input_tokens":0,"cache_read_input_tokens":{sum},"cache_creation_input_tokens":0}}}}}}"#
            )
        };
        for s in [33_917u64, 33_917, 34_216] {
            let ev: StreamEvent = serde_json::from_str(&asst(s)).unwrap();
            translate(ev, &mut carry);
        }

        let result = r#"{"type":"result","stop_reason":"end_turn","subtype":"success",
            "usage":{"input_tokens":0,"cache_read_input_tokens":68133,"cache_creation_input_tokens":0},
            "modelUsage":{"claude-opus-5":{"inputTokens":1,"contextWindow":1000000}}}"#;
        let ev: StreamEvent = serde_json::from_str(result).unwrap();
        match translate(ev, &mut carry).as_slice() {
            [AgentEvent::TurnComplete { context, .. }] => {
                let c = context.usable().expect("context present");
                assert_eq!(
                    c.used_tokens, 34_216,
                    "must use the LAST assistant reading, not result's turn-sum"
                );
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    /// The carry must not survive a turn boundary: a turn producing no
    /// assistant usage would otherwise report the PREVIOUS turn's context as
    /// current, which is a stale reading dressed as a live one.
    #[test]
    fn usage_carry_resets_between_turns() {
        let mut carry = None;
        let asst = r#"{"type":"assistant","message":{"id":"m","content":[],
            "usage":{"input_tokens":0,"cache_read_input_tokens":500,"cache_creation_input_tokens":0}}}"#;
        translate(serde_json::from_str(asst).unwrap(), &mut carry);
        assert!(carry.is_some(), "assistant usage should be captured");

        let result = r#"{"type":"result","stop_reason":"end_turn",
            "modelUsage":{"m":{"inputTokens":1,"contextWindow":1000}}}"#;
        translate(serde_json::from_str(result).unwrap(), &mut carry);
        assert!(carry.is_none(), "carry must be cleared at turn end");

        // A second turn with no assistant usage must report nothing, not 500.
        match translate(serde_json::from_str(result).unwrap(), &mut carry).as_slice() {
            [AgentEvent::TurnComplete { context, .. }] => {
                assert!(
                    context.usable().is_none(),
                    "a turn without its own reading must not inherit the last one"
                );
                // …and the absence is RECORDED as its own fact: the window
                // arrived, the numerator did not (rc3 P7).
                assert_eq!(context.verdict, ContextVerdict::NoUsage);
                assert_eq!(context.reported_window, Some(1000));
                assert_eq!(context.used_tokens, None);
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    /// Older claude-code builds — and any turn that fails before a model
    /// responds — omit `modelUsage` entirely. That must stay benign.
    #[test]
    fn translate_without_model_usage_leaves_context_none() {
        let line = r#"{"type":"result","stop_reason":"end_turn","subtype":"success"}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev, &mut None).as_slice() {
            [AgentEvent::TurnComplete { context, .. }] => {
                assert!(context.usable().is_none());
                // A `result` with no `modelUsage` reports no window, and that
                // is recorded rather than dropped (rc3 P7).
                assert_eq!(context.verdict, ContextVerdict::NoWindow);
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    /// Round 9: `short_line` sliced the unparseable line at BYTE 160. Model
    /// stdout is arbitrary text — a box-drawing run puts a 3-byte char across
    /// that offset — and slicing inside a char PANICS, inside the stdout pump,
    /// so the agent simply stops answering. Truncate by chars.
    #[test]
    fn short_line_truncates_multibyte_text_without_panicking() {
        for pad in 0..3 {
            let mut line = "x".repeat(pad);
            line.push_str(&"─".repeat(120)); // 3 bytes each: > 160 bytes
            let short = short_line(&line);
            assert!(short.ends_with('…'), "no ellipsis: {short}");
            assert!(short.chars().count() <= 161, "too long: {}", short.chars().count());
            assert!(!short.contains('\u{FFFD}'));
        }
        assert_eq!(short_line("short"), "short");
    }
}
