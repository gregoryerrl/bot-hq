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
use crate::signaling::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
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

    let body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes(),
        Err(err) => {
            warn!(?err, "read body");
            return Ok(text_response(StatusCode::BAD_REQUEST, "body read error"));
        }
    };

    let rpc: JsonRpcRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(err) => {
            warn!(?err, "json-rpc parse");
            return Ok(json_response(
                StatusCode::OK,
                &JsonRpcResponse::err(
                    json!(null),
                    JsonRpcError::new(JsonRpcError::PARSE_ERROR, "invalid JSON"),
                ),
            ));
        }
    };

    debug!(method = %rpc.method, %caller.session_id, %caller.agent, "rpc");

    let id_for_err = rpc.id.clone().unwrap_or(json!(null));
    match dispatch(rpc, &caller, &bridge).await {
        Ok(None) => Ok(text_response(StatusCode::ACCEPTED, "")),
        Ok(Some(resp)) => Ok(json_response(StatusCode::OK, &resp)),
        Err(err) => Ok(json_response(
            StatusCode::OK,
            &JsonRpcResponse::err(id_for_err, err),
        )),
    }
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

fn text_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from(body.to_owned())))
        .expect("static response")
}

fn json_response<T: serde::Serialize>(status: StatusCode, body: &T) -> Response<Full<Bytes>> {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .expect("static response")
}

/// Render the mcp-config.json content claude-code expects for one agent.
///
/// Single entry under `mcpServers`, type `"http"` pointing at the per-agent
/// URL. The CLI passes through `env` to the subprocess — but since we use
/// HTTP transport (not stdio), env is unused.
pub fn mcp_config_json(server_addr: SocketAddr, session_id: &str, agent: &str) -> String {
    let url = format!("http://{}/sessions/{}/{}/mcp", server_addr, session_id, agent);
    serde_json::to_string_pretty(&json!({
        "mcpServers": {
            "bot-hq-signaling": {
                "type": "http",
                "url": url
            }
        }
    }))
    .unwrap_or_else(|_| "{}".into())
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
        let s = mcp_config_json(addr, "sess1", "brian");
        assert!(s.contains("mcpServers"));
        assert!(s.contains("bot-hq-signaling"));
        assert!(s.contains("\"type\": \"http\""));
        assert!(s.contains("http://127.0.0.1:54321/sessions/sess1/brian/mcp"));
    }
}
