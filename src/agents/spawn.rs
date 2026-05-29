//! Spawn a `claude-code` subprocess wired up with stream-json IO + the
//! MCP-signaling server. Returns an `AgentHandle` the core layer drives.

use anyhow::{Context, Result};
use std::sync::LazyLock;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

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

/// Sync, signal-safe child reaper. Walks the registered PIDs and sends
/// SIGKILL via libc directly — no tokio, no async, no Drop chain.
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
        // SAFETY: libc::kill is async-signal-safe + thread-safe; valid
        // pids are u32 from std/tokio's child.id() which fits in i32 for
        // every realistic process number on darwin/linux.
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
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
    },
    /// System/init event — agent is ready and reporting its session metadata.
    /// (The wire `SystemEvent::Init` also carries `model`/`cwd`, but no
    /// consumer reads them, so they are not forwarded here.)
    Init { session_id: Option<String> },
    /// Process exited. Carries exit-status string for log/observability.
    Exited(String),
    /// Catch-all for fatal errors the supervisor wants to surface.
    Error(String),
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
        CHILD_PIDS.lock().unwrap_or_else(|p| p.into_inner()).insert(pid);
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
    // Brian (HANDS) + Emma (solo) run with `--dangerously-skip-permissions`:
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
        // through it and any role:"system" message is hoisted into the
        // top-level `system` field before it reaches DeepSeek. `--bare` is
        // retained as defense-in-depth + to keep Rain lean (skips plugin sync
        // + hooks + LSP + CLAUDE.md autodiscovery), but it does NOT eliminate
        // the injection on claude-code >= 2.1.156 — a fresh --bare Rain still
        // 400s on a fixed messages[11], which is why the proxy exists.
        // `--bare` still honors --mcp-config (signaling) + ANTHROPIC_AUTH_TOKEN
        // bearer auth. Brian/Emma hit real Anthropic, so they skip --bare.
        cmd.arg("--bare");
        cmd.args(["--permission-mode", "dontAsk"]);
        cmd.args([
            "--allowedTools",
            "Read Grep Glob WebFetch WebSearch TodoWrite BashOutput KillShell Bash mcp__bot-hq-signaling",
        ]);
        cmd.args([
            "--disallowedTools",
            "Edit Write NotebookEdit Task \
             Bash(git commit:*) Bash(git push:*) Bash(git branch:*) \
             Bash(git checkout:*) Bash(git switch:*) Bash(git reset:*) \
             Bash(git merge:*) Bash(git rebase:*) Bash(git add:*) \
             Bash(git stash:*) Bash(git restore:*) Bash(git rm:*) \
             Bash(git tag:*) Bash(git cherry-pick:*) Bash(git apply:*) \
             Bash(gh pr:*) Bash(gh issue:*) Bash(gh release:*) \
             Bash(gh api:*) Bash(gh repo:*)",
        ]);
    } else {
        cmd.arg("--dangerously-skip-permissions");
    }

    // Env-vars per ARCHITECTURE.md "Agents" section.
    cmd.env("ANTHROPIC_MODEL", &cfg.config.model_name);
    // BOT_HQ_SESSION_ID is read by the git pre-push hook to overlay
    // session-scoped approvals onto the resolved policy.
    cmd.env("BOT_HQ_SESSION_ID", &cfg.session_id);
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
    // Agents with no base_url (Brian/Emma → real Anthropic) get no override
    // and never touch the proxy.
    if let Some(base) = crate::agents::llm_proxy::resolve_anthropic_base_url(
        cfg.config.base_url.as_deref(),
        crate::agents::llm_proxy::proxy_addr(),
    ) {
        cmd.env("ANTHROPIC_BASE_URL", base);
    }

    if let Some(wd) = &cfg.working_dir {
        cmd.current_dir(wd);
    }

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
        }
    }

    #[test]
    fn command_has_required_flags() {
        let argv = debug_command(&cfg());
        assert_eq!(argv[0], "claude");
        assert!(argv.iter().any(|a| a == "-p"));
        assert!(argv.iter().any(|a| a == "--verbose"));
        assert!(argv.windows(2).any(|w| w[0] == "--input-format" && w[1] == "stream-json"));
        assert!(argv.windows(2).any(|w| w[0] == "--output-format" && w[1] == "stream-json"));
        assert!(argv.windows(2).any(|w| w[0] == "--mcp-config" && w[1] == "/tmp/mcp.json"));
        assert!(argv.iter().any(|a| a == "--strict-mcp-config"));
        assert!(argv.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(argv.windows(2).any(|w| w[0] == "--append-system-prompt" && w[1] == "be terse"));
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
            argv.windows(2).any(|w| w[0] == "--permission-mode" && w[1] == "dontAsk"),
            "expected `--permission-mode dontAsk`: {argv:?}"
        );
        // Allowlist keeps read-only investigation + the signaling MCP.
        let allowed = argv
            .windows(2)
            .find(|w| w[0] == "--allowedTools")
            .map(|w| w[1].clone())
            .expect("--allowedTools present");
        for t in ["Read", "Grep", "Glob", "Bash", "mcp__bot-hq-signaling"] {
            assert!(allowed.contains(t), "allowlist missing {t}: {allowed}");
        }
        // Denylist covers the mutation surface from the 2026-05-28 incident.
        let denied = argv
            .windows(2)
            .find(|w| w[0] == "--disallowedTools")
            .map(|w| w[1].clone())
            .expect("--disallowedTools present");
        for t in ["Edit", "Write", "NotebookEdit", "Bash(git commit:*)", "Bash(git push:*)", "Bash(gh issue:*)", "Bash(gh pr:*)"] {
            assert!(denied.contains(t), "denylist missing {t}: {denied}");
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
    fn rain_gets_bare_minimal_mode() {
        // Rain talks to a third-party Anthropic-compatible gateway (DeepSeek
        // via ANTHROPIC_BASE_URL). claude-code >= 2.1.156 serializes a
        // SessionStart hook's `additionalContext` (the superpowers plugin
        // injects one) as a `role:"system"` entry in the `messages` array.
        // The real Anthropic API tolerates it; stricter gateways reject it
        // ("unknown variant `system`, expected user or assistant" → 400).
        // `--bare` skips plugin sync so the injection never happens, while
        // still honoring --mcp-config and ANTHROPIC_AUTH_TOKEN bearer auth.
        let mut c = cfg();
        c.agent_name = "rain".into();
        c.config.agent_name = "rain".into();
        let argv = debug_command(&c);
        assert!(
            argv.iter().any(|a| a == "--bare"),
            "Rain must run --bare so plugin SessionStart hooks can't inject a \
             system-role message that non-Anthropic gateways reject: {argv:?}"
        );
    }

    #[test]
    fn resume_session_id_emits_resume_flag() {
        let mut c = cfg();
        c.resume_session_id = Some("abc-123-uuid".into());
        let argv = debug_command(&c);
        assert!(
            argv.windows(2).any(|w| w[0] == "--resume" && w[1] == "abc-123-uuid"),
            "expected `--resume abc-123-uuid` in argv: {argv:?}"
        );
    }
}
