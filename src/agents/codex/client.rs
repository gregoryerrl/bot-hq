//! A JSON-RPC client over the app-server's stdio: newline-delimited JSON,
//! `"jsonrpc"` omitted on the wire (as Codex writes it), three message
//! shapes — a RESPONSE to one of our requests (`id` + `result`/`error`), a
//! NOTIFICATION (`method`, no `id`), and a SERVER REQUEST (`id` + `method`),
//! which the server waits on: an unanswered one hangs the turn, and the ring
//! with it (plan-eyes G4). So every server request is answered here, at once,
//! by [`answer_server_request`] — approvals declined, a user-input request
//! redirected at `ask_user_choice`, anything unknown refused with `-32601` —
//! and reported to the driver as a notification-shaped event so the
//! transcript shows it happened.
//!
//! Generic over the streams so the tests drive it through `tokio::io::duplex`
//! without a child process; the driver hands it the child's stdout/stdin.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// One line from the server that was not a response to us.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

/// A server request the client answered, reported for the transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct AnsweredRequest {
    pub method: String,
    pub params: Value,
    pub answer: ServerAnswer,
}

/// What a server request was answered with.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerAnswer {
    Result(Value),
    Error { code: i64, message: String },
}

/// Everything the server sends that the driver reads, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification(Notification),
    Answered(AnsweredRequest),
    /// The stream ended (the child exited or closed stdout).
    Closed,
}

/// A response's error half.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

/// The client: one writer, one reader task, the pending-response map.
pub struct CodexClient {
    out_tx: mpsc::Sender<String>,
    pending: Pending,
    next_id: AtomicU64,
    /// How long a request may wait for its response.
    pub timeout: Duration,
}

/// The text the model reads when it asks the user something through Codex's
/// own channel, which bot-hq does not surface.
pub const REQUEST_USER_INPUT_REDIRECT: &str =
    "Interactive questions are not available in this harness. Use the `ask_user_choice` MCP tool \
     to ask the user; it parks the question and the answer arrives as a message.";

/// The answer for a server request, by method (G4). Approvals are declined
/// (the posture never asks for them: `approval_policy never`), permissions
/// are granted nothing, a user-input request gets the redirect text as the
/// answer to each of its questions, and every other method — token refresh,
/// attestation, forms, elicitation, anything newer than this build — is
/// refused with `-32601`.
pub fn answer_server_request(method: &str, params: &Value) -> ServerAnswer {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            ServerAnswer::Result(json!({ "decision": "decline" }))
        }
        "item/permissions/requestApproval" => {
            ServerAnswer::Result(json!({ "permissions": [], "scope": "turn" }))
        }
        "item/tool/requestUserInput" => {
            // One answer per question, keyed by the question's id, each the
            // redirect text; the model sees them as the user's reply.
            let answers: serde_json::Map<String, Value> = params
                .get("questions")
                .and_then(|q| q.as_array())
                .map(|qs| {
                    qs.iter()
                        .filter_map(|q| q.get("id").and_then(|id| id.as_str()))
                        .map(|id| (id.to_string(), json!({ "answers": [REQUEST_USER_INPUT_REDIRECT] })))
                        .collect()
                })
                .unwrap_or_default();
            ServerAnswer::Result(json!({ "answers": answers }))
        }
        _ => ServerAnswer::Error {
            code: -32601,
            message: format!("bot-hq does not answer {method}"),
        },
    }
}

impl CodexClient {
    /// Wire a client onto `reader` (the server's stdout) and `writer` (its
    /// stdin). Returns the client and the stream of everything the server
    /// sends that is not a response.
    pub fn start<R, W>(reader: R, writer: W) -> (Self, mpsc::Receiver<Incoming>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
        let (in_tx, in_rx) = mpsc::channel::<Incoming>(1024);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        // Writer: one line per message, flushed.
        tokio::spawn(async move {
            let mut writer = writer;
            while let Some(line) = out_rx.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if writer.write_all(b"\n").await.is_err() {
                    break;
                }
                if writer.flush().await.is_err() {
                    break;
                }
            }
        });

