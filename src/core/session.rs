//! Session lifecycle: open + close.
//!
//! `open_session` is the load-bearing entry: persists the row, reads the
//! system prompt from CL, spawns Brian + Rain, kicks off the duo event pumps,
//! and registers the session in `AppState`.

use crate::agents::{spawn_agent, AgentEvent, AgentHandle, SpawnConfig};
use crate::core::duo::{pump_agent, DuoConfig};
use crate::core::ipav::IpavState;
use crate::paths::Paths;
use crate::signaling::{mcp_config_json, SignalingBridge};
use crate::storage::{AgentConfig, Author, MessageKind, Session, Storage};
use tokio::sync::mpsc;
#[allow(unused_imports)]
use crate::core::ipav::IpavPhase;
use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

pub struct OpenSessionRequest {
    pub title: String,
    pub working_repo_path: Option<PathBuf>,
}

/// A live session — the handles owned by `AppState`.
pub struct SessionHandle {
    pub id: String,
    pub title: String,
    pub working_repo_path: Option<PathBuf>,
    pub ipav: Arc<Mutex<IpavState>>,
    pub brian: AgentHandle,
    pub rain: AgentHandle,
    /// Shared "duo is awaiting user input" flag. Set by the bridge when any
    /// user-blocking MCP tool fires; checked by the duo pumps before they
    /// forward Brian↔Rain chunks; cleared by `core::AppState::broadcast` when
    /// the user replies. See `duo::DuoConfig::is_awaiting`.
    pub awaiting: Arc<std::sync::atomic::AtomicBool>,
    /// Keeps the mcp-config temp files alive for the lifetime of the session.
    _mcp_temp: TempDir,
}

/// Emma's solo singleton session — different shape from `SessionHandle` because
/// Emma is a single agent with no duo / peer-forwarding / IPAV state.
pub struct EmmaHandle {
    pub agent: AgentHandle,
    _mcp_temp: TempDir,
}

pub async fn open_session(
    req: OpenSessionRequest,
    paths: &Paths,
    storage: Storage,
    bridge: Arc<SignalingBridge>,
    signaling_addr: SocketAddr,
) -> Result<SessionHandle> {
    let id = Uuid::new_v4().to_string();
    let session = storage
        .create_session(
            &id,
            &req.title,
            req.working_repo_path.as_ref().and_then(|p| p.to_str()),
        )
        .await
        .context("creating session row")?;

    spawn_session_handle(
        session,
        req.working_repo_path,
        paths,
        storage,
        bridge,
        signaling_addr,
    )
    .await
}

/// Spawn subprocesses for a session row that ALREADY EXISTS in storage (e.g.,
/// the seeded Emma singleton). Idempotency check belongs to the caller — this
/// always spawns a fresh handle.
pub async fn spawn_existing_session(
    session_id: &str,
    paths: &Paths,
    storage: Storage,
    bridge: Arc<SignalingBridge>,
    signaling_addr: SocketAddr,
) -> Result<SessionHandle> {
    let session = storage
        .get_session(session_id)
        .await
        .context("looking up session row")?
        .ok_or_else(|| anyhow::anyhow!("session {session_id} not found"))?;
    let working_repo_path = session.working_repo_path.as_ref().map(PathBuf::from);
    spawn_session_handle(session, working_repo_path, paths, storage, bridge, signaling_addr)
        .await
}

