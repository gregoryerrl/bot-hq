//! Spawn a Codex participant: `codex app-server` over stdio, the start
//! sequence (initialize → hooks/list → thread/start|resume), and the driver
//! that turns the handle's channels into turns and the server's notifications
//! into [`AgentEvent`]s. Returns the same [`AgentHandle`] claude-code's spawn
//! returns, so the retry supervisor and the participant's pump drive it
//! unchanged.
//!
//! Posture: `sandbox_mode danger-full-access` + `approval_policy never` (the
//! `--dangerously-skip-permissions` of this connector), with EVERY gate in
//! bot-hq's own hook (`policy-check codex-gate`, file-resident in the managed
//! home, trusted per spawn through `hooks.state`). The pinned feature set
//! ([`DISABLED_FEATURES`]) removes what the hook cannot see or bot-hq cannot
//! bill for: tty sessions (`write_stdin` would carry commands past
//! PreToolUse — measured, s-de77e2f0 G3), Codex sub-agents, app connectors,
//! plugins, goals, image generation, worktrees, browsers, the desktop
//! surfaces, self-installing dependencies and the unbounded retries that
//! hang a turn on a dropped connection.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use super::client::{CodexClient, Incoming, RequestFailure};
use super::events::{Translator, CODEX_MCP_SERVER_NAME};
use super::home::{hooks_state_override, write_hooks_json, HookTrust, ListedHook, RenderedHook};
use crate::agents::protocol::{ControlRequest, OutgoingUserMessage};
use crate::agents::spawn::{AgentEvent, AgentHandle, AgentHealth, SpawnConfig, CHILD_PIDS};

/// What a Codex spawn needs beyond the shared [`SpawnConfig`]: resolved by
/// `core::session::participant_spawn_config` for a `codex` row, `None` for a
/// claude-code row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSpawn {
    /// The `codex` binary (for tests). Defaults to `"codex"`.
    pub bin: Option<String>,
    /// bot-hq's MCP endpoint for this participant, WITHOUT the secret: the
    /// secret rides the `x-bot-hq-mcp-token` header, sourced from the
    /// `BOT_HQ_MCP_TOKEN` env var, so it never sits on argv.
    pub mcp_url: String,
    pub mcp_token: String,
    /// The trust grant for bot-hq's hook in the home, as `hooks/list` reported
    /// it at the probe (empty on the very first spawn into a home — the start
    /// sequence then refuses with the reason, and the probe's cache fills it
    /// for the next spawn).
    pub hook_trust: Vec<HookTrust>,
    /// The binary the gate hook runs (for tests, whose `current_exe` is the
    /// test harness). Defaults to this process's own.
    pub hook_exe: Option<std::path::PathBuf>,
}

/// The Codex features bot-hq turns off for every participant (plan P3 table).
pub const DISABLED_FEATURES: &[&str] = &[
    "unified_exec_tty",
    "multi_agent",
    "multi_agent_v2",
    "collaboration_modes",
    "apps",
    "tool_suggest",
    "plugins",
    "plugin_sharing",
    "remote_plugin",
    "recommended_plugins",
    "goals",
    "image_generation",
    "worktrees",
    "in_app_local_automation",
    "in_app_browser",
    "in_app_voice",
    "realtime_conversation",
    "workspace_dependencies",
    "skill_mcp_dependency_install",
    "unbounded_connection_retries",
    "computer_use",
    "browser_use",
    "browser_use_external",
    "browser_use_full_cdp_access",
    "browser_annotation_api",
    "memories",
];

/// Inherited environment that would pick the child's credential or billing:
/// the API keys (an API key in the environment bills the API, not the
/// subscription), the base URL, and the home pointer itself — bot-hq sets
/// its own. Every inherited `CODEX_*` variable goes too (`CODEX_THREAD_ID`,
/// `CODEX_CI`, … leak in when bot-hq is launched from a Codex shell).
pub const CODEX_AUTH_ENV_SCRUB: &[&str] = &["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL", "CODEX_HOME"];
/// The `CODEX_*` names a Codex shell exports, removed by name as well as by
/// prefix so the removal is pinned whether or not the parent carries them.
pub const CODEX_SHELL_EXPORTS: &[&str] = &[
    "CODEX_THREAD_ID",
    "CODEX_SESSION_ID",
    "CODEX_TOOL_CALL_ID",
    "CODEX_CI",
    "CODEX_VERSION",
    "CODEX_MANAGED_BY_NPM",
    "CODEX_MANAGED_PACKAGE_ROOT",
    "CODEX_APPLY_PATCH_PRESERVE_LINE_ENDINGS",
];

/// The connector's name for the MCP server it registers.
pub fn mcp_server_name() -> &'static str {
    CODEX_MCP_SERVER_NAME
}

/// The `-c key=value` overrides a participant is spawned with.
pub fn config_overrides(cfg: &SpawnConfig, codex: &CodexSpawn) -> Vec<String> {
    let server = CODEX_MCP_SERVER_NAME;
    let mut out = vec![
        "approval_policy=\"never\"".to_string(),
        "sandbox_mode=\"danger-full-access\"".to_string(),
        format!("mcp_servers.{server}.url={}", toml_str(&codex.mcp_url)),
        format!("mcp_servers.{server}.env_http_headers={{\"{}\"=\"BOT_HQ_MCP_TOKEN\"}}", crate::signaling::MCP_TOKEN_HEADER),
        format!("mcp_servers.{server}.omit_tools_from=[\"deferred\",\"code_mode\"]"),
        format!("mcp_servers.{server}.default_tools_approval_mode=\"approve\""),
        format!("mcp_servers.{server}.required=true"),
        "project_doc_fallback_filenames=[\"CLAUDE.md\"]".to_string(),
        "shell_environment_policy.inherit=\"all\"".to_string(),
        "shell_environment_policy.exclude=[\"BOT_HQ_MCP_TOKEN\"]".to_string(),
        "history.persistence=\"none\"".to_string(),
        "web_search=\"live\"".to_string(),
        "memories.generate_memories=false".to_string(),
        "memories.use_memories=false".to_string(),
        hooks_state_override(&codex.hook_trust),
    ];
    if let Some(window) = cfg.config.context_window.filter(|w| *w > 0) {
        // The meter warns before Codex compacts: Codex's own limit sits where
        // the claude path's reserve does (`pump::COMPACT_RESERVE_TOKENS`).
        let limit = (window as u64).saturating_sub(COMPACT_RESERVE_TOKENS);
        out.push(format!("model_auto_compact_token_limit={limit}"));
    }
    out
}

