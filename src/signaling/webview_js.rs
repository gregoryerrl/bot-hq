//! The `webview_*` MCP tools: composition of the shared JavaScript snippets,
//! the result-returning evaluation on the bot-hq window, and the lease that
//! serializes mutating callers (feedback #109, Plan Batch 6 B).
//!
//! **The snippets are files** (`src/signaling/webview/*.js`), included here
//! byte for byte and executed byte-identical by the frontend's jsdom fixture
//! (`frontend/src/lib/webviewSnippets.test.ts`) through the same one-line
//! composition — so the guards an agent relies on are proven on a real DOM, not
//! on substrings of a Rust string. Each action snippet is an arrow function
//! `(args) => result` that validates EVERYTHING (lease generation and deadline,
//! route, session, selector) before any effect and returns a structured
//! outcome; `guard.js` is bound as `GUARD` beside it.
//!
//! **Every call answers truthfully.** `window.eval` was fire-and-forget: a
//! tool reported `{ok: true}` whether or not a selector matched, whether or not
//! the window showed the session the caller meant — the collisions #109
//! describes. Now the snippet's own result comes back through
//! `eval_with_callback`, and a callback that does not arrive in time is
//! reported as `outcome: "unknown"`: the script was queued and may still run.
//!
//! **The lease** is optional and advisory for readers, exclusive for
//! mutators: while one caller holds it, every other caller's mutating tool is
//! refused before any script is queued (`leased`). A holder is an authenticated
//! incarnation — (session, agent, the HTTP token proof `token_proof_is_current`
//! accepts) — so a replaced runtime cannot act on its predecessor's lease. Each
//! acquire, renew, release or takeover bumps a generation that the host mirrors
//! into the window (`mirror.js`, monotonic), and every admitted mutator carries
//! the generation it was admitted under plus, for a holder, its deadline: the
//! snippet refuses `lease_lost` / `lease_expired` AT ACTUATION, so an action
//! that sat in the main-thread queue past a takeover or past its own deadline
//! never lands (eyes-2, 2026-10-27T03:40Z). The mirror update is queued under
//! the lease lock and `webview_lease` returns only after the window acknowledged
//! it — FIFO on the main thread means every action queued before the transfer
//! has run by then — or says `established: false` when the window did not
//! answer in time.
use serde_json::{json, Value};
#[cfg(test)]
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::protocol::JsonRpcError;

const GUARD_JS: &str = include_str!("webview/guard.js");
const CONTEXT_JS: &str = include_str!("webview/context.js");
const MIRROR_JS: &str = include_str!("webview/mirror.js");
const CLICK_JS: &str = include_str!("webview/click.js");
const TYPE_JS: &str = include_str!("webview/type.js");
const SCROLL_JS: &str = include_str!("webview/scroll.js");
const PRESS_KEY_JS: &str = include_str!("webview/press_key.js");

/// One `eval_with_callback` round trip on the main window.
pub(crate) const EVAL_TIMEOUT: Duration = Duration::from_secs(3);
/// The longest a lease may be asked for; a holder renews by asking again.
pub(crate) const LEASE_MAX_TTL: Duration = Duration::from_secs(60);

/// The composition the jsdom fixture reproduces verbatim: `guard.js` bound as
/// `GUARD`, the action applied to its JSON arguments, and any exception turned
/// into an `unknown` outcome — the action may have acted before it threw, and
/// on Windows WebView2 an uncaught exception would come back as a bare JSON
/// `null` (ExecuteScript's contract), which is no outcome at all.
pub(crate) fn compose(action: &str, args: &Value) -> String {
    format!("(() => {{ try {{ const GUARD = {GUARD_JS}; return ({action})({args}); }} catch (e) {{ return {{ outcome: \"unknown\", ok: false, matched: null, reason: \"exception\", detail: String(e && e.message || e) }}; }} }})()")
}

