//! HTTP server hosting the MCP-subset endpoint.
//!
//! Listens on `127.0.0.1:<ephemeral>`. Endpoint shape:
//!
//! ```text
//! POST /sessions/<session_id>/<agent>/mcp     (JSON-RPC body)
//! ```
//!
//! Agents get a per-agent `mcp-config.json` from [`mcp_config_json`] pointing
//! at the agent-specific URL. The path carries the caller identity used to
//! attribute tool calls.

use crate::policy::ViolationKind;
use crate::signaling::bridge::{ApprovalContext, SignalingBridge};
use crate::signaling::jsonrpc::{dispatch, CallerIdentity};
use crate::signaling::response::{decode_jsonrpc_body, dispatch_outcome_to_response, text_response};
use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

pub struct SignalingServer {
    pub local_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    pub bridge: Arc<SignalingBridge>,
    /// Path to the persisted `signaling-addr` file (set by `set_addr_file` at
    /// startup). Removed on clean shutdown so a stale file doesn't outlive the
    /// process. `None` in tests / when the addr was never persisted.
    addr_file: Option<PathBuf>,
}

impl SignalingServer {
    pub fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }

    /// Record the path of the persisted `signaling-addr` file so it can be
    /// removed when the server drops. Called from `main.rs` after the address
    /// has been written.
    pub fn set_addr_file(&mut self, path: PathBuf) {
        self.addr_file = Some(path);
    }
}

impl Drop for SignalingServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        // Best-effort cleanup of the persisted address. (Crash paths skip Drop;
        // the next startup overwrites the file, and the hook's HTTP path is
        // unreachable without a live session anyway, so a stale file is
        // self-correcting — this just keeps clean exits tidy.)
        if let Some(path) = self.addr_file.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Bind an ephemeral port on `127.0.0.1` and start serving. Returns once the
/// listener is bound (NOT once it's accepted its first connection).
pub async fn start_signaling_server(bridge: Arc<SignalingBridge>) -> Result<SignalingServer> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding signaling HTTP listener")?;
    let local_addr = listener.local_addr().context("reading bound addr")?;
    let (sd_tx, sd_rx) = oneshot::channel::<()>();
    let bridge_for_loop = Arc::clone(&bridge);

    tokio::spawn(async move {
        info!(addr = %local_addr, "signaling server listening");
        let mut sd_rx = sd_rx;
        loop {
            tokio::select! {
                _ = &mut sd_rx => {
                    info!(addr = %local_addr, "signaling server shutting down");
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let io = TokioIo::new(stream);
                            let bridge_for_conn = Arc::clone(&bridge_for_loop);
                            tokio::spawn(async move {
                                let svc = service_fn(move |req| {
                                    let bridge = Arc::clone(&bridge_for_conn);
                                    async move { handle_request(req, bridge).await }
                                });
                                if let Err(err) = http1::Builder::new().serve_connection(io, svc).await {
                                    warn!(?err, "signaling connection error");
                                }
                            });
                        }
                        Err(err) => {
                            warn!(?err, "accept failed");
                        }
                    }
                }
            }
        }
    });

    Ok(SignalingServer {
        local_addr,
        shutdown: Some(sd_tx),
        bridge,
        addr_file: None,
    })
}

async fn handle_request(
    req: Request<Incoming>,
    bridge: Arc<SignalingBridge>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();

    if method != Method::POST {
        return Ok(text_response(StatusCode::METHOD_NOT_ALLOWED, "POST only"));
    }

    // Dedicated route for the git pre-push hook subprocess (push_gate=ask). It
    // is NOT an agent and has no per-(session,agent) MCP identity, so it bypasses
    // the JSON-RPC + HANDS-only tool gate and calls `request_approval` directly.
    if path == "/hooks/pre-push" {
        return Ok(handle_pre_push(req.into_body(), bridge).await);
    }

    let caller = match parse_path(&path) {
        Some(c) => c,
        None => {
            return Ok(text_response(
                StatusCode::NOT_FOUND,
                "expected /sessions/<id>/<agent>/mcp",
            ));
        }
    };

    let rpc = match decode_jsonrpc_body(req.into_body()).await {
        Ok(r) => r,
        Err(resp) => return Ok(resp),
    };

    debug!(method = %rpc.method, %caller.session_id, %caller.agent, "rpc");

    let id_for_err = rpc.id.clone().unwrap_or(json!(null));
    Ok(dispatch_outcome_to_response(
        dispatch(rpc, &caller, &bridge).await,
        id_for_err,
    ))
}

