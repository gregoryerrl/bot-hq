//! JSON-RPC client for bot-hq's own signaling MCP server.
//!
//! The native loop reaches bot-hq's signaling tools over HTTP rather than by
//! calling [`SignalingBridge`](crate::signaling::SignalingBridge) in-process.
//! That is deliberate: the HTTP dispatch point is where `HANDS_ONLY_TOOLS` /
//! `EYES_ONLY_TOOLS` role enforcement lives, and going around it would give a
//! native agent a second, unenforced path to the same tools.
//!
//! The transport is as simple as it looks — `POST /sessions/<id>/<agent>/mcp`
//! with a JSON-RPC body and a JSON-RPC body back (`signaling/server.rs`). No
//! SSE, no Streamable HTTP, no session handshake beyond `initialize`. That is
//! why this is ~150 lines of `reqwest` instead of a dependency on `rmcp`.
//!
//! External stdio MCP servers (chrome-devtools, discord, clockify) are **not**
//! handled here and are not needed for v1: `user_mcp_servers_for_agent` gives
//! EYES an empty external-server map by design, and EYES is the v1 target.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::warn;

use super::wire::ToolOutcome;

/// The server key `mcp_config_json` always writes.
const SIGNALING_KEY: &str = "bot-hq-signaling";

pub struct McpClient {
    http: reqwest::Client,
    url: String,
    next_id: AtomicU64,
}

