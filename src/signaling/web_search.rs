//! Webview-driven web search — a model-agnostic `web_search` MCP tool.
//!
//! The search runs INSIDE bot-hq's process, so it works for any agent model
//! (Rain on a third-party gateway included): a hidden Tauri webview is
//! navigated to a search engine, an injected `initialization_script` scrapes
//! the rendered results and calls the `web_search_callback` Tauri command,
//! and [`run_search`] awaits that round-trip over a oneshot channel.
//!
//! The make-or-break is whether the EXTERNAL engine page can reach Tauri IPC
//! (`window.__TAURI_INTERNALS__.invoke`). Tauri v2 allows it when the search
//! webview is matched by a capability with a `remote.urls` entry — see
//! `capabilities/search.json`. The injected `send()` is wrapped in try/catch,
//! so if IPC is unavailable the search just times out cleanly (no hang).
//!
//! Concurrency: a single permit serializes searches so the fixed webview
//! label `search` never collides; the awaiting side ALWAYS destroys the
//! webview (success, error, or timeout) so no hidden window is orphaned.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{oneshot, Semaphore};

/// Fixed label for the hidden search webview. One at a time (serialized by the
/// registry's permit), so the label never collides; matched by the
/// `search-webview` capability's `webviews` field.
const SEARCH_WEBVIEW_LABEL: &str = "search";

/// How long to wait for the injected extractor to fire the callback before
/// giving up and tearing the webview down.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);

/// One search result row, returned to the agent and parsed from the JS callback.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Managed Tauri state: serializes searches and correlates an in-flight search
/// `id` to the oneshot the `web_search_callback` command fires. Single-use per
/// id — whoever takes the sender first (the callback, or the timeout cleanup)
/// resolves the search.
pub struct SearchRegistry {
    next_id: AtomicU64,
    permit: Semaphore,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Vec<Hit>, String>>>>,
}

impl Default for SearchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchRegistry {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(0),
            permit: Semaphore::new(1),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn register(&self, tx: oneshot::Sender<Result<Vec<Hit>, String>>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.pending.lock().unwrap().insert(id, tx);
        id
    }

    /// Take the sender for `id` (single-use; oneshot is consumed on send).
    fn take(&self, id: u64) -> Option<oneshot::Sender<Result<Vec<Hit>, String>>> {
        self.pending.lock().unwrap().remove(&id)
    }
}

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

/// Build the search-engine results URL. Bing's HTML results render in a real
/// browser without an API key and is more scrape-tolerant than Google;
/// DuckDuckGo's index IS Bing's, so quality is comparable while avoiding DDG's
/// raw-HTTP CAPTCHA wall. Swappable — the engine + selectors are the part to
/// retune after the live U1/U4 validation.
pub fn engine_url(query: &str) -> String {
    format!("https://www.bing.com/search?q={}", encode_query(query))
}

/// Build the `initialization_script` injected into the search webview. It
/// polls for rendered results, detects CAPTCHA walls, scrapes hits, and calls
/// the `web_search_callback` Tauri command with `{ id, results }` or
/// `{ id, error }`. The `id` is baked in so the callback correlates to the
/// right oneshot. The invoke is try/caught: if IPC is unavailable the search
/// times out cleanly rather than throwing.
pub fn extractor_js(id: u64) -> String {
    format!(
        r#"(function() {{
  var ID = {id};
  function send(payload) {{
    try {{
      payload.id = ID;
      window.__TAURI_INTERNALS__.invoke('web_search_callback', payload);
    }} catch (e) {{ /* IPC unavailable — awaiting side will time out */ }}
  }}
  function isCaptcha() {{
    var t = (document.body && document.body.innerText || '').toLowerCase();
    return t.indexOf('captcha') !== -1
      || t.indexOf('are you a robot') !== -1
      || t.indexOf('unusual traffic') !== -1
      || t.indexOf('confirm this search was made by a human') !== -1;
  }}
  function extract() {{
    var hits = [];
    var nodes = document.querySelectorAll('li.b_algo');
    for (var i = 0; i < nodes.length; i++) {{
      var li = nodes[i];
      var a = li.querySelector('h2 a');
      var sn = li.querySelector('.b_caption p') || li.querySelector('.b_algoSlug') || li.querySelector('p');
      if (a && a.href) {{
        hits.push({{
          title: (a.innerText || '').trim(),
          url: a.href,
          snippet: sn ? (sn.innerText || '').trim() : ''
        }});
      }}
    }}
    return hits;
  }}
  var tries = 0;
  var timer = setInterval(function() {{
    tries++;
    if (isCaptcha()) {{ clearInterval(timer); send({{ error: 'captcha' }}); return; }}
    var hits = extract();
    if (hits.length > 0) {{ clearInterval(timer); send({{ results: hits }}); return; }}
    if (tries > 40) {{ clearInterval(timer); send({{ error: 'no results (timeout or selector mismatch)' }}); }}
  }}, 250);
}})();"#
    )
}