/// Every reply an agent reads has the one shape: a callback the window answered
/// with something else — `null` (an uncaught exception on WebView2), a bare
/// value, an object without an `outcome` — is reported as `unknown`, never
/// forwarded as if it were an outcome.
pub(crate) fn normalize_result(value: Value) -> Value {
    let outcome = value.get("outcome").and_then(Value::as_str);
    if matches!(outcome, Some("done" | "refused" | "unknown")) {
        return value;
    }
    let raw = value.to_string();
    json!({"outcome": "unknown", "ok": false, "matched": null, "reason": "malformed_result",
        "detail": format!("the window answered with {} instead of an outcome; the action may have run", &raw[..crate::text::floor_char_boundary(&raw, 200)])})
}

/// The snippet for a mutating tool, or `None` when `name` is not one.
fn action_source(name: &str) -> Option<&'static str> {
    Some(match name {
        "webview_click" => CLICK_JS,
        "webview_type" => TYPE_JS,
        "webview_scroll" => SCROLL_JS,
        "webview_press_key" => PRESS_KEY_JS,
        _ => return None,
    })
}

pub(crate) fn is_mutating_tool(name: &str) -> bool {
    action_source(name).is_some()
}

/// The arguments a mutating snippet receives: the caller's, validated, plus
/// the lease stamp the host admitted it under.
pub(crate) fn action_args(name: &str, args: &Value, lease: &LeaseStamp) -> Result<Value, JsonRpcError> {
    use super::tool_args::arg_required_str;
    let opt_str = |key: &str| -> Result<Option<String>, JsonRpcError> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(JsonRpcError::new(JsonRpcError::INVALID_PARAMS, format!("{key} must be a string"))),
        }
    };
    let mut out = json!({
        "expected_route": opt_str("expected_route")?,
        "expected_session_id": opt_str("expected_session_id")?,
        "lease": {"gen": lease.generation, "until_ms": lease.until_ms},
    });
    match name {
        "webview_click" => {
            out["selector"] = Value::String(arg_required_str(args, "selector")?);
        }
        "webview_type" => {
            out["selector"] = Value::String(arg_required_str(args, "selector")?);
            out["text"] = Value::String(arg_required_str(args, "text")?);
        }
        "webview_scroll" => {
            let y = args.get("y").and_then(Value::as_i64).ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "y is required".to_string()))?;
            out["y"] = json!(y);
            if let Some(selector) = opt_str("selector")? {
                out["selector"] = Value::String(selector);
            }
        }
        "webview_press_key" => {
            out["key"] = Value::String(arg_required_str(args, "key")?);
            if let Some(selector) = opt_str("selector")? {
                out["selector"] = Value::String(selector);
            }
        }
        other => return Err(JsonRpcError::new(JsonRpcError::METHOD_NOT_FOUND, format!("unknown tool {other}"))),
    }
    Ok(out)
}

/// The JavaScript of a mutating tool call — a `METHOD_NOT_FOUND` error for any
/// other name, an `INVALID_PARAMS` error for missing arguments.
pub(crate) fn action_js(name: &str, args: &Value, lease: &LeaseStamp) -> Result<String, JsonRpcError> {
    let source = action_source(name).ok_or_else(|| JsonRpcError::new(JsonRpcError::METHOD_NOT_FOUND, format!("unknown tool {name}")))?;
    Ok(compose(source, &action_args(name, args, lease)?))
}

pub(crate) fn context_js() -> String {
    compose(CONTEXT_JS, &json!({}))
}

pub(crate) fn mirror_js(generation: u64) -> String {
    compose(MIRROR_JS, &json!({"gen": generation}))
}

/// What a snippet's evaluation came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvalOutcome {
    /// The snippet ran and returned this JSON.
    Returned(Value),
    /// No callback within [`EVAL_TIMEOUT`]: the script was queued and may
    /// still run. Never read as "did not happen".
    Timeout,
    /// The main window does not exist (or no app handle: a test bridge).
    NoWebview,
}

