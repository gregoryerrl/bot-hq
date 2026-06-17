//! Spawn a `claude-code` subprocess wired up with stream-json IO + the
//! MCP-signaling server. Returns an `AgentHandle` the core layer drives.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::agents::events;
use crate::agents::input;
use crate::agents::protocol::OutgoingUserMessage;
use crate::storage::AgentConfig;

/// Global registry of live claude-code child PIDs. Updated by
/// `spawn_agent` (insert) and the lifecycle task (remove on exit). Read
/// by `reap_all_children` from `main.rs`'s panic hook + signal handler
/// so the children get SIGKILL even when the tokio runtime can't be
/// trusted (panic-abort / SIGTERM paths skip Drop chains entirely).
pub static CHILD_PIDS: LazyLock<Mutex<HashSet<u32>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Sync, signal-safe child reaper. Walks the registered PIDs and
/// force-kills each via the per-platform `kill_child` (unix SIGKILL /
/// Windows TerminateProcess) — no tokio, no async, no Drop chain.
///
/// Uses `try_lock` (not `lock`) so the panic hook can't deadlock against
/// a spawn-in-progress on another thread, and so a same-thread panic
/// mid-`insert()` doesn't recurse. Worst case on contention: one
/// cleanup cycle skipped — preferable to a hang.
pub fn reap_all_children() {
    let pids: Vec<u32> = match CHILD_PIDS.try_lock() {
        Ok(g) => g.iter().copied().collect(),
        Err(_) => return,
    };
    for pid in pids {
        kill_child(pid);
    }
}

/// Force-kill one child by PID. Best-effort: a kill that fails (process
/// already gone, access denied) is skipped, matching the unix kill(2)
/// semantics of ignoring the return value.
#[cfg(unix)]
fn kill_child(pid: u32) {
    // SAFETY: libc::kill is async-signal-safe + thread-safe; valid
    // pids are u32 from std/tokio's child.id() which fits in i32 for
    // every realistic process number on darwin/linux.
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

/// Windows twin of the SIGKILL path. OpenProcess/TerminateProcess are
/// plain Win32 calls, callable from any thread including a panic hook —
/// no async-signal-safety concept applies on Windows. A NULL handle
/// (process already exited, or access denied) is skipped. Note Windows
/// has no kill-children-on-parent-exit semantics, so this walk is just
/// as load-bearing here as the unix one (Ghost-Brian).
#[cfg(windows)]
fn kill_child(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_TERMINATE,
    };
    // SAFETY: handle is null-checked before use and closed exactly once;
    // TerminateProcess on a PROCESS_TERMINATE handle is documented
    // thread-safe.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

/// Liveness of an agent's retry supervisor, surfaced to the UI as a health dot
/// (B2). Plain enum — the serializable Tauri payload is built at the
/// `tauri_events` boundary via [`AgentHealth::as_str`], so the agents layer
/// stays free of `specta`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentHealth {
    /// Running normally.
    Running,
    /// Hit a transient API error; backing off + auto-resuming.
    Retrying,
    /// Supervisor gave up — permanent error / exhausted retries / exited.
    Dead,
}

impl AgentHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentHealth::Running => "running",
            AgentHealth::Retrying => "retrying",
            AgentHealth::Dead => "dead",
        }
    }
}

/// High-level events a session-orchestrator consumes from an agent.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Plain prose chunk from the assistant.
    Text(String),
    /// Agent invoked a tool (typically `ask_user_choice` or `mark_awaiting_user`).
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Tool call's result echoed back into the conversation (after MCP fulfilled it).
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    /// Agent finished its turn (the `result` stream event).
    TurnComplete {
        stop_reason: Option<String>,
        subtype: Option<String>,
        /// True when the turn FAILED — `result.is_error`, a non-`success`
        /// subtype, or a populated `api_error_status` (e.g. an API 400). A
        /// failed turn's buffered text must NOT be peer-forwarded: forwarding
        /// it bounces the error to the peer, the peer replies, and that
        /// re-triggers the failing agent — an unbounded error-spam loop
        /// (Rain on the DeepSeek gateway, 2026-05-29).
        is_error: bool,
        /// Upstream API HTTP status when the turn failed on an API error
        /// (e.g. `529` Overloaded, `503`, `429`). `None` on success or on a
        /// non-API failure. The retry supervisor reads this to decide whether
        /// the failure is transient (auto-resume) or permanent (surface it).
        api_error_status: Option<u16>,
    },
    /// System/init event — agent is ready and reporting its session metadata.
    /// (The wire `SystemEvent::Init` also carries `model`/`cwd`, but no
    /// consumer reads them, so they are not forwarded here.)
    Init { session_id: Option<String> },
    /// Process exited. Carries exit-status string for log/observability.
    Exited(String),
    /// Catch-all for fatal errors the supervisor wants to surface.
    Error(String),
    /// Retry-supervisor liveness transition (B2), relayed by the duo pump to
    /// the UI as a health dot. Not produced by the stream-json translator —
    /// emitted directly by `supervise` at running/retrying/dead transitions.
    Health(AgentHealth),
}

/// Classify an upstream API HTTP status as transient (worth an automatic
/// resume + retry) vs. permanent (surface to the user — retrying won't help).
///
/// Transient: overload / rate-limit / gateway / timeout statuses that usually
/// clear on their own within seconds — `408` request timeout, `425` too early,
/// `429` rate limit, `500` internal, `502` bad gateway, `503` unavailable,
/// `504` gateway timeout, `529` overloaded (the Anthropic "API Error:
/// Overloaded" that stranded a session 2026-06-01). Everything else — notably
/// `400`/`401`/`403`/`404`/`413`/`422` — is a permanent/semantic failure where
/// a blind retry just re-fails (e.g. the DeepSeek system-role 400).
pub fn is_transient_api_error(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504 | 529)
}

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub agent_name: String,
    pub config: AgentConfig,
    pub system_prompt: String,
    pub mcp_config_path: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    /// Override the claude binary (for tests). Defaults to `"claude"`.
    pub claude_bin: Option<String>,
    /// Session this agent belongs to. Exported as `BOT_HQ_SESSION_ID` so
    /// the git pre-push hook can resolve session-scoped approvals.
    pub session_id: String,
    /// claude-code session UUID to resume (per-agent, captured from a prior
    /// spawn's `init` stream-json event and persisted on the bot-hq session
    /// row). When Some, the command line gains `--resume <uuid>` so the
    /// child picks up its previous conversation. When None, claude assigns
    /// a fresh UUID — we capture that one in the next `init` event.
    pub resume_session_id: Option<String>,
    /// Project name (CL / policy key) this session targets, if any. Threaded so
    /// HANDS can be spawned with a PreToolUse `tool-blocklist` hook bound to
    /// the project's `policy.yaml`. `None` for the projectless singleton.
    pub project: Option<String>,
    /// bot-hq data dir — the injected PreToolUse hook needs it to resolve the
    /// project's policy at tool-call time.
    pub data_dir: PathBuf,
    /// Per-session effort override (from the create dialog). When Some, it wins
    /// over the persistent per-agent override resolved from claude-overrides.json.
    pub session_effort: Option<String>,
    /// Per-session ultracode override (from the create dialog). When Some, it
    /// wins over the persistent per-agent override. Brian-only at runtime (EYES
    /// gets no --settings).
    pub session_ultracode: Option<bool>,
}

/// Driver handle for one running agent subprocess.
pub struct AgentHandle {
    pub name: String,
    pub event_rx: mpsc::Receiver<AgentEvent>,
    pub input_tx: mpsc::Sender<OutgoingUserMessage>,
    kill_tx: Option<oneshot::Sender<()>>,
}

