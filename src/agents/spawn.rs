//! Spawn a `claude-code` subprocess wired up with stream-json IO + the
//! MCP-signaling server. Returns an `AgentHandle` the core layer drives.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::agents::events;
use crate::agents::input;
use crate::agents::protocol::OutgoingUserMessage;
use crate::storage::AgentConfig;

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
    },
    /// System/init event — agent is ready and reporting its session metadata.
    Init {
        model: Option<String>,
        cwd: Option<String>,
        session_id: Option<String>,
    },
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

    let stdin = child.stdin.take().context("subprocess missing stdin")?;
    let stdout = child.stdout.take().context("subprocess missing stdout")?;
    let stderr = child.stderr.take().context("subprocess missing stderr")?;

    tokio::spawn(events::pump_events(stdout, event_tx.clone()));
    tokio::spawn(events::pump_stderr(stderr, cfg.agent_name.clone()));
    tokio::spawn(input::pump_inputs(stdin, input_rx));

    let event_tx_for_lifecycle = event_tx.clone();
    let agent_name = cfg.agent_name.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = kill_rx => {
                info!(agent = %agent_name, "kill signalled");
                let _ = child.kill().await;
                let _ = event_tx_for_lifecycle
                    .send(AgentEvent::Exited("killed by supervisor".into()))
                    .await;
            }
            res = child.wait() => {
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

    // bot-hq is the permission layer (policy.yaml + UI dialogs gate every
    // risky tool call). Letting claude-code prompt the user in parallel would
    // be a double-gate that only confuses things — and worse, those prompts
    // appear in the agent's stream-json output, never reach our UI, and the
    // agent hangs. So skip claude-code's built-in permission prompts entirely
    // and rely on our own enforcement (src/policy + signaling/bridge).
    cmd.arg("--dangerously-skip-permissions");

    // Env-vars per ARCHITECTURE.md "Agents" section.
    cmd.env("ANTHROPIC_MODEL", &cfg.config.model_name);
    if let Some(token) = &cfg.config.auth_token {
        if !token.is_empty() {
            cmd.env("ANTHROPIC_AUTH_TOKEN", token);
        }
    }
    if let Some(base) = &cfg.config.base_url {
        if !base.is_empty() {
            cmd.env("ANTHROPIC_BASE_URL", base);
        }
    }

    if let Some(wd) = &cfg.working_dir {
        cmd.current_dir(wd);
    }

    cmd
}

/// Build the path-string form of the claude command for diagnostics / logging.
/// Not used by spawn; tests use it to assert flag set.
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
    }
}
