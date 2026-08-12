//! Spike: a minimal *native* Rust agent loop against the Claude Messages API.
//!
//! # Why this exists
//!
//! bot-hq currently rents its agent loop from `claude-code`: we spawn the CLI,
//! it decides which tools to call and when, and we steer from the outside via
//! hooks, MCP tools, and policy. Everything else — UI, sessions, gates, the
//! Context Library, duo orchestration — is already ours.
//!
//! The open question is whether bot-hq could own the loop itself. This file
//! answers it end to end at the smallest possible scale: authenticate, call
//! `POST /v1/messages`, receive a `tool_use`, execute it locally in Rust, feed
//! the result back, iterate to a final answer.
//!
//! # What it deliberately does NOT do
//!
//! No streaming (SSE), no prompt caching, no MCP bridging, no permission
//! gating, no context compaction, no session persistence, no Brian/Rain
//! wiring. Those are the *next* questions if this holds up.
//!
//! # Auth
//!
//! Needs a **Console API key**. A Claude Pro/Max subscription cannot drive
//! this: subscription OAuth is bound server-side to Claude Code and claude.ai,
//! so the loop you own is the loop you pay API rates for. That tradeoff is the
//! finding, not a bug — see the session's `investigate` doc.
//!
//! # Run
//!
//! ```sh
//! ANTHROPIC_API_KEY=sk-ant-... cargo run --example native_loop -- \
//!   "how many dependencies does Cargo.toml declare?"
//!
//! # cheap run:
//! BOTHQ_SPIKE_MODEL=claude-haiku-4-5 ANTHROPIC_API_KEY=... \
//!   cargo run --example native_loop -- "..."
//! ```
//!
//! Examples are excluded from `cargo build --release`, so nothing here reaches
//! the shipped binary. Deleting the spike is `rm examples/native_loop.rs`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-opus-5";
const MAX_TOKENS: u32 = 16_000;

/// Runaway loops are a real cost risk. Bail loudly rather than spin.
const MAX_TURNS: usize = 12;

/// Cap tool output so one `read_file` can't blow the context window.
const MAX_FILE_BYTES: usize = 256 * 1024;

struct Config {
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl Config {
    fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "ANTHROPIC_API_KEY is unset.\n\n\
                     This spike needs a Console API key \
                     (https://platform.claude.com/settings/keys).\n\
                     A Claude Pro/Max subscription CANNOT drive it — subscription OAuth is \
                     bound server-side to Claude Code and claude.ai, so it will be rejected \
                     here regardless of how the token is supplied."
                )
            })?;

        let model = std::env::var("BOTHQ_SPIKE_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        Ok(Self { api_key, model, max_tokens: MAX_TOKENS })
    }
}

/// Read-only by construction: no bash, no write, no network. Safe to leave
/// running unattended, which is what makes this a spike and not a liability.
fn tool_defs() -> Value {
    json!([
        {
            "name": "read_file",
            "description": "Read a UTF-8 text file from the project. \
                            Paths are relative to the project root.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path relative to the project root, e.g. \"Cargo.toml\"."
                    }
                },
                "required": ["path"]
            }
        },
        {
            "name": "list_dir",
            "description": "List the entries of a directory in the project. \
                            Directories are suffixed with '/'.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path relative to the project root. \
                                        Use \".\" for the root itself."
                    }
                },
                "required": ["path"]
            }
        }
    ])
}

/// Resolve `rel` beneath `root`, refusing anything that escapes.
///
/// `root` MUST already be canonicalized. Canonicalizing the candidate too is
/// what makes this hold against `..` *and* symlinks — a plain string prefix
/// check passes both. Note `Path::join` lets an absolute `rel` replace the
/// base entirely, so "/etc/passwd" lands outside `root` and is caught here.
fn resolve_in_root(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let canonical = root
        .join(rel)
        .canonicalize()
        .map_err(|e| format!("cannot resolve {rel:?}: {e}"))?;

    if !canonical.starts_with(root) {
        return Err(format!("{rel:?} resolves outside the project root — refused"));
    }
    Ok(canonical)
}

/// `Err` becomes a `tool_result` with `is_error: true`, which the model reads
/// and recovers from. Errors are inputs to the loop, not terminations of it.
fn exec_tool(name: &str, input: &Value, root: &Path) -> Result<String, String> {
    let path = input
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing required string field \"path\"".to_string())?;

    let target = resolve_in_root(root, path)?;

    match name {
        "read_file" => {
            let bytes = std::fs::read(&target).map_err(|e| format!("read failed: {e}"))?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(format!(
                    "file is {} bytes; this spike caps reads at {MAX_FILE_BYTES}",
                    bytes.len()
                ));
            }
            String::from_utf8(bytes).map_err(|_| "file is not valid UTF-8".to_string())
        }
        "list_dir" => {
            let mut names: Vec<String> = std::fs::read_dir(&target)
                .map_err(|e| format!("list failed: {e}"))?
                .filter_map(Result::ok)
                .map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    match entry.file_type() {
                        Ok(t) if t.is_dir() => format!("{name}/"),
                        _ => name,
                    }
                })
                .collect();
            names.sort();
            Ok(names.join("\n"))
        }
        other => Err(format!("unknown tool {other:?}")),
    }
}

