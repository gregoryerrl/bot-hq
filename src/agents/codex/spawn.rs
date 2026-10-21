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
    /// The model catalog written for this spawn (`home::write_model_catalog`):
    /// Codex's own, with the row's window on the row's model. `None` when the
    /// home has no catalog yet — Codex's default stands.
    pub model_catalog: Option<std::path::PathBuf>,
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
        // The disabled `multi_agent*` features do not remove the sub-agent
        // tools on 0.163.0: Codex resolves the multi-agent version as this
        // switch → the model catalog's own value → the feature flags, and the
        // account's catalog marks every model v1/v2 (measured 2026-10-21: the
        // tools stayed listed with both flags off). An enabled `multi_agent_v2`
        // would still outrank it, so the flag stays disabled too.
        "agents.enabled=false".to_string(),
        hooks_state_override(&codex.hook_trust),
    ];
    if let Some(window) = cfg.config.context_window.filter(|w| *w > 0) {
        // The row's window is the one Codex runs the model at: its registry
        // defaults are small (272,000; 500,000 on gpt-6.1-sol) and the
        // override is clamped to the catalog's `max_context_window` (872,000
        // on every model of the user's account) — so the catalog itself is
        // handed over with the row's window in it (`model_catalog_json`,
        // written by `home::write_model_catalog`; the backend served 853,310
        // tokens of context to gpt-6-astra under it, measured 2026-10-21).
        // Codex keeps 5% back and reports the rest as the window the meter
        // sees; the compaction point goes the claude path's reserve below
        // that so the handoff rows come first (`pump::COMPACT_RESERVE_TOKENS`).
        out.push(format!("model_context_window={window}"));
        // A RESUMED thread keeps the window it was created with (measured
        // 2026-10-21: a thread from the 872,000 days reported 828,400 after
        // its row moved to 1,050,000), so a compaction point placed from the
        // row would sit above the window it runs and never fire. Codex's own
        // point, derived from the thread's window, is right for it; the
        // row's placement is for the thread this spawn creates.
        if cfg.resume_session_id.is_none() {
            let limit = usable_window(window as u64).saturating_sub(COMPACT_RESERVE_TOKENS);
            out.push(format!("model_auto_compact_token_limit={limit}"));
        }
        if let Some(catalog) = &codex.model_catalog {
            out.push(format!("model_catalog_json={}", toml_str(&catalog.display().to_string())));
        }
    }
    out
}

/// The reserve the claude path keeps below the window before its context
/// rows fire (`core::pump::COMPACT_RESERVE_TOKENS`), applied to Codex's own
/// compaction point so the handoff rows come first.
pub const COMPACT_RESERVE_TOKENS: u64 = 33_000;

/// The share of a model's context window Codex lets inference use — the rest
/// is its headroom for prompts, tool overhead and output
/// (`effective_context_window_percent`, 95 for every model of the user's
/// account). What Codex reports as `modelContextWindow` is this share of the
/// window it runs: 872,000 configured reports 828,400 (measured 2026-10-21).
pub const USABLE_WINDOW_PERCENT: u64 = 95;

/// The window Codex reports for a configured one.
pub fn usable_window(window: u64) -> u64 {
    window.saturating_mul(USABLE_WINDOW_PERCENT) / 100
}

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
    let Started { thread_id, version, resume_failed, limit } = match started {
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
    for text in [resume_failed, limit].into_iter().flatten() {
        let _ = event_tx.send(AgentEvent::Notice(text)).await;
    }

    let translator = translator_for(&cfg, &home);
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
        server_baseline: child_pid.and_then(server_tree),
        data_dir: cfg.data_dir.clone(),
        session_id: cfg.session_id.clone(),
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

/// The participant's translator: the row's model and window, and the home's
/// sign-in command for the `unauthorized` notice.
fn translator_for(cfg: &SpawnConfig, home: &str) -> Translator {
    Translator::new(
        cfg.config.model_name.clone(),
        crate::agents::account_setup::codex_login_command(crate::agents::account_setup::Shell::host(), home),
    )
    .with_configured_window(cfg.config.context_window.and_then(|w| u64::try_from(w).ok()))
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
    // The usage window, advisory: a failed read says nothing.
    let limit = client
        .request("account/rateLimits/read", json!({}))
        .await
        .ok()
        .and_then(|r| rate_limit_notice(&r, &cfg.agent_name, &cfg.config.model_name));
    Ok(Started { thread_id, version, resume_failed, limit })
}

/// What the start sequence produced.
struct Started {
    thread_id: String,
    version: String,
    /// N1: a resume that fell back to a fresh thread, said in the channel.
    resume_failed: Option<String>,
    /// The account's usage window is exhausted: said once, where the session
    /// starts, and the spawn goes ahead (advisory, like the Claude mark).
    limit: Option<String>,
}

/// The advisory notice for an `account/rateLimits/read` answer whose primary
/// window is used up (or whose backend says a limit was reached), or `None`.
/// Codex gives the reset as epoch seconds; it is shown in UTC.
pub fn rate_limit_notice(result: &Value, agent: &str, model: &str) -> Option<String> {
    let limits = &result["rateLimits"];
    let used = limits["primary"]["usedPercent"].as_u64();
    let reached = limits["rateLimitReachedType"].as_str();
    if used.is_none_or(|u| u < 100) && reached.is_none() {
        return None;
    }
    let reset = limits["primary"]["resetsAt"]
        .as_i64()
        .and_then(|secs| chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0))
        .map(|t| format!(", resets at {}", t.format("%Y-%m-%d %H:%MZ")))
        .unwrap_or_default();
    Some(format!(
        "ℹ [bot-hq] {agent} starts on {model} with its ChatGPT usage window at {}%{reset}{} — it may not answer until then.",
        used.unwrap_or(100),
        reached.map(|r| format!(" ({r})")).unwrap_or_default()
    ))
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

/// The app-server's own processes at the driver's start, and where Codex's
/// own binaries live.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerTree {
    /// The pids of the server's own tree (the native binary under npm's
    /// wrapper, whatever Codex started for itself before the first turn).
    pids: Vec<u32>,
    /// The directory of the server's own `codex` binary — the vendor `bin/`
    /// under npm's package on macOS — where its helpers live too. `None`
    /// when no process of the tree is a `codex` binary (then nothing is a
    /// helper).
    codex_dir: Option<std::path::PathBuf>,
}