/// Handle `POST /hooks/pre-push` from the git pre-push hook subprocess. Body:
/// `{ "session_id": "...", "agent": "brian", "branch": "..."? }`. Surfaces a
/// `request_approval` (kind=push_gate) prompt and blocks until the user picks,
/// then replies `{ "approved": <bool> }`. The hook maps `approved` → exit 0
/// (push proceeds) / not-approved → exit 1 (blocked). Reuses the same
/// `PendingChoice` → `resolve_choice` → `PushGate` violation path as the
/// agent-facing `request_approval` MCP tool, but without the HANDS-only gate
/// (the hook isn't an agent).
async fn handle_pre_push(
    body: Incoming,
    bridge: Arc<SignalingBridge>,
) -> Response<Full<Bytes>> {
    let bytes = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => return text_response(StatusCode::BAD_REQUEST, &format!("body read failed: {e}")),
    };
    let v: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return text_response(StatusCode::BAD_REQUEST, &format!("bad json: {e}")),
    };
    let Some(session_id) = v.get("session_id").and_then(|s| s.as_str()) else {
        return text_response(StatusCode::BAD_REQUEST, "missing session_id");
    };
    // Only HANDS (brian) / the solo helper (emma) push; default to brian if the
    // hook couldn't read BOT_HQ_AGENT (graceful — affects only the tray label).
    let agent = v
        .get("agent")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("brian");
    let branch = v
        .get("branch")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (action, question) = match &branch {
        Some(b) => (
            format!("git push ({b})"),
            format!("Allow `git push` to `{b}` in this session's repo?"),
        ),
        None => (
            "git push".to_string(),
            "Allow `git push` in this session's repo?".to_string(),
        ),
    };
    let ctx = ApprovalContext {
        kind: ViolationKind::PushGate,
        action,
        detail: branch,
    };

    let approved = match bridge
        .request_approval(
            session_id.to_string(),
            agent.to_string(),
            question,
            vec!["Approve".to_string(), "Reject".to_string()],
            ctx,
        )
        .await
    {
        Ok(picked) => picked == "Approve",
        Err(e) => {
            // Canceled before the user picked (e.g. superseded). Treat as not
            // approved — the hook fail-closes and blocks the push.
            warn!(%session_id, error = %e, "pre-push request_approval did not resolve to a pick");
            false
        }
    };

    text_response(
        StatusCode::OK,
        &json!({ "approved": approved }).to_string(),
    )
}

fn parse_path(path: &str) -> Option<CallerIdentity> {
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    if parts.len() != 4 || parts[0] != "sessions" || parts[3] != "mcp" {
        return None;
    }
    Some(CallerIdentity {
        session_id: parts[1].to_string(),
        agent: parts[2].to_string(),
    })
}

/// Render the mcp-config.json content claude-code expects for one agent.
///
/// `bot-hq-signaling` (HTTP transport, per-agent URL) is always included.
/// `extra_servers` is merged on top — typically the user's own
/// `~/.claude/settings.json` `mcpServers` map, loaded by
/// [`load_user_mcp_servers`]. The agent runs with `--strict-mcp-config`, so
/// without this merge the subagent has no access to chrome-devtools /
/// brave-devtools / playwright / etc. that the parent claude-code session
/// has. The user's explicit intent when configuring those MCPs is that
/// their claude sessions can use them — spawned bot-hq subagents are part
/// of that intent.
///
/// `bot-hq-signaling` always wins on key collision (the user's settings
/// could in principle contain a key by that name; ours is load-bearing).
pub fn mcp_config_json(
    server_addr: SocketAddr,
    session_id: &str,
    agent: &str,
    extra_servers: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let url = format!("http://{}/sessions/{}/{}/mcp", server_addr, session_id, agent);
    let mut servers = serde_json::Map::new();
    for (name, val) in extra_servers {
        servers.insert(name.clone(), val.clone());
    }
    servers.insert(
        "bot-hq-signaling".to_string(),
        json!({ "type": "http", "url": url }),
    );
    serde_json::to_string_pretty(&json!({ "mcpServers": servers }))
        .unwrap_or_else(|_| "{}".into())
}