async fn post_messages(
    client: &reqwest::Client,
    cfg: &Config,
    messages: &[Value],
) -> Result<Value> {
    // Deliberately absent: `temperature`, `top_p`, `top_k`, and
    // `thinking.budget_tokens`. All four are hard 400s on Claude Opus 5.
    let body = json!({
        "model": cfg.model,
        "max_tokens": cfg.max_tokens,
        "tools": tool_defs(),
        "messages": messages,
    });

    // `reqwest` is built with `default-features = false` in this crate, so the
    // `json` feature is off and `.json()` does not exist — serialize by hand.
    let payload = serde_json::to_vec(&body).context("serializing request body")?;

    let resp = client
        .post(API_URL)
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .body(payload)
        .send()
        .await
        .context("POST /v1/messages failed")?;

    let status = resp.status();
    let text = resp.text().await.context("reading response body")?;

    if !status.is_success() {
        // Surface the body verbatim — the API's error messages name the exact
        // offending field, which is most of the debugging value.
        bail!("API returned {status}: {text}");
    }

    serde_json::from_str(&text).context("parsing response JSON")
}

async fn run(
    client: &reqwest::Client,
    cfg: &Config,
    root: &Path,
    task: &str,
) -> Result<String> {
    let mut messages = vec![json!({ "role": "user", "content": task })];

    for turn in 1..=MAX_TURNS {
        let resp = post_messages(client, cfg, &messages).await?;
        let stop_reason = resp.get("stop_reason").and_then(Value::as_str).unwrap_or("");

        // TRAP 3 — check `refusal` BEFORE touching `content`. Claude Opus 5
        // runs safety classifiers; on a decline `content` can be empty, so
        // code that indexes `content[0]` unconditionally panics here.
        if stop_reason == "refusal" {
            let details = resp.get("stop_details").cloned().unwrap_or(Value::Null);
            bail!("model declined the request (stop_details: {details})");
        }

        let content = resp
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for block in &content {
            match block.get("type").and_then(Value::as_str) {
                Some("thinking") => eprintln!("  [turn {turn}] thinking"),
                Some("tool_use") => {
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("?");
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    eprintln!("  [turn {turn}] tool_use {name} {input}");
                }
                _ => {}
            }
        }

        match stop_reason {
            "end_turn" => {
                return Ok(content
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(""));
            }

            "max_tokens" => bail!(
                "hit max_tokens ({}) — on Opus 5 thinking and response text share \
                 this budget, so raise it rather than assuming the answer was long",
                cfg.max_tokens
            ),

            "tool_use" => {
                // TRAP 1 — echo the FULL content array back, `thinking` blocks
                // included and byte-identical. This is the bug hand-rolled
                // loops hit most: drop or edit those blocks and the request
                // succeeds, then the NEXT one 400s on a signature/ordering
                // check. Pushing `content` wholesale is what avoids it.
                messages.push(json!({ "role": "assistant", "content": content }));

                // TRAP 2 — every `tool_result` goes in ONE user message.
                // Splitting them across messages silently trains the model out
                // of making parallel tool calls.
                let mut results = Vec::new();
                for block in &content {
                    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                    let name = block.get("name").and_then(Value::as_str).unwrap_or_default();
                    let input = block.get("input").cloned().unwrap_or(Value::Null);

                    results.push(match exec_tool(name, &input, root) {
                        Ok(out) => json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": out,
                        }),
                        Err(msg) => json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": msg,
                            "is_error": true,
                        }),
                    });
                }

                if results.is_empty() {
                    bail!("stop_reason was tool_use but no tool_use block was present");
                }
                messages.push(json!({ "role": "user", "content": results }));
            }

            other => bail!("unhandled stop_reason {other:?}"),
        }
    }

    bail!("gave up after {MAX_TURNS} turns without reaching end_turn")
}

#[tokio::main]
async fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_else(|| {
        "How many dependencies does Cargo.toml declare? Read the file and count them.".to_string()
    });

    let cfg = Config::from_env()?;
    let root = std::env::current_dir()
        .context("reading current directory")?
        .canonicalize()
        .context("canonicalizing project root")?;

    eprintln!("model : {}", cfg.model);
    eprintln!("root  : {}", root.display());
    eprintln!("task  : {task}\n");

    let client = reqwest::Client::builder()
        .build()
        .context("building HTTP client")?;

    let started = std::time::Instant::now();
    let answer = run(&client, &cfg, &root, &task).await?;

    eprintln!("\n--- {:.1}s ---\n", started.elapsed().as_secs_f64());
    println!("{answer}");
    Ok(())
}
