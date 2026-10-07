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
//! accepts), and replacing any tool call the gateway cannot replay (a
//! `tool_use` with an empty id or name, plus the `tool_result` that answers
//! it — the model's own malformed output, which claude-code replays on every
//! later request until the conversation dies of `400 … non-empty string
//! tool_call_id`; OpenRouter → MiMo, 2026-10-07) — then forwards to the real
//! upstream over TLS and streams the response straight back. Source-agnostic:
//! it strips the alien role no matter which hook/mechanism injected it.
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

/// Rewrite an Anthropic `/v1/messages` request body for a gateway stricter
/// than Anthropic's API. Two independent passes over one parse:
///
/// 1. no `messages[]` entry keeps `role:"system"` — each such entry's text is
///    hoisted into the top-level `system` field and the entry is removed;
/// 2. no tool call the gateway cannot replay survives — see
///    [`strip_unrunnable_tool_calls`].
///
/// Returns the input unchanged (borrowed) when it isn't a JSON object, has no
/// `messages` array, or neither pass changed anything — so non-message
/// requests (token counting, model listing) and already-clean bodies pass
/// through untouched, byte for byte.
fn normalize_messages_body(raw: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let Ok(mut v) = serde_json::from_slice::<Value>(raw) else {
        return std::borrow::Cow::Borrowed(raw);
    };
    let Some(obj) = v.as_object_mut() else {
        return std::borrow::Cow::Borrowed(raw);
    };
    if !obj.get("messages").is_some_and(Value::is_array) {
        return std::borrow::Cow::Borrowed(raw);
    }

    let hoisted = hoist_system_messages(obj);
    let stripped = strip_unrunnable_tool_calls(obj);
    if !hoisted && stripped == 0 {
        return std::borrow::Cow::Borrowed(raw);
    }
    if stripped > 0 {
        // Once per request, and the pair stays in the transcript, so this
        // repeats on every later request of that conversation: a state line,
        // not an alarm.
        info!(
            stripped,
            "proxy: replaced tool call(s) with no id or name that the gateway could not replay"
        );
    }

    match serde_json::to_vec(&v) {
        Ok(bytes) => std::borrow::Cow::Owned(bytes),
        Err(_) => std::borrow::Cow::Borrowed(raw),
    }
}

/// Pass 1: hoist every `role:"system"` entry out of `messages[]` into the
/// top-level `system` field. `true` when at least one entry moved.
fn hoist_system_messages(obj: &mut Map<String, Value>) -> bool {
    let mut hoisted: Vec<String> = Vec::new();
    let mut moved = false;
    if let Some(arr) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
        arr.retain(|m| {
            if m.get("role").and_then(|r| r.as_str()) == Some("system") {
                let text = extract_text(m.get("content"));
                if !text.is_empty() {
                    hoisted.push(text);
                }
                moved = true;
                false
            } else {
                true
            }
        });
    }
    merge_into_system(obj, &hoisted);
    moved
}

/// What the replaced `tool_result` says in its place. The model has to learn
/// that its call did not run: at the moment of the 2026-10-07 poisoning the
/// participant believed it had halted the session.
const UNRUNNABLE_CALL_RESULT: &str = "[bot-hq] Your previous tool call carried no id or name, \
     so it did not run";

/// What an assistant message left with no block the API accepts alone says.
const UNRUNNABLE_CALL_PLACEHOLDER: &str =
    "[bot-hq] (a tool call with no id or name was removed here)";

