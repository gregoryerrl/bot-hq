//! Shared JavaScript snippet builders for the `webview_*` MCP tools. The
//! internal (jsonrpc.rs) and external (external_jsonrpc.rs) dispatch surfaces
//! build byte-identical JS; only their `eval_in_webview` wrapper (handle
//! source) differs. These builders are the single source of truth for the
//! snippets. Selectors / text / keys are JSON-encoded so user input can't
//! break out of the string literal.

/// Click the first element matching `selector`.
pub(super) fn click_js(selector: &str) -> String {
    let sel = serde_json::to_string(selector).unwrap();
    format!(
        "(() => {{ const el = document.querySelector({sel}); \
         if (el) {{ el.click(); }} \
         else {{ console.warn('webview_click: no element matches', {sel}); }} }})();"
    )
}

/// Set `text` on the first element matching `selector`, routing through the
/// native value setter so React's onChange fires (plain `el.value = …`
/// bypasses React).
pub(super) fn type_js(selector: &str, text: &str) -> String {
    let sel = serde_json::to_string(selector).unwrap();
    let txt = serde_json::to_string(text).unwrap();
    format!(
        "(() => {{ \
           const el = document.querySelector({sel}); \
           if (!el) {{ console.warn('webview_type: no element matches', {sel}); return; }} \
           el.focus(); \
           const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; \
           const desc = Object.getOwnPropertyDescriptor(proto, 'value'); \
           if (desc && desc.set) {{ desc.set.call(el, {txt}); }} else {{ el.value = {txt}; }} \
           el.dispatchEvent(new Event('input', {{ bubbles: true }})); \
           el.dispatchEvent(new Event('change', {{ bubbles: true }})); \
         }})();"
    )
}

/// Scroll an element (`selector`) or the window to vertical position `y`.
pub(super) fn scroll_js(selector: Option<&str>, y: i64) -> String {
    match selector {
        Some(sel) => {
            let sel_json = serde_json::to_string(sel).unwrap();
            format!(
                "(() => {{ const el = document.querySelector({sel_json}); \
                 if (el) {{ el.scrollTop = {y}; }} \
                 else {{ console.warn('webview_scroll: no element matches', {sel_json}); }} }})();"
            )
        }
        None => format!("window.scrollTo({{ top: {y}, behavior: 'auto' }});"),
    }
}

/// Dispatch keydown/keypress/keyup for `key` at `selector` (or the active
/// element / document when no selector is given).
pub(super) fn press_key_js(selector: Option<&str>, key: &str) -> String {
    let key_json = serde_json::to_string(key).unwrap();
    let target_expr = match selector {
        Some(sel) => {
            let sel_json = serde_json::to_string(sel).unwrap();
            format!("(document.querySelector({sel_json}) || document.activeElement || document)")
        }
        None => "(document.activeElement || document)".to_string(),
    };
    format!(
        "(() => {{ const target = {target_expr}; \
         for (const type of ['keydown', 'keypress', 'keyup']) {{ \
           target.dispatchEvent(new KeyboardEvent(type, {{ key: {key_json}, bubbles: true, cancelable: true }})); \
         }} }})();"
    )
}

/// Build the JS for a `webview_*` automation tool from its call args, or
/// `Ok(None)` when `name` isn't a webview tool. Lets the internal + external
/// MCP dispatchers share the arg-parsing + snippet selection; each still
/// supplies its own `eval_in_webview` (the AppHandle source differs).
pub(super) fn webview_tool_js(
    name: &str,
    args: &serde_json::Value,
) -> Result<Option<String>, super::protocol::JsonRpcError> {
    use super::protocol::JsonRpcError;
    use super::tool_args::arg_required_str;
    let js = match name {
        "webview_click" => click_js(&arg_required_str(args, "selector")?),
        "webview_type" => type_js(
            &arg_required_str(args, "selector")?,
            &arg_required_str(args, "text")?,
        ),
        "webview_scroll" => {
            let y = args
                .get("y")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "y is required".to_string())
                })?;
            scroll_js(args.get("selector").and_then(serde_json::Value::as_str), y)
        }
        "webview_press_key" => press_key_js(
            args.get("selector").and_then(serde_json::Value::as_str),
            &arg_required_str(args, "key")?,
        ),
        _ => return Ok(None),
    };
    Ok(Some(js))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_js_json_encodes_selector() {
        let js = click_js("[data-id=\"x\"]");
        assert!(js.contains("document.querySelector(\"[data-id=\\\"x\\\"]\")"));
        assert!(js.contains("el.click()"));
    }

    #[test]
    fn scroll_js_window_vs_element() {
        assert!(scroll_js(None, 0).starts_with("window.scrollTo"));
        assert!(scroll_js(Some(".pane"), 100).contains("el.scrollTop = 100"));
    }

    #[test]
    fn press_key_js_defaults_to_active_element() {
        assert!(press_key_js(None, "Enter").contains("document.activeElement"));
        assert!(press_key_js(Some("#in"), "a").contains("document.querySelector"));
    }
}
