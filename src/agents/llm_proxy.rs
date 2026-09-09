//! Local normalizing proxy for participants whose model row carries a custom
//! `base_url` — an Anthropic-compatible gateway that is not Anthropic's (the
//! DeepSeek gateway is the one this was built against; the routing rule is
//! per model row, never per participant name — rc3 D10).
//!
//! ## Why this exists
//!
//! claude-code (>= 2.1.156) serializes a `SessionStart` hook's
//! `additionalContext` — and potentially other request-build-time context —
//! as a `role:"system"` entry *inside* the request's `messages` array. The
//! real Anthropic API tolerates this. Stricter Anthropic-compatible gateways
//! (DeepSeek) reject it with
//! `400 ... messages[N].role: unknown variant `system``, killing every turn.
//!
//! `--bare` (skip plugin sync / hooks / LSP) *reduces* but does **not**
//! eliminate the injection — verified empirically (2026-07, on the reviewer
//! participant of that day's roster): a fresh `--bare` process still 400s on
//! a fixed `messages[11]`. The injection happens at
//! request-build time and is not stored in the transcript, so it cannot be
//! sanitized at the source from bot-hq's side.
//!
//! ## What it does
//!
//! A tiny localhost reverse proxy. The routed participant's `ANTHROPIC_BASE_URL` points at it
//! (`http://127.0.0.1:<port>/<hex(real-upstream)>`); for each request it
//! rewrites the JSON body — hoisting any `role:"system"` message out of
//! `messages[]` and into the top-level `system` field (which every gateway
//! accepts) — then forwards to the real upstream over TLS and streams the
//! response straight back. Source-agnostic: it strips the alien role no
//! matter which hook/mechanism injected it.
//!
//! Only participants whose model has a custom `base_url` route through it; a
//! participant on the first-party API hits it directly and never touches the
//! proxy.

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::TryStreamExt;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::HeaderMap;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Map, Value};
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::{LazyLock, OnceLock};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

// `UnsyncBoxBody` (not `BoxBody`): reqwest's response byte-stream is `Send`
// but not necessarily `Sync`, and a streaming proxy response only needs the
// connection task to be `Send`. `BoxBody` would impose an unmet `Sync` bound.
type ProxyBody = UnsyncBoxBody<Bytes, io::Error>;

/// Hop-by-hop headers that must NOT be forwarded across a proxy boundary
/// (RFC 7230 §6.1) plus framing headers we let the receiving stack recompute.
const STRIPPED_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Shared reqwest client (connection pooling). rustls-TLS per Cargo features.
/// No overall timeout — streaming completions can run for minutes.
static CLIENT: LazyLock<reqwest::Client> =
    LazyLock::new(|| reqwest::Client::builder().build().unwrap_or_default());

/// Process-wide proxy singleton. Set once at startup via [`install_global`];
/// read at agent-spawn time via [`proxy_addr`]. Mirrors the `CHILD_PIDS`
/// global precedent — the proxy is a true process singleton, so threading its
/// address through AppState + every spawn signature would be pure noise.
static PROXY: OnceLock<LlmProxy> = OnceLock::new();