impl AgentHandle {
    /// Best-effort kill. Idempotent (subsequent calls no-op).
    pub fn kill(&mut self) {
        if let Some(tx) = self.kill_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for AgentHandle {
    fn drop(&mut self) {
        self.kill();
    }
}

pub async fn spawn_agent(cfg: SpawnConfig) -> Result<AgentHandle> {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(256);
    let (input_tx, input_rx) = mpsc::channel::<OutgoingUserMessage>(64);
    let (kill_tx, kill_rx) = oneshot::channel::<()>();

    let mut cmd = build_command(&cfg);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "spawning claude-code for agent {}; bin={}",
            cfg.agent_name,
            cfg.claude_bin.as_deref().unwrap_or("claude")
        )
    })?;

    // Register PID for crash-path reaping. None on platforms that don't
    // expose pids (we only ship darwin/linux) or after the child has
    // already been reaped — the registration is best-effort either way.
    let child_pid = child.id();
    if let Some(pid) = child_pid {
        CHILD_PIDS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(pid);
    }

    let stdin = child.stdin.take().context("subprocess missing stdin")?;
    let stdout = child.stdout.take().context("subprocess missing stdout")?;
    let stderr = child.stderr.take().context("subprocess missing stderr")?;

    tokio::spawn(events::pump_events(stdout, event_tx.clone()));
    tokio::spawn(events::pump_stderr(stderr, cfg.agent_name.clone()));
    tokio::spawn(input::pump_inputs(stdin, input_rx, cfg.agent_name.clone()));

    let event_tx_for_lifecycle = event_tx.clone();
    let agent_name = cfg.agent_name.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = kill_rx => {
                info!(agent = %agent_name, "kill signalled");
                let _ = child.kill().await;
                if let Some(pid) = child_pid {
                    CHILD_PIDS.lock().unwrap_or_else(|p| p.into_inner()).remove(&pid);
                }
                let _ = event_tx_for_lifecycle
                    .send(AgentEvent::Exited("killed by supervisor".into()))
                    .await;
            }
            res = child.wait() => {
                if let Some(pid) = child_pid {
                    CHILD_PIDS.lock().unwrap_or_else(|p| p.into_inner()).remove(&pid);
                }
                let msg = match res {
                    Ok(status) => format!("status={status:?}"),
                    Err(e) => format!("wait error: {e}"),
                };
                warn!(agent = %agent_name, msg = %msg, "agent exited");
                let _ = event_tx_for_lifecycle.send(AgentEvent::Exited(msg)).await;
            }
        }
    });

    info!(agent = %cfg.agent_name, "agent spawned");

    Ok(AgentHandle {
        name: cfg.agent_name,
        event_rx,
        input_tx,
        kill_tx: Some(kill_tx),
    })
}

/// Retry policy for the agent supervisor: how many consecutive transient API
/// failures to absorb (auto-resume) before surfacing the error and stopping,
/// plus the backoff schedule between attempts. A successful turn resets the
/// budget.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        // ~2s, 4s, 8s, 16s, 30s — ≈60s of patience over 5 attempts, which
        // comfortably outlasts a typical Anthropic "Overloaded" blip, then
        // gives up with a clear message so a real outage doesn't loop forever.
        Self {
            max_retries: 5,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Backoff before the Nth retry (1-based): `base * 2^(n-1)`, capped at
    /// `max_delay`.
    pub fn backoff(&self, attempt: u32) -> Duration {
        let shift = attempt.saturating_sub(1).min(16);
        self.base_delay
            .saturating_mul(1u32 << shift)
            .min(self.max_delay)
    }
}

/// Spawn an agent under a retry supervisor. The returned `AgentHandle` exposes
/// STABLE event/input channels: when a child dies on a *transient* upstream API
/// error (e.g. `529` Overloaded), the supervisor auto-resumes it
/// (`--resume <uuid>`) with capped backoff and a continue-nudge — transparently
/// to the caller and the peer pump, with no channel rewiring. A permanent error
/// (e.g. `400`), a clean exit, or exhausting `max_retries` ends the supervisor
/// and closes the channels (the peer pump then unwinds on its own).
///
/// The first incarnation is spawned synchronously so spawn failures surface to
/// the caller via `?`, matching `spawn_agent`'s contract.
pub async fn spawn_supervised_agent(cfg: SpawnConfig, policy: RetryPolicy) -> Result<AgentHandle> {
    let (out_event_tx, out_event_rx) = mpsc::channel::<AgentEvent>(256);
    let (out_input_tx, out_input_rx) = mpsc::channel::<OutgoingUserMessage>(64);
    let (kill_tx, kill_rx) = oneshot::channel::<()>();

    let name = cfg.agent_name.clone();
    let first = spawn_agent(cfg.clone()).await?;

    tokio::spawn(supervise(
        cfg,
        policy,
        first,
        out_event_tx,
        out_input_rx,
        kill_rx,
        spawn_agent,
    ));

    Ok(AgentHandle {
        name,
        event_rx: out_event_rx,
        input_tx: out_input_tx,
        kill_tx: Some(kill_tx),
    })
}