        // Reader: route each line.
        let reader_pending = Arc::clone(&pending);
        let reader_out = out_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            loop {
                let line = match lines.next_line().await {
                    Ok(Some(l)) => l,
                    Ok(None) | Err(_) => break,
                };
                if line.trim().is_empty() {
                    continue;
                }
                let v: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::debug!(%e, line = %line.chars().take(200).collect::<String>(), "codex: unparseable line");
                        continue;
                    }
                };
                let id = v.get("id").cloned();
                let method = v.get("method").and_then(|m| m.as_str()).map(str::to_string);
                match (id, method) {
                    // A response to one of ours.
                    (Some(id), None) => {
                        let Some(id) = id.as_u64() else { continue };
                        let waiter = reader_pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                        if let Some(tx) = waiter {
                            let outcome = match v.get("error") {
                                Some(err) => Err(RpcError {
                                    code: err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1),
                                    message: err
                                        .get("message")
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("unknown error")
                                        .to_string(),
                                }),
                                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = tx.send(outcome);
                        }
                    }
                    // A server request: answer it, then report it.
                    (Some(id), Some(method)) => {
                        let params = v.get("params").cloned().unwrap_or(Value::Null);
                        let answer = answer_server_request(&method, &params);
                        let reply = match &answer {
                            ServerAnswer::Result(r) => json!({ "id": id, "result": r }),
                            ServerAnswer::Error { code, message } => {
                                json!({ "id": id, "error": { "code": code, "message": message } })
                            }
                        };
                        let _ = reader_out.send(reply.to_string()).await;
                        if in_tx
                            .send(Incoming::Answered(AnsweredRequest { method, params, answer }))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    // A notification.
                    (None, Some(method)) => {
                        let params = v.get("params").cloned().unwrap_or(Value::Null);
                        if in_tx.send(Incoming::Notification(Notification { method, params })).await.is_err() {
                            break;
                        }
                    }
                    (None, None) => {}
                }
            }
            // The stream ended: fail every waiter, tell the driver.
            let waiters: Vec<_> = reader_pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .drain()
                .map(|(_, tx)| tx)
                .collect();
            for tx in waiters {
                let _ = tx.send(Err(RpcError { code: -32000, message: "the app-server stream closed".into() }));
            }
            let _ = in_tx.send(Incoming::Closed).await;
        });

        (
            Self { out_tx, pending, next_id: AtomicU64::new(1), timeout: Duration::from_secs(60) },
            in_rx,
        )
    }

    /// Send a request and wait for its response (or the timeout).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).insert(id, tx);
        let line = json!({ "id": id, "method": method, "params": params }).to_string();
        if self.out_tx.send(line).await.is_err() {
            self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
            return Err(anyhow!("{method}: the app-server's stdin is closed"));
        }
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!("{method}: {e}")),
            Ok(Err(_)) => Err(anyhow!("{method}: the app-server stream closed before it answered")),
            Err(_) => {
                self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                Err(anyhow!("{method}: no answer from the app-server within {:?}", self.timeout))
            }
        }
    }

    /// Send a notification (no response expected).
    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let line = json!({ "method": method, "params": params }).to_string();
        self.out_tx
            .send(line)
            .await
            .map_err(|_| anyhow!("{method}: the app-server's stdin is closed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake server end of a duplex pair: reads what the client wrote,
    /// writes lines at it.
    struct FakeServer {
        lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    }

    fn pair() -> (CodexClient, mpsc::Receiver<Incoming>, FakeServer) {
        let (client_side, server_side) = tokio::io::duplex(1 << 16);
        let (c_read, c_write) = tokio::io::split(client_side);
        let (s_read, s_write) = tokio::io::split(server_side);
        let (client, incoming) = CodexClient::start(c_read, c_write);
        let server = FakeServer { lines: BufReader::new(s_read).lines(), writer: s_write };
        (client, incoming, server)
    }

    impl FakeServer {
        async fn read(&mut self) -> Value {
            let line = self.lines.next_line().await.unwrap().expect("a line from the client");
            serde_json::from_str(&line).unwrap()
        }
        async fn write(&mut self, v: Value) {
            self.writer.write_all(format!("{v}\n").as_bytes()).await.unwrap();
            self.writer.flush().await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_request_is_answered_by_id_and_an_error_is_an_err() {
        let (client, _incoming, mut server) = pair();
        let ask = tokio::spawn(async move { client.request("thread/start", json!({"model": "gpt-6-astra"})).await });
        let req = server.read().await;
        assert_eq!(req["method"], "thread/start");
        assert_eq!(req["params"]["model"], "gpt-6-astra");
        assert!(req.get("jsonrpc").is_none(), "the header is omitted on the wire, as Codex writes it");
        let id = req["id"].as_u64().unwrap();
        // An unrelated response first: ignored, not confused with ours.
        server.write(json!({"id": id + 100, "result": {"nope": true}})).await;
        server.write(json!({"id": id, "result": {"thread": {"id": "thr_1"}}})).await;
        let got = ask.await.unwrap().unwrap();
        assert_eq!(got["thread"]["id"], "thr_1");

        let (client, _incoming, mut server) = pair();
        let ask = tokio::spawn(async move { client.request("turn/start", json!({})).await });
        let req = server.read().await;
        server
            .write(json!({"id": req["id"], "error": {"code": -32001, "message": "busy"}}))
            .await;
        let err = ask.await.unwrap().unwrap_err().to_string();
        assert!(err.contains("busy") && err.contains("-32001"), "{err}");
    }

    #[tokio::test]
    async fn notifications_arrive_in_order_and_the_close_is_reported() {
        let (_client, mut incoming, mut server) = pair();
        server.write(json!({"method": "turn/started", "params": {"turn": {"id": "t1"}}})).await;
        server.write(json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "text": "hi"}}})).await;
        server.write(json!({"method": "remoteControl/status/changed", "params": {"status": "disabled"}, "emittedAtMs": 1})).await;
        let first = incoming.recv().await.unwrap();
        assert_eq!(first, Incoming::Notification(Notification { method: "turn/started".into(), params: json!({"turn": {"id": "t1"}}) }));
        let second = incoming.recv().await.unwrap();
        assert!(matches!(second, Incoming::Notification(n) if n.method == "item/completed"));
        let third = incoming.recv().await.unwrap();
        assert!(matches!(third, Incoming::Notification(n) if n.method == "remoteControl/status/changed"));
        drop(server);
        assert_eq!(incoming.recv().await.unwrap(), Incoming::Closed);
    }

    /// G4: a server request never goes unanswered, whatever it is. The reply
    /// goes out at once with the server's id; the driver sees what was
    /// answered.
    #[tokio::test]
    async fn every_server_request_is_answered_at_once() {
        let (_client, mut incoming, mut server) = pair();
        server
            .write(json!({"id": 7, "method": "item/commandExecution/requestApproval", "params": {"itemId": "x", "command": "rm -rf /"}}))
            .await;
        let reply = server.read().await;
        assert_eq!(reply, json!({"id": 7, "result": {"decision": "decline"}}));
        assert!(matches!(incoming.recv().await.unwrap(), Incoming::Answered(a) if a.method == "item/commandExecution/requestApproval" && a.answer == ServerAnswer::Result(json!({"decision": "decline"}))));

        server.write(json!({"id": 8, "method": "item/fileChange/requestApproval", "params": {}})).await;
        assert_eq!(server.read().await, json!({"id": 8, "result": {"decision": "decline"}}));
        let _ = incoming.recv().await;

        server.write(json!({"id": 9, "method": "item/permissions/requestApproval", "params": {}})).await;
        assert_eq!(server.read().await, json!({"id": 9, "result": {"permissions": [], "scope": "turn"}}));
        let _ = incoming.recv().await;

        server
            .write(json!({"id": 10, "method": "item/tool/requestUserInput", "params": {"questions": [{"id": "q1", "header": "Which?"}, {"id": "q2", "header": "Sure?"}]}}))
            .await;
        let reply = server.read().await;
        assert_eq!(reply["id"], 10);
        assert_eq!(reply["result"]["answers"]["q1"]["answers"][0], REQUEST_USER_INPUT_REDIRECT);
        assert_eq!(reply["result"]["answers"]["q2"]["answers"][0], REQUEST_USER_INPUT_REDIRECT);
        let _ = incoming.recv().await;

        for method in ["account/chatgptAuthTokens/refresh", "attestation/generate", "openai/form", "mcpServer/elicitation/request", "something/new"] {
            server.write(json!({"id": 11, "method": method, "params": {}})).await;
            let reply = server.read().await;
            assert_eq!(reply["id"], 11);
            assert_eq!(reply["error"]["code"], -32601, "{method}");
            let _ = incoming.recv().await;
        }
    }

    #[tokio::test]
    async fn a_closed_stream_fails_the_waiting_request() {
        let (mut client, _incoming, mut server) = pair();
        client.timeout = Duration::from_secs(5);
        let ask = tokio::spawn(async move { client.request("thread/start", json!({})).await });
        let _ = server.read().await;
        drop(server);
        let err = ask.await.unwrap().unwrap_err().to_string();
        assert!(err.contains("closed"), "{err}");
    }

    #[tokio::test]
    async fn a_silent_server_times_the_request_out() {
        let (mut client, _incoming, mut server) = pair();
        client.timeout = Duration::from_millis(50);
        let err = tokio::join!(client.request("hooks/list", json!({})), server.read()).0.unwrap_err().to_string();
        assert!(err.contains("no answer"), "{err}");
    }
}