/// Read the user's claude-code MCP server config (across both locations
/// claude-code uses) and return the merged `mcpServers` map so spawned
/// subagents inherit the same MCP surface as the user's own claude-code
/// sessions.
///
/// `paths` are searched in order; entries from later paths win on key
/// collision. Production order: `~/.claude/settings.json` (older
/// per-user config) then `~/.claude.json` (the live config that
/// claude-code maintains). claude-code's live config takes precedence
/// because it's the source-of-truth claude-code itself uses — the
/// `settings.json` copy is often stale (e.g. an older `--browser-url`
/// port).
///
/// Filters out any entry whose key is in [`super::RESERVED_MCP_KEYS`] — see
/// that constant for the rationale.
///
/// Missing files / parse errors / missing `mcpServers` keys are tolerated
/// silently (with a debug log). We never fail spawn on this; the subagent
/// just falls back to `bot-hq-signaling` only.
pub fn load_user_mcp_servers(
    paths: &[std::path::PathBuf],
) -> serde_json::Map<String, serde_json::Value> {
    let mut merged: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for path in paths {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %path.display(), %e, "user MCP settings absent");
                continue;
            }
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                warn!(path = %path.display(), %e, "user MCP settings unparseable — skipping");
                continue;
            }
        };
        let Some(map) = parsed.get("mcpServers").and_then(|v| v.as_object()) else {
            continue;
        };
        for (k, v) in map {
            if super::RESERVED_MCP_KEYS.contains(&k.as_str()) {
                continue;
            }
            merged.insert(k.clone(), v.clone());
        }
    }
    merged
}