/// Supervisor task body. Bridges one child incarnation at a time onto the
/// stable outer channels, retrying transient API failures. Generic over the
/// respawn fn so the retry logic is testable with fake incarnations.
#[allow(clippy::too_many_arguments)]
async fn supervise<S, Fut>(
    mut cfg: SpawnConfig,
    policy: RetryPolicy,
    first: AgentHandle,
    out_event_tx: mpsc::Sender<AgentEvent>,
    mut out_input_rx: mpsc::Receiver<OutgoingUserMessage>,
    mut kill_rx: oneshot::Receiver<()>,
    mut spawn_next: S,
) where
    S: FnMut(SpawnConfig) -> Fut + Send,
    Fut: std::future::Future<Output = Result<AgentHandle>> + Send,
{
    let agent = cfg.agent_name.clone();
    let mut incarnation = first;
    let mut consecutive_transient: u32 = 0;
    let mut pending_nudge: Option<String> = None;

    loop {
        // A freshly respawned `--resume` child idles until it receives input —
        // nudge it to pick up the interrupted turn.
        if let Some(nudge) = pending_nudge.take() {
            let _ = incarnation
                .input_tx
                .send(OutgoingUserMessage::text(nudge))
                .await;
        }

        let mut last_error_status: Option<u16> = None;

        // Bridge this incarnation until its event channel CLOSES. Closure (not
        // the `Exited` event) is the end-of-incarnation signal: the channel
        // closes only once both the stdout pump and the lifecycle task have
        // dropped their senders, so every event — including the final
        // `TurnComplete` carrying the error status — has already been received.
        // This makes classification race-free regardless of Exited/Result order.
        loop {
            tokio::select! {
                biased;
                _ = &mut kill_rx => {
                    incarnation.kill();
                    return;
                }
                msg = out_input_rx.recv() => {
                    match msg {
                        Some(msg) => {
                            if let Err(e) = incarnation.input_tx.send(msg).await {
                                // The incarnation's stdin pump has died (its
                                // receiver dropped), so the child is now deaf to
                                // ALL input — yet its event channel can stay open
                                // while stdout lingers, so the `None => break`
                                // path below would NOT catch it. Bridging on would
                                // silently drop every user/peer message (the #4
                                // user→HANDS desync, invisible + unrecoverable).
                                // Tear down instead: dropping `out_input_rx` closes
                                // the public sender → `is_stale()` → the next
                                // `ensure_session_started` evicts + respawns.
                                warn!(agent = %agent, error = %e, "incarnation input pump died; terminating supervisor so the session goes stale and respawns");
                                incarnation.kill();
                                return;
                            }
                        }
                        None => {
                            // Caller dropped the handle → tear down.
                            incarnation.kill();
                            return;
                        }
                    }
                }
                ev = incarnation.event_rx.recv() => {
                    match ev {
                        // Suppress Exited: forwarding it would make the peer
                        // pump terminate before a possible retry. Channel close
                        // below is the real signal.
                        Some(AgentEvent::Exited(reason)) => {
                            debug!(agent = %agent, %reason, "incarnation exited; awaiting channel close");
                        }
                        Some(ev) => {
                            match &ev {
                                AgentEvent::Init { session_id: Some(id) } => {
                                    cfg.resume_session_id = Some(id.clone());
                                }
                                AgentEvent::TurnComplete { is_error, api_error_status, .. } => {
                                    if *is_error {
                                        last_error_status = *api_error_status;
                                    } else {
                                        // A healthy turn clears the retry budget;
                                        // if we'd been retrying, signal recovery.
                                        if consecutive_transient > 0 {
                                            let _ = out_event_tx
                                                .send(AgentEvent::Health(AgentHealth::Running))
                                                .await;
                                        }
                                        consecutive_transient = 0;
                                        last_error_status = None;
                                    }
                                }
                                _ => {}
                            }
                            let _ = out_event_tx.send(ev).await;
                        }
                        None => break, // incarnation fully ended
                    }
                }
            }
        }

        let transient = last_error_status
            .map(is_transient_api_error)
            .unwrap_or(false);

        if transient && consecutive_transient < policy.max_retries {
            consecutive_transient += 1;
            let status = last_error_status.unwrap_or(0);
            let delay = policy.backoff(consecutive_transient);
            warn!(
                agent = %agent, status, attempt = consecutive_transient,
                delay_ms = delay.as_millis() as u64,
                "agent hit transient API error; auto-resuming after backoff"
            );
            let _ = out_event_tx
                .send(AgentEvent::Health(AgentHealth::Retrying))
                .await;
            tokio::select! {
                _ = &mut kill_rx => return,
                _ = tokio::time::sleep(delay) => {}
            }
            pending_nudge = Some(format!(
                "[bot-hq] Your previous turn was interrupted by a transient upstream API error \
                 (HTTP {status}) and has been automatically resumed. Continue exactly where you \
                 left off — re-issue the action you were about to take. Do NOT repeat work you \
                 already completed or committed."
            ));
            match spawn_next(cfg.clone()).await {
                Ok(next) => {
                    incarnation = next;
                    continue;
                }
                Err(e) => {
                    warn!(agent = %agent, error = %e, "respawn failed after transient error");
                    let _ = out_event_tx
                        .send(AgentEvent::Text(format!(
                            "⚠️ Could not resume after a transient API error (HTTP {status}): {e}. \
                             Reopen the session to retry."
                        )))
                        .await;
                    return;
                }
            }
        }

        if transient {
            // Budget exhausted — a real outage, not a blip. Surface it.
            let status = last_error_status.unwrap_or(0);
            warn!(agent = %agent, status, retries = consecutive_transient, "transient API errors exhausted retry budget");
            let _ = out_event_tx
                .send(AgentEvent::Text(format!(
                    "⚠️ Stopped after {consecutive_transient} consecutive transient API errors \
                     (last: HTTP {status}). The upstream API stayed unavailable — reopen the \
                     session to resume from here."
                )))
                .await;
        }
        // Clean exit / permanent error / retries exhausted: returning drops
        // `out_event_tx`, so the peer pump sees its channel close and unwinds.
        return;
    }
}