/// Handle to the running proxy. Dropping it shuts the listener down (used in
/// tests); the production instance lives in [`PROXY`] for the process lifetime.
pub struct LlmProxy {
    pub local_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for LlmProxy {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Store the started proxy as the process singleton. Idempotent — a second
/// call is ignored (the first instance wins).
pub fn install_global(proxy: LlmProxy) {
    let _ = PROXY.set(proxy);
}

/// Address of the running proxy, or `None` if it never started.
pub fn proxy_addr() -> Option<SocketAddr> {
    PROXY.get().map(|p| p.local_addr)
}

/// The upstreams this process has actually routed a participant to. The
/// proxy is "stateless" in that the upstream rides the request path — but
/// that path is written by whoever connects, so without this list the proxy
/// was an open forwarder: any local process (or a browser page that found the
/// port) could relay a request to any URL through bot-hq, loopback and
/// RFC 1918 hosts included, with the upstream's response headers copied back
/// (2026-09-06 sweep, S3). An upstream enters the set only through
/// [`proxied_base_url`], i.e. when a spawn resolves a model row's `base_url`,
/// and stays for the process lifetime.
static ALLOWED_UPSTREAMS: LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

fn allow_upstream(base: &str) {
    ALLOWED_UPSTREAMS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(base.to_string());
}

fn upstream_is_allowed(base: &str) -> bool {
    ALLOWED_UPSTREAMS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(base)
}

/// Decide the `ANTHROPIC_BASE_URL` value for an agent.
///
/// - No custom base_url (real Anthropic) → `None` (don't set the env var).
/// - Custom base_url + proxy up → route through the proxy, encoding the real
///   upstream in the path so the proxy stays stateless.
/// - Custom base_url + proxy down → use it directly (graceful fallback; the
///   400 may resurface, but the agent isn't dead-in-the-water on a config we
///   couldn't proxy).
///
/// Pure + total so it's unit-testable without the global. Deliberately
/// module-private: production callers go through [`proxied_base_url`], which
/// also registers the upstream with the proxy — a URL built here alone is
/// refused by [`handle_proxy`], so a caller that could reach this directly
/// would produce agents whose every request 403s.
fn resolve_anthropic_base_url(
    configured: Option<&str>,
    proxy_addr: Option<SocketAddr>,
) -> Option<String> {
    let base = configured.map(str::trim).filter(|s| !s.is_empty())?;
    match proxy_addr {
        Some(addr) => Some(format!("http://{addr}/{}", hex_encode(base.as_bytes()))),
        None => Some(base.to_string()),
    }
}

/// [`resolve_anthropic_base_url`] for a real spawn: the same decision, and
/// when it routes through the proxy the upstream is registered so the proxy
/// will forward to it. The only producer of allowed upstreams by design — the
/// two spawn paths (`agents::spawn`, `tauri_cmd::docs`) call this, nothing
/// else touches the list.
pub fn proxied_base_url(configured: Option<&str>, proxy_addr: Option<SocketAddr>) -> Option<String> {
    let resolved = resolve_anthropic_base_url(configured, proxy_addr)?;
    if proxy_addr.is_some() {
        // Registered as the proxy will see it after `hex_decode`: the trimmed
        // configured value, byte for byte.
        if let Some(base) = configured.map(str::trim).filter(|s| !s.is_empty()) {
            allow_upstream(base);
        }
    }
    Some(resolved)
}

/// Bind an ephemeral localhost port and start serving. Returns once bound.
pub async fn start_llm_proxy() -> Result<LlmProxy> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding llm proxy listener")?;
    let local_addr = listener.local_addr().context("reading llm proxy addr")?;
    let (sd_tx, sd_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        info!(addr = %local_addr, "llm normalizing proxy listening");
        let mut sd_rx = sd_rx;
        loop {
            tokio::select! {
                _ = &mut sd_rx => {
                    info!(addr = %local_addr, "llm proxy shutting down");
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let io = TokioIo::new(stream);
                            tokio::spawn(async move {
                                let svc = service_fn(handle_proxy);
                                if let Err(err) =
                                    http1::Builder::new().serve_connection(io, svc).await
                                {
                                    warn!(?err, "llm proxy connection error");
                                }
                            });
                        }
                        Err(err) => warn!(?err, "llm proxy accept failed"),
                    }
                }
            }
        }
    });

    Ok(LlmProxy {
        local_addr,
        shutdown: Some(sd_tx),
    })
}

async fn handle_proxy(req: Request<Incoming>) -> Result<Response<ProxyBody>, Infallible> {
    // A browser page that found the port gets nothing — not even the 403's
    // body is readable to it, but the forward must not happen either. See
    // `core::loopback` for why this header test is sufficient.
    if crate::core::loopback::browser_originated(req.headers()) {
        return Ok(text_resp(StatusCode::FORBIDDEN, "not for browsers"));
    }
    // Path shape: /<hex(upstream-base-url)>/<suffix...><?query>
    let pq = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/")
        .to_string();
    let trimmed = pq.trim_start_matches('/');
    let (hex_seg, rest) = trimmed.split_once('/').unwrap_or((trimmed, ""));
    let upstream = match hex_decode(hex_seg) {
        Some(u) => u,
        None => return Ok(text_resp(StatusCode::BAD_GATEWAY, "bad upstream encoding")),
    };
    // The path names the upstream, but only an upstream a spawn registered
    // (`proxied_base_url`) is forwarded to — otherwise this is an open relay.
    if !upstream_is_allowed(&upstream) {
        warn!(%upstream, "proxy refused: upstream was never registered by a spawn");
        return Ok(text_resp(
            StatusCode::FORBIDDEN,
            "upstream not registered with this bot-hq process",
        ));
    }
    let target = format!("{}/{}", upstream.trim_end_matches('/'), rest);

    let method = req.method().clone();
    let req_headers = forward_headers(req.headers());

    let raw_body = match req.into_body().collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            return Ok(text_resp(
                StatusCode::BAD_REQUEST,
                &format!("read body: {e}"),
            ))
        }
    };
    let body = normalize_messages_body(&raw_body).into_owned();

    debug!(%target, in_len = raw_body.len(), out_len = body.len(), "proxy forward");

    let upstream_resp = CLIENT
        .request(method, target.as_str())
        .headers(req_headers)
        .body(body)
        .send()
        .await;

    match upstream_resp {
        Ok(resp) => {
            let status = resp.status();
            let mut builder = Response::builder().status(status);
            for (k, v) in resp.headers() {
                if is_stripped(k.as_str()) {
                    continue;
                }
                builder = builder.header(k.clone(), v.clone());
            }
            let stream = resp
                .bytes_stream()
                .map_ok(hyper::body::Frame::data)
                .map_err(io::Error::other);
            let body = http_body_util::StreamBody::new(stream).boxed_unsync();
            match builder.body(body) {
                Ok(r) => Ok(r),
                Err(e) => Ok(text_resp(
                    StatusCode::BAD_GATEWAY,
                    &format!("build response: {e}"),
                )),
            }
        }
        Err(e) => Ok(text_resp(
            StatusCode::BAD_GATEWAY,
            &format!("upstream request failed: {e}"),
        )),
    }
}