/// Shared spawn logic for both fresh and existing sessions: spawn Brian + Rain,
/// kick the duo pumps, return the handle.
async fn spawn_session_handle(
    session: Session,
    working_repo_path: Option<PathBuf>,
    paths: &Paths,
    storage: Storage,
    bridge: Arc<SignalingBridge>,
    signaling_addr: SocketAddr,
) -> Result<SessionHandle> {
    let project = working_repo_path
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(str::to_string);

    // Register session→project with the bridge so policy-aware MCP tools can
    // resolve `<data_dir>/projects/<project>/policy.yaml` per-call.
    bridge
        .register_session(session.id.clone(), project.clone())
        .await;

    // Audit policy.yaml files for mutations BEFORE we load them into the
    // system prompt. If the agent (or some other process) modified a policy
    // file between sessions, we want it logged. v1 is audit-only.
    if let Err(err) = crate::policy::audit_policy_files(
        &paths.data_dir,
        project.as_deref(),
        bridge.violations_log(),
        &session.id,
        "<session-spawn>",
    ) {
        tracing::warn!(%err, "policy audit failed at session spawn");
    }

    // Install git hooks in the working repo as the mechanical backstop.
    // Per DeepSeek-V4-Pro's review: MCP tools = auditable primary path,
    // git hooks = unconditional enforcement. Failure to install is non-fatal
    // (logged warn) — the agent's MCP tool calls still provide the primary
    // safety layer; we just lose the backstop until the user fixes the repo.
    if let Some(repo) = working_repo_path.as_ref() {
        match crate::policy::install_hooks(repo, &paths.data_dir, project.as_deref()) {
            Ok(report) if report.not_a_git_repo => {
                tracing::info!(
                    repo = %repo.display(),
                    "working_repo_path has no .git/ — skipping hook install"
                );
            }
            Ok(report) => {
                tracing::info!(
                    repo = %repo.display(),
                    installed = ?report.installed,
                    updated = ?report.updated,
                    sidecar = ?report.sidecar,
                    unchanged = ?report.unchanged,
                    "git hooks installed for session"
                );
            }
            Err(err) => {
                tracing::warn!(
                    repo = %repo.display(),
                    %err,
                    "failed to install git hooks — MCP-only enforcement active"
                );
            }
        }
    }

    let mcp_temp = TempDir::new().context("creating mcp-config temp dir")?;

    let brian_cfg = storage
        .get_agent_config("brian")
        .await?
        .unwrap_or_else(default_agent_config("brian"));
    let rain_cfg = storage
        .get_agent_config("rain")
        .await?
        .unwrap_or_else(default_agent_config("rain"));

    // Record the model names we're about to spawn with. Session header reads
    // these so it reflects the live (frozen-at-spawn) model, not the current
    // DB value, which can drift after a config swap.
    if let Err(e) = storage
        .set_session_spawn_models(&session.id, &brian_cfg.model_name, &rain_cfg.model_name)
        .await
    {
        warn!(?e, "set_session_spawn_models");
    }

    let brian = spawn_agent_for(
        &session.id,
        "brian",
        brian_cfg,
        paths,
        &project,
        signaling_addr,
        mcp_temp.path(),
        working_repo_path.clone(),
    )
    .await?;
    let rain = spawn_agent_for(
        &session.id,
        "rain",
        rain_cfg,
        paths,
        &project,
        signaling_addr,
        mcp_temp.path(),
        working_repo_path.clone(),
    )
    .await?;

    let ipav = Arc::new(Mutex::new(IpavState::default()));
    let awaiting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Register the flag with the bridge so user-blocking MCP tools can set it
    // synchronously (before the agent's next chunk volleys). The duo pumps
    // read the same Arc, so updates propagate to both pumps with no
    // additional plumbing.
    bridge
        .register_session_awaiting(session.id.clone(), Arc::clone(&awaiting))
        .await;

    // Per-agent pumps need to be spawned BEFORE we move the handles, so we
    // pull the receivers + input senders here. The handles keep their other
    // fields (kill signal, etc.).
    let mut brian_handle = brian;
    let mut rain_handle = rain;

    let brian_events =
        std::mem::replace(&mut brian_handle.event_rx, tokio::sync::mpsc::channel(1).1);
    let rain_events = std::mem::replace(&mut rain_handle.event_rx, tokio::sync::mpsc::channel(1).1);

    let rain_input = rain_handle.input_tx.clone();
    let brian_input = brian_handle.input_tx.clone();

    let storage_clone = storage.clone();
    let ipav_clone = Arc::clone(&ipav);
    let session_id_clone = session.id.clone();
    let brian_cfg = DuoConfig {
        awaiting: Some(Arc::clone(&awaiting)),
        ..DuoConfig::new(session_id_clone, Author::Brian, Author::Rain)
    };
    tokio::spawn(async move {
        pump_agent(brian_cfg, brian_events, rain_input, storage_clone, ipav_clone).await;
    });

    let storage_clone = storage.clone();
    let ipav_clone = Arc::clone(&ipav);
    let session_id_clone = session.id.clone();
    let rain_cfg = DuoConfig {
        awaiting: Some(Arc::clone(&awaiting)),
        ..DuoConfig::new(session_id_clone, Author::Rain, Author::Brian)
    };
    tokio::spawn(async move {
        pump_agent(rain_cfg, rain_events, brian_input, storage_clone, ipav_clone).await;
    });

    info!(session_id = %session.id, title = %session.title, "session opened");

    Ok(SessionHandle {
        id: session.id,
        title: session.title,
        working_repo_path,
        ipav,
        brian: brian_handle,
        rain: rain_handle,
        awaiting,
        _mcp_temp: mcp_temp,
    })
}

