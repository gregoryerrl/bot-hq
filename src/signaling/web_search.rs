//! Webview-driven web search — a model-agnostic `web_search` MCP tool.
//!
//! Runs INSIDE bot-hq's process, so it works for any agent model (Rain on a
//! third-party gateway included): a hidden Tauri webview navigates a search
//! engine, and Rust polls the rendered DOM via `Webview::eval_with_callback`
//! (a RUST-initiated eval whose JSON result returns through a callback).
//!
//! Why eval, not `invoke`: Tauri v2 structurally blocks a remote-origin page
//! from invoking app `#[tauri::command]`s (the ACL permission schema only
//! knows `core:*`/plugin permissions). `eval_with_callback` inverts the
//! direction (Rust → webview → callback → Rust), bypassing the IPC ACL — no
//! capability, no command, no initialization script. Reading the
//! server-rendered DOM directly also dodges hidden-webview timer throttling.
//!
//! Engine FALLBACK CASCADE: engines are tried in priority order (Google →
//! Bing); the first to return non-empty results wins. A CAPTCHA / empty page /
//! timeout on one engine falls through to the next, so a single engine
//! CAPTCHA-ing mid-task doesn't break search. Bing is the proven anchor.
//!
//! Serialized one-at-a-time (a process-global permit) so the fixed webview
//! label `search` never collides; each engine attempt ALWAYS destroys the
//! webview before the next.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Semaphore};

/// Fixed label for the hidden search webview (serialized, so it never collides).
const SEARCH_WEBVIEW_LABEL: &str = "search";

/// Per-engine wall-clock budget (page load + render + scrape) before falling
/// through to the next engine.
const PER_ENGINE_DEADLINE: Duration = Duration::from_secs(8);

/// Delay between DOM-scrape attempts while a page is still rendering.
const POLL_INTERVAL: Duration = Duration::from_millis(700);

/// Per-eval callback wait (a single `eval_with_callback` round-trip).
const EVAL_TIMEOUT: Duration = Duration::from_secs(3);

/// Settle time after destroying one engine's webview before building the next
/// (the label is reused; avoids a duplicate-label build race).
const TEARDOWN_SETTLE: Duration = Duration::from_millis(300);

/// Serializes searches to one at a time (the webview label is fixed).
static SEARCH_PERMIT: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(1));

/// Shared CAPTCHA / bot-wall detector JS (returns `{error:'captcha'}`).
const CAPTCHA_JS: &str = "var __t=(document.body&&document.body.innerText||'').toLowerCase(); \
if(__t.indexOf('captcha')!==-1||__t.indexOf('are you a robot')!==-1||__t.indexOf('unusual traffic')!==-1\
||__t.indexOf('confirm this search was made by a human')!==-1||__t.indexOf('detected unusual')!==-1) \
return {error:'captcha'};";

/// One search result row, returned to the agent.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Shape the extraction JS evaluates to (Tauri serializes the eval result to a
/// JSON string; we parse it back into this).
#[derive(Debug, Deserialize)]
struct ExtractResult {
    #[serde(default)]
    results: Option<Vec<Hit>>,
    #[serde(default)]
    error: Option<String>,
}

/// A search engine in the fallback cascade: a results-URL builder + the JS that
/// scrapes its results page into `{results:[...]}` or `{error:'captcha'}`.
struct Engine {
    name: &'static str,
    url: fn(&str) -> String,
    extract: fn() -> String,
}

/// Priority order. Google first (best results), Bing as the proven anchor.
/// DuckDuckGo deliberately excluded. Reorder / add engines here.
const ENGINES: &[Engine] = &[
    Engine {
        name: "google",
        url: google_url,
        extract: google_extract,
    },
    Engine {
        name: "bing",
        url: bing_url,
        extract: bing_extract,
    },
];

/// Percent-encode a query per RFC 3986 unreserved set (ALPHA / DIGIT / -._~);
/// everything else becomes %XX. Avoids pulling in a urlencoding crate.
fn encode_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len() * 3);
    for b in q.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn google_url(q: &str) -> String {
    format!("https://www.google.com/search?q={}&hl=en", encode_query(q))
}

fn bing_url(q: &str) -> String {
    format!("https://www.bing.com/search?q={}", encode_query(q))
}