/// The reserve the claude path keeps below the window before its context
/// rows fire (`core::pump::COMPACT_RESERVE_TOKENS`), applied to Codex's own
/// compaction point so the handoff rows come first.
pub const COMPACT_RESERVE_TOKENS: u64 = 33_000;

fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The command for one participant.
pub fn build_codex_command(cfg: &SpawnConfig, codex: &CodexSpawn) -> tokio::process::Command {
    let bin = codex.bin.clone().unwrap_or_else(|| "codex".to_string());
    let mut cmd = crate::process::tokio_command(&bin);
    crate::appimage_env::scrub_tokio(&mut cmd);
    // The inherited credential variables by name, and every inherited
    // `CODEX_*` by prefix (`CODEX_THREAD_ID`, `CODEX_CI`, … leak in when
    // bot-hq is launched from a Codex shell): the parent's environment is
    // read here, at build time.
    for name in CODEX_AUTH_ENV_SCRUB.iter().chain(CODEX_SHELL_EXPORTS.iter()) {
        cmd.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("CODEX_") {
            cmd.env_remove(&name);
        }
    }
    cmd.arg("app-server").arg("--listen").arg("stdio://");
    for feature in DISABLED_FEATURES {
        cmd.arg("--disable").arg(feature);
    }
    for override_ in config_overrides(cfg, codex) {
        cmd.arg("-c").arg(override_);
    }
    if let Some(home) = cfg.account_dir.as_deref().filter(|h| !h.is_empty()) {
        cmd.env("CODEX_HOME", home);
    }
    cmd.env("BOT_HQ_SESSION_ID", &cfg.session_id);
    cmd.env("BOT_HQ_AGENT", &cfg.agent_name);
    cmd.env("BOT_HQ_MCP_TOKEN", &codex.mcp_token);
    if let Some(dir) = &cfg.working_dir {
        cmd.current_dir(dir);
    }
    cmd
}

/// The argv, for tests and the log.
pub fn debug_codex_command(cfg: &SpawnConfig, codex: &CodexSpawn) -> Vec<String> {
    let cmd = build_codex_command(cfg, codex);
    let std_cmd = cmd.as_std();
    let mut out = vec![std_cmd.get_program().to_string_lossy().to_string()];
    out.extend(std_cmd.get_args().map(|a| a.to_string_lossy().to_string()));
    out
}

/// bot-hq's effort knob (`low|medium|high|max`, the claude vocabulary) on
/// Codex's scale: the same four names exist — `model/list` on 0.163.0
/// advertises `low, medium, high, xhigh, max, ultra` for both `gpt-6-astra`
/// and `gpt-6.1-sol` (measured, s-de77e2f0; an unadvertised value fails
/// every `turn/start`) — and `ultra` (automatic task delegation) is never
/// sent.
pub fn codex_effort(effort: Option<&str>) -> Option<&'static str> {
    match effort?.trim().to_ascii_lowercase().as_str() {
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "max" => Some("max"),
        _ => None,
    }
}

/// One `hooks/list` entry as [`ListedHook`] (the fields the trust rules read).
pub fn listed_hooks(result: &Value) -> Vec<ListedHook> {
    let mut out = Vec::new();
    for cwd in result["data"].as_array().into_iter().flatten() {
        for h in cwd["hooks"].as_array().into_iter().flatten() {
            out.push(ListedHook {
                key: h["key"].as_str().unwrap_or("").to_string(),
                event_name: h["eventName"].as_str().unwrap_or("").to_string(),
                matcher: h["matcher"].as_str().map(str::to_string),
                command: h["command"].as_str().unwrap_or("").to_string(),
                source_path: h["sourcePath"].as_str().unwrap_or("").to_string(),
                current_hash: h["currentHash"].as_str().unwrap_or("").to_string(),
                trust_status: h["trustStatus"].as_str().unwrap_or("").to_string(),
                enabled: h["enabled"].as_bool().unwrap_or(false),
            });
        }
    }
    out
}

/// The Codex version from `initialize`'s `userAgent`
/// (`bot-hq/0.163.0 (Mac OS …)`), or the whole string when it is not that shape.
pub fn version_from_user_agent(user_agent: &str) -> String {
    user_agent
        .split_whitespace()
        .next()
        .and_then(|first| first.split_once('/'))
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| user_agent.to_string())
}

/// The notifications the translator never reads, suppressed at `initialize`
/// (EYES 4295ae7d): the streamed deltas are the bulk of a turn's traffic and
/// carry nothing the transcript shows.
pub const OPT_OUT_NOTIFICATIONS: &[&str] = &[
    "item/agentMessage/delta",
    "item/reasoning/textDelta",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/summaryPartAdded",
    "item/commandExecution/outputDelta",
    "item/fileChange/outputDelta",
    "item/plan/delta",
    "remoteControl/status/changed",
];

/// `initialize` + `initialized`, returning the version the server reports.
pub async fn initialize(client: &CodexClient) -> Result<String> {
    let init = client
        .request(
            "initialize",
            json!({
                "clientInfo": { "name": "bot-hq", "title": "bot-hq", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "optOutNotificationMethods": OPT_OUT_NOTIFICATIONS },
            }),
        )
        .await?;
    client.notify("initialized", json!({})).await?;
    Ok(version_from_user_agent(init["userAgent"].as_str().unwrap_or("")))
}

