//! End-to-end HTTP test of the external (bearer-token-authed) MCP server.

use bot_hq::core::AppState as CoreAppState;
use bot_hq::paths::Paths;
use bot_hq::signaling::{
    start_external_server, start_signaling_server, ExternalServer, SignalingBridge,
};
use bot_hq::storage::Storage;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct TestEnv {
    _tmp: TempDir,
    _core: Arc<CoreAppState>,
    external: ExternalServer,
    token: String,
}

impl TestEnv {
    fn addr(&self) -> SocketAddr {
        self.external.local_addr
    }
}

async fn setup() -> TestEnv {
    let tmp = TempDir::new().unwrap();
    let paths = Paths::for_data_dir(tmp.path().to_path_buf());
    paths.init().unwrap();
    let token = paths.read_mcp_token().unwrap();
    let storage = Storage::open(&paths.db_path).await.unwrap();
    let bridge = SignalingBridge::new();
    let internal = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let core = Arc::new(CoreAppState::new(paths, storage, internal).await);
    let external = start_external_server(Arc::clone(&core), 0, token.clone())
        .await
        .unwrap();
    TestEnv {
        _tmp: tmp,
        _core: core,
        external,
        token,
    }
}

/// Minimal HTTP-over-tokio client. Returns (status_line, body).
async fn http_post(
    addr: SocketAddr,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (String, String) {
    let mut req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n",
        len = body.len(),
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);

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

fn auth_header(token: &str) -> (&'static str, String) {
    ("Authorization", format!("Bearer {token}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_rejects_no_token() {
    let env = setup().await;
    let (status, _body) = http_post(env.addr(), "/mcp", &[], r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#).await;
    assert!(status.contains("401"), "got: {status}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_rejects_wrong_token() {
    let env = setup().await;
    let h = auth_header("not-the-right-token");
    let (status, _body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    )
    .await;
    assert!(status.contains("401"), "got: {status}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_rejects_get() {
    let env = setup().await;
    let req = format!(
        "GET /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n",
        addr = env.addr()
    );
    let mut sock = TcpStream::connect(env.addr()).await.unwrap();
    sock.write_all(req.as_bytes()).await.unwrap();
    sock.flush().await.ok();
    let mut buf = Vec::new();
    sock.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    assert!(text.starts_with("HTTP/1.1 405"), "got: {text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_rejects_bad_path() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, _body) = http_post(env.addr(), "/wrong-path", &[(h.0, &h.1)], r#"{}"#).await;
    assert!(status.contains("404"), "got: {status}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_round_trip() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    )
    .await;
    assert!(status.contains("200"), "got: {status}");
    assert!(body.contains("\"protocolVersion\""), "body: {body}");
    assert!(body.contains("\"name\":\"bot-hq\""), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_list_returns_iter1_set() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    )
    .await;
    assert!(status.contains("200"), "got: {status}");
    assert!(body.contains("list_sessions"), "body: {body}");
    assert!(body.contains("create_session"), "body: {body}");
    assert!(body.contains("send_message"), "body: {body}");
    assert!(body.contains("get_session_messages"), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_sessions_returns_emma() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#,
    )
    .await;
    assert!(status.contains("200"), "got: {status}");
    // The seeded emma session row is always active. Inner JSON is escaped
    // inside the text-content block, so we look for the escaped key:value.
    assert!(body.contains(r#"\"id\":\"emma\""#), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_session_messages_for_emma() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_session_messages","arguments":{"session_id":"emma"}}}"#,
    )
    .await;
    assert!(status.contains("200"), "got: {status}");
    // Fresh DB — emma has no messages yet. Inner JSON is escaped in the
    // outer JSON-RPC response.
    assert!(body.contains(r#"messages\":[]"#), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_tool_name_returns_error() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nonexistent","arguments":{}}}"#,
    )
    .await;
    assert!(status.contains("200"), "got: {status}");
    assert!(body.contains("unknown tool"), "body: {body}");
}

// ---- iter 2 tools ---------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_list_returns_all_iter2_tools() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (_status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    )
    .await;
    for tool in &[
        "advance_phase",
        "resolve_choice",
        "close_session",
        "restart_emma",
        "get_emma_messages",
        "get_pending_choices",
        "get_status",
    ] {
        assert!(body.contains(tool), "tool {tool} missing from tools/list");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_emma_messages_returns_empty_on_fresh_install() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"get_emma_messages","arguments":{}}}"#,
    )
    .await;
    assert!(status.contains("200"));
    assert!(body.contains(r#"messages\":[]"#), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_pending_choices_returns_empty_when_no_parked() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"get_pending_choices","arguments":{}}}"#,
    )
    .await;
    assert!(status.contains("200"));
    assert!(body.contains(r#"pending_choices\":[]"#), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_status_includes_version_and_addrs() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"get_status","arguments":{}}}"#,
    )
    .await;
    assert!(status.contains("200"));
    assert!(body.contains(r#"version\":"#), "body: {body}");
    assert!(body.contains(r#"signaling_addr\":"#), "body: {body}");
    assert!(body.contains(r#"active_duo_sessions\":0"#), "body: {body}");
    assert!(body.contains(r#"emma_started\":false"#), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn advance_phase_invalid_phase_errors() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"advance_phase","arguments":{"session_id":"emma","phase":"X"}}}"#,
    )
    .await;
    assert!(status.contains("200"));
    assert!(body.contains("phase must be one of"), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn advance_phase_unknown_session_errors() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"advance_phase","arguments":{"session_id":"does-not-exist","phase":"I"}}}"#,
    )
    .await;
    assert!(status.contains("200"));
    assert!(body.contains("no live session"), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_choice_unknown_id_errors() {
    let env = setup().await;
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"resolve_choice","arguments":{"choice_id":"bogus","picked":"x"}}}"#,
    )
    .await;
    assert!(status.contains("200"));
    assert!(body.contains("no pending choice"), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_session_on_inserted_row_succeeds() {
    // Insert a session row directly via storage (no agent spawn). close_session
    // skips the agent-kill path when there's no live HashMap entry and just
    // marks the row closed in storage.
    let env = setup().await;
    env._core
        .storage
        .create_session("test-close", "test", None)
        .await
        .unwrap();
    let h = auth_header(&env.token);
    let (status, body) = http_post(
        env.addr(),
        "/mcp",
        &[(h.0, &h.1)],
        r#"{"jsonrpc":"2.0","id":16,"method":"tools/call","params":{"name":"close_session","arguments":{"session_id":"test-close","archive":true}}}"#,
    )
    .await;
    assert!(status.contains("200"));
    assert!(body.contains(r#"ok\":true"#), "body: {body}");
}