/// Google extraction (best-guess selectors — Google's DOM is obfuscated and
/// shifts; tune from live results). Walks result `h3`s up to their anchor and
/// decodes `/url?q=` redirects. Falls back to the Bing engine if empty.
fn google_extract() -> String {
    format!(
        r#"(function() {{
  {CAPTCHA_JS}
  function gUrl(href) {{
    try {{
      if (href.indexOf('/url?') !== -1) {{
        var m = href.match(/[?&](?:q|url)=([^&]+)/);
        if (m) return decodeURIComponent(m[1]);
      }}
    }} catch (e) {{}}
    return href;
  }}
  var hits = [], seen = {{}};
  var h3s = document.querySelectorAll('#search h3, #rso h3, div.g h3');
  for (var i = 0; i < h3s.length; i++) {{
    var h3 = h3s[i];
    var a = h3.closest ? h3.closest('a') : null;
    if (!a) {{ var p = h3.parentElement; while (p && p.tagName !== 'A') p = p.parentElement; a = p; }}
    if (a && a.href) {{
      var u = gUrl(a.href);
      if (u.indexOf('http') !== 0 || seen[u]) continue;
      seen[u] = 1;
      var cont = h3.closest ? h3.closest('div.g, div[data-hveid], div[data-sokoban-container]') : null;
      var sn = cont ? cont.querySelector('div[data-sncf], .VwiC3b, span.aCOpRe') : null;
      hits.push({{ title: (h3.innerText || '').trim(), url: u, snippet: sn ? (sn.innerText || '').trim() : '' }});
    }}
  }}
  return {{ results: hits }};
}})()"#
    )
}

/// Bing extraction (proven). Decodes Bing's `bing.com/ck/a?...&u=a1<base64url>`
/// click-redirects into real destination URLs (falls back to the raw href).
fn bing_extract() -> String {
    format!(
        r#"(function() {{
  {CAPTCHA_JS}
  function realUrl(href) {{
    try {{
      var m = href.match(/[?&]u=([^&]+)/);
      if (href.indexOf('bing.com/ck/a') !== -1 && m) {{
        var enc = decodeURIComponent(m[1]);
        if (enc.slice(0, 2) === 'a1') enc = enc.slice(2);
        var b64 = enc.replace(/-/g, '+').replace(/_/g, '/');
        while (b64.length % 4) b64 += '=';
        return atob(b64);
      }}
    }} catch (e) {{}}
    return href;
  }}
  var hits = [];
  var nodes = document.querySelectorAll('li.b_algo');
  for (var i = 0; i < nodes.length; i++) {{
    var li = nodes[i];
    var a = li.querySelector('h2 a');
    var sn = li.querySelector('.b_caption p') || li.querySelector('.b_algoSlug') || li.querySelector('p');
    if (a && a.href) {{
      hits.push({{ title: (a.innerText || '').trim(), url: realUrl(a.href), snippet: sn ? (sn.innerText || '').trim() : '' }});
    }}
  }}
  return {{ results: hits }};
}})()"#
    )
}

/// Run a single `eval_with_callback` round-trip on the search webview, awaiting
/// the JSON result (or `None` on timeout / missing webview).
async fn eval_once(app: &tauri::AppHandle, js: String) -> Option<String> {
    use tauri::Manager;
    let (tx, rx) = oneshot::channel::<String>();
    let slot = Arc::new(Mutex::new(Some(tx)));
    let app_e = app.clone();
    let slot_cb = Arc::clone(&slot);
    let scheduled = app.run_on_main_thread(move || {
        match app_e.get_webview_window(SEARCH_WEBVIEW_LABEL) {
            Some(wv) => {
                let slot_inner = Arc::clone(&slot_cb);
                let r = wv.eval_with_callback(js, move |json| {
                    if let Some(tx) = slot_inner.lock().unwrap().take() {
                        let _ = tx.send(json);
                    }
                });
                if r.is_err() {
                    if let Some(tx) = slot_cb.lock().unwrap().take() {
                        let _ = tx.send(String::new());
                    }
                }
            }
            None => {
                if let Some(tx) = slot_cb.lock().unwrap().take() {
                    let _ = tx.send(String::new());
                }
            }
        }
    });
    if scheduled.is_err() {
        return None;
    }
    match tokio::time::timeout(EVAL_TIMEOUT, rx).await {
        Ok(Ok(json)) => Some(json),
        _ => None,
    }
}