/// Spawn one Codex participant. Fails — with the reason, posted by the caller
/// — when the home is missing, the hooks file cannot be written, the gate
/// hook is not armed, or the thread cannot be started.
pub async fn spawn_codex_agent(cfg: SpawnConfig) -> Result<AgentHandle> {
    let codex = cfg
        .codex
        .clone()
        .ok_or_else(|| anyhow!("{}: a Codex spawn needs its Codex settings", cfg.agent_name))?;
    let home = cfg
        .account_dir
        .clone()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| anyhow!("{}: a Codex row needs a Codex home of its own (Settings → Models)", cfg.agent_name))?;
    let exe = match &codex.hook_exe {
        Some(exe) => exe.clone(),
        None => std::env::current_exe().context("locating the bot-hq binary for the gate hook")?,
    };
    write_hooks_json(Path::new(&home), &exe, &cfg.data_dir)?;
    let rendered = RenderedHook::new(Path::new(&home), &exe, &cfg.data_dir);

    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(256);
    let (input_tx, input_rx) = mpsc::channel::<OutgoingUserMessage>(64);
    let (control_tx, control_rx) = mpsc::channel::<ControlRequest>(8);
    let (kill_tx, kill_rx) = oneshot::channel::<()>();

    let mut cmd = build_codex_command(&cfg, &codex);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.as_std_mut().process_group(0);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning codex app-server for {}; bin={}", cfg.agent_name, codex.bin.as_deref().unwrap_or("codex")))?;
    let child_pid = child.id();
    if let Some(pid) = child_pid {
        CHILD_PIDS.lock().unwrap_or_else(|p| p.into_inner()).insert(pid);
    }
    let stdin = child.stdin.take().context("app-server missing stdin")?;
    let stdout = child.stdout.take().context("app-server missing stdout")?;
    let stderr = child.stderr.take().context("app-server missing stderr")?;
    tokio::spawn(crate::agents::events::pump_stderr(stderr, cfg.agent_name.clone()));

    let (client, incoming) = CodexClient::start(stdout, stdin);
    let started = start_thread(&client, &cfg, &rendered).await;
    let Started { thread_id, version, resume_failed } = match started {
        Ok(v) => v,
        Err(e) => {
            // The child is useless without a thread; reap it before failing.
            kill_group(child_pid);
            let _ = child.kill().await;
            if let Some(pid) = child_pid {
                CHILD_PIDS.lock().unwrap_or_else(|p| p.into_inner()).remove(&pid);
            }
            return Err(e);
        }
    };
    info!(agent = %cfg.agent_name, %thread_id, %version, "codex participant started");
    let _ = event_tx
        .send(AgentEvent::Init { session_id: Some(thread_id.clone()), api_key_source: Some("none".into()) })
        .await;
    if let Some(text) = resume_failed {
        let _ = event_tx.send(AgentEvent::Notice(text)).await;
    }

    let translator = Translator::new(
        cfg.config.model_name.clone(),
        crate::agents::account_setup::codex_login_command(crate::agents::account_setup::Shell::host(), &home),
    );
    let effort = codex_effort(cfg.overrides.effort.as_deref()).map(str::to_string);
    let driver = Driver {
        client: Arc::new(client),
        thread_id,
        translator,
        effort,
        agent_name: cfg.agent_name.clone(),
        rendered,
        cwd: cfg.working_dir.as_ref().map(|d| d.display().to_string()),
        deadlines: InterruptDeadlines::default(),
        server_pid: child_pid,
    };
    let event_tx_for_lifecycle = event_tx.clone();
    let agent_name = cfg.agent_name.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = driver.run(incoming, event_tx, input_rx, control_rx) => {}
            _ = kill_rx => {
                info!(agent = %agent_name, "kill signalled");
            }
        }
        // The group AND the tree: a command Codex started in its own process
        // group is not in the group, but it is under the server's pid.
        if let Some(pid) = child_pid {
            crate::process::kill_process_tree(pid);
        }
        kill_group(child_pid);
        let _ = child.kill().await;
        if let Some(pid) = child_pid {
            CHILD_PIDS.lock().unwrap_or_else(|p| p.into_inner()).remove(&pid);
        }
        let _ = event_tx_for_lifecycle.send(AgentEvent::Exited("codex app-server ended".into())).await;
    });

    Ok(AgentHandle::from_parts(cfg.agent_name, cfg.session_id, event_rx, input_tx, control_tx, kill_tx))
}

fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        // SAFETY: kill(2) is thread-safe; -pid targets the group the child leads.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// initialize → hooks/list (the gate must be armed) → thread/start or
/// thread/resume. Returns the thread id and the Codex version.
async fn start_thread(client: &CodexClient, cfg: &SpawnConfig, rendered: &RenderedHook) -> Result<Started> {
    let version = initialize(client).await?;
    let cwd = cfg.working_dir.as_ref().map(|d| d.display().to_string());
    let listed = client.request("hooks/list", json!({ "cwds": cwd.iter().collect::<Vec<_>>() })).await?;
    rendered
        .armed(&listed_hooks(&listed))
        .map_err(|why| anyhow!("{} can't start: {why}. Nothing runs on Codex without bot-hq's gate hook.", cfg.agent_name))?;
    let instructions = std::fs::read_to_string(&cfg.system_prompt_path)
        .with_context(|| format!("reading the system prompt at {}", cfg.system_prompt_path.display()))?;
    let mut params = json!({
        "model": cfg.config.model_name,
        "developerInstructions": instructions,
        "sandbox": "danger-full-access",
        "approvalPolicy": "never",
    });
    if let Some(cwd) = &cwd {
        params["cwd"] = json!(cwd);
    }
    let mut resume_failed = None;
    let result = match &cfg.resume_session_id {
        Some(thread_id) => {
            let mut p = params.clone();
            p["threadId"] = json!(thread_id);
            match client.request("thread/resume", p).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(agent = %cfg.agent_name, %thread_id, %e, "thread/resume failed; starting a fresh thread");
                    resume_failed = Some(format!(
                        "{} could not resume its Codex thread {thread_id} ({e}); it starts a fresh one and \
                         remembers nothing of the old — its handoff document is what carries over.",
                        cfg.agent_name
                    ));
                    client.request("thread/start", params).await?
                }
            }
        }
        None => client.request("thread/start", params).await?,
    };
    let thread_id = result["thread"]["id"]
        .as_str()
        .ok_or_else(|| anyhow!("{}: thread/start returned no thread id", cfg.agent_name))?
        .to_string();
    Ok(Started { thread_id, version, resume_failed })
}

/// What the start sequence produced.
struct Started {
    thread_id: String,
    version: String,
    /// N1: a resume that fell back to a fresh thread, said in the channel.
    resume_failed: Option<String>,
}

/// How long an interrupt waits for the turn to end before the driver gives
/// the app-server up (G2): `plain` for an ordinary command, `atomic` for one
/// mid `git commit` / `git push` / migration (the working tree is given that
/// long not to be left half-written — `core::state::ATOMIC_OP_DEFERRAL_CAP`'s
/// posture). What is killed is the app-server's OWN process tree, the one
/// bot-hq spawned (its pid is bot-hq's), never a number Codex reported: the
/// command item's `processId` is Codex's PTY identifier, a random i32, not an
/// OS pid (openai/codex#17821; EYES blocking d681ab5a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptDeadlines {
    pub plain: std::time::Duration,
    pub atomic: std::time::Duration,
}

impl Default for InterruptDeadlines {
    fn default() -> Self {
        Self { plain: std::time::Duration::from_secs(2), atomic: std::time::Duration::from_secs(8) }
    }
}