#[allow(clippy::too_many_arguments)]
async fn spawn_agent_for(
    session_id: &str,
    agent_name: &str,
    config: AgentConfig,
    paths: &Paths,
    project: &Option<String>,
    signaling_addr: SocketAddr,
    mcp_temp_dir: &std::path::Path,
    working_dir: Option<PathBuf>,
) -> Result<AgentHandle> {
    let system_prompt = read_system_prompt(paths, agent_name, project.as_deref())?;
    let mcp_config_path = mcp_temp_dir.join(format!("{agent_name}-mcp.json"));
    let json = mcp_config_json(signaling_addr, session_id, agent_name);
    std::fs::write(&mcp_config_path, json)
        .with_context(|| format!("writing mcp-config to {}", mcp_config_path.display()))?;

    let spawn_cfg = SpawnConfig {
        agent_name: agent_name.to_string(),
        config,
        system_prompt,
        mcp_config_path: Some(mcp_config_path),
        working_dir,
        claude_bin: None,
    };
    spawn_agent(spawn_cfg).await
}

/// Assemble the system prompt for an agent at spawn time. Layers:
///
///   1. **Hardcoded role** (from `agents::prompts`) — identity + ask-close
///      convention. Baked into the binary so user can't break it.
///   2. **`~/.bot-hq/general-rules.md`** — shared boilerplate (optional).
///   3. **`~/.bot-hq/agents/<name>/custom-instruction.md`** — user-editable
///      overrides per agent (optional).
///   4. **Policy directive block** — rendered from policy.yaml, project-aware.
///
/// Project context (conventions / notes / decisions) is NOT injected here —
/// agents read those via the `Read` tool when assigned a project task. This
/// keeps spawn-time prompts compact and lets sessions hop projects without a
/// fresh spawn.
///
/// Missing optional files are logged at debug and skipped. Policy parse
/// errors propagate — broken YAML should surface loudly.
pub fn read_system_prompt(paths: &Paths, agent: &str, project: Option<&str>) -> Result<String> {
    let mut out = String::new();

    // 1. Hardcoded role.
    let role = crate::agents::role_for(agent);
    if !role.is_empty() {
        out.push_str(role);
        if !out.ends_with("\n\n") {
            out.push_str("\n\n");
        }
    }

    // 1b. CL location anchor + layout reminder. Without this, agents know
    // the Read tool exists but don't proactively consult the CL when given
    // a project task — and they wander into legacy archives by accident.
    out.push_str(&format!(
        "## Context Library\n\n\
         Your Context Library is at `{cl}`. Single source of truth — never \
         reason about any other `~/.bot-hq*` path as current state (those \
         are archives from prior installs).\n\n\
         Per-project layout at `{cl}projects/<project>/`:\n\
         - `conventions.md` — repo, stack, commands, gates, disguise rules\n\
         - `notes.md` — current state, recurring trouble, gotchas\n\
         - `decisions.md` — chronological log of prior decisions (read this \
         before proposing changes that touch the same area — avoids re-doing \
         settled work)\n\
         - `policy.yaml` — machine-enforced gates + forbidden-commit-word list\n\n\
         **Before starting work on a project, Read `conventions.md` + \
         `notes.md` for that project.** Don't ask the user for facts that \
         live in the CL — Read them yourself.\n\n",
        cl = paths.data_dir.display()
    ));

    // 2 + 3. CL slots — optional.
    let slots = [
        paths.data_dir.join("general-rules.md"),
        paths.data_dir
            .join(format!("agents/{agent}/custom-instruction.md")),
    ];
    for slot in slots {
        match std::fs::read_to_string(&slot) {
            Ok(s) if !s.trim().is_empty() => {
                out.push_str(&s);
                if !out.ends_with("\n\n") {
                    out.push_str("\n\n");
                }
            }
            Ok(_) => {} // empty file — silently skip
            Err(err) => {
                tracing::debug!(path = %slot.display(), %err, "optional CL slot absent");
            }
        }
    }

    // 4. Policy directive block — project-aware.
    let policy = crate::policy::Policy::resolve(&paths.data_dir, project)
        .context("resolving project policy")?;
    let block = policy.render_system_prompt_block();
    if !block.is_empty() {
        out.push_str(&block);
        if !out.ends_with("\n\n") {
            out.push_str("\n\n");
        }
    }
    Ok(out)
}