/// Copy request headers, dropping hop-by-hop + framing headers the upstream
/// client recomputes (notably `content-length`, which changes after rewrite).
fn forward_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (k, v) in src {
        if is_stripped(k.as_str()) {
            continue;
        }
        out.append(k.clone(), v.clone());
    }
    out
}

fn is_stripped(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    STRIPPED_HEADERS.contains(&lower.as_str())
}

fn text_resp(status: StatusCode, msg: &str) -> Response<ProxyBody> {
    let body = Full::new(Bytes::copy_from_slice(msg.as_bytes()))
        .map_err(|never| match never {})
        .boxed_unsync();
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(body)
        .unwrap_or_else(|_| {
            Response::new(
                Full::new(Bytes::new())
                    .map_err(|never| match never {})
                    .boxed_unsync(),
            )
        })
}

/// Rewrite an Anthropic `/v1/messages` request body so no `messages[]` entry
/// has `role:"system"`: each such entry's text is hoisted into the top-level
/// `system` field and the entry is removed. Returns the input unchanged
/// (borrowed) when it isn't a JSON object, has no `messages` array, or has no
/// system-role messages — so non-message requests (token counting, model
/// listing) and already-clean bodies pass through untouched.
fn normalize_messages_body(raw: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let Ok(mut v) = serde_json::from_slice::<Value>(raw) else {
        return std::borrow::Cow::Borrowed(raw);
    };
    let Some(obj) = v.as_object_mut() else {
        return std::borrow::Cow::Borrowed(raw);
    };
    let has_system = obj
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .any(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        })
        .unwrap_or(false);
    if !has_system {
        return std::borrow::Cow::Borrowed(raw);
    }

    let mut hoisted: Vec<String> = Vec::new();
    if let Some(arr) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
        arr.retain(|m| {
            if m.get("role").and_then(|r| r.as_str()) == Some("system") {
                let text = extract_text(m.get("content"));
                if !text.is_empty() {
                    hoisted.push(text);
                }
                false
            } else {
                true
            }
        });
    }
    merge_into_system(obj, &hoisted);

    match serde_json::to_vec(&v) {
        Ok(bytes) => std::borrow::Cow::Owned(bytes),
        Err(_) => std::borrow::Cow::Borrowed(raw),
    }
}

/// Extract the plain text of a message `content` field — either a bare string
/// or an array of content blocks (concatenating `text`-type blocks).
fn extract_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => {
            let mut parts = Vec::new();
            for b in blocks {
                let is_text = b
                    .get("type")
                    .and_then(|t| t.as_str())
                    .is_none_or(|ty| ty == "text");
                if is_text {
                    if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                        parts.push(t.to_string());
                    }
                }
            }
            parts.join("\n")
        }
        _ => String::new(),
    }
}