impl EvalOutcome {
    /// The tool's reply: the snippet's own result, or the host's truthful
    /// substitute for the two cases where there is none.
    pub(crate) fn into_reply(self, context: &str) -> Value {
        match self {
            Self::Returned(value) => value,
            Self::Timeout => json!({"outcome": "unknown", "ok": false, "matched": null, "reason": "timeout",
                "detail": format!("the window did not answer within {}s; the {context} was queued and may still run — read the state (webview_screenshot) before repeating it", EVAL_TIMEOUT.as_secs())}),
            Self::NoWebview => json!({"outcome": "refused", "ok": false, "matched": false, "reason": "no_webview",
                "detail": "the bot-hq main window is not available"}),
        }
    }
}

/// A test seam: an evaluator standing in for the main window.
#[cfg(test)]
pub(crate) type EvalOverride = Arc<dyn Fn(String) -> EvalOutcome + Send + Sync>;

/// The lease generation and deadline a mutator was admitted under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LeaseStamp {
    pub generation: u64,
    /// The holder's deadline as Unix epoch milliseconds — the wall clock the
    /// window compares with `Date.now()` — or `None` for a caller admitted
    /// because no lease was held.
    pub until_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct LeaseHolder {
    pub session_id: String,
    pub agent: String,
    pub proof: super::bridge::McpTokenProof,
    pub until: Instant,
    pub until_ms: u64,
    pub generation: u64,
}

#[derive(Debug, Default)]
pub(crate) struct LeaseState {
    pub generation: u64,
    pub holder: Option<LeaseHolder>,
}

