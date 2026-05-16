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