/// The per-participant driver state.
struct Driver {
    client: Arc<CodexClient>,
    thread_id: String,
    translator: Translator,
    effort: Option<String>,
    agent_name: String,
    rendered: RenderedHook,
    cwd: Option<String>,
    deadlines: InterruptDeadlines,
    /// The app-server child bot-hq spawned — the ONLY pid the driver ever
    /// signals (its tree holds the running commands). `None` in tests.
    server_pid: Option<u32>,
}

impl Driver {
    /// Run until the server's stream closes or the input side is dropped.
    async fn run(
        mut self,
        mut incoming: mpsc::UnboundedReceiver<Incoming>,
        event_tx: mpsc::Sender<AgentEvent>,
        mut input_rx: mpsc::Receiver<OutgoingUserMessage>,
        mut control_rx: mpsc::Receiver<ControlRequest>,
    ) {
        // The turn in flight, and the pages the transport TOOK but no turn
        // has carried yet: a page that arrived mid-turn and could not be
        // steered in, or whose `turn/start` failed. They go out — all of them,
        // oldest first — at the next chance (the next page, or the moment the
        // running turn completes), and a completion is reported only once
        // nothing is queued: the sequencer committed those rows when the
        // handle took them, so a page is never dropped (EYES e264a387) and
        // the ring dealt ONE turn (plan-eyes P2).
        let mut active_turn: Option<String> = None;
        let mut queued: Vec<String> = Vec::new();
        let mut inputs_open = true;
        let mut controls_open = true;
        // Whether the command running right now is an atomic git/migration
        // step (the interrupt then waits the longer deadline).
        let mut running_atomic: Option<bool> = None;
        // An interrupt in flight: when its deadline fires.
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            tokio::select! {
                _ = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                    // The turn did not end in time: the app-server's own tree is
                    // killed — the commands under it with it — and the driver
                    // ends; the lifecycle reaps what is left and the next message
                    // resumes the thread (its rollout is on disk). The ONLY pid
                    // signalled is the one bot-hq spawned (EYES d681ab5a).
                    warn!(agent = %self.agent_name, "turn did not end after the interrupt; giving the app-server up");
                    if let Some(pid) = self.server_pid {
                        crate::process::kill_process_tree(pid);
                    }
                    let _ = event_tx.send(AgentEvent::Notice(format!(
                        "{} did not stop after an interrupt; its Codex app-server was reaped and the next message resumes its thread.",
                        self.agent_name
                    ))).await;
                    break;
                }
                msg = input_rx.recv(), if inputs_open => {
                    let Some(msg) = msg else { inputs_open = false; continue };
                    let text = msg.message.content;
                    match &active_turn {
                        None => {
                            queued.push(text);
                            match self.start_queued(&mut queued).await {
                                Ok(turn_id) => active_turn = Some(turn_id),
                                Err(e) => {
                                    // The pages stay queued; the ring hears a failed
                                    // turn and deals again once the cause is fixed.
                                    let _ = event_tx.send(AgentEvent::Notice(format!("{}: {e:#}", self.agent_name))).await;
                                    let _ = event_tx.send(AgentEvent::TurnComplete {
                                        stop_reason: Some("failed".into()), subtype: Some("failed".into()),
                                        is_error: true, api_error_status: None,
                                        context: crate::agents::spawn::ContextReport::none(crate::agents::spawn::ContextVerdict::NoUsage),
                                    }).await;
                                }
                            }
                        }
                        Some(turn_id) => match self.steer(turn_id, &text).await {
                            Ok(()) => {}
                            // The server REFUSED the steer (the turn was ending):
                            // the page is this participant's to read next.
                            Err(RequestFailure::Rpc { error, .. }) => {
                                warn!(agent = %self.agent_name, %error, "turn/steer refused; queued for the next turn");
                                queued.push(text);
                            }
                            // Nothing heard: the server may have applied it. Not
                            // re-sent (a duplicate page would be read twice) and
                            // not dropped silently — said in the channel (EYES 4295ae7d).
                            Err(e @ RequestFailure::Timeout { .. }) => {
                                let _ = event_tx.send(AgentEvent::Notice(format!(
                                    "{}: a page's delivery is of unknown outcome ({e}); if the participant does not act on it, send it again.",
                                    self.agent_name
                                ))).await;
                            }
                            Err(e @ RequestFailure::Closed { .. }) => {
                                warn!(agent = %self.agent_name, %e, "turn/steer: the server is gone");
                                queued.push(text);
                            }
                        },
                    }
                }
                ctl = control_rx.recv(), if controls_open => {
                    let Some(_ctl) = ctl else { controls_open = false; continue };
                    self.interrupt();
                    if active_turn.is_some() && deadline.is_none() {
                        let wait = if running_atomic == Some(true) { self.deadlines.atomic } else { self.deadlines.plain };
                        deadline = Some(tokio::time::Instant::now() + wait);
                    }
                }
                inc = incoming.recv() => {
                    let Some(inc) = inc else { break };
                    if inc == Incoming::Closed {
                        break;
                    }
                    // A gate hook that failed to RUN (not one that blocked) leaves
                    // the participant unguarded: Codex lets the call through. The
                    // turn is interrupted and the participant stalled (EYES 32a97dc5).
                    if gate_hook_failed(&inc) {
                        self.interrupt();
                        let _ = event_tx.send(AgentEvent::Health(AgentHealth::Stalled)).await;
                    }
                    track_running_command(&inc, &mut running_atomic);
                    for ev in self.translator.translate(&inc) {
                        if let AgentEvent::TurnComplete { .. } = &ev {
                            active_turn = None;
                            deadline = None;
                            running_atomic = None;
                            if !queued.is_empty() {
                                match self.start_queued(&mut queued).await {
                                    Ok(turn_id) => {
                                        active_turn = Some(turn_id);
                                        // Not reported: the dealt turn goes on.
                                        continue;
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(AgentEvent::Notice(format!("{}: {e:#}", self.agent_name))).await;
                                    }
                                }
                            }
                        }
                        if event_tx.send(ev).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Start a turn with everything queued, oldest first. On success the
    /// queue is empty; on failure it is untouched (nothing is lost).
    async fn start_queued(&self, queued: &mut Vec<String>) -> Result<String> {
        let text = queued.join(crate::storage::WIRE_JOIN);
        let turn_id = self.start_turn(&text).await?;
        queued.clear();
        Ok(turn_id)
    }

    /// `turn/interrupt`, fire-and-forget — the response says nothing the
    /// driver acts on (the turn's end arrives as `turn/completed`).
    fn interrupt(&self) {
        let client = Arc::clone(&self.client);
        let thread_id = self.thread_id.clone();
        let name = self.agent_name.clone();
        tokio::spawn(async move {
            if let Err(e) = client.request("turn/interrupt", json!({ "threadId": thread_id })).await {
                warn!(agent = %name, %e, "turn/interrupt failed");
            }
        });
    }

    /// G7/M10: the gate hook is re-read before EVERY turn. The file sits in
    /// a home the participant can write to, and Codex skips an untrusted or
    /// missing hook silently — so an unarmed gate refuses the turn, loudly.
    async fn gate_armed(&self) -> Result<()> {
        let listed = self
            .client
            .request("hooks/list", json!({ "cwds": self.cwd.iter().collect::<Vec<_>>() }))
            .await?;
        self.rendered
            .armed(&listed_hooks(&listed))
            .map_err(|why| anyhow!("{why}. Nothing runs on Codex without bot-hq's gate hook; this turn was not started."))
    }

    async fn start_turn(&self, text: &str) -> Result<String> {
        self.gate_armed().await?;
        let mut params = json!({
            "threadId": self.thread_id,
            "input": [{ "type": "text", "text": text }],
        });
        if let Some(effort) = &self.effort {
            params["effort"] = json!(effort);
        }
        let result = self.client.request("turn/start", params).await?;
        result["turn"]["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("turn/start returned no turn id"))
    }

    async fn steer(&self, turn_id: &str, text: &str) -> Result<(), RequestFailure> {
        self.client
            .request(
                "turn/steer",
                json!({ "threadId": self.thread_id, "expectedTurnId": turn_id, "input": [{ "type": "text", "text": text }] }),
            )
            .await
            .map(|_| ())
    }
}

/// Keep `running_atomic` at the command executing now: whether it is an
/// atomic git/migration step the interrupt should give more time. The item's
/// `processId` is deliberately NOT read: it is Codex's PTY identifier, not an
/// OS pid (EYES d681ab5a).
fn track_running_command(inc: &Incoming, running_atomic: &mut Option<bool>) {
    let Incoming::Notification(n) = inc else { return };
    let item = &n.params["item"];
    if item["type"].as_str() != Some("commandExecution") {
        return;
    }
    match n.method.as_str() {
        "item/started" => {
            let command = item["command"].as_str().unwrap_or("");
            *running_atomic =
                Some(command.contains("git commit") || command.contains("git push") || command.contains("migrate"));
        }
        "item/completed" => *running_atomic = None,
        _ => {}
    }
}

/// Whether an incoming message is a FAILED run of the gate hook
/// (`hook/completed` with `status: failed` on `preToolUse`): Codex ran the
/// call anyway, so the participant is unguarded from here.
fn gate_hook_failed(inc: &Incoming) -> bool {
    let Incoming::Notification(n) = inc else { return false };
    n.method == "hook/completed"
        && n.params["run"]["eventName"].as_str() == Some("preToolUse")
        && n.params["run"]["status"].as_str() == Some("failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::spawn::ContextVerdict;
    use crate::storage::AgentConfig;
    use std::path::PathBuf;

    fn cfg() -> (SpawnConfig, CodexSpawn) {
        let codex = CodexSpawn {
            bin: Some("codex".into()),
            mcp_url: "http://127.0.0.1:5055/sessions/s-1/eyes/mcp".into(),
            mcp_token: "secret-token".into(),
            hook_trust: vec![HookTrust { key: "/h/hooks.json:pre_tool_use:0:0".into(), hash: "sha256:abc".into() }],
            hook_exe: None,
        };
        let cfg = SpawnConfig {
            agent_name: "eyes".into(),
            config: AgentConfig {
                agent_name: "eyes".into(),
                provider: "openai".into(),
                model_name: "gpt-6-astra".into(),
                base_url: None,
                auth_token: None,
                updated_at: String::new(),
                context_window: Some(475_000),
                cli_settings: None,
                claude_config_dir: Some("/h".into()),
                runtime: "codex".into(),
            },
            system_prompt_path: PathBuf::from("/tmp/prompt.txt"),
            mcp_config_path: None,
            working_dir: Some(PathBuf::from("/tmp/repo")),
            claude_bin: None,
            session_id: "s-1".into(),
            resume_session_id: None,
            project: None,
            data_dir: PathBuf::from("/data"),
            capabilities: crate::agents::ResolvedCapabilities::Known(crate::agents::CapabilitySet::preset_eyes()),
            overrides: crate::claude_config::AgentOverride::default(),
            blank_cli_attribution: true,
            account_dir: Some("/h".into()),
            account_email: Some("me@example.com".into()),
            account_org: Some("acct".into()),
            codex: Some(codex.clone()),
        };
        (cfg, codex)
    }

    fn envs(cmd: &tokio::process::Command) -> Vec<(String, Option<String>)> {
        cmd.as_std()
            .get_envs()
            .map(|(k, v)| (k.to_string_lossy().to_string(), v.map(|v| v.to_string_lossy().to_string())))
            .collect()
    }

    /// The argv: the server over stdio, every pinned feature off, the posture,
    /// bot-hq's MCP server with its secret in a header from the env, direct
    /// tool exposure, the CLAUDE.md fallback, the trust grant, the compaction
    /// point — and never the secret, a tty, or a sub-agent.
    #[test]
    fn the_command_pins_the_feature_set_posture_and_mcp_wiring() {
        let (cfg, codex) = cfg();
        let argv = debug_codex_command(&cfg, &codex);
        assert_eq!(&argv[..3], ["codex", "app-server", "--listen"]);
        assert_eq!(argv[3], "stdio://");
        let disabled: Vec<&str> = argv.windows(2).filter(|w| w[0] == "--disable").map(|w| w[1].as_str()).collect();
        for f in ["unified_exec_tty", "multi_agent", "apps", "plugins", "goals", "image_generation", "worktrees", "unbounded_connection_retries", "memories", "computer_use"] {
            assert!(disabled.contains(&f), "{f} must be disabled: {disabled:?}");
        }
        assert!(!disabled.contains(&"code_mode_host") && !disabled.contains(&"unified_exec") && !disabled.contains(&"hooks"), "the shell and the hooks stay on");
        let overrides: Vec<&str> = argv.windows(2).filter(|w| w[0] == "-c").map(|w| w[1].as_str()).collect();
        for o in [
            "approval_policy=\"never\"",
            "sandbox_mode=\"danger-full-access\"",
            "mcp_servers.bot_hq.url=\"http://127.0.0.1:5055/sessions/s-1/eyes/mcp\"",
            "mcp_servers.bot_hq.env_http_headers={\"x-bot-hq-mcp-token\"=\"BOT_HQ_MCP_TOKEN\"}",
            "mcp_servers.bot_hq.omit_tools_from=[\"deferred\",\"code_mode\"]",
            "mcp_servers.bot_hq.default_tools_approval_mode=\"approve\"",
            "mcp_servers.bot_hq.required=true",
            "project_doc_fallback_filenames=[\"CLAUDE.md\"]",
            "shell_environment_policy.inherit=\"all\"",
            "shell_environment_policy.exclude=[\"BOT_HQ_MCP_TOKEN\"]",
            "history.persistence=\"none\"",
            "memories.generate_memories=false",
            "hooks.state={\"/h/hooks.json:pre_tool_use:0:0\"={enabled=true,trusted_hash=\"sha256:abc\"}}",
            "model_auto_compact_token_limit=442000",
        ] {
            assert!(overrides.contains(&o), "missing {o}: {overrides:?}");
        }
        assert!(!argv.iter().any(|a| a.contains("secret-token")), "the secret never rides argv: {argv:?}");
        assert!(!argv.iter().any(|a| a == "--ephemeral"), "resume needs the thread on disk");

        let cmd = build_codex_command(&cfg, &codex);
        let env = envs(&cmd);
        let get = |k: &str| env.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone());
        assert_eq!(get("CODEX_HOME"), Some(Some("/h".into())));
        assert_eq!(get("BOT_HQ_SESSION_ID"), Some(Some("s-1".into())));
        assert_eq!(get("BOT_HQ_AGENT"), Some(Some("eyes".into())));
        assert_eq!(get("BOT_HQ_MCP_TOKEN"), Some(Some("secret-token".into())));
        for scrubbed in ["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL", "CODEX_THREAD_ID", "CODEX_CI"] {
            assert_eq!(get(scrubbed), Some(None), "{scrubbed} must be removed");
        }
        assert_eq!(cmd.as_std().get_current_dir(), Some(Path::new("/tmp/repo")));
    }

    #[test]
    fn effort_maps_onto_codex_without_ultra_and_the_version_is_parsed() {
        assert_eq!(codex_effort(Some("low")), Some("low"));
        assert_eq!(codex_effort(Some("Medium")), Some("medium"));
        assert_eq!(codex_effort(Some("high")), Some("high"));
        assert_eq!(codex_effort(Some("max")), Some("max"));
        assert_eq!(codex_effort(Some("ultra")), None);
        assert_eq!(codex_effort(Some("")), None);
        assert_eq!(codex_effort(None), None);
        assert_eq!(version_from_user_agent("bot-hq-probe/0.163.0 (Mac OS 26.5.2; arm64) WarpTerminal/x (bot-hq-probe; 0.0.1)"), "0.163.0");
        assert_eq!(version_from_user_agent("odd"), "odd");
    }

    #[test]
    fn hooks_list_entries_are_read_into_listed_hooks() {
        let v = json!({"data": [{"cwd": "/tmp/repo", "hooks": [{"key": "/h/hooks.json:pre_tool_use:0:0", "eventName": "preToolUse", "handlerType": "command", "command": "\"/opt/bot-hq\" policy-check codex-gate --data-dir \"/data\"", "matcher": ".*", "sourcePath": "/h/hooks.json", "source": "user", "enabled": true, "currentHash": "sha256:abc", "trustStatus": "trusted"}], "warnings": [], "errors": []}]});
        let listed = listed_hooks(&v);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, "/h/hooks.json:pre_tool_use:0:0");
        assert_eq!(listed[0].trust_status, "trusted");
        assert!(listed[0].enabled);
        assert_eq!(listed[0].matcher.as_deref(), Some(".*"));
        assert!(listed_hooks(&json!({"data": []})).is_empty());
    }

    // ---- the driver, against a scripted server over a duplex pair ----------

    struct Fake {
        lines: tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    }

    impl Fake {
        async fn read(&mut self) -> Value {
            let line = self.lines.next_line().await.unwrap().expect("a request");
            serde_json::from_str(&line).unwrap()
        }
        async fn write(&mut self, v: Value) {
            use tokio::io::AsyncWriteExt as _;
            self.writer.write_all(format!("{v}\n").as_bytes()).await.unwrap();
            self.writer.flush().await.unwrap();
        }
        async fn answer(&mut self, req: &Value, result: Value) {
            self.write(json!({"id": req["id"], "result": result})).await;
        }
        /// The per-turn gate check (G7): answer `hooks/list` with bot-hq's
        /// hook listed, enabled and trusted.
        async fn hooks_armed(&mut self) {
            let req = self.read().await;
            assert_eq!(req["method"], "hooks/list", "the gate is checked before every turn");
            let rendered = RenderedHook::new(Path::new("/h"), &std::env::current_exe().unwrap(), Path::new("/data"));
            self.answer(&req, json!({"data": [{"cwd": "/tmp/repo", "hooks": [{"key": rendered.key, "eventName": "preToolUse", "handlerType": "command", "command": rendered.command, "matcher": ".*", "sourcePath": rendered.source_path, "source": "user", "enabled": true, "currentHash": "sha256:abc", "trustStatus": "trusted"}], "warnings": [], "errors": []}]})).await;
        }
        async fn turn_completed(&mut self, turn_id: &str) {
            self.write(json!({"method": "turn/completed", "params": {"threadId": "thr", "turn": {"id": turn_id, "status": "completed", "items": [], "error": null}}})).await;
        }
    }

    struct Harness {
        fake: Fake,
        events: mpsc::Receiver<AgentEvent>,
        input: mpsc::Sender<OutgoingUserMessage>,
        control: mpsc::Sender<ControlRequest>,
    }

    fn harness() -> Harness {
        use tokio::io::AsyncBufReadExt as _;
        #[allow(unused_imports)]
        use tokio::io::AsyncWriteExt as _;
        let (client_side, server_side) = tokio::io::duplex(1 << 16);
        let (c_read, c_write) = tokio::io::split(client_side);
        let (s_read, s_write) = tokio::io::split(server_side);
        let (mut client, incoming) = CodexClient::start(c_read, c_write);
        client.timeout = std::time::Duration::from_millis(400);
        let (event_tx, events) = mpsc::channel(64);
        let (input, input_rx) = mpsc::channel(8);
        let (control, control_rx) = mpsc::channel(8);
        let driver = Driver {
            client: Arc::new(client),
            thread_id: "thr".into(),
            translator: Translator::new("gpt-6-astra", ""),
            effort: Some("high".into()),
            agent_name: "eyes".into(),
            rendered: RenderedHook::new(Path::new("/h"), &std::env::current_exe().unwrap(), Path::new("/data")),
            cwd: Some("/tmp/repo".into()),
            deadlines: InterruptDeadlines { plain: std::time::Duration::from_millis(400), atomic: std::time::Duration::from_millis(800) },
            server_pid: SERVER_PID.with(|p| p.get()),
        };
        tokio::spawn(driver.run(incoming, event_tx, input_rx, control_rx));
        Harness { fake: Fake { lines: tokio::io::BufReader::new(s_read).lines(), writer: s_write }, events, input, control }
    }

    thread_local! {
        /// A stand-in app-server pid for one test's harness (`None` otherwise).
        static SERVER_PID: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    }

    async fn next_event(h: &mut Harness) -> AgentEvent {
        tokio::time::timeout(std::time::Duration::from_secs(5), h.events.recv()).await.expect("an event in time").expect("the event stream is open")
    }

    /// A delivered page starts a turn with the effort; the model's text and
    /// the completion come back as the pump expects them.
    #[tokio::test]
    async fn a_page_starts_a_turn_and_its_completion_is_reported() {
        let mut h = harness();
        h.input.send(OutgoingUserMessage::text("[user · 02:00Z] hello")).await.unwrap();
        h.fake.hooks_armed().await;
        let req = h.fake.read().await;
        assert_eq!(req["method"], "turn/start");
        assert_eq!(req["params"]["threadId"], "thr");
        assert_eq!(req["params"]["input"][0]["text"], "[user · 02:00Z] hello");
        assert_eq!(req["params"]["effort"], "high");
        h.fake.answer(&req, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        h.fake.write(json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "m", "text": "hi back", "phase": "final_answer"}}})).await;
        h.fake.turn_completed("t1").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Text(t) if t == "hi back"));
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { is_error: false, context, .. } if context.verdict == ContextVerdict::NoUsage));
    }

    /// P2: a page that arrives mid-turn is steered in; when the steer fails
    /// (the turn just completed) the page starts the next turn at once and
    /// the dealt turn's completion is reported only after THAT turn — a row
    /// the transport took is never lost and never double-counted.
    #[tokio::test]
    async fn a_mid_turn_page_is_steered_or_starts_the_next_turn_before_completion_is_reported() {
        let mut h = harness();
        h.input.send(OutgoingUserMessage::text("page 1")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        // Mid-turn page: steered.
        h.input.send(OutgoingUserMessage::text("page 2")).await.unwrap();
        let steer = h.fake.read().await;
        assert_eq!(steer["method"], "turn/steer");
        assert_eq!(steer["params"]["expectedTurnId"], "t1");
        assert_eq!(steer["params"]["input"][0]["text"], "page 2");
        h.fake.answer(&steer, json!({"turnId": "t1"})).await;
        // A page whose steer the server refuses (the turn is ending).
        h.input.send(OutgoingUserMessage::text("page 3")).await.unwrap();
        let steer = h.fake.read().await;
        assert_eq!(steer["method"], "turn/steer");
        h.fake.write(json!({"id": steer["id"], "error": {"code": -32600, "message": "turn t1 is not active"}})).await;
        // Now the turn completes: the queued page starts the next turn and
        // the completion is NOT reported yet.
        h.fake.turn_completed("t1").await;
        h.fake.hooks_armed().await;
        let start2 = h.fake.read().await;
        assert_eq!(start2["method"], "turn/start");
        assert_eq!(start2["params"]["input"][0]["text"], "page 3");
        h.fake.answer(&start2, json!({"turn": {"id": "t2", "status": "inProgress", "items": []}})).await;
        h.fake.write(json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "m", "text": "done", "phase": "final_answer"}}})).await;
        h.fake.turn_completed("t2").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Text(t) if t == "done"), "no completion before the queued page ran");
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { .. }));
        assert!(h.events.try_recv().is_err(), "exactly one completion for the dealt turn");
    }

    /// G7/M10: the hooks file is writable by the participant and Codex skips
    /// an untrusted or missing hook silently, so the gate is re-checked before
    /// EVERY turn — a turn on an unarmed gate is refused, with the reason, and
    /// the completion the ring waits for is an error.
    #[tokio::test]
    async fn a_turn_is_refused_when_the_gate_hook_is_no_longer_armed() {
        let mut h = harness();
        h.input.send(OutgoingUserMessage::text("page 1")).await.unwrap();
        let req = h.fake.read().await;
        assert_eq!(req["method"], "hooks/list");
        h.fake.answer(&req, json!({"data": [{"cwd": "/tmp/repo", "hooks": [], "warnings": [], "errors": []}]})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Notice(n) if n.contains("not listed")));
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { is_error: true, .. }));
        // No turn/start went out: the next thing the server reads is the
        // next page's check, not a turn.
        h.input.send(OutgoingUserMessage::text("page 2")).await.unwrap();
        let req = h.fake.read().await;
        assert_eq!(req["method"], "hooks/list");
    }

    /// EYES e264a387: a page whose `turn/start` failed is not thrown away —
    /// it goes out with the next page, oldest first; and EYES 4295ae7d: a
    /// steer whose outcome is unknown (a timeout) is neither re-sent nor
    /// dropped silently.
    #[tokio::test]
    async fn a_page_whose_start_failed_rides_the_next_start_and_a_timed_out_steer_is_said_not_resent() {
        let mut h = harness();
        // Page 1: the gate is unarmed → the start fails; the page is kept.
        h.input.send(OutgoingUserMessage::text("page 1")).await.unwrap();
        let req = h.fake.read().await;
        assert_eq!(req["method"], "hooks/list");
        h.fake.answer(&req, json!({"data": [{"cwd": "/tmp/repo", "hooks": [], "warnings": [], "errors": []}]})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Notice(_)));
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { is_error: true, .. }));
        // Page 2: the gate is back; the turn carries BOTH pages, oldest first.
        h.input.send(OutgoingUserMessage::text("page 2")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        assert_eq!(start["method"], "turn/start");
        let text = start["params"]["input"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("page 1") && text.ends_with("page 2"), "{text}");
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        // A steer the server never answers: said, not re-queued.
        h.input.send(OutgoingUserMessage::text("page 3")).await.unwrap();
        let steer = h.fake.read().await;
        assert_eq!(steer["method"], "turn/steer");
        // (the harness client's timeout is short — see `harness`)
        assert!(matches!(next_event(&mut h).await, AgentEvent::Notice(n) if n.contains("unknown outcome")));
        h.fake.turn_completed("t1").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { is_error: false, .. }), "no re-sent page starts a turn");
        h.input.send(OutgoingUserMessage::text("page 4")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        assert_eq!(start["params"]["input"][0]["text"], "page 4", "page 3 was not re-sent");
    }

    /// EYES 32a97dc5: a gate hook that failed to RUN interrupts the turn and
    /// stalls the participant — Codex lets the call through when its hook
    /// fails, so the participant is unguarded until it runs again.
    #[tokio::test]
    async fn a_failed_gate_hook_run_interrupts_and_stalls() {
        let mut h = harness();
        h.fake.write(json!({"method": "hook/completed", "params": {"threadId": "thr", "run": {"eventName": "preToolUse", "status": "failed", "sourcePath": "/h/hooks.json", "entries": [{"kind": "stderr", "text": "No such file or directory"}]}}})).await;
        let req = h.fake.read().await;
        assert_eq!(req["method"], "turn/interrupt");
        h.fake.answer(&req, json!({})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Health(AgentHealth::Stalled)));
        assert!(matches!(next_event(&mut h).await, AgentEvent::Notice(n) if n.contains("FAILED")));
        // A hook that BLOCKED is the gate working: nothing happens.
        h.fake.write(json!({"method": "hook/completed", "params": {"threadId": "thr", "run": {"eventName": "preToolUse", "status": "blocked", "sourcePath": "/h/hooks.json"}}})).await;
        h.fake.write(json!({"method": "turn/started", "params": {}})).await;
        assert!(tokio::time::timeout(std::time::Duration::from_millis(300), h.fake.lines.next_line()).await.is_err(), "no interrupt for a block");
    }

    /// G2/P7: an interrupt is given a deadline. Codex answers `turn/interrupt`
    /// at once but lets the running command finish (measured: `sleep 100`
    /// lived 70 s past it), so a user message, a stray-turn stop or a halt
    /// would wait out a `cargo build`. When no `turn/completed` arrives in
    /// time the driver kills the running command's process tree (its
    /// `processId`); when the turn still does not end, the driver ends and the
    /// app-server is reaped — the next message resumes the thread.
    /// EYES blocking d681ab5a: the item's `processId` is NOT an OS pid (a
    /// random i32 PTY id), so the deadline never signals it — the tree killed
    /// is the app-server's own, the pid bot-hq spawned. A canary process
    /// holding the number Codex reported must survive.
    #[tokio::test]
    async fn an_unanswered_interrupt_kills_the_servers_own_tree_and_never_the_reported_id() {
        // The stand-in app-server: a real tree (a shell with a sleeping child).
        let mut server = crate::process::command("sh").args(["-c", "sleep 300 & wait"]).spawn().unwrap();
        let server_pid = server.id();
        // The canary: whatever process holds the number Codex reports.
        let mut canary = crate::process::command("sleep").arg("300").spawn().unwrap();
        let reported = canary.id();
        SERVER_PID.with(|p| p.set(Some(server_pid)));
        let mut h = harness();
        SERVER_PID.with(|p| p.set(None));
        h.input.send(OutgoingUserMessage::text("build it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        h.fake.write(json!({"method": "item/started", "params": {"item": {"type": "commandExecution", "id": "exec-1", "command": "/bin/zsh -lc 'cargo build'", "processId": reported.to_string(), "status": "inProgress"}}})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::ToolUse { .. }));
        h.control.send(ControlRequest::interrupt("user-preempt")).await.unwrap();
        let req = h.fake.read().await;
        assert_eq!(req["method"], "turn/interrupt");
        h.fake.answer(&req, json!({})).await;
        // After the deadline: the server's tree is dead, the driver posted the
        // notice and ended, and the canary is untouched.
        let t0 = std::time::Instant::now();
        loop {
            if server.try_wait().unwrap().is_some() {
                break;
            }
            assert!(t0.elapsed() < std::time::Duration::from_secs(5), "the server tree was not killed at the deadline");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(t0.elapsed() >= std::time::Duration::from_millis(300), "not before the deadline");
        let mut saw_notice = false;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(6), h.events.recv()).await.unwrap() {
                Some(AgentEvent::Notice(n)) if n.contains("reaped") => saw_notice = true,
                Some(_) => {}
                None => break,
            }
        }
        assert!(saw_notice, "the notice was posted before the driver ended");
        assert!(canary.try_wait().unwrap().is_none(), "the process holding Codex's reported id must survive");
        canary.kill().unwrap();
        let _ = canary.wait();
    }

    /// The deadline is cleared by a completion: an interrupt that ends the turn
    /// in time kills nothing and the driver goes on.
    #[tokio::test]
    async fn an_interrupt_answered_by_a_completion_kills_nothing() {
        let mut h = harness();
        h.input.send(OutgoingUserMessage::text("quick")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        h.control.send(ControlRequest::interrupt("cancel")).await.unwrap();
        let req = h.fake.read().await;
        assert_eq!(req["method"], "turn/interrupt");
        h.fake.answer(&req, json!({})).await;
        h.fake.write(json!({"method": "turn/completed", "params": {"threadId": "thr", "turn": {"id": "t1", "status": "interrupted", "items": [], "error": null}}})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { stop_reason: Some(s), .. } if s == "interrupted"));
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        // Still alive: a new page starts a turn.
        h.input.send(OutgoingUserMessage::text("again")).await.unwrap();
        h.fake.hooks_armed().await;
        assert_eq!(h.fake.read().await["method"], "turn/start");
    }

    #[tokio::test]
    async fn an_interrupt_asks_the_server_to_interrupt_the_thread_and_a_closed_stream_ends_the_driver() {
        let mut h = harness();
        h.control.send(ControlRequest::interrupt("cancel")).await.unwrap();
        let req = h.fake.read().await;
        assert_eq!(req["method"], "turn/interrupt");
        assert_eq!(req["params"]["threadId"], "thr");
        h.fake.answer(&req, json!({})).await;
        drop(h.fake);
        assert!(tokio::time::timeout(std::time::Duration::from_secs(5), h.events.recv()).await.unwrap().is_none(), "the event stream closes when the driver ends");
    }
}
