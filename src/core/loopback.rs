//! What a loopback server may assume about who is calling it — and what it
//! may not.
//!
//! bot-hq's two HTTP listeners (the MCP signaling server and the LLM
//! normalizing proxy) bind `127.0.0.1` on an ephemeral port. "Loopback" is a
//! statement about the network path, not about the caller: every process on
//! the machine can reach the port, and so can **a web page in the user's
//! browser** — a cross-origin `fetch()` to `http://127.0.0.1:<port>` with
//! `mode: "no-cors"` is sent without a preflight, and although the page cannot
//! read the reply, the request's side effect lands. That is the classic
//! DNS-rebinding / localhost-CSRF shape (2026-09-06 sweep, S1 + S3).
//!
//! The two servers' legitimate clients — the claude-code MCP client, the
//! claude-code process talking to a proxied gateway, and the git/PreToolUse
//! hook subprocesses — are not browsers and never send the browser-only
//! request headers. A browser always does: `Origin` on every cross-origin
//! POST (`no-cors` included), and the `Sec-Fetch-*` metadata headers on every
//! request, which page script cannot suppress or forge. So the presence of
//! either is a reliable "this came from a browser" signal, and refusing on it
//! closes the vector without a shared secret the browser could not present
//! anyway.
//!
//! Observed, not assumed (2026-09-06, claude-code 2.1.251 driven at a
//! header-logging stub through `--mcp-config`): its MCP client sends exactly
//! `Accept`, `Accept-Encoding`, `Content-Type: application/json`,
//! `User-Agent: claude-code/<v> (sdk-cli)`, `mcp-protocol-version` (and
//! `mcp-method` on the discover probe), `Connection`, `Host`,
//! `Content-Length` — no `Origin`, no `Sec-Fetch-*` — plus one `GET` with
//! `Accept: text/event-stream` for the SSE channel, which the server answers
//! 405 as before. Re-run that probe before tightening anything here.

use hyper::header::HeaderMap;

/// True when the request carries a header only a browser sends. Callers
/// refuse such requests outright — see the module doc for why this is both
/// sufficient and safe for bot-hq's non-browser clients.
pub fn browser_originated(headers: &HeaderMap) -> bool {
    headers.contains_key("origin")
        || headers
            .keys()
            .any(|k| k.as_str().starts_with("sec-fetch-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(HeaderName::from_static(k), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn non_browser_clients_pass() {
        assert!(!browser_originated(&headers(&[])));
        assert!(!browser_originated(&headers(&[
            ("content-type", "application/json"),
            ("user-agent", "claude-code/2.1.251"),
            ("x-bot-hq-hook-token", "abc"),
        ])));
    }

    #[test]
    fn origin_or_fetch_metadata_marks_a_browser() {
        assert!(browser_originated(&headers(&[("origin", "http://evil.example")])));
        // `Origin: null` (a sandboxed frame / a data: URL page) is still a browser.
        assert!(browser_originated(&headers(&[("origin", "null")])));
        assert!(browser_originated(&headers(&[("sec-fetch-mode", "no-cors")])));
        assert!(browser_originated(&headers(&[("sec-fetch-site", "cross-site")])));
    }
}