impl ServerTree {
    /// Whether a process found under the app-server later is one of Codex's
    /// own helpers rather than a command: a `codex-*` binary (never the
    /// `codex` binary itself — a nested `codex` is a command, however it was
    /// launched) that lives in the same directory as the server's own
    /// `codex` AND whose parent is one of the server's own processes. A
    /// command named `codex-build` lives elsewhere; a nested `codex` under a
    /// shell has a command for a parent; a `codex` exec'd straight from the
    /// installation directory is the bare binary (EYES f3e7f82f, dfcddc4e)
    /// — all three are stopped. What stays indistinguishable: a command that
    /// exec's the real `codex-code-mode-host` binary itself.
    fn is_helper(&self, server_pid: u32, program: &str, ppid: u32) -> bool {
        let Some(dir) = &self.codex_dir else { return false };
        let path = std::path::Path::new(program);
        let helper_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("codex-"));
        helper_name && path.parent() == Some(dir.as_path()) && (ppid == server_pid || self.pids.contains(&ppid))
    }
}

/// The app-server's own process tree at the driver's start — see
/// `Driver::server_baseline`. `None` when it could not be listed (and
/// always where there is no tree walk).
fn server_tree(pid: u32) -> Option<ServerTree> {
    #[cfg(unix)]
    {
        let own = crate::process::descendants_named(pid)?;
        let is_codex = |program: &str| std::path::Path::new(program).file_name().and_then(|n| n.to_str()) == Some("codex");
        let codex_dir = crate::process::program_of(pid)
            .filter(|p| is_codex(p))
            .or_else(|| own.iter().map(|d| d.program.clone()).find(|p| is_codex(p)))
            .and_then(|p| std::path::Path::new(&p).parent().map(|d| d.to_path_buf()));
        Some(ServerTree { pids: own.into_iter().map(|d| d.pid).collect(), codex_dir })
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

/// Arm the interrupt deadline for the turn in flight, once: a second
/// interrupt (another control, a failed hook) while one is pending must not
/// push the deadline back.
fn arm_deadline(deadline: &mut Option<tokio::time::Instant>, atomic: bool, deadlines: &InterruptDeadlines) {
    if deadline.is_none() {
        let wait = if atomic { deadlines.atomic } else { deadlines.plain };
        *deadline = Some(tokio::time::Instant::now() + wait);
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
    /// The app-server's OWN processes, listed once when the driver started
    /// (after `thread/start`): the native binary under npm's wrapper and
    /// whatever Codex started for itself. A descendant that is not among
    /// them is taken for a command's process — a working assumption (a helper
    /// Codex starts lazily would be taken for one too), only ever SUBTRACTED
    /// from a fresh listing, never signalled on its own. `None` when the
    /// listing failed at start: an unknown tree is never "clean", and never
    /// an empty baseline that would make the native binary look like a
    /// command (EYES 37ff33f6). Carries the directory of Codex's own native
    /// binary, so a helper it starts later from the same directory
    /// (`codex-code-mode-host`) is told from a command by where it lives and
    /// who started it — not by its name (EYES f3e7f82f).
    server_baseline: Option<ServerTree>,
    /// Where the handoff files live, and whose they are — for the text put
    /// back after a compaction.
    data_dir: std::path::PathBuf,
    session_id: String,
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
        // The commands running right now, and whether each is an atomic
        // git/migration step (an interrupt then waits the longer deadline).
        let mut running = RunningCommands::new();
        // An interrupt in flight: when its deadline fires.
        let mut deadline: Option<tokio::time::Instant> = None;
        // An interrupt went out during this turn: the completion that follows
        // stops the commands Codex leaves running (it ends the turn, not the
        // command — measured 2026-10-21), waiting out the deadline first when
        // an atomic step was in flight at the interrupt.
        let mut stop_commands: Option<bool> = None;
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
                    self.interrupt(active_turn.as_deref());
                    if active_turn.is_some() {
                        let atomic = atomic_in_flight(&running);
                        arm_deadline(&mut deadline, atomic, &self.deadlines);
                        stop_commands = Some(stop_commands.unwrap_or(false) || atomic);
                    }
                }
                inc = incoming.recv() => {
                    let Some(inc) = inc else { break };
                    if inc == Incoming::Closed {
                        break;
                    }
                    // A gate hook that failed to RUN (not one that blocked) leaves
                    // the participant unguarded: Codex lets the call through. The
                    // turn is interrupted and the participant stalled (EYES 32a97dc5),
                    // under the same deadline as a user's interrupt — an unguarded
                    // turn that ignores the interrupt must not run on.
                    if gate_hook_failed(&inc) {
                        self.interrupt(active_turn.as_deref());
                        if active_turn.is_some() {
                            let atomic = atomic_in_flight(&running);
                            arm_deadline(&mut deadline, atomic, &self.deadlines);
                            stop_commands = Some(stop_commands.unwrap_or(false) || atomic);
                        }
                        let _ = event_tx.send(AgentEvent::Health(AgentHealth::Stalled)).await;
                    }
                    track_running_command(&inc, &mut running);
                    // Codex compacted the thread: put the handoff document and
                    // the user's words since back into the model-visible history
                    // (`thread/inject_items`); when that fails, they go in front
                    // of the next page instead (plan B3 / P5).
                    let compacted = matches!(&inc, Incoming::Notification(n) if n.method == "thread/compacted");
                    let mut reinjected = Vec::new();
                    if compacted {
                        if let Some((text, has_doc, words)) = self.compaction_text() {
                            match self.inject(&text).await {
                                Ok(()) => {
                                    reinjected.push(AgentEvent::HandoffInjected { has_doc });
                                    if let Some(count) = words {
                                        reinjected.push(AgentEvent::UserWordsInjected { count });
                                    }
                                }
                                Err(e) => {
                                    warn!(agent = %self.agent_name, %e, "thread/inject_items failed; the handoff rides the next page");
                                    queued.insert(0, text);
                                }
                            }
                        }
                    }
                    for ev in self.translator.translate(&inc).into_iter().chain(reinjected) {
                        if let AgentEvent::TurnComplete { .. } = &ev {
                            // The turn an interrupt ended may have left its
                            // command running: stop it BEFORE the completion is
                            // reported and before the next turn starts, so
                            // nothing of the next turn is ever in the kill set.
                            if let Some(atomic_at_interrupt) = stop_commands.take() {
                                let atomic = atomic_at_interrupt || atomic_in_flight(&running);
                                match self.stop_interrupted_commands(atomic, deadline).await {
                                    Ok(0) => {}
                                    Ok(n) => {
                                        let _ = event_tx.send(AgentEvent::Notice(format!(
                                            "{}'s interrupted command was still running after the turn ended; {n} process{} stopped.",
                                            self.agent_name, if n == 1 { " was" } else { "es were" }
                                        ))).await;
                                    }
                                    Err(why) => {
                                        // Unknown is not clean: the app-server's own
                                        // tree goes, as when a turn does not end.
                                        warn!(agent = %self.agent_name, %why, "could not tell whether the interrupted command still runs; giving the app-server up");
                                        if let Some(pid) = self.server_pid {
                                            crate::process::kill_process_tree(pid);
                                        }
                                        let _ = event_tx.send(AgentEvent::Notice(format!(
                                            "{}: after an interrupt bot-hq could not tell whether its command still runs ({why}); its Codex app-server was reaped and the next message resumes its thread.",
                                            self.agent_name
                                        ))).await;
                                        return;
                                    }
                                }
                            }
                            active_turn = None;
                            deadline = None;
                            running.clear();
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

    /// The text put back after a compaction: the participant's rendered
    /// handoff file (`policy-check compact-handoff`'s text, age-stamped now)
    /// and the user's words since it was written (`compact-user-words`'s);
    /// `None` when there is nothing rendered for this participant. Returns
    /// the text, whether a real doc was in it, and how many user messages.
    fn compaction_text(&self) -> Option<(String, bool, Option<usize>)> {
        let sid = Some(self.session_id.as_str());
        let handoff = crate::policy::hooks::compact_handoff_text(&self.data_dir, sid, &self.agent_name, chrono::Utc::now())?;
        let has_doc = !handoff.contains(crate::agents::handoff::NO_DOC_MARKER);
        let words = crate::policy::hooks::compact_user_words_text(&self.data_dir, sid, &self.agent_name);
        let count = words.as_ref().map(|w| w.matches("\n[").count());
        let text = match words {
            Some(w) => format!("{handoff}\n\n{w}"),
            None => handoff,
        };
        Some((text, has_doc, count))
    }

    /// Append one user message to the thread's model-visible history.
    async fn inject(&self, text: &str) -> Result<(), RequestFailure> {
        self.client
            .request(
                "thread/inject_items",
                json!({
                    "threadId": self.thread_id,
                    "items": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] }],
                }),
            )
            .await
            .map(|_| ())
    }

    /// `turn/interrupt` for the turn in flight, fire-and-forget — the response
    /// says nothing the driver acts on (the turn's end arrives as
    /// `turn/completed`; the deadline covers a turn that does not end). The
    /// request names the turn: `TurnInterruptParams` requires `turnId`, and
    /// without it Codex 0.163.0 refuses every interrupt with `-32600 missing
    /// field turnId` (so until 2026-10-21 only the deadline ever stopped a
    /// participant). With no turn in flight there is nothing to send.
    fn interrupt(&self, turn_id: Option<&str>) {
        let Some(turn_id) = turn_id else { return };
        let client = Arc::clone(&self.client);
        let params = json!({ "threadId": self.thread_id, "turnId": turn_id });
        let name = self.agent_name.clone();
        tokio::spawn(async move {
            if let Err(e) = client.request("turn/interrupt", params).await {
                warn!(agent = %name, %e, "turn/interrupt failed");
            }
        });
    }

    /// Stop the commands an interrupted turn left running: every live
    /// descendant of the app-server that is not in its own tree, children
    /// first. Codex answers a `turn/interrupt` by ending the turn and lets the
    /// command run on (measured 2026-10-21: `sleep 180` outlived its
    /// interrupted turn), so the driver does what Codex does not. With an
    /// atomic git/migration step in flight the command is given until the
    /// interrupt's own deadline — the same allowance the deadline gives — to
    /// finish by itself. Returns how many processes were killed; `Err` when
    /// the tree could not be listed, which the caller treats as "not clean".
    /// Unix only: elsewhere there is no tree walk, and the command runs on as
    /// before (a documented limitation, like the deadline's tree kill).
    async fn stop_interrupted_commands(&self, atomic: bool, until: Option<tokio::time::Instant>) -> Result<usize, String> {
        let Some(pid) = self.server_pid else { return Ok(0) };
        #[cfg(not(unix))]
        {
            let _ = (pid, atomic, until);
            return Ok(0);
        }
        #[cfg(unix)]
        {
            let Some(baseline) = self.server_baseline.as_ref() else {
                return Err("the app-server's own processes were never listed (ps failed at its start)".to_string());
            };
            // What is NOT a command: the app-server's own tree, and the helpers
            // Codex starts lazily under it — `codex-code-mode-host` appeared
            // only at the first `exec` call and was killed by this cleanup on
            // 2026-10-21T13:12Z ("1 process was stopped" with no command in
            // flight). A helper is known by where it lives and who started it
            // (`ServerTree::is_helper`), never by its name alone.
            let list = || -> Result<Vec<u32>, String> {
                let live = crate::process::descendants_named(pid).ok_or_else(|| "ps could not list the app-server's processes".to_string())?;
                Ok(live
                    .into_iter()
                    .filter(|d| !baseline.pids.contains(&d.pid) && !baseline.is_helper(pid, &d.program, d.ppid))
                    .map(|d| d.pid)
                    .collect())
            };
            let newer = loop {
                let newer = list()?;
                if newer.is_empty() {
                    return Ok(0);
                }
                if atomic {
                    if let Some(until) = until {
                        let now = tokio::time::Instant::now();
                        if now < until {
                            tokio::time::sleep((until - now).min(std::time::Duration::from_millis(250))).await;
                            continue;
                        }
                    }
                }
                break newer;
            };
            crate::process::kill_each(&newer);
            // Gone for real before the next turn may start: SIGKILL is
            // asynchronous, and a kill that failed is only visible here. Each
            // target is asked itself (`is_gone`), not the tree — a child whose
            // parent died first leaves the tree alive, reparented.
            let gone_by = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                let survivors: Vec<u32> = newer.iter().copied().filter(|p| !crate::process::is_gone(*p)).collect();
                if survivors.is_empty() {
                    return Ok(newer.len());
                }
                if tokio::time::Instant::now() >= gone_by {
                    return Err(format!("{} process(es) survived SIGKILL: {survivors:?}", survivors.len()));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
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
/// The command items in flight, by item id, each with whether it is an
/// atomic git/migration step. Codex can run more than one command at once,
/// so a single flag would let a later plain command hide an active commit
/// (EYES 1331cc1b): the question is always "is ANY of them atomic".
type RunningCommands = std::collections::HashMap<String, bool>;

fn track_running_command(inc: &Incoming, running: &mut RunningCommands) {
    let Incoming::Notification(n) = inc else { return };
    let item = &n.params["item"];
    if item["type"].as_str() != Some("commandExecution") {
        return;
    }
    let id = item["id"].as_str().unwrap_or("").to_string();
    match n.method.as_str() {
        "item/started" => {
            let command = item["command"].as_str().unwrap_or("");
            running.insert(id, command.contains("git commit") || command.contains("git push") || command.contains("migrate"));
        }
        "item/completed" => {
            running.remove(&id);
        }
        _ => {}
    }
}

/// Whether an atomic git/migration command is among those in flight.
fn atomic_in_flight(running: &RunningCommands) -> bool {
    running.values().any(|atomic| *atomic)
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
            model_catalog: Some(PathBuf::from("/h/bot-hq-catalog-gpt-6-astra.json")),
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
                context_window: Some(872_000),
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
        for f in ["unified_exec_tty", "multi_agent", "multi_agent_v2", "apps", "plugins", "goals", "image_generation", "worktrees", "unbounded_connection_retries", "memories", "computer_use"] {
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
            // The sub-agent tools follow the model catalog unless this switch is
            // off; the disabled `multi_agent*` features alone leave them listed.
            "agents.enabled=false",
            "hooks.state={\"/h/hooks.json:pre_tool_use:0:0\"={enabled=true,trusted_hash=\"sha256:abc\"}}",
            // The row's window is the one Codex runs the model at; Codex keeps
            // 5% of it back (measured: 872,000 reports 828,400), and its
            // compaction point sits the claude path's reserve below that.
            "model_context_window=872000",
            "model_auto_compact_token_limit=795400",
            // The catalog carries the row's window past Codex's default cap.
            "model_catalog_json=\"/h/bot-hq-catalog-gpt-6-astra.json\"",
        ] {
            assert!(overrides.contains(&o), "missing {o}: {overrides:?}");
        }
        assert!(!argv.iter().any(|a| a.contains("secret-token")), "the secret never rides argv: {argv:?}");
        assert!(!argv.iter().any(|a| a == "--ephemeral"), "resume needs the thread on disk");
        // A resume keeps the window and the catalog (a failed resume starts a
        // fresh thread under them) but leaves the compaction point to Codex:
        // the resumed thread keeps the window it was created with.
        let mut resuming = cfg.clone();
        resuming.resume_session_id = Some("01a15d68-385e-71b0-a853-d45801d319e7".into());
        let resumed: Vec<String> = config_overrides(&resuming, &codex);
        assert!(resumed.iter().any(|o| o == "model_context_window=872000"));
        assert!(resumed.iter().any(|o| o.starts_with("model_catalog_json=")));
        assert!(!resumed.iter().any(|o| o.starts_with("model_auto_compact_token_limit=")), "{resumed:?}");

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

    /// The row's window reaches the translator, so a thread Codex runs at a
    /// smaller window than the row asked for is reported (events.rs).
    #[test]
    fn the_translator_knows_the_rows_window() {
        let (cfg, _) = cfg();
        let mut t = translator_for(&cfg, "/h");
        let usage = crate::agents::codex::client::Incoming::Notification(crate::agents::codex::client::Notification {
            method: "thread/tokenUsage/updated".into(),
            params: serde_json::json!({"tokenUsage": {"last": {"inputTokens": 90}, "modelContextWindow": 258_400}}),
        });
        let events = t.translate(&usage);
        assert!(matches!(&events[..], [AgentEvent::AssistantUsage { .. }, AgentEvent::Notice(n)] if n.contains("872,000")), "{events:?}");
        assert_eq!(t.model, "gpt-6-astra");
        // The host shell's sign-in command (PowerShell on Windows), for the home.
        assert!(t.login_command.contains("'/h'") && t.login_command.contains("codex login"), "{}", t.login_command);
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

    /// The measured `account/rateLimits/read` shape (s-de77e2f0): a window at
    /// 0% says nothing; one used up, or a reached-type, is one advisory line
    /// with the reset in UTC.
    #[test]
    fn a_used_up_usage_window_is_an_advisory_notice() {
        let fresh = json!({"ordinaryUsageAllowed": true, "rateLimits": {"limitId": "codex", "primary": {"usedPercent": 0, "windowDurationMins": 10080, "resetsAt": 1793152132}, "secondary": null, "planType": "pro", "rateLimitReachedType": null}});
        assert_eq!(rate_limit_notice(&fresh, "eyes", "gpt-6-astra"), None);
        let used = json!({"rateLimits": {"primary": {"usedPercent": 100, "windowDurationMins": 10080, "resetsAt": 1793152132}, "rateLimitReachedType": null}});
        let n = rate_limit_notice(&used, "eyes", "gpt-6-astra").unwrap();
        assert!(n.contains("eyes") && n.contains("gpt-6-astra") && n.contains("100%") && n.contains("2026-10-28"), "{n}");
        let reached = json!({"rateLimits": {"primary": {"usedPercent": 97}, "rateLimitReachedType": "rate_limit_reached"}});
        assert!(rate_limit_notice(&reached, "eyes", "m").unwrap().contains("rate_limit_reached"));
        assert_eq!(rate_limit_notice(&json!({}), "eyes", "m"), None);
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
        async fn turn_interrupted(&mut self, turn_id: &str) {
            self.write(json!({"method": "turn/completed", "params": {"threadId": "thr", "turn": {"id": turn_id, "status": "interrupted", "items": [], "error": null}}})).await;
        }
        /// The server's own check on `turn/interrupt`: `turnId` is required
        /// and names the turn in flight (Codex 0.163.0 answers `-32600` to
        /// anything else), so the fake insists on it before answering.
        async fn answer_interrupt(&mut self, req: &Value, expected_turn: &str) {
            assert_eq!(req["method"], "turn/interrupt");
            assert_eq!(req["params"]["threadId"], "thr");
            assert_eq!(req["params"]["turnId"], expected_turn, "the interrupt must name the turn in flight: {req}");
            self.answer(req, json!({})).await;
        }
        /// Nothing reaches the server within the window.
        async fn silent_for(&mut self, window: std::time::Duration) {
            assert!(tokio::time::timeout(window, self.lines.next_line()).await.is_err(), "the server was sent a request");
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
            server_baseline: if BASELINE_UNKNOWN.with(|b| b.get()) { None } else { SERVER_PID.with(|p| p.get()).and_then(server_tree) },
            data_dir: DATA_DIR.with(|d| d.borrow().clone()).unwrap_or_else(|| std::path::PathBuf::from("/nonexistent-data-dir")),
            session_id: "s-test".into(),
        };
        tokio::spawn(driver.run(incoming, event_tx, input_rx, control_rx));
        Harness { fake: Fake { lines: tokio::io::BufReader::new(s_read).lines(), writer: s_write }, events, input, control }
    }

    thread_local! {
        /// A stand-in app-server pid for one test's harness (`None` otherwise).
        static SERVER_PID: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
        /// A data dir with rendered handoff files, for the compaction test.
        static DATA_DIR: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
        /// Build the next harness as if `ps` had failed at the driver's start.
        static BASELINE_UNKNOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// A compaction puts the rendered handoff file (and the user's words)
    /// back through `thread/inject_items`, then reports what it put back;
    /// when the injection is refused, the text rides the next page instead.
    #[tokio::test]
    async fn a_compaction_puts_the_handoff_back_or_prefixes_the_next_page() {
        let data = tempfile::tempdir().unwrap();
        let local = crate::paths::Paths::for_data_dir(data.path().to_path_buf()).local_dir;
        let dir = crate::agents::handoff::dir_under(&local);
        let path = crate::agents::handoff::file_path(&dir, "s-test", "eyes").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[bot-hq] Your context was just compacted.\nHANDOFF BODY HERE\n").unwrap();
        DATA_DIR.with(|d| *d.borrow_mut() = Some(data.path().to_path_buf()));
        let mut h = harness();
        DATA_DIR.with(|d| *d.borrow_mut() = None);
        h.fake.write(json!({"method": "thread/compacted", "params": {"threadId": "thr", "turnId": "t0"}})).await;
        let req = h.fake.read().await;
        assert_eq!(req["method"], "thread/inject_items");
        assert_eq!(req["params"]["threadId"], "thr");
        let text = req["params"]["items"][0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("HANDOFF BODY HERE"), "{text}");
        assert_eq!(req["params"]["items"][0]["role"], "user");
        h.fake.answer(&req, json!({})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Compacted { .. }));
        assert!(matches!(next_event(&mut h).await, AgentEvent::HandoffInjected { has_doc: true }));
        // Refused: the text goes in front of the next page.
        h.fake.write(json!({"method": "thread/compacted", "params": {"threadId": "thr", "turnId": "t1"}})).await;
        let req = h.fake.read().await;
        assert_eq!(req["method"], "thread/inject_items");
        h.fake.write(json!({"id": req["id"], "error": {"code": -32600, "message": "no"}})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Compacted { .. }));
        h.input.send(OutgoingUserMessage::text("next page")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        let text = start["params"]["input"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("[bot-hq] Your context was just compacted.") && text.ends_with("next page"), "{text}");
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
    /// fails, so the participant is unguarded until it runs again. The
    /// interrupt names the turn in flight, and a completion in time clears
    /// its deadline so nothing is reaped.
    #[tokio::test]
    async fn a_failed_gate_hook_run_interrupts_and_stalls() {
        let mut h = harness();
        h.input.send(OutgoingUserMessage::text("run it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        // A hook that BLOCKED is the gate working: nothing happens.
        h.fake.write(json!({"method": "hook/completed", "params": {"threadId": "thr", "run": {"eventName": "preToolUse", "status": "blocked", "sourcePath": "/h/hooks.json"}}})).await;
        h.fake.silent_for(std::time::Duration::from_millis(300)).await;
        // A hook that FAILED: the turn is interrupted, the participant stalled.
        h.fake.write(json!({"method": "hook/completed", "params": {"threadId": "thr", "run": {"eventName": "preToolUse", "status": "failed", "sourcePath": "/h/hooks.json", "entries": [{"kind": "stderr", "text": "No such file or directory"}]}}})).await;
        let req = h.fake.read().await;
        h.fake.answer_interrupt(&req, "t1").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Health(AgentHealth::Stalled)));
        assert!(matches!(next_event(&mut h).await, AgentEvent::Notice(n) if n.contains("FAILED")));
        // The turn ends in time: the deadline is cleared, the driver lives on.
        h.fake.turn_interrupted("t1").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { stop_reason: Some(s), .. } if s == "interrupted"));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(900), h.events.recv()).await.is_err(), "nothing is reaped after a completion");
        h.input.send(OutgoingUserMessage::text("again")).await.unwrap();
        h.fake.hooks_armed().await;
        assert_eq!(h.fake.read().await["method"], "turn/start");
    }

    /// The failed-hook interrupt carries the same deadline as a user's: a turn
    /// that ignores it is given up (the notice is posted and the driver ends)
    /// instead of running on unguarded.
    #[tokio::test]
    async fn a_failed_gate_hook_whose_turn_does_not_end_reaches_the_deadline() {
        let mut h = harness();
        h.input.send(OutgoingUserMessage::text("run it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        h.fake.write(json!({"method": "hook/completed", "params": {"threadId": "thr", "run": {"eventName": "preToolUse", "status": "failed", "sourcePath": "/h/hooks.json", "entries": []}}})).await;
        let req = h.fake.read().await;
        h.fake.answer_interrupt(&req, "t1").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::Health(AgentHealth::Stalled)));
        assert!(reaped_then_closed(&mut h).await, "the deadline fired and the driver ended");
    }

    /// Drain the events until the driver ends; whether the reaped notice was
    /// among them.
    async fn reaped_then_closed(h: &mut Harness) -> bool {
        let mut saw_notice = false;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(6), h.events.recv()).await.unwrap() {
                Some(AgentEvent::Notice(n)) if n.contains("reaped") => saw_notice = true,
                Some(_) => {}
                None => break,
            }
        }
        saw_notice
    }

    /// G2/P7: an interrupt is given a deadline. The 2026-10-21 probe saw Codex
    /// answer `turn/interrupt` at once and let the running command finish
    /// (`sleep 100` lived 70 s past it; the probe's request shape was not
    /// checked), so a user message, a stray-turn stop or a halt could wait
    /// out a `cargo build`. When no `turn/completed` arrives in
    /// time the driver kills the running command's process tree (its
    /// `processId`); when the turn still does not end, the driver ends and the
    /// app-server is reaped — the next message resumes the thread.
    /// EYES blocking d681ab5a: the item's `processId` is NOT an OS pid (a
    /// random i32 PTY id), so the deadline never signals it — the tree killed
    /// is the app-server's own, the pid bot-hq spawned. A canary process
    /// holding the number Codex reported must survive.
    #[cfg(unix)]
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
        h.fake.answer_interrupt(&req, "t1").await;
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
        assert!(reaped_then_closed(&mut h).await, "the notice was posted before the driver ended");
        assert!(canary.try_wait().unwrap().is_none(), "the process holding Codex's reported id must survive");
        canary.kill().unwrap();
        let _ = canary.wait();
    }

    /// A stand-in app-server whose "own" tree is one `sleep 300`, and which
    /// starts a `sleep 400` — "the command" — on SIGUSR1.
    #[cfg(unix)]
    fn command_server() -> (ServerGuard, u32, Vec<u32>) {
        let shell = crate::process::command("sh")
            .args(["-c", "sleep 300 & trap 'sleep 400 &' USR1; while :; do wait; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = shell.id();
        let own = wait_for_descendants(pid, 1);
        (ServerGuard(shell), pid, own)
    }

    /// The stand-in server's tree dies with the test, pass or fail — a
    /// leftover would hold the harness's pipes open.
    #[cfg(unix)]
    struct ServerGuard(std::process::Child);
    #[cfg(unix)]
    impl ServerGuard {
        /// Whether the stand-in server itself has exited (reaped here — the
        /// test is its parent, so a bare `kill(pid, 0)` would still see it).
        fn exited(&mut self) -> bool {
            for _ in 0..40 {
                if self.0.try_wait().ok().flatten().is_some() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            false
        }
    }
    #[cfg(unix)]
    impl Drop for ServerGuard {
        fn drop(&mut self) {
            crate::process::kill_process_tree(self.0.id());
            let _ = self.0.wait();
        }
    }

    /// SIGKILL is delivered asynchronously: wait (up to 2 s) for the pid to go.
    #[cfg(unix)]
    fn wait_dead(pid: u32) -> bool {
        for _ in 0..40 {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    #[cfg(unix)]
    fn wait_for_descendants(pid: u32, at_least: usize) -> Vec<u32> {
        for _ in 0..100 {
            let d = crate::process::descendants(pid);
            if d.len() >= at_least {
                return d;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the stand-in server never had {at_least} child(ren)");
    }

    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    /// Tell the stand-in server to start "the command"; returns its pid.
    #[cfg(unix)]
    fn start_command(server_pid: u32, own: &[u32]) -> u32 {
        unsafe {
            libc::kill(server_pid as i32, libc::SIGUSR1);
        }
        for _ in 0..100 {
            if let Some(p) = crate::process::descendants(server_pid).into_iter().find(|p| !own.contains(p)) {
                return p;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the command never started");
    }

    /// Codex answers an interrupt by ending the turn and lets the command run
    /// on (measured 2026-10-21: `sleep 180` outlived its interrupted turn). The
    /// driver stops it on the completion that follows — before the completion
    /// is reported and before the next queued page starts a turn — and leaves
    /// the app-server's own tree alone.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_interrupted_turns_command_is_stopped_before_the_turn_ends() {
        let (server, server_pid, own) = command_server();
        SERVER_PID.with(|p| p.set(Some(server_pid)));
        let mut h = harness();
        SERVER_PID.with(|p| p.set(None));
        h.input.send(OutgoingUserMessage::text("build it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        let command = start_command(server_pid, &own);
        h.fake.write(json!({"method": "item/started", "params": {"item": {"type": "commandExecution", "id": "exec-1", "command": "/bin/zsh -lc 'sleep 400'", "processId": "12345", "status": "inProgress"}}})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::ToolUse { .. }));
        // A page that arrives mid-turn and is refused: it starts the next turn.
        h.input.send(OutgoingUserMessage::text("next page")).await.unwrap();
        let steer = h.fake.read().await;
        h.fake.write(json!({"id": steer["id"], "error": {"code": -32600, "message": "turn t1 is not active"}})).await;
        // The interrupt, then Codex ends the turn — the command runs on.
        h.control.send(ControlRequest::interrupt("user-preempt")).await.unwrap();
        let req = h.fake.read().await;
        h.fake.answer_interrupt(&req, "t1").await;
        assert!(alive(command), "Codex left the command running");
        h.fake.turn_interrupted("t1").await;
        // The queued page's turn starts only once the command is GONE — not
        // merely signalled: no wait here, the driver did the waiting.
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        assert_eq!(start["method"], "turn/start");
        assert!(!alive(command), "the command was gone before the next turn started");
        assert!(own.iter().all(|p| alive(*p)) && alive(server_pid), "the app-server's own tree is untouched");
        let notice = next_event(&mut h).await;
        assert!(matches!(&notice, AgentEvent::Notice(n) if n.contains("1 process was stopped")), "{notice:?}");
        drop(server);
    }

    /// An atomic git/migration step in flight at the interrupt is given until
    /// the interrupt's own deadline to finish by itself, then stopped.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_atomic_command_gets_the_deadlines_grace_before_it_is_stopped() {
        let (server, server_pid, own) = command_server();
        SERVER_PID.with(|p| p.set(Some(server_pid)));
        let mut h = harness();
        SERVER_PID.with(|p| p.set(None));
        h.input.send(OutgoingUserMessage::text("commit it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        let command = start_command(server_pid, &own);
        // The commit, and a plain command beside it: the plain one's completion
        // must not hide the commit (EYES 1331cc1b).
        h.fake.write(json!({"method": "item/started", "params": {"item": {"type": "commandExecution", "id": "exec-1", "command": "/bin/zsh -lc 'git commit -m x'", "status": "inProgress"}}})).await;
        h.fake.write(json!({"method": "item/started", "params": {"item": {"type": "commandExecution", "id": "exec-2", "command": "/bin/zsh -lc 'ls'", "status": "inProgress"}}})).await;
        h.fake.write(json!({"method": "item/completed", "params": {"item": {"type": "commandExecution", "id": "exec-2", "command": "/bin/zsh -lc 'ls'", "status": "completed", "exitCode": 0}}})).await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::ToolUse { .. }));
        assert!(matches!(next_event(&mut h).await, AgentEvent::ToolUse { .. }));
        assert!(matches!(next_event(&mut h).await, AgentEvent::ToolResult { .. }));
        let t0 = std::time::Instant::now();
        h.control.send(ControlRequest::interrupt("user-preempt")).await.unwrap();
        let req = h.fake.read().await;
        h.fake.answer_interrupt(&req, "t1").await;
        h.fake.turn_interrupted("t1").await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(alive(command), "the commit is given the atomic deadline (800 ms here) to finish");
        let done = next_event(&mut h).await;
        assert!(matches!(&done, AgentEvent::Notice(n) if n.contains("1 process was stopped")), "{done:?}");
        assert!(t0.elapsed() >= std::time::Duration::from_millis(700), "not before the deadline: {:?}", t0.elapsed());
        assert!(!alive(command), "gone before the notice, not merely signalled");
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { stop_reason: Some(s), .. } if s == "interrupted"));
        drop(server);
    }

    /// An app-server whose own tree could not be listed at start has no
    /// baseline: at the first interrupted completion the driver cannot tell a
    /// command from the native binary, so it gives the whole server up (the
    /// reaped notice names why) instead of killing by guess (EYES 37ff33f6).
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unknown_baseline_reaps_the_server_instead_of_guessing() {
        let (server, server_pid, own) = command_server();
        SERVER_PID.with(|p| p.set(Some(server_pid)));
        BASELINE_UNKNOWN.with(|b| b.set(true));
        let mut h = harness();
        BASELINE_UNKNOWN.with(|b| b.set(false));
        SERVER_PID.with(|p| p.set(None));
        h.input.send(OutgoingUserMessage::text("build it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        let command = start_command(server_pid, &own);
        h.control.send(ControlRequest::interrupt("user-preempt")).await.unwrap();
        let req = h.fake.read().await;
        h.fake.answer_interrupt(&req, "t1").await;
        h.fake.turn_interrupted("t1").await;
        let mut saw = false;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(6), h.events.recv()).await.unwrap() {
                Some(AgentEvent::Notice(n)) if n.contains("could not tell") && n.contains("never listed") => saw = true,
                Some(_) => {}
                None => break,
            }
        }
        assert!(saw, "the reaped notice names the unknown baseline");
        let mut server = server;
        assert!(wait_dead(command) && server.exited(), "the whole server tree is gone");
        drop(server);
    }

    /// A helper Codex starts lazily under the app-server — `codex-code-mode-host`
    /// at the first `exec` call, after the baseline — is Codex's, not a
    /// command: it lives beside the server's own `codex` binary and the
    /// server started it. The cleanup leaves it alone and stops everything
    /// else beside it: a user's `codex-build` from another directory, and a
    /// nested `codex` a command started (EYES f3e7f82f — never by name alone).
    #[cfg(unix)]
    #[tokio::test]
    async fn a_codex_helper_started_after_the_baseline_survives_the_cleanup() {
        let sleep = String::from_utf8_lossy(&crate::process::command("sh").args(["-c", "command -v sleep"]).output().unwrap().stdout).trim().to_string();
        // Codex's own directory: the server's `codex` binary and its helper
        // (symlinks to `sleep` — a COPY of a macOS system binary loses its
        // signature and dies at exec). A user's tool lives elsewhere.
        let vendor = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        for (dir, name) in [(&vendor, "codex"), (&vendor, "codex-code-mode-host"), (&elsewhere, "codex-build")] {
            std::os::unix::fs::symlink(&sleep, dir.path().join(name)).unwrap();
        }
        let codex = vendor.path().join("codex").display().to_string();
        let helper = vendor.path().join("codex-code-mode-host").display().to_string();
        let user_tool = elsewhere.path().join("codex-build").display().to_string();
        // The stand-in server: a shell whose own child is the `codex` binary
        // (so the tree has a codex directory). On SIGUSR1 it starts the lazy
        // helper, a user's codex-build, and a command (`sh`) that runs a
        // nested codex.
        // …and a `codex` launched straight from the installation directory
        // by the server itself (EYES dfcddc4e): a command, not a helper.
        let script = format!(
            "{codex} 300 & trap '{helper} 400 & {user_tool} 400 & {codex} 400 & sh -c \"{codex} 401; true\" &' USR1; while :; do wait; done"
        );
        let shell = crate::process::command("sh").args(["-c", &script]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let server_pid = shell.id();
        let own = wait_for_descendants(server_pid, 1);
        let server = ServerGuard(shell);
        SERVER_PID.with(|p| p.set(Some(server_pid)));
        let mut h = harness();
        SERVER_PID.with(|p| p.set(None));
        h.input.send(OutgoingUserMessage::text("build it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        unsafe {
            libc::kill(server_pid as i32, libc::SIGUSR1);
        }
        // Wait for all four newcomers: the helper, codex-build, the sh, and
        // the nested codex under the sh.
        let (mut helper_pid, mut user_pid, mut nested_pid, mut direct_pid) = (None, None, None, None);
        for _ in 0..100 {
            for d in crate::process::descendants_named(server_pid).unwrap_or_default() {
                if own.contains(&d.pid) {
                    continue;
                }
                if d.program == helper {
                    helper_pid = Some(d.pid);
                } else if d.program == user_tool {
                    user_pid = Some(d.pid);
                } else if d.program == codex && d.ppid != server_pid {
                    nested_pid = Some(d.pid);
                } else if d.program == codex && d.ppid == server_pid {
                    direct_pid = Some(d.pid);
                }
            }
            if helper_pid.is_some() && user_pid.is_some() && nested_pid.is_some() && direct_pid.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let helper_pid = helper_pid.expect("the helper started");
        let user_pid = user_pid.expect("codex-build started");
        let nested_pid = nested_pid.expect("the nested codex started under a command");
        let direct_pid = direct_pid.expect("the directly launched codex started");
        h.control.send(ControlRequest::interrupt("user-preempt")).await.unwrap();
        let req = h.fake.read().await;
        h.fake.answer_interrupt(&req, "t1").await;
        h.fake.turn_interrupted("t1").await;
        let notice = next_event(&mut h).await;
        // codex-build, the sh, the nested codex and the directly launched
        // codex: four stopped; the helper not.
        assert!(matches!(&notice, AgentEvent::Notice(n) if n.contains("4 processes were stopped")), "{notice:?}");
        assert!(!alive(user_pid), "a user's codex-build is a command");
        assert!(!alive(nested_pid), "a nested codex under a command is a command");
        assert!(!alive(direct_pid), "a codex launched straight from the installation directory is a command");
        assert!(alive(helper_pid), "Codex's helper, beside its binary and started by it, is left alone");
        assert!(own.iter().all(|p| alive(*p)), "the server's own codex binary is untouched");
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { .. }));
        drop(server);
    }

    /// A completion that no interrupt asked for stops nothing: a command the
    /// model left running on purpose is its own business.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_completion_without_an_interrupt_stops_nothing() {
        let (server, server_pid, own) = command_server();
        SERVER_PID.with(|p| p.set(Some(server_pid)));
        let mut h = harness();
        SERVER_PID.with(|p| p.set(None));
        h.input.send(OutgoingUserMessage::text("serve it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        let command = start_command(server_pid, &own);
        h.fake.turn_completed("t1").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { is_error: false, .. }));
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(alive(command), "nothing asked for it to stop");
        drop(server);
    }

    /// The commands in flight are tracked by item: a plain command starting
    /// or completing beside an atomic one does not hide the atomic one, in
    /// either order (EYES 1331cc1b).
    #[test]
    fn atomic_work_is_tracked_per_command_item() {
        let started = |id: &str, cmd: &str| Incoming::Notification(super::super::client::Notification { method: "item/started".into(), params: json!({"item": {"type": "commandExecution", "id": id, "command": cmd}}) });
        let completed = |id: &str| Incoming::Notification(super::super::client::Notification { method: "item/completed".into(), params: json!({"item": {"type": "commandExecution", "id": id}}) });
        let mut running = RunningCommands::new();
        track_running_command(&started("a", "git commit -m x"), &mut running);
        track_running_command(&started("b", "ls"), &mut running);
        assert!(atomic_in_flight(&running), "a plain command starting beside the commit");
        track_running_command(&completed("b"), &mut running);
        assert!(atomic_in_flight(&running), "the plain one completing first");
        track_running_command(&completed("a"), &mut running);
        assert!(!atomic_in_flight(&running));
        // The other order.
        track_running_command(&started("c", "ls"), &mut running);
        track_running_command(&started("d", "git push"), &mut running);
        assert!(atomic_in_flight(&running));
        track_running_command(&completed("d"), &mut running);
        assert!(!atomic_in_flight(&running), "the plain one still runs, nothing atomic does");
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
        h.fake.answer_interrupt(&req, "t1").await;
        h.fake.turn_interrupted("t1").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { stop_reason: Some(s), .. } if s == "interrupted"));
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        // Still alive: a new page starts a turn.
        h.input.send(OutgoingUserMessage::text("again")).await.unwrap();
        h.fake.hooks_armed().await;
        assert_eq!(h.fake.read().await["method"], "turn/start");
    }

    /// The interrupt names the turn in flight — the second turn's id, not the
    /// first's — and with no turn in flight nothing is sent, nothing is
    /// reaped, and the next page still starts a turn (EYES, plan review).
    #[tokio::test]
    async fn an_interrupt_names_the_turn_in_flight_and_an_idle_one_sends_nothing() {
        let mut h = harness();
        h.input.send(OutgoingUserMessage::text("one")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        h.fake.turn_completed("t1").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { is_error: false, .. }));
        h.input.send(OutgoingUserMessage::text("two")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t2", "status": "inProgress", "items": []}})).await;
        h.control.send(ControlRequest::interrupt("cancel")).await.unwrap();
        let req = h.fake.read().await;
        h.fake.answer_interrupt(&req, "t2").await;
        h.fake.turn_interrupted("t2").await;
        assert!(matches!(next_event(&mut h).await, AgentEvent::TurnComplete { stop_reason: Some(s), .. } if s == "interrupted"));
        // Idle: no request, no deadline, no reap.
        h.control.send(ControlRequest::interrupt("cancel")).await.unwrap();
        h.fake.silent_for(std::time::Duration::from_millis(300)).await;
        assert!(tokio::time::timeout(std::time::Duration::from_millis(900), h.events.recv()).await.is_err(), "an idle interrupt reaps nothing");
        h.input.send(OutgoingUserMessage::text("three")).await.unwrap();
        h.fake.hooks_armed().await;
        assert_eq!(h.fake.read().await["method"], "turn/start");
    }

    /// A rejected interrupt (the server's `-32600`) is no reason to wait: the
    /// deadline still gives the turn up.
    #[tokio::test]
    async fn a_rejected_interrupt_still_reaches_the_deadline() {
        let mut h = harness();
        h.input.send(OutgoingUserMessage::text("build it")).await.unwrap();
        h.fake.hooks_armed().await;
        let start = h.fake.read().await;
        h.fake.answer(&start, json!({"turn": {"id": "t1", "status": "inProgress", "items": []}})).await;
        h.control.send(ControlRequest::interrupt("cancel")).await.unwrap();
        let req = h.fake.read().await;
        assert_eq!(req["method"], "turn/interrupt");
        h.fake.write(json!({"id": req["id"], "error": {"code": -32600, "message": "Invalid request: missing field `turnId`"}})).await;
        assert!(reaped_then_closed(&mut h).await, "the deadline fired and the driver ended");
    }

    #[tokio::test]
    async fn an_idle_interrupt_sends_nothing_and_a_closed_stream_ends_the_driver() {
        let mut h = harness();
        h.control.send(ControlRequest::interrupt("cancel")).await.unwrap();
        h.fake.silent_for(std::time::Duration::from_millis(300)).await;
        drop(h.fake);
        assert!(tokio::time::timeout(std::time::Duration::from_secs(5), h.events.recv()).await.unwrap().is_none(), "the event stream closes when the driver ends");
    }
}