/// Try one engine: build a hidden webview at its results URL, poll the DOM via
/// `eval_with_callback` until results appear / CAPTCHA / deadline, then ALWAYS
/// destroy the webview. `Ok(hits)` (possibly empty), or `Err` for CAPTCHA/timeout.
async fn try_engine(app: &tauri::AppHandle, url: String, js: String) -> Result<Vec<Hit>, String> {
    use tauri::Manager;

    let app_for_build = app.clone();
    let url_for_log = url.clone();
    let scheduled = app.run_on_main_thread(move || {
        let parsed = match tauri::Url::parse(&url) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "web_search: bad engine URL");
                return;
            }
        };
        match tauri::WebviewWindowBuilder::new(
            &app_for_build,
            SEARCH_WEBVIEW_LABEL,
            tauri::WebviewUrl::External(parsed),
        )
        .visible(false)
        .build()
        {
            Ok(_) => tracing::info!(url = %url_for_log, "web_search: hidden webview built"),
            Err(e) => tracing::warn!(error = %e, "web_search: webview build FAILED"),
        }
    });
    if let Err(e) = scheduled {
        return Err(format!("failed to schedule webview: {e}"));
    }

    let deadline = Instant::now() + PER_ENGINE_DEADLINE;
    let outcome: Result<Vec<Hit>, String> = loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        if let Some(json) = eval_once(app, js.clone()).await {
            if let Ok(parsed) = serde_json::from_str::<ExtractResult>(&json) {
                if let Some(e) = parsed.error {
                    break Err(e);
                }
                if let Some(hits) = parsed.results {
                    if !hits.is_empty() {
                        break Ok(hits);
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            break Err("timeout".to_string());
        }
    };

    // Always destroy the hidden webview, then let it settle before the next engine.
    let app_for_close = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app_for_close.get_webview_window(SEARCH_WEBVIEW_LABEL) {
            let _ = w.destroy();
        }
    });
    tokio::time::sleep(TEARDOWN_SETTLE).await;

    outcome
}

/// Run a web search across the engine fallback cascade. Returns the first
/// engine's non-empty results; if all engines CAPTCHA / time out / return
/// nothing, returns the last failure.
pub async fn run_search(
    app: tauri::AppHandle,
    query: &str,
    num_results: Option<usize>,
) -> Result<Vec<Hit>, String> {
    // Serialize: one search (one "search" webview) at a time.
    let _permit = SEARCH_PERMIT
        .acquire()
        .await
        .map_err(|_| "search permit closed".to_string())?;

    let mut last = "no engine returned results".to_string();
    for engine in ENGINES {
        match try_engine(&app, (engine.url)(query), (engine.extract)()).await {
            Ok(hits) if !hits.is_empty() => {
                tracing::info!(engine = engine.name, count = hits.len(), "web_search: results");
                let mut hits = hits;
                if let Some(n) = num_results {
                    hits.truncate(n);
                }
                return Ok(hits);
            }
            Ok(_) => {
                last = format!("{}: no results", engine.name);
                tracing::info!(engine = engine.name, "web_search: empty, trying next engine");
            }
            Err(e) => {
                last = format!("{}: {e}", engine.name);
                tracing::info!(engine = engine.name, error = %e, "web_search: failed, trying next engine");
            }
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_query_percent_encodes_reserved() {
        assert_eq!(encode_query("rust programming"), "rust%20programming");
        assert_eq!(encode_query("a+b&c"), "a%2Bb%26c");
        assert_eq!(encode_query("plain-text_1.0~"), "plain-text_1.0~");
    }

    #[test]
    fn engine_urls_encode_query() {
        assert!(google_url("hello world").starts_with("https://www.google.com/search?q=hello%20world"));
        assert!(bing_url("hello world").starts_with("https://www.bing.com/search?q=hello%20world"));
    }

    #[test]
    fn cascade_order_is_google_then_bing() {
        let names: Vec<&str> = ENGINES.iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["google", "bing"]);
    }

    #[test]
    fn extraction_js_is_sync_no_ipc() {
        for js in [google_extract(), bing_extract()] {
            assert!(js.contains("captcha")); // shared CAPTCHA detection
            assert!(js.contains("results:")); // returns {results:[...]}
            // No invoke / IPC — extraction reads the DOM, evaluated from Rust.
            assert!(!js.contains("__TAURI_INTERNALS__"));
            assert!(!js.contains("invoke"));
        }
        assert!(bing_extract().contains("li.b_algo")); // Bing selector
        assert!(google_extract().contains("h3")); // Google selector
    }

    #[test]
    fn extract_result_parses_results_and_error() {
        let r: ExtractResult =
            serde_json::from_str(r#"{"results":[{"title":"t","url":"u","snippet":"s"}]}"#).unwrap();
        assert_eq!(r.results.unwrap().len(), 1);
        let e: ExtractResult = serde_json::from_str(r#"{"error":"captcha"}"#).unwrap();
        assert_eq!(e.error.unwrap(), "captcha");
    }
}