impl McpClient {
    pub fn new(url: impl Into<String>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .build()
                .context("building MCP HTTP client")?,
            url: url.into(),
            next_id: AtomicU64::new(1),
        })
    }

    /// Build from the per-agent `mcp-config.json` that `spawn_agent_for`
    /// already writes (`core/session.rs:795`) — so the native path picks up the
    /// same URL the CLI path would have used, with no new plumbing.
    pub fn from_mcp_config(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading mcp-config at {}", path.display()))?;
        Self::new(parse_signaling_url(&raw)?)
    }

    /// One JSON-RPC round trip. Errors here are transport/protocol failures;
    /// a tool that *ran* and failed comes back as a normal result with
    /// `isError: true` and is not an `Err`.
    async fn rpc(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut req = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(p) = params {
            req.as_object_mut()
                .expect("json! built an object")
                .insert("params".into(), p);
        }

        // `reqwest` is built with `default-features = false`, so the `json`
        // feature is off and `.json()` does not exist — serialize by hand.
        let payload = serde_json::to_vec(&req).context("serializing JSON-RPC request")?;

        let resp = self
            .http
            .post(&self.url)
            .header("content-type", "application/json")
            .body(payload)
            .send()
            .await
            .with_context(|| format!("POST {method} to signaling MCP"))?;

        let status = resp.status();
        let text = resp.text().await.context("reading MCP response body")?;
        if !status.is_success() {
            bail!("signaling MCP returned {status}: {text}");
        }

        let body: Value = serde_json::from_str(&text)
            .with_context(|| format!("parsing MCP response for {method}"))?;

        if let Some(err) = body.get("error") {
            bail!("{method} failed: {err}");
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| anyhow!("{method} response has neither result nor error"))
    }

    /// MCP handshake. A failure here is fatal for the agent — it means the
    /// signaling server is unreachable, so none of bot-hq's tools exist.
    pub async fn initialize(&self) -> Result<Value> {
        self.rpc(
            "initialize",
            Some(json!({
                "protocolVersion": crate::signaling::protocol::PROTOCOL_VERSION,
                "clientInfo": { "name": "bot-hq-native", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": {}
            })),
        )
        .await
    }

    /// Raw MCP tool descriptors. Feed through [`mcp_tools_to_anthropic`] before
    /// putting them in a Messages request.
    pub async fn list_tools(&self) -> Result<Vec<Value>> {
        let result = self.rpc("tools/list", None).await?;
        Ok(result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Call a tool and convert whatever happens into something the model can
    /// read. **Never returns `Err`**: a transport failure, a JSON-RPC error and
    /// a tool that ran and failed all become `is_error` outcomes, because errors
    /// are inputs to the agent loop, not terminations of it.
    pub async fn call_tool(&self, id: &str, name: &str, args: Value) -> ToolOutcome {
        match self
            .rpc("tools/call", Some(json!({ "name": name, "arguments": args })))
            .await
        {
            Ok(result) => tool_result_to_outcome(id, &result),
            Err(e) => {
                warn!(tool = %name, error = %e, "signaling tool call failed");
                ToolOutcome {
                    tool_use_id: id.to_string(),
                    content: format!("tool call failed: {e}"),
                    is_error: true,
                }
            }
        }
    }
}

/// Pull the signaling server URL out of an `mcp-config.json` body.
pub fn parse_signaling_url(raw: &str) -> Result<String> {
    let cfg: Value = serde_json::from_str(raw).context("parsing mcp-config JSON")?;
    cfg.get("mcpServers")
        .and_then(|s| s.get(SIGNALING_KEY))
        .and_then(|s| s.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("mcp-config has no mcpServers.{SIGNALING_KEY}.url"))
}

/// Convert MCP tool descriptors to Anthropic Messages `tools` entries.
///
/// The two formats differ by exactly one field name — MCP says `inputSchema`,
/// the Messages API says `input_schema` — and getting it wrong is a silent
/// failure mode: the request is accepted and the model simply never calls the
/// tool, because as far as it can tell the tool takes no arguments.
pub fn mcp_tools_to_anthropic(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|t| {
            let name = t.get("name").and_then(Value::as_str)?;
            let schema = t
                .get("inputSchema")
                .or_else(|| t.get("input_schema"))
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            Some(json!({
                "name": name,
                "description": t.get("description").and_then(Value::as_str).unwrap_or(""),
                "input_schema": schema,
            }))
        })
        .collect()
}

/// Flatten an MCP `tools/call` result into a [`ToolOutcome`].
///
/// MCP returns `{ content: [{ type: "text", text }...], isError: bool }`; the
/// Messages API wants a single string. Non-text blocks are skipped rather than
/// rendered, and multiple text blocks are newline-joined.
pub fn tool_result_to_outcome(id: &str, result: &Value) -> ToolOutcome {
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    ToolOutcome {
        tool_use_id: id.to_string(),
        content: text,
        is_error: result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_url_mcp_config_json_writes() {
        let addr: std::net::SocketAddr = "127.0.0.1:54321".parse().unwrap();
        let raw = crate::signaling::mcp_config_json(addr, "s1", "rain", &serde_json::Map::new());
        assert_eq!(
            parse_signaling_url(&raw).unwrap(),
            "http://127.0.0.1:54321/sessions/s1/rain/mcp"
        );
    }

    #[test]
    fn ignores_other_servers_in_the_config() {
        let raw = r#"{"mcpServers":{
            "discord":{"command":"node","args":["x.js"]},
            "bot-hq-signaling":{"type":"http","url":"http://127.0.0.1:1/sessions/a/b/mcp"}
        }}"#;
        assert_eq!(
            parse_signaling_url(raw).unwrap(),
            "http://127.0.0.1:1/sessions/a/b/mcp"
        );
    }

    #[test]
    fn missing_signaling_entry_is_an_error_not_a_default() {
        let raw = r#"{"mcpServers":{"discord":{"command":"node"}}}"#;
        assert!(parse_signaling_url(raw).is_err());
    }

    #[test]
    fn renames_input_schema_for_the_messages_api() {
        let tools = vec![json!({
            "name": "ask_user_choice",
            "description": "Ask the user.",
            "inputSchema": { "type": "object", "properties": { "question": { "type": "string" } } }
        })];
        let out = mcp_tools_to_anthropic(&tools);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["name"], "ask_user_choice");
        // The silent-failure field: wrong casing here and the model never calls
        // the tool, because it looks argument-less.
        assert!(out[0].get("inputSchema").is_none());
        assert_eq!(out[0]["input_schema"]["properties"]["question"]["type"], "string");
    }

    #[test]
    fn tool_without_a_name_is_dropped_rather_than_sent_malformed() {
        let tools = vec![json!({ "description": "no name" })];
        assert!(mcp_tools_to_anthropic(&tools).is_empty());
    }

    #[test]
    fn tool_without_a_schema_gets_an_empty_object_schema() {
        let tools = vec![json!({ "name": "ping" })];
        let out = mcp_tools_to_anthropic(&tools);
        assert_eq!(out[0]["input_schema"]["type"], "object");
    }

    #[test]
    fn flattens_text_blocks_and_carries_is_error() {
        let result = json!({
            "content": [{ "type": "text", "text": "line one" }, { "type": "text", "text": "line two" }],
            "isError": true
        });
        let out = tool_result_to_outcome("tu_1", &result);
        assert_eq!(out.tool_use_id, "tu_1");
        assert_eq!(out.content, "line one\nline two");
        assert!(out.is_error);
    }

    #[test]
    fn absent_is_error_means_success() {
        let result = json!({ "content": [{ "type": "text", "text": "ok" }] });
        assert!(!tool_result_to_outcome("tu_1", &result).is_error);
    }
}
