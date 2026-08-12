//! Spike: a native Rust agent loop that runs on a **Claude Max subscription** —
//! no API key.
//!
//! # The constraint this is built around
//!
//! Subscription OAuth is bound server-side to Claude Code. You cannot lift the
//! token out and call `/v1/messages` with it. So if you want subscription
//! billing, the `claude` binary has to stay in the picture.
//!
//! But "the binary stays" is NOT the same as "claude-code's agent loop stays",
//! and that distinction is the whole point of this file. Two CLI flags collapse
//! claude-code into something close to a bare completion engine:
//!
//! * `--system-prompt` **replaces** claude-code's system prompt outright
//!   (unlike `--append-system-prompt`, which bot-hq uses today). We install our
//!   own operator contract instead.
//! * `--disallowedTools` strips every built-in. With no tools, claude-code has
//!   nothing to orchestrate — it can only answer.
//!
//! What's left is a model endpoint. bot-hq supplies the tool catalog in the
//! prompt, parses the model's chosen action out of the reply, executes it
//! natively in Rust, and feeds the observation back. **The loop lives here, in
//! Rust — not in claude-code.** That is the thing the API-key spike proved was
//! possible; this proves it's possible without leaving the subscription.
//!
//! # The cost of doing it this way
//!
//! Structured `tool_use` blocks are gone — they're a Messages API feature, and
//! we're going through a harness now. Tool calls come back as JSON in free
//! text, which we parse. That is strictly less reliable than native tool
//! calling and is the main thing to weigh. `parse_action` is deliberately
//! forgiving (strips fences, scans for the outermost object) and a parse
//! failure is fed back to the model as an observation rather than aborting.
//!
//! # Traps encoded here
//!
//! * **`--bare` would break this.** Its own help text: under `--bare`, "Anthropic
//!   auth is strictly ANTHROPIC_API_KEY or apiKeyHelper — OAuth and keychain are
//!   never read." `--bare` is therefore incompatible with subscription auth.
//! * `--input-format stream-json` is what keeps the process alive across turns.
//!   Without it the CLI answers once and exits, and there is no loop to own.
//! * `--verbose` is required alongside `--output-format stream-json`.
//! * Inherited `CLAUDE*` env from a parent claude-code session confuses the
//!   child; we clear it so the spike behaves like a fresh shell.
//!
//! # Run
//!
//! ```sh
//! cargo run --example subscription_loop -- "how many dependencies does Cargo.toml declare?"
//! ```
//!
//! Uses whatever `claude` is already logged in as. No API key is read.
//! Examples are excluded from `cargo build --release`, so nothing here ships.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const MAX_TURNS: usize = 12;
const MAX_FILE_BYTES: usize = 256 * 1024;
/// A single turn on a small file is seconds; a minute means something wedged.
const TURN_TIMEOUT: Duration = Duration::from_secs(180);

/// Every built-in, so the model is left with nothing to orchestrate and must
/// answer through our contract instead.
const DISALLOWED: &str = "Bash Edit Write Read Glob Grep WebFetch WebSearch Task \
                          TodoWrite NotebookEdit MultiEdit LS Agent ToolSearch \
                          BashOutput KillShell";

/// Our operator contract — this fully replaces claude-code's system prompt.
const SYSTEM_PROMPT: &str = r#"You are the reasoning core of an agent loop. You have NO tools.
A Rust program executes actions on your behalf and reports observations back.

Reply with EXACTLY ONE JSON object and nothing else. No prose. No markdown fences.

Available actions:
  {"action":"read_file","path":"<path relative to project root>"}
  {"action":"list_dir","path":"<path relative to project root, '.' for root>"}
  {"action":"final","answer":"<your complete answer to the task>"}

Rules:
- Emit one action per reply. You will receive an OBSERVATION, then reply again.
- Use "final" as soon as you can answer. Do not pad with extra lookups.
- If an observation reports an error, adjust and try a different action."#;

// ---------------------------------------------------------------- tools

/// Resolve `rel` beneath `root`, refusing anything that escapes.
///
/// `root` MUST already be canonicalized. Canonicalizing the candidate too is
/// what makes this hold against `..` *and* symlinks — a string prefix check
/// passes both. Note `Path::join` lets an absolute `rel` replace the base, so
/// "/etc/passwd" lands outside `root` and is caught here.
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