/// Append hoisted system text to the top-level `system` field, preserving its
/// existing shape (string → string, array of blocks → push a text block,
/// absent → new string). Remove+reinsert avoids a borrow conflict.
fn merge_into_system(obj: &mut Map<String, Value>, hoisted: &[String]) {
    if hoisted.is_empty() {
        return;
    }
    let extra = hoisted.join("\n\n");
    let new_system = match obj.remove("system") {
        Some(Value::String(s)) if !s.is_empty() => Value::String(format!("{s}\n\n{extra}")),
        Some(Value::String(_)) => Value::String(extra),
        Some(Value::Array(mut blocks)) => {
            blocks.push(json!({ "type": "text", "text": extra }));
            Value::Array(blocks)
        }
        // `system` is spec'd as string | array | absent; any other shape is
        // unexpected — fall back to a plain string so the field stays valid.
        _ => Value::String(extra),
    };
    obj.insert("system".to_string(), new_system);
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

fn hex_decode(s: &str) -> Option<String> {
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let url = "https://api.deepseek.com/anthropic";
        let enc = hex_encode(url.as_bytes());
        assert!(enc.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hex_decode(&enc).as_deref(), Some(url));
    }

    #[test]
    fn hex_decode_rejects_malformed() {
        assert_eq!(hex_decode(""), None);
        assert_eq!(hex_decode("abc"), None); // odd length
        assert_eq!(hex_decode("zz"), None); // non-hex
    }

    #[test]
    fn resolve_base_url_cases() {
        let addr: SocketAddr = "127.0.0.1:9000".parse().unwrap();
        // No custom base url → no env var.
        assert_eq!(resolve_anthropic_base_url(None, Some(addr)), None);
        assert_eq!(resolve_anthropic_base_url(Some(""), Some(addr)), None);
        assert_eq!(resolve_anthropic_base_url(Some("   "), Some(addr)), None);
        // Custom base url, proxy down → passthrough.
        assert_eq!(
            resolve_anthropic_base_url(Some("https://api.deepseek.com/anthropic"), None).as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        // Custom base url, proxy up → routed with hex-encoded upstream.
        let routed =
            resolve_anthropic_base_url(Some("https://api.deepseek.com/anthropic"), Some(addr))
                .unwrap();
        assert!(routed.starts_with("http://127.0.0.1:9000/"));
        let hex = routed.rsplit('/').next().unwrap();
        assert_eq!(
            hex_decode(hex).as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
    }

    #[test]
    fn normalize_hoists_system_message_string_content() {
        let body = json!({
            "model": "deepseek-v4-pro",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "system", "content": "INJECTED CONTEXT"},
                {"role": "assistant", "content": "ok"}
            ]
        });
        let raw = body.to_string();
        let out = normalize_messages_body(raw.as_bytes());
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let msgs = parsed["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2, "system message removed from messages[]");
        assert!(
            !msgs.iter().any(|m| m["role"] == "system"),
            "no role:system remains"
        );
        assert_eq!(parsed["system"], json!("INJECTED CONTEXT"));
    }

    #[test]
    fn normalize_hoists_system_message_array_content() {
        let body = json!({
            "messages": [
                {"role": "system", "content": [
                    {"type": "text", "text": "block one"},
                    {"type": "text", "text": "block two"}
                ]},
                {"role": "user", "content": "go"}
            ]
        });
        let raw = body.to_string();
        let out = normalize_messages_body(raw.as_bytes());
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["messages"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["system"], json!("block one\nblock two"));
    }

    #[test]
    fn normalize_appends_to_existing_string_system() {
        let body = json!({
            "system": "BASE PROMPT",
            "messages": [
                {"role": "system", "content": "EXTRA"},
                {"role": "user", "content": "x"}
            ]
        });
        let raw = body.to_string();
        let out = normalize_messages_body(raw.as_bytes());
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["system"], json!("BASE PROMPT\n\nEXTRA"));
    }

    #[test]
    fn normalize_pushes_block_to_array_system() {
        let body = json!({
            "system": [{"type": "text", "text": "BASE"}],
            "messages": [
                {"role": "system", "content": "EXTRA"},
                {"role": "user", "content": "x"}
            ]
        });
        let raw = body.to_string();
        let out = normalize_messages_body(raw.as_bytes());
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let sys = parsed["system"].as_array().unwrap();
        assert_eq!(sys.len(), 2);
        assert_eq!(sys[1], json!({"type": "text", "text": "EXTRA"}));
    }

    #[test]
    fn normalize_leaves_clean_body_untouched() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "yo"}
            ]
        });
        let raw = body.to_string();
        let out = normalize_messages_body(raw.as_bytes());
        assert!(
            matches!(out, std::borrow::Cow::Borrowed(_)),
            "no copy when clean"
        );
        assert_eq!(out.as_ref(), raw.as_bytes());
    }

    #[test]
    fn normalize_passes_through_non_json() {
        let raw = b"not json at all";
        let out = normalize_messages_body(raw);
        assert_eq!(out.as_ref(), raw);
    }

    #[test]
    fn normalize_passes_through_body_without_messages() {
        let raw = json!({"model": "x"}).to_string();
        let out = normalize_messages_body(raw.as_bytes());
        assert!(matches!(out, std::borrow::Cow::Borrowed(_)));
    }

    /// End-to-end proof: a body that WOULD 400 on a strict gateway comes back
    /// 200 through the proxy, and the upstream receives a body with no
    /// `role:"system"`. This is the verification the `--bare` fix lacked.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn proxy_strips_system_role_end_to_end() {
        use std::sync::{Arc, Mutex};

        // Mock "DeepSeek": 400 if it sees role:"system", else 200. Records the
        // received body so we can assert it was cleaned.
        let received: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let received_for_svc = Arc::clone(&received);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let rec = Arc::clone(&received_for_svc);
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let rec = Arc::clone(&rec);
                        async move {
                            let bytes = req.into_body().collect().await.unwrap().to_bytes();
                            *rec.lock().unwrap() = bytes.to_vec();
                            let saw_system = serde_json::from_slice::<Value>(&bytes)
                                .ok()
                                .and_then(|v| {
                                    v.get("messages").and_then(|m| m.as_array()).map(|a| {
                                        a.iter().any(|m| {
                                            m.get("role").and_then(|r| r.as_str()) == Some("system")
                                        })
                                    })
                                })
                                .unwrap_or(false);
                            let status = if saw_system {
                                StatusCode::BAD_REQUEST
                            } else {
                                StatusCode::OK
                            };
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .status(status)
                                    .body(Full::new(Bytes::from_static(b"{\"ok\":true}")))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = http1::Builder::new().serve_connection(io, svc).await;
                });
            }
        });

        let proxy = start_llm_proxy().await.unwrap();
        let upstream_url = format!("http://{upstream_addr}");
        // The URL a real spawn hands the agent — which is also what registers
        // the upstream; a hand-built `hex_encode` URL alone is refused (see
        // `proxy_refuses_an_upstream_no_spawn_registered`).
        let base = proxied_base_url(Some(&upstream_url), Some(proxy.local_addr)).unwrap();
        let proxy_url = format!("{base}/v1/messages");

        let dirty_body = json!({
            "model": "deepseek-v4-pro",
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "system", "content": "INJECTED"},
                {"role": "assistant", "content": "hi"}
            ]
        });

        let resp = reqwest::Client::new()
            .post(proxy_url.as_str())
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&dirty_body).unwrap())
            .send()
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            reqwest::StatusCode::OK,
            "proxy must turn a would-be-400 into a 200 by stripping role:system"
        );
        let got: Value = serde_json::from_slice(&received.lock().unwrap()).unwrap();
        assert!(
            !got["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["role"] == "system"),
            "upstream must receive a body with no role:system"
        );
        assert_eq!(got["system"], json!("INJECTED"), "system text was hoisted");
    }

    /// The proxy is not an open relay (S3): an upstream that no spawn
    /// registered is refused before any connection is attempted. The forged
    /// upstream is TEST-NET-1 (RFC 5737, never routable) so a regression
    /// fails fast with a 502 instead of hanging on a connect — or, worse,
    /// reaching something real from CI — and the 403 proves the forward
    /// never started.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn proxy_refuses_an_upstream_no_spawn_registered() {
        let proxy = start_llm_proxy().await.unwrap();
        // A URL of the exact shape a spawn would produce, built WITHOUT the
        // registering wrapper — what a local process or a page would forge.
        let forged = resolve_anthropic_base_url(
            Some("http://192.0.2.1/latest/meta-data"),
            Some(proxy.local_addr),
        )
        .unwrap();
        let resp = reqwest::Client::new()
            .post(format!("{forged}/v1/messages"))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(resp.text().await.unwrap().contains("not registered"));
    }

    /// A browser page that finds the port gets a 403 even for a registered
    /// upstream: `Origin` / `Sec-Fetch-*` are headers only a browser sends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn proxy_refuses_browser_shaped_requests() {
        let proxy = start_llm_proxy().await.unwrap();
        let base = proxied_base_url(Some("http://127.0.0.1:9/registered"), Some(proxy.local_addr))
            .unwrap();
        for (k, v) in [("origin", "http://evil.example"), ("sec-fetch-mode", "no-cors")] {
            let resp = reqwest::Client::new()
                .post(format!("{base}/v1/messages"))
                .header(k, v)
                .body("{}")
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{k}: {v}");
        }
    }
}
