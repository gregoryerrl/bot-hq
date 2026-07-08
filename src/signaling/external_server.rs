//! Bearer-token-authed MCP HTTP server for external drivers.
//!
//! Distinct from `signaling::server` (which exposes UI-signaling tools to
//! agent subprocesses bot-hq itself spawns). This server is for external MCP
//! clients — another Claude Code session, a test driver, a third-party agent —
//! that want to drive bot-hq from outside.
//!
//! Listen address: `127.0.0.1:<port>` only (defense in depth — refuses remote
//! connections at the bind layer). Default port 7892, overridable via
//! `BOT_HQ_EXTERNAL_MCP_PORT`. Auth: `Authorization: Bearer <token>` where
//! token lives at `<data_dir>/mcp-token` (UUIDv4, 0o600 perms).
//!
//! If the port is in use at startup, the listener fails to bind and the main
//! binary logs a warning and continues without external MCP — the internal
//! agent server is unaffected. To disable entirely, set
//! `BOT_HQ_EXTERNAL_MCP_DISABLED=1`.

use crate::core::AppState as CoreAppState;
use crate::signaling::external_jsonrpc::dispatch_external;
use crate::signaling::response::{decode_jsonrpc_body, dispatch_outcome_to_response, text_response};
use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue, AUTHORIZATION};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

pub struct ExternalServer {
    pub local_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ExternalServer {
    pub fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ExternalServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Bind `127.0.0.1:<port>` and start serving. Soft-fails on port conflict:
/// returns the bind error to the caller, who's expected to log + skip rather
/// than crash. Token is read once at startup; manual rotation requires
/// restart.
pub async fn start_external_server(
    core: Arc<CoreAppState>,
    port: u16,
    token: String,
) -> Result<ExternalServer> {
    let bind_addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("static addr");
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("binding external MCP listener at {bind_addr}"))?;
    let local_addr = listener.local_addr().context("reading bound addr")?;
    let (sd_tx, sd_rx) = oneshot::channel::<()>();
    let token_arc = Arc::new(token);
    let core_for_loop = Arc::clone(&core);

    tokio::spawn(async move {
        info!(addr = %local_addr, "external MCP server listening");
        let mut sd_rx = sd_rx;
        loop {
            tokio::select! {
                _ = &mut sd_rx => {
                    info!(addr = %local_addr, "external MCP server shutting down");
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let io = TokioIo::new(stream);
                            let core_for_conn = Arc::clone(&core_for_loop);
                            let token_for_conn = Arc::clone(&token_arc);
                            tokio::spawn(async move {
                                let svc = service_fn(move |req| {
                                    let core = Arc::clone(&core_for_conn);
                                    let token = Arc::clone(&token_for_conn);
                                    async move { handle_request(req, core, token).await }
                                });
                                if let Err(err) = http1::Builder::new()
                                    .serve_connection(io, svc)
                                    .await
                                {
                                    debug!(?err, "external connection ended");
                                }
                            });
                        }
                        Err(err) => warn!(?err, "external accept failed"),
                    }
                }
            }
        }
    });

    Ok(ExternalServer {
        local_addr,
        shutdown: Some(sd_tx),
    })
}

/// CORS for browser-context callers — bot-hq plugin panels `fetch()`ing the
/// driver cross-origin from their `bhq-plugin://` documents. The webview
/// preflights any fetch carrying Authorization + a JSON body; without these
/// headers the preflight is refused and the browser blocks the call before
/// it ever reaches auth. `*` grants nothing by itself: every real request
/// still needs the bearer token, and `*` is incompatible with credentialed
/// (cookie) mode — it only lets an already-authorized caller READ the
/// response it paid for. Applied to EVERY response, 401s included, so a
/// browser caller can tell bad-token apart from server-gone.
fn with_cors(mut resp: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    let headers = resp.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    resp
}

async fn handle_request(
    req: Request<Incoming>,
    core: Arc<CoreAppState>,
    expected_token: Arc<String>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    Ok(with_cors(handle_inner(req, core, expected_token).await))
}

async fn handle_inner(
    req: Request<Incoming>,
    core: Arc<CoreAppState>,
    expected_token: Arc<String>,
) -> Response<Full<Bytes>> {
    // CORS preflight: answered for any path, BEFORE auth — browsers strip
    // custom headers from OPTIONS, so a preflight can never carry the
    // bearer. A 204 leaks nothing and executes nothing.
    if req.method() == Method::OPTIONS {
        return Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Full::new(Bytes::new()))
            .expect("static preflight response");
    }
    // POST only.
    if req.method() != Method::POST {
        return text_response(StatusCode::METHOD_NOT_ALLOWED, "POST only");
    }
    // Single endpoint at /mcp.
    if req.uri().path() != "/mcp" {
        return text_response(StatusCode::NOT_FOUND, "expected /mcp");
    }

    // Authorization: Bearer <token>
    let auth_value = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let token = match auth_value {
        Some(t) => t.trim(),
        None => {
            return text_response(StatusCode::UNAUTHORIZED, "missing bearer token");
        }
    };

    // Constant-time compare. `subtle::ConstantTimeEq` short-circuits to false
    // when lengths differ, so we don't need a length check.
    let ok = bool::from(token.as_bytes().ct_eq(expected_token.as_bytes()));
    if !ok {
        return text_response(StatusCode::UNAUTHORIZED, "invalid token");
    }

    // Body → JSON-RPC.
    let rpc = match decode_jsonrpc_body(req.into_body()).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    debug!(method = %rpc.method, "external rpc");
    let id_for_err = rpc.id.clone().unwrap_or(json!(null));
    dispatch_outcome_to_response(dispatch_external(rpc, &core).await, id_for_err)
}

