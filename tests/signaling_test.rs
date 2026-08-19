//! End-to-end HTTP test of the MCP signaling server.

use bot_hq::signaling::{start_signaling_server, SignalingBridge};
use bot_hq::storage::Storage;
use serde_json::json;
use std::sync::Arc;

/// A bridge holding session `s1` with the roster every real session is seeded
/// with. Needed by any test that calls a GATED tool: the gate resolves the
/// caller's grants from `session_participants`, and a bridge with no storage
/// resolves to "unreadable", which fails closed.
async fn seeded_bridge() -> Arc<SignalingBridge> {
    let bridge = SignalingBridge::new();
    let storage = Storage::memory().await.unwrap();
    bridge.set_storage(storage.clone()).await;
    storage
        .create_session("s1", "signaling", None)
        .await
        .unwrap();
    storage.ensure_session_roster("s1", bot_hq::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
    bridge
}
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Minimal HTTP-over-tokio client: we don't want to pull in hyper or reqwest
/// as dev-deps just for these tests.
async fn http_post_json(addr: std::net::SocketAddr, path: &str, body: &str) -> String {
    let req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len(),
    );
    let mut sock = TcpStream::connect(addr).await.unwrap();
    sock.write_all(req.as_bytes()).await.unwrap();
    sock.flush().await.ok();
    let mut buf = Vec::new();
    sock.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    let (_head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    body.to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_initialize_round_trip() {
    let bridge = SignalingBridge::new();
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let addr = server.local_addr;
    let body = http_post_json(
        addr,
        "/sessions/s1/hands/mcp",
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    )
    .await;
    assert!(body.contains("\"protocolVersion\""));
    assert!(body.contains("bot-hq-signaling"));
    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_tools_list_round_trip() {
    let bridge = SignalingBridge::new();
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let addr = server.local_addr;
    let body = http_post_json(
        addr,
        "/sessions/s1/hands/mcp",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    )
    .await;
    assert!(body.contains("ask_user_choice"));
    assert!(body.contains("mark_awaiting_user"));
    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_ask_user_choice_parks_immediately() {
    // ask_user_choice is non-blocking: the HTTP tool call returns a parked ack
    // (`{status:"parked", choice_id}`) immediately, without waiting for any
    // resolution. The pick is delivered later out-of-band.
    let bridge = seeded_bridge().await;
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let addr = server.local_addr;

    let body = http_post_json(
        addr,
        "/sessions/s1/hands/mcp",
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "ask_user_choice",
                "arguments": { "question": "pick", "options": ["a", "b"] }
            }
        })
        .to_string(),
    )
    .await;
    assert!(body.contains("parked"), "body: {body}");
    assert!(body.contains("choice_id"), "body: {body}");
    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_rejects_bad_path() {
    let bridge = SignalingBridge::new();
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let addr = server.local_addr;
    let body = http_post_json(addr, "/bad/path", r#"{}"#).await;
    assert!(body.contains("expected /sessions"));
    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_rejects_get() {
    let bridge = SignalingBridge::new();
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let addr = server.local_addr;
    let req = format!(
        "GET /sessions/s1/hands/mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    );
    let mut sock = TcpStream::connect(addr).await.unwrap();
    sock.write_all(req.as_bytes()).await.unwrap();
    sock.flush().await.ok();
    let mut buf = Vec::new();
    sock.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    assert!(text.starts_with("HTTP/1.1 405"));
    server.shutdown();
}

/// Posts a request and returns the (status_line, body) split. Mirrors the
/// pattern the deleted `external_mcp_test::http_post` used, so notification (202) tests can
/// observe the status line, not just the body.
async fn http_post_with_status(
    addr: std::net::SocketAddr,
    path: &str,
    body: &str,
) -> (String, String) {
    let req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len(),
    );
    let mut sock = TcpStream::connect(addr).await.unwrap();
    sock.write_all(req.as_bytes()).await.unwrap();
    sock.flush().await.ok();
    let mut buf = Vec::new();
    sock.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status_line = head.lines().next().unwrap_or("").to_string();
    (status_line, body.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_parse_error_returns_jsonrpc_envelope() {
    let bridge = SignalingBridge::new();
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let addr = server.local_addr;
    let (status, body) =
        http_post_with_status(addr, "/sessions/s1/hands/mcp", "{ not valid json").await;
    assert!(status.contains("200"), "got: {status}");
    assert!(
        body.contains(r#""code":-32700"#),
        "expected PARSE_ERROR envelope: {body}"
    );
    assert!(body.contains("invalid JSON"), "body: {body}");
    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_notification_returns_202_accepted() {
    let bridge = SignalingBridge::new();
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let addr = server.local_addr;
    let (status, body) = http_post_with_status(
        addr,
        "/sessions/s1/hands/mcp",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await;
    assert!(status.contains("202"), "got: {status}");
    assert!(body.is_empty(), "expected empty body for 202, got: {body:?}");
    server.shutdown();
}
