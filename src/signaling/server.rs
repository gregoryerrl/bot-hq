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

use crate::signaling::bridge::SignalingBridge;
use crate::signaling::jsonrpc::{dispatch, CallerIdentity};
use crate::signaling::response::{decode_jsonrpc_body, dispatch_outcome_to_response, text_response};
use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

pub struct SignalingServer {
    pub local_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    pub bridge: Arc<SignalingBridge>,
}

impl SignalingServer {
    pub fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for SignalingServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
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

/// Names that must be filtered from the merged user MCP map before being
/// written to a subagent's mcp-config.json.
///
/// - `bot-hq`: the user's top-level bot-hq MCP. Exposing it inside a
///   bot-hq-spawned agent creates a recursive driver loop; the agent
///   already has `bot-hq-signaling` for the per-(session,agent) channel.
/// - `claude-in-chrome`: claude-code rejects this name as reserved when
///   it appears in a `--mcp-config` file, so the whole subprocess exits
///   on startup. (The capability is still available to the parent
///   claude-code session via its own built-in path.)
const RESERVED_MCP_KEYS: &[&str] = &["bot-hq", "claude-in-chrome"];

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
/// Filters out any entry whose key is in [`RESERVED_MCP_KEYS`] — see that
/// constant for the rationale.
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
            if RESERVED_MCP_KEYS.contains(&k.as_str()) {
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