pub(crate) fn epoch_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_composition_is_the_one_the_jsdom_fixture_reproduces() {
        let js = compose("((args) => args)", &json!({"a": 1}));
        assert_eq!(js, format!("(() => {{ try {{ const GUARD = {GUARD_JS}; return (((args) => args))({{\"a\":1}}); }} catch (e) {{ return {{ outcome: \"unknown\", ok: false, matched: null, reason: \"exception\", detail: String(e && e.message || e) }}; }} }})()"));
        // The files open with line comments; a comment after `=` ends at its
        // line and the arrow function follows — the fixture evaluates exactly
        // this string, which is what makes the two environments one proof.
        assert!(GUARD_JS.trim_start().starts_with("//") && GUARD_JS.contains("\n((args) =>"), "{GUARD_JS}");
    }

    #[test]
    fn every_mutating_tool_gets_its_snippet_and_the_lease_stamp() {
        let stamp = LeaseStamp { generation: 4, until_ms: Some(1_700_000_000_000) };
        let js = action_js("webview_click", &json!({"selector": "#b", "expected_route": "/sessions/s1"}), &stamp).unwrap();
        assert!(js.contains(CLICK_JS) && js.contains("\"expected_route\":\"/sessions/s1\"") && js.contains("\"lease\":{\"gen\":4,\"until_ms\":1700000000000}"), "{js}");
        let js = action_js("webview_type", &json!({"selector": "#t", "text": "x\"y"}), &LeaseStamp { generation: 0, until_ms: None }).unwrap();
        assert!(js.contains(TYPE_JS) && js.contains("\"text\":\"x\\\"y\"") && js.contains("\"until_ms\":null"), "{js}");
        let js = action_js("webview_scroll", &json!({"y": 12}), &stamp).unwrap();
        assert!(js.contains(SCROLL_JS) && js.contains("\"y\":12") && !js.contains("\"selector\""), "{js}");
        let js = action_js("webview_press_key", &json!({"key": "Enter", "selector": "#c"}), &stamp).unwrap();
        assert!(js.contains(PRESS_KEY_JS) && js.contains("\"key\":\"Enter\"") && js.contains("\"selector\":\"#c\""), "{js}");
        assert_eq!(action_js("webview_screenshot", &json!({}), &stamp).unwrap_err().code, JsonRpcError::METHOD_NOT_FOUND);
        assert_eq!(action_js("webview_click", &json!({}), &stamp).unwrap_err().code, JsonRpcError::INVALID_PARAMS);
        assert_eq!(action_js("webview_click", &json!({"selector": "#b", "expected_route": 5}), &stamp).unwrap_err().code, JsonRpcError::INVALID_PARAMS);
        assert!(["webview_click", "webview_type", "webview_scroll", "webview_press_key"].iter().all(|t| is_mutating_tool(t)));
        assert!(!is_mutating_tool("webview_screenshot") && !is_mutating_tool("webview_lease"));
    }

    #[test]
    fn a_missing_callback_is_unknown_never_done() {
        let reply = EvalOutcome::Timeout.into_reply("click");
        assert_eq!((reply["outcome"].as_str(), reply["ok"].as_bool(), reply["reason"].as_str()), (Some("unknown"), Some(false), Some("timeout")));
        assert!(reply["matched"].is_null());
        assert!(reply["detail"].as_str().unwrap().contains("may still run"));
        let reply = EvalOutcome::NoWebview.into_reply("click");
        assert_eq!((reply["outcome"].as_str(), reply["reason"].as_str()), (Some("refused"), Some("no_webview")));
        assert_eq!(EvalOutcome::Returned(json!({"outcome": "done"})).into_reply("click"), json!({"outcome": "done"}));
    }

    #[test]
    fn a_reply_without_an_outcome_is_unknown() {
        for raw in [Value::Null, json!(true), json!("ok"), json!({"foo": 1}), json!({"outcome": "maybe"})] {
            let reply = normalize_result(raw.clone());
            assert_eq!((reply["outcome"].as_str(), reply["reason"].as_str()), (Some("unknown"), Some("malformed_result")), "{raw}");
            assert!(reply["detail"].as_str().unwrap().contains(&raw.to_string()));
        }
        for ok in ["done", "refused", "unknown"] {
            assert_eq!(normalize_result(json!({"outcome": ok, "x": 1})), json!({"outcome": ok, "x": 1}));
        }
    }

    #[test]
    fn the_snippets_declare_their_shape() {
        // Each file is one arrow function taking `args`; the actions call GUARD
        // first and never touch the DOM before it answered.
        for (name, src) in [("guard", GUARD_JS), ("context", CONTEXT_JS), ("mirror", MIRROR_JS), ("click", CLICK_JS), ("type", TYPE_JS), ("scroll", SCROLL_JS), ("press_key", PRESS_KEY_JS)] {
            assert!(src.trim_end().ends_with("})"), "{name}.js must be one arrow function");
            assert!(src.contains("((args) =>"), "{name}.js takes `args`");
        }
        for (name, src) in [("click", CLICK_JS), ("type", TYPE_JS), ("scroll", SCROLL_JS), ("press_key", PRESS_KEY_JS)] {
            // Code only: the files open with comments that name the effects.
            let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
            let guard_at = code.find("GUARD(args)").unwrap_or_else(|| panic!("{name}.js calls GUARD"));
            let first_effect = ["click()", "dispatchEvent", "scrollTop =", "scrollTo(", ".focus()", "desc.set.call"].iter().filter_map(|m| code.find(m)).min();
            assert!(first_effect.is_none_or(|at| at > guard_at), "{name}.js acts before its guard");
        }
        assert!(!PRESS_KEY_JS.contains("|| document.activeElement || document)") || PRESS_KEY_JS.contains("g.el || document.activeElement"), "a missed selector never falls back to the active element");
        assert!(MIRROR_JS.contains("args.gen > current"), "the mirror is monotonic");
        assert!(GUARD_JS.contains("lease_expired") && GUARD_JS.contains("Date.now() > lease.until_ms"), "expiry is checked at actuation");
    }
}