/// Default locations for the user's claude-code MCP config, in
/// least-to-most-trusted order. Later entries win on key collision.
/// `~/.claude.json` is the live config claude-code maintains (most
/// trusted); `~/.claude/settings.json` is the older per-user config
/// (often a stale snapshot, so it gets seeded first then overwritten).
pub fn default_user_settings_paths() -> Vec<std::path::PathBuf> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let home = std::path::PathBuf::from(home);
    vec![home.join(".claude/settings.json"), home.join(".claude.json")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_path_ok() {
        let p = parse_path("/sessions/abc-123/brian/mcp").unwrap();
        assert_eq!(p.session_id, "abc-123");
        assert_eq!(p.agent, "brian");
    }

    #[test]
    fn parse_path_reject() {
        assert!(parse_path("/").is_none());
        assert!(parse_path("/sessions/abc/brian").is_none());
        assert!(parse_path("/other/abc/brian/mcp").is_none());
    }

    #[tokio::test]
    async fn pre_push_route_approve_then_reject() {
        use crate::policy::ViolationsLog;
        use crate::signaling::SignalingEvent;

        let data = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(data.path());
        // with_policy already returns Arc<Self> (matches the action_gate tests).
        let bridge = SignalingBridge::with_policy(log, data.path().to_path_buf());
        let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
        let url = format!("http://{}/hooks/pre-push", server.local_addr);
        let client = reqwest::Client::new();

        // Approve → {"approved": true}. reqwest here lacks the `json` feature,
        // so send a raw body + parse the response text.
        let mut sub = bridge.subscribe();
        let url_a = url.clone();
        let client_a = client.clone();
        let call = tokio::spawn(async move {
            let body = json!({ "session_id": "s1", "agent": "brian", "branch": "main" }).to_string();
            let resp = client_a
                .post(&url_a)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .unwrap();
            let txt = resp.text().await.unwrap();
            serde_json::from_str::<serde_json::Value>(&txt).unwrap()
        });
        let cid = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        bridge.resolve_choice(&cid, "Approve".into()).await.unwrap();
        let resp = call.await.unwrap();
        assert_eq!(resp["approved"], json!(true), "approve → approved:true");

        // Reject → {"approved": false}. Also covers the no-branch body shape.
        let mut sub = bridge.subscribe();
        let call = tokio::spawn(async move {
            let body = json!({ "session_id": "s1", "agent": "brian" }).to_string();
            let resp = client
                .post(&url)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .unwrap();
            let txt = resp.text().await.unwrap();
            serde_json::from_str::<serde_json::Value>(&txt).unwrap()
        });
        let cid = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        bridge.resolve_choice(&cid, "Reject".into()).await.unwrap();
        let resp = call.await.unwrap();
        assert_eq!(resp["approved"], json!(false), "reject → approved:false");
    }

    #[tokio::test]
    async fn pre_push_route_missing_session_id_is_400() {
        use crate::policy::ViolationsLog;
        let data = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(data.path());
        // with_policy already returns Arc<Self> (matches the action_gate tests).
        let bridge = SignalingBridge::with_policy(log, data.path().to_path_buf());
        let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
        let url = format!("http://{}/hooks/pre-push", server.local_addr);
        let resp = reqwest::Client::new()
            .post(&url)
            .header("content-type", "application/json")
            .body(json!({ "agent": "brian" }).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 400);
    }

    #[test]
    fn mcp_config_shape() {
        let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
        let s = mcp_config_json(addr, "sess1", "brian", &serde_json::Map::new());
        assert!(s.contains("mcpServers"));
        assert!(s.contains("bot-hq-signaling"));
        assert!(s.contains("\"type\": \"http\""));
        assert!(s.contains("http://127.0.0.1:54321/sessions/sess1/brian/mcp"));
    }

    #[test]
    fn mcp_config_merges_user_servers() {
        let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
        let mut extras = serde_json::Map::new();
        extras.insert(
            "chrome-devtools".into(),
            json!({ "command": "node", "args": ["main.js"] }),
        );
        let s = mcp_config_json(addr, "sess1", "brian", &extras);
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        let servers = parsed.get("mcpServers").and_then(|v| v.as_object()).unwrap();
        assert!(servers.contains_key("bot-hq-signaling"));
        assert!(servers.contains_key("chrome-devtools"));
    }

    #[test]
    fn load_user_mcp_servers_filters_reserved_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "mcpServers": {
                    "bot-hq": { "command": "/Users/u/go/bin/bot-hq", "args": ["mcp"] },
                    "claude-in-chrome": { "command": "npx", "args": ["@anthropic-ai/claude-in-chrome-mcp"] },
                    "chrome-devtools": { "command": "node", "args": ["m.js"] }
                }
            }"#,
        )
        .unwrap();
        let map = load_user_mcp_servers(&[path]);
        assert!(!map.contains_key("bot-hq"), "bot-hq must be filtered to avoid recursion");
        assert!(
            !map.contains_key("claude-in-chrome"),
            "claude-in-chrome is reserved by claude-code in --mcp-config files"
        );
        assert!(map.contains_key("chrome-devtools"));
    }

    #[test]
    fn load_user_mcp_servers_missing_file_returns_empty() {
        let map = load_user_mcp_servers(&[std::path::PathBuf::from(
            "/nonexistent/path/settings.json",
        )]);
        assert!(map.is_empty());
    }

    #[test]
    fn load_user_mcp_servers_malformed_json_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        let map = load_user_mcp_servers(&[path]);
        assert!(map.is_empty());
    }

    #[test]
    fn load_user_mcp_servers_later_path_wins_on_collision() {
        // Simulates `~/.claude.json` correcting a stale entry in
        // `~/.claude/settings.json` — claude-code's live config is the
        // source of truth.
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("settings.json");
        let live = dir.path().join("claude.json");
        std::fs::write(
            &stale,
            r#"{ "mcpServers": {
                "brave-devtools": { "args": ["--browser-url=http://127.0.0.1:9222"] }
            } }"#,
        )
        .unwrap();
        std::fs::write(
            &live,
            r#"{ "mcpServers": {
                "brave-devtools": { "args": ["--browser-url=http://127.0.0.1:9225"] }
            } }"#,
        )
        .unwrap();
        let map = load_user_mcp_servers(&[stale, live]);
        let args = map["brave-devtools"]["args"][0].as_str().unwrap();
        assert!(args.contains("9225"), "expected live config to win, got: {args}");
    }
}