/// Spawn Emma's solo agent against the seeded `"emma"` session row. Single
/// agent, no peer, no IPAV. Kicks a lightweight pump that just persists Emma's
/// events to the messages table.
pub async fn spawn_emma_handle(
    paths: &Paths,
    storage: Storage,
    bridge: Arc<SignalingBridge>,
    signaling_addr: SocketAddr,
) -> Result<EmmaHandle> {
    // Register Emma's session (no project — she's the global helper, not a
    // project-scoped duo). MCP policy lookups will see no project → resolve
    // to general-policy.yaml only.
    bridge.register_session("emma".into(), None).await;

    let mcp_temp = TempDir::new().context("creating emma mcp-config temp dir")?;
    let emma_cfg = storage
        .get_agent_config("emma")
        .await?
        .unwrap_or_else(default_agent_config("emma"));
    let agent = spawn_agent_for(
        "emma", // session_id matches the seeded row
        "emma", // agent_name → hardcoded EMMA_ROLE + agents/emma/custom-instruction.md
        emma_cfg,
        paths,
        &None, // no project
        signaling_addr,
        mcp_temp.path(),
        None, // no working dir
    )
    .await?;

    // Pull event_rx out so we can drive the persistence pump.
    let mut agent_handle = agent;
    let event_rx = std::mem::replace(&mut agent_handle.event_rx, tokio::sync::mpsc::channel(1).1);
    let storage_clone = storage.clone();
    tokio::spawn(async move {
        pump_emma_agent(event_rx, storage_clone).await;
    });

    info!("emma solo session spawned");

    Ok(EmmaHandle {
        agent: agent_handle,
        _mcp_temp: mcp_temp,
    })
}

/// Persist-only pump for Emma's solo agent. No peer forwarding, no IPAV
/// buffering — just stream events into the messages table.
async fn pump_emma_agent(mut event_rx: mpsc::Receiver<AgentEvent>, storage: Storage) {
    while let Some(event) = event_rx.recv().await {
        match event {
            AgentEvent::Text(text) => {
                let _ = storage
                    .insert_message("emma", Author::Emma, MessageKind::Text, &text)
                    .await
                    .map_err(|e| warn!(?e, "persisting emma text"));
            }
            AgentEvent::ToolUse { id, name, input } => {
                let payload = serde_json::json!({
                    "tool_use_id": id,
                    "name": name,
                    "input": input,
                });
                let _ = storage
                    .insert_message(
                        "emma",
                        Author::Emma,
                        MessageKind::ToolUse,
                        &payload.to_string(),
                    )
                    .await;
            }
            AgentEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let payload = serde_json::json!({
                    "tool_use_id": tool_use_id,
                    "content": content,
                    "is_error": is_error,
                });
                let _ = storage
                    .insert_message(
                        "emma",
                        Author::Emma,
                        MessageKind::ToolResult,
                        &payload.to_string(),
                    )
                    .await;
            }
            AgentEvent::TurnComplete { .. } | AgentEvent::Init { .. } => {}
            AgentEvent::Exited(msg) => {
                warn!(msg = %msg, "emma agent exited");
                break;
            }
            AgentEvent::Error(msg) => warn!(msg = %msg, "emma agent error"),
        }
    }
}

fn default_agent_config(name: &str) -> impl FnOnce() -> AgentConfig {
    let name = name.to_string();
    move || AgentConfig {
        agent_name: name,
        provider: "anthropic".into(),
        model_name: "claude-opus-4-7".into(),
        base_url: None,
        auth_token: None,
        updated_at: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn prompt_starts_with_hardcoded_role() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let prompt = read_system_prompt(&paths, "brian", None).unwrap();
        // Hardcoded role from agents::prompts — identity + duo + ask-close.
        assert!(prompt.contains("HANDS"));
        assert!(prompt.contains("BRAIN"));
        assert!(prompt.contains("Close session"));
    }

    #[test]
    fn prompt_includes_custom_instruction_when_present() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let agent_dir = tmp.path().join("agents/brian");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("custom-instruction.md"),
            "BRIAN_CUSTOM_PREFS_X9Q",
        )
        .unwrap();
        let prompt = read_system_prompt(&paths, "brian", None).unwrap();
        assert!(prompt.contains("BRIAN_CUSTOM_PREFS_X9Q"));
    }

    #[test]
    fn project_conventions_are_no_longer_injected() {
        // Regression: project context moved out of system prompt (agents
        // read it via the Read tool on demand). conventions.md / notes.md /
        // decisions.md should NOT appear at spawn time.
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let pdir = tmp.path().join("projects/foo");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(pdir.join("conventions.md"), "FOO_CONVENTIONS_M1").unwrap();
        std::fs::write(pdir.join("notes.md"), "FOO_NOTES_M1").unwrap();
        std::fs::write(pdir.join("decisions.md"), "FOO_DECISIONS_M1").unwrap();

        let prompt = read_system_prompt(&paths, "brian", Some("foo")).unwrap();
        assert!(!prompt.contains("FOO_CONVENTIONS_M1"));
        assert!(!prompt.contains("FOO_NOTES_M1"));
        assert!(!prompt.contains("FOO_DECISIONS_M1"));
    }

    #[test]
    fn missing_optional_slots_are_fine() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        // Nothing in agents/<name>/, no general-rules.md — should still produce
        // a prompt with at minimum the hardcoded role.
        let prompt = read_system_prompt(&paths, "rain", Some("nonexistent")).unwrap();
        assert!(prompt.contains("EYES"));
    }
}