fn build_command(cfg: &SpawnConfig) -> Command {
    let bin = cfg.claude_bin.as_deref().unwrap_or("claude");
    let mut cmd = Command::new(bin);
    cmd.arg("-p")
        .args(["--input-format", "stream-json"])
        .args(["--output-format", "stream-json"])
        // `--verbose` is REQUIRED when combining `-p` + stream-json IO.
        // See docs/stream-json-events.md.
        .arg("--verbose")
        .args(["--append-system-prompt", &cfg.system_prompt]);

    // Per-agent Claude-config overrides (Settings → Claude Config). Resolved
    // from `<data_dir>/config/claude-overrides.json`; merged into the `--settings`
    // JSON + env below so a user can disable an inherited skill/plugin/MCP/
    // effort for THIS agent without touching their own ~/.claude. Fail-open.
    let mut agent_override = crate::claude_config::resolve_agent_overrides(
        &crate::claude_config::load_overrides(&cfg.data_dir),
        &cfg.agent_name,
    );
    // Per-SESSION overrides (create dialog) win over the persistent defaults.
    if cfg.session_effort.is_some() {
        agent_override.effort = cfg.session_effort.clone();
    }
    if cfg.session_ultracode.is_some() {
        agent_override.ultracode = cfg.session_ultracode;
    }

    // claude-code treats max-effort and ultracode as mutually exclusive
    // (ultracode implies xhigh + workflow orchestration; `max` is a distinct
    // effort posture, and emitting BOTH — env CLAUDE_CODE_EFFORT_LEVEL=max plus
    // "ultracode":true in --settings — is undefined). The per-surface UI already
    // prevents picking both, but a CROSS-LAYER overlay (persistent effort=max +
    // session ultracode, or the reverse) can still resolve to both. Reconcile
    // here honoring the same session-wins precedence as the overlay above:
    // whichever knob the session EXPLICITLY chose wins; otherwise ultracode wins.
    if agent_override.ultracode == Some(true) && agent_override.effort.as_deref() == Some("max") {
        let session_chose_max = cfg.session_effort.is_some() && cfg.session_ultracode.is_none();
        if session_chose_max {
            agent_override.ultracode = None;
        } else {
            agent_override.effort = None;
        }
    }

    if let Some(mcp) = &cfg.mcp_config_path {
        cmd.args(["--mcp-config", &mcp.display().to_string()])
            .arg("--strict-mcp-config");
    }

    // Resume a prior claude-code conversation for this agent if we have its
    // UUID stored. Lets a user close bot-hq and reopen the same session
    // without losing the agent's accumulated context. `--resume` coexists
    // with `-p` (`--help`: bracketed value skips the interactive picker).
    if let Some(resume_id) = &cfg.resume_session_id {
        cmd.args(["--resume", resume_id]);
    }

    // Permission posture is role-dependent.
    //
    // Brian (HANDS) runs with `--dangerously-skip-permissions`:
    // bot-hq is their permission layer (policy.yaml + UI dialogs + git hooks),
    // and letting claude-code prompt in parallel would double-gate, leak
    // prompts into stream-json (never reaching our UI), and hang the agent.
    //
    // Rain (EYES) is review-only and must be MECHANICALLY unable to mutate.
    // A prompt instruction alone failed (2026-05-28: Rain ran Edit + git
    // commit + gh issue create on a client repo). `--dangerously-skip-
    // permissions` (bypass mode) CANNOT be used to enforce this because bypass
    // mode disables the permission layer entirely — deny rules are ignored.
    // Instead: `dontAsk` (no prompts, deny-by-default) + an allowlist of read-
    // only tools + an explicit denylist of the mutation surface. Deny wins
    // over allow, so `Bash` is allowed wholesale for read-only investigation
    // while mutating git/gh invocations are blocked (verified: colon-form
    // `Bash(cmd:*)` matching holds under dontAsk on claude 2.1.x). The
    // internal MCP server `bot-hq-signaling` is allowed as a unit; its
    // HANDS-only tools are gated server-side (signaling/jsonrpc.rs).
    if cfg.agent_name == "rain" {
        // Rain reaches her model through a third-party Anthropic-compatible
        // gateway (DeepSeek, via ANTHROPIC_BASE_URL). claude-code >= 2.1.156
        // serializes a SessionStart hook's `additionalContext` (the user's
        // superpowers plugin injects one) as a `role:"system"` entry inside
        // the request's `messages` array. The real Anthropic API tolerates
        // that; DeepSeek's gateway only accepts user/assistant roles and
        // rejects it ("unknown variant `system`, expected user or assistant"
        // → API Error 400). The LOAD-BEARING fix is the local normalizing
        // proxy (`agents::llm_proxy`): Rain's ANTHROPIC_BASE_URL routes
        // through it and EVERY role:"system" entry in `messages[]` is hoisted
        // into the top-level `system` field before it reaches DeepSeek —
        // source-agnostic, so it also catches the plugin-sync injection that
        // running full (non-bare) mode brings back.
        //
        // We deliberately do NOT pass `--bare`. `--bare` (minimal mode,
        // CLAUDE_CODE_SIMPLE=1) was once kept as belt-and-suspenders against
        // that injection, but it ALSO disables claude-code's deferred-tool
        // loader (`ToolSearch`) — which left Rain with Grep/Glob/WebFetch/
        // ToolSearch/TodoWrite all inert ("exists but is not enabled in this
        // context"), i.e. her whole read-investigation surface beyond Read/
        // Bash. Since the proxy already neutralizes the role:"system"
        // injection --bare was guarding against, dropping --bare restores the
        // tool loader at no safety cost. Auth + routing are unaffected:
        // ANTHROPIC_AUTH_TOKEN + ANTHROPIC_BASE_URL are set as env below
        // regardless of mode. Read-only enforcement lives in `dontAsk` + the
        // allow/deny lists, NOT in --bare. (Trade-off: without --bare Rain now
        // syncs plugins + autodiscovers CLAUDE.md/auto-memory like Brian —
        // heavier startup; suppress per-agent via the override env if needed.)
        cmd.args(["--permission-mode", "dontAsk"]);
        cmd.args([
            "--allowedTools",
            "Read Grep Glob WebFetch WebSearch ToolSearch TodoWrite BashOutput KillShell Bash mcp__bot-hq-signaling",
        ]);
        // `gh` is denied by WRITE VERB, not blanket noun — so Rain keeps the
        // read forms the issue asks for (`gh issue view/list`, `gh pr view/diff/
        // list/status/checks`, `gh repo view`, `gh release view/list`) while every
        // mutating subcommand is blocked. Deny wins over allow, so a blanket
        // `Bash(gh issue:*)` would also kill `gh issue view`; enumerating write
        // verbs is the only way to allow reads under `dontAsk`. `gh api` stays
        // FULLY denied — it's the escape hatch that can POST/PATCH/DELETE
        // anything. New gh write verbs must be appended here (covered by
        // `rain_denies_gh_write_allows_gh_read`).
        //
        // `git branch` uses the SAME deny-by-write-verb shape (2026-06-17): the
        // blanket `Bash(git branch:*)` also blocked read-only listing (10+ false
        // denials on legit `git branch --show-current`/`-a` reads + compound
        // `git branch … && echo …` across the cross-model survey sessions). Now
        // only the mutating forms (-d/-D/-m/-c/-f/--set-upstream-to/--track/…)
        // are denied; read forms fall through to the allowed `Bash`. Residual:
        // bare `git branch <new>` creation — same accepted class as the gh
        // side-channels (covered by `rain_denies_git_branch_write_allows_read`).
        cmd.args([
            "--disallowedTools",
            "Edit Write NotebookEdit Task \
             Bash(git commit:*) Bash(git push:*) Bash(git branch -d:*) Bash(git branch -D:*) Bash(git branch --delete:*) Bash(git branch -m:*) Bash(git branch -M:*) Bash(git branch --move:*) Bash(git branch -c:*) Bash(git branch -C:*) Bash(git branch --copy:*) Bash(git branch -f:*) Bash(git branch --force:*) Bash(git branch -u:*) Bash(git branch --set-upstream-to:*) Bash(git branch --unset-upstream:*) Bash(git branch --track:*) Bash(git branch --no-track:*) Bash(git branch --edit-description:*) \
             Bash(git checkout:*) Bash(git switch:*) Bash(git reset:*) \
             Bash(git merge:*) Bash(git rebase:*) Bash(git add:*) \
             Bash(git stash:*) Bash(git restore:*) Bash(git rm:*) \
             Bash(git tag:*) Bash(git cherry-pick:*) Bash(git apply:*) \
             Bash(gh pr create:*) Bash(gh pr edit:*) Bash(gh pr close:*) \
             Bash(gh pr reopen:*) Bash(gh pr merge:*) Bash(gh pr ready:*) \
             Bash(gh pr review:*) Bash(gh pr comment:*) Bash(gh pr lock:*) \
             Bash(gh pr unlock:*) Bash(gh pr delete:*) Bash(gh pr checkout:*) \
             Bash(gh issue create:*) Bash(gh issue edit:*) Bash(gh issue close:*) \
             Bash(gh issue reopen:*) Bash(gh issue comment:*) Bash(gh issue delete:*) \
             Bash(gh issue transfer:*) Bash(gh issue pin:*) Bash(gh issue unpin:*) \
             Bash(gh issue lock:*) Bash(gh issue unlock:*) Bash(gh issue develop:*) \
             Bash(gh release create:*) Bash(gh release edit:*) Bash(gh release delete:*) \
             Bash(gh release upload:*) Bash(gh release download:*) \
             Bash(gh repo create:*) Bash(gh repo edit:*) Bash(gh repo delete:*) \
             Bash(gh repo fork:*) Bash(gh repo sync:*) Bash(gh repo rename:*) \
             Bash(gh repo archive:*) Bash(gh repo clone:*) \
             Bash(gh api:*)",
        ]);
    } else {
        cmd.arg("--dangerously-skip-permissions");

        // Mechanical backstop for HANDS. It runs in bypass mode, where
        // claude-code's native deny rules are IGNORED — so the only thing that
        // can hard-stop an outward/mutating command is a hook. Inject a
        // PreToolUse Bash hook that calls back into THIS binary's `policy-check
        // tool-gate` to match each Bash command against the GLOBAL Tool Gate
        // keyword config BEFORE it executes: a `gate` keyword blocks the direct
        // call (exit 2) and routes the agent to the `action_gate` MCP tool,
        // which surfaces Approve/Reject and runs the command on approval; an
        // `auto_allow`/unmatched command is allowed through. This replaces the
        // per-project `tool_blocklist` role after the 2026-05-29 fabricated-
        // comment incident. Rain is exempt: this hook is injected only here in
        // the HANDS branch, and she's already mechanically read-only via the
        // deny list above (her mutation surface is blocked regardless of any
        // hook). Injected via `--settings` (a process arg) so NOTHING is
        // written into the working repo's tree — disguise-safe for client repos.
        match std::env::current_exe() {
            Ok(exe) => {
                let mut hook_cmd = format!(
                    "\"{}\" policy-check tool-gate --data-dir \"{}\"",
                    exe.display(),
                    cfg.data_dir.display(),
                );
                if let Some(project) = &cfg.project {
                    hook_cmd.push_str(&format!(" --project \"{project}\""));
                }
                hook_cmd.push_str(&format!(" --session \"{}\"", cfg.session_id));
                let mut settings = serde_json::json!({
                    "hooks": {
                        "PreToolUse": [{
                            "matcher": "Bash",
                            "hooks": [{ "type": "command", "command": hook_cmd }],
                        }],
                    }
                });
                // Fold in the agent's override fragment (skillOverrides /
                // enabledPlugins / ultracode). Built with serde_json so the
                // payload is always valid — avoids claude-code's silent-ignore
                // of malformed `--settings` in `-p` mode.
                if let serde_json::Value::Object(ref mut map) = settings {
                    for (k, v) in
                        crate::claude_config::overrides::settings_fragment(&agent_override)
                    {
                        map.insert(k, v);
                    }
                }
                cmd.args(["--settings", &settings.to_string()]);
            }
            Err(e) => warn!(
                agent = %cfg.agent_name,
                error = %e,
                "current_exe() failed — tool-gate PreToolUse hook NOT injected; \
                 falling back to prompt-level gating only"
            ),
        }
    }

    // Env-vars per ARCHITECTURE.md "Agents" section.
    cmd.env("ANTHROPIC_MODEL", &cfg.config.model_name);
    // BOT_HQ_SESSION_ID is read by the git pre-push hook to overlay
    // session-scoped approvals onto the resolved policy.
    cmd.env("BOT_HQ_SESSION_ID", &cfg.session_id);
    // BOT_HQ_AGENT lets the pre-push hook attribute the push-approval prompt to
    // the pushing agent (only HANDS/Brian pushes; Rain can't push).
    // All agents route through build_command, so this lands for brian/rain.
    cmd.env("BOT_HQ_AGENT", &cfg.agent_name);
    if let Some(token) = &cfg.config.auth_token {
        if !token.is_empty() {
            cmd.env("ANTHROPIC_AUTH_TOKEN", token);
        }
    }
    // Route a custom (non-Anthropic) gateway through the local normalizing
    // proxy so any `role:"system"` message claude-code injects at request-
    // build time is hoisted out before it reaches a stricter gateway that
    // would 400 on it (Rain → DeepSeek). See `agents::llm_proxy` for the full
    // rationale. Falls back to the raw base_url if the proxy didn't start.
    // Agents with no base_url (Brian → real first-party API) get no override
    // and never touch the proxy.
    if let Some(base) = crate::agents::llm_proxy::resolve_anthropic_base_url(
        cfg.config.base_url.as_deref(),
        crate::agents::llm_proxy::proxy_addr(),
    ) {
        cmd.env("ANTHROPIC_BASE_URL", base);
    }

    // Per-agent override env (effort / auto-memory / CLAUDE.md suppression).
    // Applied to ALL agents. The skill/plugin `--settings` fragments above are
    // Brian-only (Rain gets no --settings), but these ENV overrides are the
    // lever to keep Rain lean now that she no longer runs --bare.
    for (k, v) in crate::claude_config::overrides::env_vars(&agent_override) {
        cmd.env(k, v);
    }

    // Always pin the subprocess cwd. A repo-less session must not inherit
    // the app's own cwd — in dev that's the bot-hq repo itself, and the
    // claude-code child would adopt that repo's CLAUDE.md + user-scope
    // auto-memory as session context (observed bleed: s-79f8aafe quoted
    // stale memory). data_dir always exists by spawn time (paths.rs boot
    // init creates it).
    let wd = cfg.working_dir.as_deref().unwrap_or(&cfg.data_dir);
    cmd.current_dir(wd);

    cmd
}