/// Read-only by construction. `Err` is fed back as an observation, so a bad
/// action steers the model rather than killing the run.
fn exec_action(action: &str, path: &str, root: &Path) -> Result<String, String> {
    let target = resolve_in_root(root, path)?;

    match action {
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
        other => Err(format!("unknown action {other:?}")),
    }
}

/// Recover the **first complete** JSON object from a free-text reply.
///
/// This is the tax for losing structured `tool_use`, and getting it right took
/// two measured failures — both found by the stress run, both caused by the
/// naive version of this function rather than by the model:
///
/// * **Splitting on ``` corrupts JSON whose *string values* contain a fence.**
///   Asked to quote a file verbatim, the model correctly emitted
///   `{"action":"final","answer":"...\n```\n# Binary\n..."}`; splitting on the
///   fence chopped that mid-string and the JSON lost its closing brace. The
///   "forgiving" preprocessing was itself the bug.
/// * **Taking the outermost `{`..`}` span breaks on a doubled object.** In a
///   monotonous run of `list_dir` calls the model occasionally repeated itself
///   — `{...}{...}`, the same object twice in one text block. First-brace to
///   last-brace spans both and fails with "trailing characters".
///
/// So: scan from the first `{`, track depth, and — critically — respect string
/// literals and their escapes so braces and fences *inside* a value can't move
/// the depth counter. Return at depth 0. Anything after the first object is
/// ignored, which absorbs both repetition and trailing prose.
fn parse_action(text: &str) -> Result<Value, String> {
    let start = text.find('{').ok_or("no JSON object in reply")?;

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    // Byte scan is safe here: `{`, `}`, `"`, and `\` are ASCII, so any index we
    // slice at is a char boundary even when the reply contains multibyte text.
    for (i, &b) in text.as_bytes().iter().enumerate().skip(start) {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..=i])
                        .map_err(|e| format!("JSON parse failed: {e}"));
                }
            }
            _ => {}
        }
    }

    Err("reply contains no complete JSON object (unterminated)".to_string())
}

// ---------------------------------------------------------------- transport