/// Run one web search: create a hidden webview at the engine results page,
/// inject the extractor, and await the callback. Serialized (one at a time),
/// time-bounded, and always tears the webview down.
pub async fn run_search(
    app: tauri::AppHandle,
    query: &str,
    num_results: Option<usize>,
) -> Result<Vec<Hit>, String> {
    use tauri::Manager;

    let registry = app.state::<SearchRegistry>();
    // Serialize: one search at a time so the fixed webview label can't collide.
    let _permit = registry
        .permit
        .acquire()
        .await
        .map_err(|_| "search registry closed".to_string())?;

    let (tx, rx) = oneshot::channel();
    let id = registry.register(tx);
    let url = engine_url(query);
    let js = extractor_js(id);

    // Webview creation must happen on the Tauri main thread.
    let app_for_build = app.clone();
    let schedule = app.run_on_main_thread(move || {
        let parsed = match tauri::Url::parse(&url) {
            Ok(u) => u,
            Err(_) => return, // awaiting side times out + cleans up
        };
        let _ = tauri::WebviewWindowBuilder::new(
            &app_for_build,
            SEARCH_WEBVIEW_LABEL,
            tauri::WebviewUrl::External(parsed),
        )
        .visible(false)
        .initialization_script(&js)
        .build();
    });
    if let Err(e) = schedule {
        registry.take(id);
        return Err(format!("failed to schedule search webview: {e}"));
    }

    let outcome = match tokio::time::timeout(SEARCH_TIMEOUT, rx).await {
        Ok(Ok(result)) => result,                       // callback fired
        Ok(Err(_)) => Err("search callback channel dropped".to_string()),
        Err(_) => {
            registry.take(id); // drop the stale sender
            Err("search timed out".to_string())
        }
    };

    // Always destroy the hidden webview (success, error, or timeout).
    let app_for_close = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app_for_close.get_webview_window(SEARCH_WEBVIEW_LABEL) {
            let _ = w.destroy();
        }
    });

    outcome.map(|mut hits| {
        if let Some(n) = num_results {
            hits.truncate(n);
        }
        hits
    })
}

/// Tauri command invoked BY the search webview's injected script to deliver
/// results (or an error) back to the awaiting [`run_search`]. Fire-and-forget
/// from JS; resolves the oneshot for the given `id`.
#[tauri::command]
#[specta::specta]
pub fn web_search_callback(
    registry: tauri::State<'_, SearchRegistry>,
    id: u64,
    results: Option<Vec<Hit>>,
    error: Option<String>,
) {
    if let Some(tx) = registry.take(id) {
        let outcome = match (results, error) {
            (Some(hits), _) => Ok(hits),
            (None, Some(e)) => Err(e),
            (None, None) => Err("empty search callback".to_string()),
        };
        let _ = tx.send(outcome);
    }
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
    fn engine_url_targets_engine_with_encoded_query() {
        let u = engine_url("hello world");
        assert!(u.starts_with("https://www.bing.com/search?q="));
        assert!(u.contains("hello%20world"));
    }

    #[test]
    fn extractor_js_bakes_id_and_calls_callback() {
        let js = extractor_js(7);
        assert!(js.contains("var ID = 7;"));
        assert!(js.contains("web_search_callback"));
        assert!(js.contains("__TAURI_INTERNALS__"));
        assert!(js.contains("li.b_algo")); // result selector
        assert!(js.contains("captcha")); // CAPTCHA detection
    }

    #[test]
    fn registry_register_then_take_roundtrips() {
        let reg = SearchRegistry::new();
        let (tx, _rx) = oneshot::channel();
        let id = reg.register(tx);
        assert!(reg.take(id).is_some());
        assert!(reg.take(id).is_none()); // single-use
    }
}