/// Pass 2: remove every `tool_use` block whose `id` or `name` is empty or
/// missing, and turn the `tool_result` blocks that answer them — an empty
/// `tool_use_id`, or one naming a removed call — into text that says the call
/// did not run. Returns how many blocks changed.
///
/// Why: a gateway that translates to the OpenAI shape turns a `tool_result`
/// into a `tool` message whose `tool_call_id` must be a non-empty string that
/// matches an earlier call, and rejects the whole request otherwise (`400
/// messages[N]: tool messages must include a non-empty string tool_call_id`,
/// OpenRouter → xiaomi/mimo-v2.6-pro, 2026-10-07). claude-code faithfully
/// replays the pair the model produced, so the request fails for the rest of
/// the conversation; this is the only seam between the transcript and the
/// gateway.
///
/// Shape rules, both measured against gateway 400s: the replacement text goes
/// AFTER the surviving `tool_result` blocks of its message (tool results must
/// come first), and no message is left with `content: []` or with only
/// `thinking` blocks — such an assistant message gets one text block.
fn strip_unrunnable_tool_calls(obj: &mut Map<String, Value>) -> usize {
    let Some(arr) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return 0;
    };
    let mut changed = 0usize;
    let mut dropped_ids: Vec<String> = Vec::new();
    for msg in arr.iter_mut() {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        let Some(blocks) = msg.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        match role.as_str() {
            "assistant" => {
                let before = blocks.len();
                blocks.retain(|b| {
                    if b.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                        return true;
                    }
                    let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if id.is_empty() || name.is_empty() {
                        dropped_ids.push(id.to_string());
                        false
                    } else {
                        true
                    }
                });
                let removed = before - blocks.len();
                changed += removed;
                let accepted_alone = blocks.iter().any(|b| {
                    !matches!(
                        b.get("type").and_then(|t| t.as_str()),
                        Some("thinking") | Some("redacted_thinking")
                    )
                });
                if removed > 0 && !accepted_alone {
                    blocks.push(json!({ "type": "text", "text": UNRUNNABLE_CALL_PLACEHOLDER }));
                }
            }
            "user" => {
                let mut replacements: Vec<Value> = Vec::new();
                blocks.retain(|b| {
                    if b.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                        return true;
                    }
                    let id = b.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                    if !id.is_empty() && !dropped_ids.iter().any(|d| d == id) {
                        return true;
                    }
                    let original = extract_text(b.get("content"));
                    let text = if original.is_empty() {
                        format!("{UNRUNNABLE_CALL_RESULT}. Issue it again as a complete tool call.")
                    } else {
                        format!(
                            "{UNRUNNABLE_CALL_RESULT} ({original}). Issue it again as a complete \
                             tool call."
                        )
                    };
                    replacements.push(json!({ "type": "text", "text": text }));
                    false
                });
                changed += replacements.len();
                blocks.extend(replacements);
            }
            _ => {}
        }
    }
    changed
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

    /// The 2026-10-07 poisoning, as the request carried it: MiMo (OpenRouter →
    /// Novita) answered with a `tool_use` whose id AND name were empty,
    /// claude-code replied with a `tool_result` whose `tool_use_id` was "", and
    /// every later request replayed the pair — the gateway's OpenAI translation
    /// answered `400 messages[43]: tool messages must include a non-empty string
    /// tool_call_id` until the session died. The proxy drops the call and turns
    /// its result into text that says it did not run, so the model learns that
    /// and the request never ends on an assistant message (prefill semantics
    /// through a gateway are unknown).
    ///
    /// No `role:"system"` message in this body on purpose: the hoist used to
    /// return early when there was none, and a strip placed after that return
    /// would never have run on a normal request.
    #[test]
    fn normalize_replaces_a_tool_call_with_no_id_and_its_result() {
        let body = json!({
            "model": "xiaomi/mimo-v2.6-pro",
            "messages": [
                {"role": "user", "content": "go"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "…", "signature": "sig"},
                    {"type": "text", "text": "<tool_call><function=mark_awaiting_user</parameter>…"},
                    {"type": "tool_use", "id": "", "name": "", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "", "is_error": true,
                     "content": "<tool_use_error>Error: No such tool available: </tool_use_error>"}
                ]}
            ]
        });
        let raw = body.to_string();
        let out = normalize_messages_body(raw.as_bytes());
        assert!(matches!(out, std::borrow::Cow::Owned(_)), "the body was rewritten");
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let msgs = parsed["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3, "no message is dropped: {parsed}");
        let assistant = msgs[1]["content"].as_array().unwrap();
        assert_eq!(assistant.len(), 2, "only the empty tool_use left the assistant message: {parsed}");
        assert!(assistant.iter().all(|b| b["type"] != "tool_use"));
        let user = msgs[2]["content"].as_array().unwrap();
        assert_eq!(user.len(), 1);
        assert_eq!(user[0]["type"], "text", "the result became text, not an empty message: {parsed}");
        let text = user[0]["text"].as_str().unwrap();
        for needle in ["[bot-hq]", "no id or name", "did not run", "No such tool available", "again"] {
            assert!(text.contains(needle), "the model must learn the call did not run — lacks {needle:?}: {text}");
        }
        assert!(msgs[2]["role"] == "user", "the request still ends on a user message");
        assert!(!out.windows(16).any(|w| w == br#""tool_use_id":"""#), "no empty tool_use_id survives");
    }

    /// A call with an id but no name is just as unrunnable, and the gateway
    /// would 400 on its orphaned result (`tool_call_id` with no matching call)
    /// once the call is gone — so the result that NAMES a dropped id goes too.
    /// The placeholder text lands AFTER the surviving `tool_result` blocks:
    /// Anthropic requires tool_results first in a user message, and through
    /// OpenRouter a text block between an assistant's tool_calls and their
    /// `tool` messages is a 400 of its own.
    #[test]
    fn normalize_drops_a_nameless_call_and_keeps_its_results_order() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "go"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "call_bad", "name": "", "input": {}},
                    {"type": "tool_use", "id": "call_ok", "name": "Read", "input": {"file_path": "x"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "call_bad", "content": "err"},
                    {"type": "tool_result", "tool_use_id": "call_ok", "content": "file body"}
                ]}
            ]
        });
        let raw = body.to_string();
        let out = normalize_messages_body(raw.as_bytes());
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let assistant = parsed["messages"][1]["content"].as_array().unwrap();
        assert_eq!(assistant.len(), 1);
        assert_eq!(assistant[0]["id"], "call_ok", "the healthy call stays: {parsed}");
        let user = parsed["messages"][2]["content"].as_array().unwrap();
        assert_eq!(user.len(), 2, "one result kept, one turned to text: {parsed}");
        assert_eq!(user[0]["type"], "tool_result");
        assert_eq!(user[0]["tool_use_id"], "call_ok");
        assert_eq!(user[1]["type"], "text", "the text goes AFTER the tool_results: {parsed}");
        assert!(user[1]["text"].as_str().unwrap().contains("err"));
    }

    /// An assistant message left with nothing — or with only thinking blocks,
    /// which the API will not accept alone — gets one text block saying what
    /// was there; `content: []` is a 400 of its own.
    #[test]
    fn normalize_never_leaves_an_assistant_message_empty() {
        for content in [
            json!([{"type": "tool_use", "id": "", "name": "", "input": {}}]),
            json!([
                {"type": "thinking", "thinking": "…", "signature": "s"},
                {"type": "tool_use", "id": "", "name": "x", "input": {}}
            ]),
        ] {
            let body = json!({
                "messages": [
                    {"role": "user", "content": "go"},
                    {"role": "assistant", "content": content},
                    {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": "", "content": "e"}
                    ]}
                ]
            });
            let raw = body.to_string();
            let out = normalize_messages_body(raw.as_bytes());
            let parsed: Value = serde_json::from_slice(&out).unwrap();
            let assistant = parsed["messages"][1]["content"].as_array().unwrap();
            assert!(
                assistant.iter().any(|b| b["type"] == "text" && b["text"].as_str().unwrap().contains("[bot-hq]")),
                "a placeholder text block, not an empty or thinking-only message: {parsed}"
            );
            assert!(assistant.iter().all(|b| b["type"] != "tool_use"));
        }
    }

    /// Healthy tool pairs are not touched, and a body that needs both fixes
    /// gets both: the hoist and the strip are independent passes over one
    /// parse, and the body is serialised once.
    #[test]
    fn normalize_keeps_healthy_tool_pairs_and_combines_with_the_hoist() {
        let body = json!({
            "messages": [
                {"role": "system", "content": "INJECTED"},
                {"role": "user", "content": "go"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "call_ok", "name": "Read", "input": {}},
                    {"type": "tool_use", "id": "", "name": "", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "call_ok", "content": "fine"},
                    {"type": "tool_result", "tool_use_id": "", "content": "bad"}
                ]}
            ]
        });
        let raw = body.to_string();
        let out = normalize_messages_body(raw.as_bytes());
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["system"], json!("INJECTED"), "hoisted");
        let msgs = parsed["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["content"].as_array().unwrap().len(), 1);
        assert_eq!(msgs[1]["content"][0]["id"], "call_ok");
        let user = msgs[2]["content"].as_array().unwrap();
        assert_eq!(user[0]["tool_use_id"], "call_ok", "the healthy result is untouched");
        assert_eq!(user[1]["type"], "text");

        // Control: a healthy pair alone comes back byte for byte.
        let healthy = json!({
            "messages": [
                {"role": "user", "content": "go"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "call_ok", "name": "Read", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "call_ok", "content": "fine"}
                ]}
            ]
        })
        .to_string();
        let out = normalize_messages_body(healthy.as_bytes());
        assert!(matches!(out, std::borrow::Cow::Borrowed(_)), "no copy when clean");
        assert_eq!(out.as_ref(), healthy.as_bytes());
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