struct Engine {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Engine {
    /// Spawn `claude` as a tool-less completion engine.
    fn spawn(root: &Path) -> Result<Self> {
        let mut cmd = Command::new("claude");
        cmd.current_dir(root)
            .arg("-p")
            .args(["--input-format", "stream-json"])
            .args(["--output-format", "stream-json"])
            .arg("--verbose")
            .args(["--system-prompt", SYSTEM_PROMPT])
            .args(["--disallowedTools", DISALLOWED])
            .args(["--permission-mode", "dontAsk"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        // Model axis matters for this design specifically: without structured
        // `tool_use` the contract is held by instruction-following alone, and
        // smaller models are measurably worse at that. Leave unset to use the
        // CLI's configured default.
        if let Ok(model) = std::env::var("BOTHQ_SPIKE_MODEL") {
            if !model.trim().is_empty() {
                cmd.args(["--model", &model]);
            }
        }

        // A parent claude-code session leaks CLAUDE* env into the child and
        // confuses it. Clear it so this behaves like a fresh shell.
        for (key, _) in std::env::vars() {
            if key.starts_with("CLAUDE") {
                cmd.env_remove(key);
            }
        }

        let mut child = cmd.spawn().context(
            "failed to spawn `claude` — is the CLI on PATH and logged in? (`claude /login`)",
        )?;
        let stdin = child.stdin.take().context("no stdin pipe")?;
        let stdout = BufReader::new(child.stdout.take().context("no stdout pipe")?);

        Ok(Self { child, stdin, stdout })
    }

    /// Send one user turn and read until the engine reports `result`.
    /// Returns the assistant's accumulated text.
    async fn turn(&mut self, content: &str) -> Result<String> {
        let line = serde_json::to_string(&json!({
            "type": "user",
            "message": { "role": "user", "content": content },
        }))?;

        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;

        let read = async {
            let mut answer = String::new();
            let mut buf = String::new();
            loop {
                buf.clear();
                let n = self.stdout.read_line(&mut buf).await?;
                if n == 0 {
                    bail!("engine closed stdout before returning a result");
                }
                let Ok(evt) = serde_json::from_str::<Value>(buf.trim()) else {
                    continue; // non-JSON noise
                };

                match evt.get("type").and_then(Value::as_str) {
                    Some("assistant") => {
                        if let Some(blocks) = evt
                            .pointer("/message/content")
                            .and_then(Value::as_array)
                        {
                            for b in blocks {
                                if b.get("type").and_then(Value::as_str) == Some("text") {
                                    if let Some(t) = b.get("text").and_then(Value::as_str) {
                                        answer.push_str(t);
                                    }
                                }
                            }
                        }
                    }
                    Some("result") => {
                        if evt.get("is_error").and_then(Value::as_bool) == Some(true) {
                            bail!("engine reported an error result: {evt}");
                        }
                        // Cost instrumentation. `cache_read` is the load-bearing
                        // number: if it stays 0 across turns the prefix is being
                        // re-billed every turn and this design is expensive.
                        // Note `cost_usd` is the *equivalent API price* — on a
                        // subscription you don't pay it, but it's the shadow
                        // cost and the right basis for comparing designs.
                        let u = |p: &str| {
                            evt.pointer(p).and_then(Value::as_u64).unwrap_or(0)
                        };
                        eprintln!(
                            "TURNSTAT cache_read={} cache_create={} input={} output={} api_ms={} cost_usd={:.4}",
                            u("/usage/cache_read_input_tokens"),
                            u("/usage/cache_creation_input_tokens"),
                            u("/usage/input_tokens"),
                            u("/usage/output_tokens"),
                            u("/duration_api_ms"),
                            evt.get("total_cost_usd").and_then(Value::as_f64).unwrap_or(0.0),
                        );
                        return Ok(answer);
                    }
                    _ => {}
                }
            }
        };

        tokio::time::timeout(TURN_TIMEOUT, read)
            .await
            .map_err(|_| anyhow::anyhow!("turn exceeded {}s", TURN_TIMEOUT.as_secs()))?
    }
}

// ---------------------------------------------------------------- loop

/// The agent loop. This is the part that lives in bot-hq rather than in
/// claude-code — decide, act, observe, repeat.
async fn run(engine: &mut Engine, root: &Path, task: &str) -> Result<String> {
    let mut message = format!("TASK: {task}");
    // Stress instrumentation: parse failures are the load-bearing risk of
    // giving up structured `tool_use`, so count them explicitly rather than
    // leaving them buried in the trace.
    let mut parse_failures = 0usize;

    for turn in 1..=MAX_TURNS {
        let reply = engine.turn(&message).await?;

        let action = match parse_action(&reply) {
            Ok(a) => a,
            Err(why) => {
                parse_failures += 1;
                // Log a bounded slice of what we actually got — without it a
                // failure is unattributable after the fact.
                let sample: String = reply.chars().take(160).collect();
                eprintln!("  [turn {turn}] PARSE-FAIL ({why}) got: {sample:?}");
                message = format!(
                    "OBSERVATION: your last reply could not be parsed ({why}). \
                     Reply with exactly one JSON object and nothing else."
                );
                continue;
            }
        };

        let kind = action.get("action").and_then(Value::as_str).unwrap_or("");

        if kind == "final" {
            eprintln!("  [turn {turn}] final");
            eprintln!("STATS turns={turn} parse_failures={parse_failures}");
            return Ok(action
                .get("answer")
                .and_then(Value::as_str)
                .unwrap_or("(no answer field)")
                .to_string());
        }

        let path = action.get("path").and_then(Value::as_str).unwrap_or("");
        eprintln!("  [turn {turn}] {kind} {path:?}");

        message = match exec_action(kind, path, root) {
            Ok(out) => format!("OBSERVATION ({kind} {path}):\n{out}"),
            Err(err) => format!("OBSERVATION (error): {err}"),
        };
    }

    eprintln!("STATS turns={MAX_TURNS} parse_failures={parse_failures} EXHAUSTED");
    bail!("gave up after {MAX_TURNS} turns without a final answer")
}

#[tokio::main]
async fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_else(|| {
        "How many dependencies does Cargo.toml declare? Read it and count them.".to_string()
    });

    let root = std::env::current_dir()
        .context("reading current directory")?
        .canonicalize()
        .context("canonicalizing project root")?;

    eprintln!("engine: claude (subscription auth — no API key read)");
    eprintln!("root  : {}", root.display());
    eprintln!("task  : {task}\n");

    let started = std::time::Instant::now();
    let mut engine = Engine::spawn(&root)?;

    let result = run(&mut engine, &root, &task).await;

    // Close stdin so the engine exits, then reap it.
    drop(engine.stdin);
    let _ = engine.child.wait().await;

    let answer = result?;
    eprintln!("\n--- {:.1}s ---\n", started.elapsed().as_secs_f64());
    println!("{answer}");
    Ok(())
}