/// Build the path-string form of the claude command for diagnostics / logging.
/// Not used by spawn; tests use it to assert flag set.
#[cfg(test)]
pub fn debug_command(cfg: &SpawnConfig) -> Vec<String> {
    let cmd = build_command(cfg);
    let std_cmd = cmd.as_std();
    let mut out = vec![std_cmd.get_program().to_string_lossy().to_string()];
    for arg in std_cmd.get_args() {
        out.push(arg.to_string_lossy().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::AgentConfig;
    use std::path::Path;

    fn cfg() -> SpawnConfig {
        SpawnConfig {
            agent_name: "brian".into(),
            config: AgentConfig {
                agent_name: "brian".into(),
                provider: "anthropic".into(),
                model_name: "claude-opus-4-7".into(),
                base_url: None,
                auth_token: Some("sk-test".into()),
                updated_at: String::new(),
            },
            system_prompt: "be terse".into(),
            mcp_config_path: Some(Path::new("/tmp/mcp.json").to_path_buf()),
            working_dir: Some(Path::new("/tmp/repo").to_path_buf()),
            claude_bin: Some("claude".into()),
            session_id: "test-session".into(),
            resume_session_id: None,
            project: Some("bcc-ad-manager-ad-exporter".into()),
            data_dir: Path::new("/tmp/data").to_path_buf(),
            session_effort: None,
            session_ultracode: None,
        }
    }

    #[test]
    fn agent_health_wire_strings() {
        // B2: the as_str values are the wire contract with the frontend
        // (session:agent_health payload + HealthDot styling) — lock them.
        assert_eq!(AgentHealth::Running.as_str(), "running");
        assert_eq!(AgentHealth::Retrying.as_str(), "retrying");
        assert_eq!(AgentHealth::Dead.as_str(), "dead");
    }

    #[test]
    fn transient_api_statuses_are_retryable() {
        for s in [408, 425, 429, 500, 502, 503, 504, 529] {
            assert!(is_transient_api_error(s), "{s} should be transient");
        }
    }

    #[test]
    fn permanent_api_statuses_are_not_retryable() {
        // 400 = the DeepSeek system-role rejection; auth/forbidden/not-found
        // and semantic 4xx never clear on a blind retry.
        for s in [400, 401, 403, 404, 409, 413, 422, 451] {
            assert!(!is_transient_api_error(s), "{s} should be permanent");
        }
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let p = RetryPolicy {
            max_retries: 5,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
        };
        assert_eq!(p.backoff(1), Duration::from_secs(2));
        assert_eq!(p.backoff(2), Duration::from_secs(4));
        assert_eq!(p.backoff(3), Duration::from_secs(8));
        assert_eq!(p.backoff(4), Duration::from_secs(16));
        assert_eq!(p.backoff(5), Duration::from_secs(30)); // 32 → capped
        assert_eq!(p.backoff(99), Duration::from_secs(30));
    }

    #[test]
    fn repo_less_spawn_falls_back_to_data_dir_cwd() {
        let mut c = cfg();
        c.working_dir = None;
        let cmd = build_command(&c);
        assert_eq!(cmd.as_std().get_current_dir(), Some(Path::new("/tmp/data")));
    }

    #[test]
    fn pinned_working_dir_wins_over_data_dir_fallback() {
        let cmd = build_command(&cfg());
        assert_eq!(cmd.as_std().get_current_dir(), Some(Path::new("/tmp/repo")));
    }

    #[test]
    fn overrides_merge_into_settings_and_env() {
        use crate::claude_config::{save_overrides, ClaudeOverrides, SkillVisibility};
        let dir = tempfile::tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        store
            .brian
            .skills
            .insert("note".into(), SkillVisibility::UserInvocableOnly);
        store.brian.effort = Some("high".into());
        save_overrides(dir.path(), &store).unwrap();

        let mut c = cfg(); // brian (non-rain → gets --settings)
        c.data_dir = dir.path().to_path_buf();

        // The injected --settings carries the override fragment alongside the hook.
        let args = debug_command(&c);
        let settings_arg = args
            .iter()
            .skip_while(|a| *a != "--settings")
            .nth(1)
            .expect("--settings present");
        assert!(
            settings_arg.contains("skillOverrides"),
            "got {settings_arg}"
        );
        assert!(
            settings_arg.contains("user-invocable-only"),
            "got {settings_arg}"
        );
        assert!(
            settings_arg.contains("PreToolUse"),
            "hook must survive merge"
        );

        // Effort override is injected as env.
        let cmd = build_command(&c);
        let has_effort = cmd.as_std().get_envs().any(|(k, v)| {
            k == std::ffi::OsStr::new("CLAUDE_CODE_EFFORT_LEVEL")
                && v == Some(std::ffi::OsStr::new("high"))
        });
        assert!(has_effort, "effort env should be set from override");
    }

    #[test]
    fn session_overrides_win_over_persistent() {
        use crate::claude_config::{save_overrides, ClaudeOverrides};
        let dir = tempfile::tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        store.brian.effort = Some("high".into()); // persistent default
        save_overrides(dir.path(), &store).unwrap();

        let mut c = cfg(); // brian (gets --settings)
        c.data_dir = dir.path().to_path_buf();
        c.session_effort = Some("max".into()); // per-session pick wins

        // Session effort beats the persistent "high".
        let cmd = build_command(&c);
        let has_max = cmd.as_std().get_envs().any(|(k, v)| {
            k == std::ffi::OsStr::new("CLAUDE_CODE_EFFORT_LEVEL")
                && v == Some(std::ffi::OsStr::new("max"))
        });
        assert!(has_max, "session effort should win over persistent override");
    }

    #[test]
    fn session_ultracode_clears_inherited_max_effort() {
        // Cross-layer collision: persistent effort=max + a per-session ultracode
        // pick (session effort left on Inherit). Session ultracode wins; the
        // inherited max must NOT also reach env (the documented exclusion).
        use crate::claude_config::{save_overrides, ClaudeOverrides};
        let dir = tempfile::tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        store.brian.effort = Some("max".into()); // persistent
        save_overrides(dir.path(), &store).unwrap();

        let mut c = cfg();
        c.data_dir = dir.path().to_path_buf();
        c.session_ultracode = Some(true); // session pick; session_effort stays None

        let cmd = build_command(&c);
        let has_effort_env = cmd
            .as_std()
            .get_envs()
            .any(|(k, _)| k == std::ffi::OsStr::new("CLAUDE_CODE_EFFORT_LEVEL"));
        assert!(
            !has_effort_env,
            "inherited max effort must be cleared when the session enables ultracode"
        );

        let args = debug_command(&c);
        let settings_arg = args
            .iter()
            .skip_while(|a| *a != "--settings")
            .nth(1)
            .expect("--settings present");
        assert!(settings_arg.contains("ultracode"), "got {settings_arg}");
    }

    #[test]
    fn session_max_clears_inherited_ultracode() {
        // Reverse collision: persistent ultracode + a per-session effort=max pick
        // (session ultracode left on Inherit). The session's explicit max wins;
        // ultracode must NOT also reach --settings.
        use crate::claude_config::{save_overrides, ClaudeOverrides};
        let dir = tempfile::tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        store.brian.ultracode = Some(true); // persistent
        save_overrides(dir.path(), &store).unwrap();

        let mut c = cfg();
        c.data_dir = dir.path().to_path_buf();
        c.session_effort = Some("max".into()); // session pick; session_ultracode stays None

        let cmd = build_command(&c);
        let has_max = cmd.as_std().get_envs().any(|(k, v)| {
            k == std::ffi::OsStr::new("CLAUDE_CODE_EFFORT_LEVEL")
                && v == Some(std::ffi::OsStr::new("max"))
        });
        assert!(has_max, "session effort=max should reach env");

        // brian always gets a --settings (the PreToolUse hook); assert the
        // fragment no longer carries ultracode (the persistent one was cleared).
        let args = debug_command(&c);
        let settings_arg = args
            .iter()
            .skip_while(|a| *a != "--settings")
            .nth(1)
            .expect("--settings present");
        assert!(
            !settings_arg.contains("ultracode"),
            "ultracode must be cleared by the explicit max pick; got {settings_arg}"
        );
    }

    #[test]
    fn rain_gets_override_env_but_no_settings_fragment() {
        use crate::claude_config::{save_overrides, ClaudeOverrides};
        let dir = tempfile::tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        store.all.disable_auto_memory = Some(true); // fan-out default
        save_overrides(dir.path(), &store).unwrap();

        let mut c = cfg();
        c.agent_name = "rain".into();
        c.config.agent_name = "rain".into();
        c.data_dir = dir.path().to_path_buf();

        let args = debug_command(&c);
        assert!(
            !args.iter().any(|a| a == "--settings"),
            "rain gets no --settings (the tool-gate PreToolUse hook is Brian-only)"
        );
        // env-based overrides still apply to Rain.
        let cmd = build_command(&c);
        let has = cmd.as_std().get_envs().any(|(k, v)| {
            k == std::ffi::OsStr::new("CLAUDE_CODE_DISABLE_AUTO_MEMORY")
                && v == Some(std::ffi::OsStr::new("1"))
        });
        assert!(has, "auto-memory disable env should apply to Rain too");
    }

    // ---- supervisor retry logic (fake incarnations, no real subprocess) ----

    /// A fake `AgentHandle` whose event stream the test drives directly. Push
    /// events via `ev_tx`; close the incarnation by dropping it. Observe the
    /// resume-nudge (and any peer input) via `in_rx`.
    fn fake_incarnation() -> (
        AgentHandle,
        mpsc::Sender<AgentEvent>,
        mpsc::Receiver<OutgoingUserMessage>,
    ) {
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(16);
        let (in_tx, in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (kill_tx, _kill_rx) = oneshot::channel::<()>();
        let handle = AgentHandle {
            name: "fake".into(),
            event_rx: ev_rx,
            input_tx: in_tx,
            kill_tx: Some(kill_tx),
        };
        (handle, ev_tx, in_rx)
    }

    fn errored_turn(status: u16) -> AgentEvent {
        AgentEvent::TurnComplete {
            stop_reason: None,
            subtype: Some("error_during_execution".into()),
            is_error: true,
            api_error_status: Some(status),
        }
    }

    fn clean_turn() -> AgentEvent {
        AgentEvent::TurnComplete {
            stop_reason: Some("end_turn".into()),
            subtype: Some("success".into()),
            is_error: false,
            api_error_status: None,
        }
    }

    fn instant_policy(max_retries: u32) -> RetryPolicy {
        RetryPolicy {
            max_retries,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn supervisor_resumes_after_transient_then_stops_clean() {
        let (h1, ev1, _in1) = fake_incarnation();
        let (h2, ev2, mut in2) = fake_incarnation();

        let mut queue = std::collections::VecDeque::new();
        queue.push_back(h2);
        let spawn_next = move |_c: SpawnConfig| {
            let h = queue.pop_front().expect("unexpected extra respawn");
            async move { Ok(h) }
        };

        let (out_ev_tx, mut out_ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (_out_in_tx, out_in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (_kill_tx, kill_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(supervise(
            cfg(),
            instant_policy(5),
            h1,
            out_ev_tx,
            out_in_rx,
            kill_rx,
            spawn_next,
        ));

        // Incarnation 1 hits a transient 529, then exits.
        ev1.send(errored_turn(529)).await.unwrap();
        drop(ev1);

        // The resumed incarnation is nudged to continue.
        let nudge = in2
            .recv()
            .await
            .expect("resumed incarnation should be nudged");
        assert!(
            nudge.message.content.contains("529"),
            "nudge names the status"
        );
        assert!(nudge.message.content.to_lowercase().contains("resumed"));

        // Incarnation 2 does real work and finishes cleanly.
        ev2.send(AgentEvent::Text("resumed work".into()))
            .await
            .unwrap();
        ev2.send(clean_turn()).await.unwrap();
        drop(ev2);

        task.await.unwrap();

        let mut got = Vec::new();
        while let Some(ev) = out_ev_rx.recv().await {
            got.push(ev);
        }
        assert!(
            matches!(
                got.first(),
                Some(AgentEvent::TurnComplete { is_error: true, .. })
            ),
            "errored turn is forwarded to the peer pump"
        );
        assert!(got
            .iter()
            .any(|e| matches!(e, AgentEvent::Text(t) if t == "resumed work")));
        assert!(got.iter().any(|e| matches!(
            e,
            AgentEvent::TurnComplete {
                is_error: false,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn supervisor_terminates_when_incarnation_input_pump_dies() {
        // The incarnation's stdin pump death = its input receiver dropped, while
        // its EVENT channel stays open (child still emitting). The supervisor must
        // NOT bridge to a now-deaf child forever (the #4 user→HANDS desync) — it
        // tears down so the public input channel closes (the is_stale signal),
        // WITHOUT a respawn-in-place.
        let (h1, _ev1, in1) = fake_incarnation();
        drop(in1); // kill the incarnation's stdin pump (receiver gone)

        let mut queue: std::collections::VecDeque<AgentHandle> = std::collections::VecDeque::new();
        let spawn_next = move |_c: SpawnConfig| {
            let h = queue
                .pop_front()
                .expect("input-pump death must NOT trigger a respawn-in-place");
            async move { Ok(h) }
        };

        let (out_ev_tx, _out_ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (out_in_tx, out_in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (_kill_tx, kill_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(supervise(
            cfg(),
            instant_policy(5),
            h1,
            out_ev_tx,
            out_in_rx,
            kill_rx,
            spawn_next,
        ));

        // A user message arrives; forwarding it to the dead incarnation pump
        // fails, which must terminate the supervisor. `_ev1` is kept alive so the
        // event channel stays OPEN — only the input-pump path can end the loop.
        out_in_tx
            .send(OutgoingUserMessage::text("hello"))
            .await
            .unwrap();

        task.await.unwrap();

        assert!(
            out_in_tx.is_closed(),
            "input-pump death must terminate the supervisor so the session goes stale"
        );
    }

    #[tokio::test]
    async fn supervisor_does_not_resume_permanent_error() {
        let (h1, ev1, _in1) = fake_incarnation();
        // Empty queue: any respawn pops-and-panics, failing the test.
        let mut queue: std::collections::VecDeque<AgentHandle> = std::collections::VecDeque::new();
        let spawn_next = move |_c: SpawnConfig| {
            let h = queue
                .pop_front()
                .expect("permanent error must NOT trigger a respawn");
            async move { Ok(h) }
        };

        let (out_ev_tx, mut out_ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (_out_in_tx, out_in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (_kill_tx, kill_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(supervise(
            cfg(),
            instant_policy(5),
            h1,
            out_ev_tx,
            out_in_rx,
            kill_rx,
            spawn_next,
        ));

        ev1.send(errored_turn(400)).await.unwrap(); // permanent
        drop(ev1);

        task.await.unwrap(); // returns without a respawn

        // A terminated supervisor drops its input receiver, so the stable
        // sender now reads closed — exactly the signal `SessionHandle::is_stale`
        // uses to evict + re-spawn a crashed session.
        assert!(
            _out_in_tx.is_closed(),
            "terminated supervisor must close the input channel (is_stale signal)"
        );

        let mut got = Vec::new();
        while let Some(ev) = out_ev_rx.recv().await {
            got.push(ev);
        }
        assert!(got.iter().any(|e| matches!(
            e,
            AgentEvent::TurnComplete {
                is_error: true,
                api_error_status: Some(400),
                ..
            }
        )));
        assert!(
            !got.iter()
                .any(|e| matches!(e, AgentEvent::Text(t) if t.contains("Stopped after"))),
            "permanent error must not emit the transient give-up message"
        );
    }

    #[tokio::test]
    async fn supervisor_gives_up_after_max_retries() {
        // max_retries = 2 → initial + 2 respawns = 3 incarnations, then surface.
        let (h1, ev1, _in1) = fake_incarnation();
        let (h2, ev2, mut in2) = fake_incarnation();
        let (h3, ev3, mut in3) = fake_incarnation();

        let mut queue = std::collections::VecDeque::new();
        queue.push_back(h2);
        queue.push_back(h3);
        let spawn_next = move |_c: SpawnConfig| {
            let h = queue.pop_front().expect("more respawns than budget allows");
            async move { Ok(h) }
        };

        let (out_ev_tx, mut out_ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (_out_in_tx, out_in_rx) = mpsc::channel::<OutgoingUserMessage>(16);
        let (_kill_tx, kill_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(supervise(
            cfg(),
            instant_policy(2),
            h1,
            out_ev_tx,
            out_in_rx,
            kill_rx,
            spawn_next,
        ));

        ev1.send(errored_turn(529)).await.unwrap();
        drop(ev1);
        in2.recv().await.expect("nudge to incarnation 2");
        ev2.send(errored_turn(503)).await.unwrap();
        drop(ev2);
        in3.recv().await.expect("nudge to incarnation 3");
        ev3.send(errored_turn(529)).await.unwrap();
        drop(ev3);

        task.await.unwrap();

        let mut got = Vec::new();
        while let Some(ev) = out_ev_rx.recv().await {
            got.push(ev);
        }
        assert!(
            got.iter().any(|e| matches!(
                e,
                AgentEvent::Text(t) if t.contains("Stopped after") && t.contains('2')
            )),
            "expected the give-up message after exhausting 2 retries; got {got:?}"
        );
    }

    #[test]
    fn command_has_required_flags() {
        let argv = debug_command(&cfg());
        assert_eq!(argv[0], "claude");
        assert!(argv.iter().any(|a| a == "-p"));
        assert!(argv.iter().any(|a| a == "--verbose"));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--input-format" && w[1] == "stream-json"));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--output-format" && w[1] == "stream-json"));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--mcp-config" && w[1] == "/tmp/mcp.json"));
        assert!(argv.iter().any(|a| a == "--strict-mcp-config"));
        assert!(argv.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--append-system-prompt" && w[1] == "be terse"));
        // No resume flag when SpawnConfig.resume_session_id is None.
        assert!(!argv.iter().any(|a| a == "--resume"));
    }

    #[test]
    fn rain_gets_deny_by_default_not_bypass() {
        // EYES enforcement: Rain must NOT get bypass mode (which nullifies
        // deny rules); she gets dontAsk + an allowlist + a mutation denylist.
        let mut c = cfg();
        c.agent_name = "rain".into();
        c.config.agent_name = "rain".into();
        let argv = debug_command(&c);

        assert!(
            !argv.iter().any(|a| a == "--dangerously-skip-permissions"),
            "Rain must not run in bypass mode (it ignores deny rules): {argv:?}"
        );
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "--permission-mode" && w[1] == "dontAsk"),
            "expected `--permission-mode dontAsk`: {argv:?}"
        );
        // Allowlist keeps read-only investigation + the signaling MCP.
        let allowed = argv
            .windows(2)
            .find(|w| w[0] == "--allowedTools")
            .map(|w| w[1].clone())
            .expect("--allowedTools present");
        // Web/reference tools must match Rain's role prompt (prompts.rs) — the
        // prompt promises WebFetch/WebSearch/ToolSearch, so the allowlist must
        // grant all three or claude-code silently blocks what the prompt offers.
        for t in [
            "Read",
            "Grep",
            "Glob",
            "Bash",
            "mcp__bot-hq-signaling",
            "WebFetch",
            "WebSearch",
            "ToolSearch",
        ] {
            assert!(allowed.contains(t), "allowlist missing {t}: {allowed}");
        }
        // Denylist covers the mutation surface from the 2026-05-28 incident.
        let denied = argv
            .windows(2)
            .find(|w| w[0] == "--disallowedTools")
            .map(|w| w[1].clone())
            .expect("--disallowedTools present");
        for t in [
            "Edit",
            "Write",
            "NotebookEdit",
            "Bash(git commit:*)",
            "Bash(git push:*)",
            "Bash(gh issue create:*)",
            "Bash(gh pr merge:*)",
        ] {
            assert!(denied.contains(t), "denylist missing {t}: {denied}");
        }
    }

    #[test]
    fn rain_denies_gh_write_allows_gh_read() {
        // Issue (2026-06-05): Rain should keep read-only `gh` (view/list/diff)
        // while every mutating `gh` form stays blocked. Deny wins over allow, so
        // the denylist must NOT contain a blanket `gh <noun>:*` (that would also
        // kill the read forms) and MUST enumerate the write verbs.
        let mut c = cfg();
        c.agent_name = "rain".into();
        c.config.agent_name = "rain".into();
        let argv = debug_command(&c);
        let denied = argv
            .windows(2)
            .find(|w| w[0] == "--disallowedTools")
            .map(|w| w[1].clone())
            .expect("--disallowedTools present");

        // Every mutating gh verb is blocked.
        for t in [
            "Bash(gh pr create:*)",
            "Bash(gh pr edit:*)",
            "Bash(gh pr close:*)",
            "Bash(gh pr merge:*)",
            "Bash(gh pr comment:*)",
            "Bash(gh pr checkout:*)",
            "Bash(gh issue create:*)",
            "Bash(gh issue edit:*)",
            "Bash(gh issue close:*)",
            "Bash(gh issue comment:*)",
            "Bash(gh issue delete:*)",
            "Bash(gh release create:*)",
            "Bash(gh release delete:*)",
            "Bash(gh repo create:*)",
            "Bash(gh repo delete:*)",
            "Bash(gh repo clone:*)",
            // The escape hatch — gh api can POST/PATCH/DELETE anything.
            "Bash(gh api:*)",
        ] {
            assert!(
                denied.contains(t),
                "gh write verb not denied: {t}\n{denied}"
            );
        }

        // No blanket noun deny survives (it would block the read forms).
        for blanket in [
            "Bash(gh pr:*)",
            "Bash(gh issue:*)",
            "Bash(gh repo:*)",
            "Bash(gh release:*)",
        ] {
            assert!(
                !denied.contains(blanket),
                "blanket gh deny would block read forms: {blanket}"
            );
        }

        // Read forms have no dedicated deny entry, so they fall through to the
        // allowed `Bash` (a `view`/`list`/`diff` substring must not appear as a
        // denied pattern).
        for read in [
            "Bash(gh issue view:*)",
            "Bash(gh pr view:*)",
            "Bash(gh pr diff:*)",
            "Bash(gh repo view:*)",
        ] {
            assert!(
                !denied.contains(read),
                "read form should not be explicitly denied: {read}"
            );
        }
    }

    #[test]
    fn rain_denies_git_branch_write_allows_read() {
        // 2026-06-17 cross-model survey: the blanket `Bash(git branch:*)` deny
        // blocked read-only listing too — DeepSeek-EYES hit 10+ false denials on
        // legit `git branch --show-current`/`-a` reads (incl. compound
        // `git branch … && echo …`). Mirror the gh deny-by-write-verb shape: only
        // mutating git-branch forms denied, read forms fall through to allowed Bash.
        let mut c = cfg();
        c.agent_name = "rain".into();
        c.config.agent_name = "rain".into();
        let argv = debug_command(&c);
        let denied = argv
            .windows(2)
            .find(|w| w[0] == "--disallowedTools")
            .map(|w| w[1].clone())
            .expect("--disallowedTools present");

        // Every mutating git-branch form is blocked.
        for t in [
            "Bash(git branch -d:*)",
            "Bash(git branch -D:*)",
            "Bash(git branch --delete:*)",
            "Bash(git branch -m:*)",
            "Bash(git branch -c:*)",
            "Bash(git branch -f:*)",
            "Bash(git branch --force:*)",
            "Bash(git branch --set-upstream-to:*)",
            "Bash(git branch --track:*)",
        ] {
            assert!(
                denied.contains(t),
                "git branch write form not denied: {t}\n{denied}"
            );
        }

        // The blanket noun deny must NOT survive (it blocked read-only listing).
        assert!(
            !denied.contains("Bash(git branch:*)"),
            "blanket git branch deny would block read forms: {denied}"
        );

        // Read forms have no dedicated deny entry — they fall through to allowed Bash.
        for read in ["Bash(git branch --show-current:*)", "Bash(git branch -a:*)"] {
            assert!(
                !denied.contains(read),
                "read form should not be explicitly denied: {read}"
            );
        }
    }

    #[test]
    fn brian_still_gets_bypass() {
        // HANDS keeps full power — bypass mode, no allow/deny lists.
        let argv = debug_command(&cfg()); // cfg() is brian
        assert!(argv.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(!argv.iter().any(|a| a == "--permission-mode"));
        assert!(!argv.iter().any(|a| a == "--allowedTools"));
        assert!(!argv.iter().any(|a| a == "--disallowedTools"));
        // Brian hits the real Anthropic API, which tolerates the system-role
        // message claude-code injects from plugin SessionStart hooks, so he
        // does NOT need --bare (and would lose CLAUDE.md/LSP if he had it).
        assert!(!argv.iter().any(|a| a == "--bare"));
    }

    #[test]
    fn rain_runs_without_bare_so_tool_loader_works() {
        // Rain must NOT run `--bare`. `--bare` (CLAUDE_CODE_SIMPLE=1) disables
        // claude-code's deferred-tool loader (`ToolSearch`), which left Rain's
        // Grep/Glob/WebFetch/ToolSearch/TodoWrite inert ("exists but is not
        // enabled in this context") — her whole read surface beyond Read/Bash.
        // The role:"system" injection --bare once guarded against is
        // neutralized by `llm_proxy` (it hoists every such entry out of
        // `messages[]` into the top-level `system` field), so dropping --bare
        // restores the tool surface at no safety cost.
        let mut c = cfg();
        c.agent_name = "rain".into();
        c.config.agent_name = "rain".into();
        let argv = debug_command(&c);
        assert!(
            !argv.iter().any(|a| a == "--bare"),
            "Rain must NOT run --bare (it disables the ToolSearch tool loader); \
             the llm_proxy handles the role:system injection instead: {argv:?}"
        );
    }

    #[test]
    fn resume_session_id_emits_resume_flag() {
        let mut c = cfg();
        c.resume_session_id = Some("abc-123-uuid".into());
        let argv = debug_command(&c);
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "--resume" && w[1] == "abc-123-uuid"),
            "expected `--resume abc-123-uuid` in argv: {argv:?}"
        );
    }

    #[test]
    fn brian_gets_tool_gate_pretooluse_hook() {
        let argv = debug_command(&cfg()); // cfg() is brian
        let settings = argv
            .windows(2)
            .find(|w| w[0] == "--settings")
            .map(|w| w[1].clone())
            .expect("brian must get --settings carrying the PreToolUse hook");
        assert!(settings.contains("PreToolUse"), "settings: {settings}");
        assert!(
            settings.contains("policy-check tool-gate"),
            "hook must call the gate subcommand: {settings}"
        );
        assert!(
            settings.contains("bcc-ad-manager-ad-exporter"),
            "hook must be bound to the session's project: {settings}"
        );
        assert!(
            settings.contains("\"matcher\":\"Bash\""),
            "hook must match the Bash tool: {settings}"
        );
    }

    #[test]
    fn rain_does_not_get_tool_gate_hook() {
        // The tool-gate PreToolUse hook is injected via --settings in the HANDS
        // (Brian) branch only; Rain is already mechanically read-only via the
        // deny list, so she gets no --settings at all.
        let mut c = cfg();
        c.agent_name = "rain".into();
        c.config.agent_name = "rain".into();
        let argv = debug_command(&c);
        assert!(
            !argv.iter().any(|a| a == "--settings"),
            "Rain must NOT get --settings: {argv:?}"
        );
    }
}
