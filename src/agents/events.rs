//! Stdout/stderr readers for an agent subprocess. Parses one stream-json
//! event per line; translates wire events into high-level `AgentEvent`.

use crate::agents::protocol::*;
use crate::agents::spawn::AgentEvent;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Pump stdout-like stream until EOF; send translated events to `tx`.
/// Generic over the reader type so this is testable with `tokio::io::duplex`.
pub async fn pump_events<R: AsyncRead + Unpin>(reader: R, tx: mpsc::Sender<AgentEvent>) {
    let buf = BufReader::new(reader);
    let mut lines = buf.lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<StreamEvent>(trimmed) {
                    Ok(ev) => {
                        for app_ev in translate(ev) {
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
        debug!(target: "agent_stderr", agent = %agent_name, msg = %line);
    }
}

fn short_line(s: &str) -> String {
    if s.len() <= 160 {
        s.to_string()
    } else {
        format!("{}…", &s[..160])
    }
}

/// Translate a wire `StreamEvent` to zero or more `AgentEvent`s.
/// `assistant` events with multiple content blocks fan out to multiple events.
pub fn translate(ev: StreamEvent) -> Vec<AgentEvent> {
    match ev {
        StreamEvent::System(sys) => match sys {
            SystemEvent::Init {
                model,
                cwd,
                session_id,
                ..
            } => vec![AgentEvent::Init {
                model,
                cwd,
                session_id,
            }],
            _ => Vec::new(),
        },
        StreamEvent::Assistant(asst) => asst
            .message
            .content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(AgentEvent::Text(text)),
                ContentBlock::ToolUse { id, name, input } => Some(AgentEvent::ToolUse {
                    id,
                    name,
                    input,
                }),
                ContentBlock::Thinking { .. } => None,
                ContentBlock::Other => None,
            })
            .collect(),
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
            vec![AgentEvent::TurnComplete {
                stop_reason: r.stop_reason,
                subtype: r.subtype,
                is_error,
            }]
        }
        StreamEvent::RateLimit(_) => Vec::new(),
        StreamEvent::Unknown => Vec::new(),
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
        // set TurnComplete.is_error so the duo pump suppresses peer-forwarding
        // (otherwise the error text volleys into an unbounded loop).
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"api_error_status":400,"stop_reason":null}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match translate(ev).as_slice() {
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
        match translate(ev).as_slice() {
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
        match translate(ev).as_slice() {
            [AgentEvent::TurnComplete { is_error, .. }] => {
                assert!(!*is_error, "success result must not be marked errored")
            }
            other => panic!("expected one clean TurnComplete, got {other:?}"),
        }
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
}
