//! Session lifecycle: open + close.
//!
//! `open_session` is the load-bearing entry: persists the row, reads the
//! system prompt from CL, spawns Brian + Rain, kicks off the duo event pumps,
//! and registers the session in `AppState`.

use crate::agents::{spawn_supervised_agent, AgentEvent, AgentHandle, RetryPolicy, SpawnConfig};
use crate::core::duo::{pump_agent, DuoConfig};
use crate::core::ipav::IpavState;
use crate::paths::Paths;
use crate::signaling::{default_user_settings_paths, load_user_mcp_servers, mcp_config_json, SignalingBridge};
use crate::storage::{AgentConfig, Author, MessageKind, Session, Storage};
use tokio::sync::mpsc;
use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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
    /// HEAD of `working_repo_path` captured at session spawn. The session
    /// view's Apply tab diffs the current working tree against this anchor
    /// (`git diff <session_start_sha>`) so the user sees everything Brian
    /// applied this session — committed, staged, and unstaged — even right
    /// after a commit lands (`git diff HEAD` would show empty in that case).
    /// None when no working repo, no `.git/`, or the spawn-time `git rev-parse`
    /// failed. Not persisted: subprocess restart = fresh capture or fallback.
    pub session_start_sha: Option<String>,
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

impl SessionHandle {
    /// Fan a wire message to both agents' stdin. Send errors are ignored: a
    /// closed input channel means the subprocess is already gone, which this
    /// caller can't remediate.
    pub async fn send_to_both(&self, msg: crate::agents::OutgoingUserMessage) {
        let _ = self.brian.input_tx.send(msg.clone()).await;
        let _ = self.rain.input_tx.send(msg).await;
    }
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

    // Resolve the project's on-disk CL root once. Honors `projects.cl_path`
    // (folder-view registration with non-default location) and falls back to
    // the convention `<data_dir>/projects/<name>/`. Used for both the policy
    // audit and the per-agent system prompt below.
    let project_root: Option<PathBuf> = match project.as_deref() {
        Some(p) => storage.cl_path_for_project(&paths.data_dir, p).await.ok(),
        None => None,
    };

    // Audit policy.yaml files for mutations BEFORE we load them into the
    // system prompt. If the agent (or some other process) modified a policy
    // file between sessions, we want it logged. v1 is audit-only.
    if let Err(err) = crate::policy::audit_policy_files_at_root(
        &paths.data_dir,
        project.as_deref(),
        project_root.as_deref(),
        bridge.violations_log(),
        &session.id,
        "<session-spawn>",
    ) {
        tracing::warn!(%err, "policy audit failed at session spawn");
    }

    // Seed the canonical session-policy snapshot WRITE-IF-ABSENT. Once seeded,
    // this file (incl. any gear-tab user edits) is the SOLE policy for the
    // session — `Policy::resolve_at_root` returns it verbatim. Write-if-absent
    // so re-opening a session preserves edits made during a prior run; a fresh
    // snapshot freezes the resolved general+project blueprint plus the global
    // Tool-Gate keyword list at spawn. Best-effort: a write failure is logged
    // (resolve falls back to the live blueprint merge) but never blocks spawn.
    match crate::policy::session_policy::read_session_policy(&paths.data_dir, &session.id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            match crate::policy::Policy::resolve_at_root(
                &paths.data_dir,
                project.as_deref(),
                project_root.as_deref(),
                None,
            ) {
                Ok(seed) => {
                    let tool_gate = crate::policy::tool_gate::load(&paths.data_dir);
                    let sp = crate::policy::SessionPolicy {
                        policy: seed,
                        tool_gate,
                    };
                    if let Err(err) = crate::policy::session_policy::write_session_policy(
                        &paths.data_dir,
                        &session.id,
                        &sp,
                    ) {
                        tracing::warn!(%err, session_id = %session.id, "failed to seed session-policy snapshot");
                    }
                }
                Err(err) => tracing::warn!(
                    %err,
                    session_id = %session.id,
                    "resolving blueprint policy to seed session snapshot failed"
                ),
            }
        }
        Err(err) => tracing::warn!(
            %err,
            session_id = %session.id,
            "reading existing session-policy snapshot failed; not re-seeding"
        ),
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

    // Capture the working repo's HEAD SHA so the session view's Apply tab can
    // diff against it (covers committed + staged + unstaged in one `git diff`).
    // None when no repo / no `.git/` / git invocation failed — the view then
    // falls back to `git diff HEAD` with an anchor-lost note, then to the
    // latest phase='apply' session doc, then to an empty state.
    let session_start_sha: Option<String> = if let Some(repo) = working_repo_path.as_ref() {
        let repo = repo.clone();
        tokio::task::spawn_blocking(move || -> Option<String> {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
            (!sha.is_empty()).then_some(sha)
        })
        .await
        .ok()
        .flatten()
    } else {
        None
    };
    if let Some(ref sha) = session_start_sha {
        tracing::info!(session_id = %session.id, %sha, "captured session_start_sha");
    } else {
        tracing::debug!(session_id = %session.id, "no session_start_sha (no repo or git failed)");
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

    // Resume each agent's prior claude-code conversation if we have its UUID
    // stored on the session row. First-time spawn = None for both; the `init`
    // stream-json event will fire and `duo::pump_agent` persists the UUID so
    // the next reopen of this session can resume.
    let brian_resume = session.brian_claude_session_id.clone();
    let rain_resume = session.rain_claude_session_id.clone();

    let brian = spawn_agent_for(
        &session.id,
        "brian",
        brian_cfg,
        paths,
        &project,
        project_root.as_deref(),
        signaling_addr,
        mcp_temp.path(),
        working_repo_path.clone(),
        brian_resume,
    )
    .await?;
    let rain = spawn_agent_for(
        &session.id,
        "rain",
        rain_cfg,
        paths,
        &project,
        project_root.as_deref(),
        signaling_addr,
        mcp_temp.path(),
        working_repo_path.clone(),
        rain_resume,
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
        bridge: Some(Arc::clone(&bridge)),
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
        bridge: Some(Arc::clone(&bridge)),
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
        session_start_sha,
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
    project_root: Option<&Path>,
    signaling_addr: SocketAddr,
    mcp_temp_dir: &std::path::Path,
    working_dir: Option<PathBuf>,
    resume_session_id: Option<String>,
) -> Result<AgentHandle> {
    let system_prompt =
        read_system_prompt(paths, agent_name, project.as_deref(), project_root)?;
    let mcp_config_path = mcp_temp_dir.join(format!("{agent_name}-mcp.json"));
    let user_servers = user_mcp_servers_for_agent(agent_name);
    let json = mcp_config_json(signaling_addr, session_id, agent_name, &user_servers);
    std::fs::write(&mcp_config_path, json)
        .with_context(|| format!("writing mcp-config to {}", mcp_config_path.display()))?;

    let spawn_cfg = SpawnConfig {
        agent_name: agent_name.to_string(),
        config,
        system_prompt,
        mcp_config_path: Some(mcp_config_path),
        working_dir,
        claude_bin: None,
        session_id: session_id.to_string(),
        resume_session_id,
        project: project.clone(),
        data_dir: paths.data_dir.clone(),
    };
    // Supervised: a transient upstream API error (e.g. 529 Overloaded) auto-
    // resumes the agent with capped backoff instead of stranding the session.
    spawn_supervised_agent(spawn_cfg, RetryPolicy::default()).await
}

/// Decide which user MCP servers to expose to an agent at spawn time.
///
/// EYES (Rain) gets an empty map — only `bot-hq-signaling` will be in the
/// generated mcp-config.json. Without external MCPs (`brave-devtools`,
/// `chrome-devtools`, `discord`, etc.) Rain literally cannot drive
/// side-effects: the role contract is enforced at the tool boundary
/// instead of relying on prompt discipline the model rationalizes around
/// when a "next step" looks obvious. Rain still has claude-code's
/// built-in read-only tools (`Read`, `Grep`, `Glob`, `WebFetch`,
/// `WebSearch`, `ToolSearch`, `TodoWrite`), which are what
/// EYES needs to review HANDS' work.
///
/// HANDS (Brian) and the singleton Emma get the full merged set from the
/// user's claude-code config so they can drive browsers, talk to Discord,
/// etc.
pub fn user_mcp_servers_for_agent(
    agent_name: &str,
) -> serde_json::Map<String, serde_json::Value> {
    if agent_name == "rain" {
        serde_json::Map::new()
    } else {
        load_user_mcp_servers(&default_user_settings_paths())
    }
}

/// Assemble the system prompt for an agent at spawn time. Layers:
///
///   1. **Hardcoded role** (from `agents::prompts`) — identity + ask-close
///      convention. Baked into the binary so user can't break it.
///   2. **CL location anchor** — index-first orientation.
///   3. **Hardcoded `GENERAL_RULES`** (from `agents::general_rules`) — shared
///      conventions every agent follows. Baked into the binary so the load-
///      bearing parts (commit hygiene, push gates, CL workflow, IPAV, prod
///      safety) can't drift if a user edits a CL file.
///   4. **`<data_dir>/custom-general-rules.md`** — user-editable additions
///      to the universal rules (optional).
///   5. **`<data_dir>/agents/<name>/custom-instruction.md`** — per-agent
///      overrides (optional).
///   6. **Policy directive block** — rendered from policy.yaml, project-aware.
///
/// Project context (conventions / notes / decisions) is NOT injected here —
/// agents read those via the `Read` tool when assigned a project task. This
/// keeps spawn-time prompts compact and lets sessions hop projects without a
/// fresh spawn.
///
/// Missing optional files are logged at debug and skipped. Policy parse
/// errors propagate — broken YAML should surface loudly.
pub fn read_system_prompt(
    paths: &Paths,
    agent: &str,
    project: Option<&str>,
    project_root: Option<&Path>,
) -> Result<String> {
    let mut out = String::new();

    // 1. Hardcoded role.
    let role = crate::agents::role_for(agent);
    if !role.is_empty() {
        push_section(&mut out, role);
    }

    // 2. CL location anchor + index-first workflow. Without this, agents
    // wander into legacy archives by accident OR blind-Read a fixed set of
    // filenames and miss the rest of the CL. The full tool signatures for
    // cl_index_search / cl_register_read / cl_rescan live in GENERAL_RULES
    // (layer 3 below) — here we just establish the orientation.
    out.push_str(&format!(
        "## Context Library\n\n\
         Your Context Library lives at `{cl}`. Single source of truth — \
         other `~/.bot-hq*` paths are archives from prior installs, ignore \
         them.\n\n\
         **Index-first.** The CL is indexed in SQLite; each file has a \
         description so you can decide what's worth opening without burning \
         context on irrelevant files. Call `cl_index_search(project=<your \
         project>)` BEFORE reaching for `Read` on any CL path. Pass \
         `\"_globals\"` for system-level / cross-project notes, your \
         session's project name for project-scoped notes, or omit `project` \
         to search everything. Folders carry their own descriptions in \
         `cl_folders` — `cl_folder_search(project=<your project>)` returns \
         folder-level summaries so you can scope a sweep before opening \
         individual files. Tool signatures for `cl_index_search`, \
         `cl_folder_search`, `cl_register_read`, `cl_rescan` are in the \
         General rules section below.\n\n\
         **Bare-filename heuristic.** If the user references a bare \
         filename (e.g. \"work on task 1 from tasks.md\", \"check eod.md\") \
         and it's NOT in your working repo, do NOT keep `Glob`-searching \
         broader paths. Try `cl_index_search(project=\"_globals\", \
         query=<name>)` next — common cross-project files like `tasks.md` \
         and `eod.md` live at the CL root and surface as `_globals` rows. \
         Only fall back to `ask_user_choice` if `_globals` also misses.\n\n\
         Per-project conventional files at `{cl}projects/<project>/` \
         (the index covers everything under this path, not just these):\n\
         - `conventions.md` — repo, stack, commands, gates, disguise rules\n\
         - `notes.md` — current state, recurring trouble, gotchas\n\
         - `decisions.md` — chronological log of prior decisions\n\
         - `policy.yaml` — machine-enforced gates (already rendered into \
         this prompt if the project has one)\n\n\
         Trust the index over a hardcoded filename list. Don't ask the user \
         for facts that live in the CL.\n\n",
        cl = paths.data_dir.display()
    ));

    // 3. Hardcoded universal rules — always present.
    push_section(&mut out, crate::agents::GENERAL_RULES);

    // 4 + 5. Optional user-editable slots: custom-general-rules.md applies to
    // all agents; agents/<name>/custom-instruction.md is per-agent.
    let slots = [
        paths.data_dir.join("custom-general-rules.md"),
        paths.data_dir
            .join(format!("agents/{agent}/custom-instruction.md")),
    ];
    for slot in slots {
        match std::fs::read_to_string(&slot) {
            Ok(s) if !s.trim().is_empty() => push_section(&mut out, &s),
            Ok(_) => {} // empty file — silently skip
            Err(err) => {
                tracing::debug!(path = %slot.display(), %err, "optional CL slot absent");
            }
        }
    }

    // 6. Policy directive block — project-aware. Honors a non-default
    // `projects.cl_path` when the caller resolved one (folder-view
    // registration with an off-convention location).
    let policy =
        crate::policy::Policy::resolve_at_root(&paths.data_dir, project, project_root, None)
            .context("resolving project policy")?;
    let block = policy.render_system_prompt_block();
    if !block.is_empty() {
        push_section(&mut out, &block);
    }
    Ok(out)
}

/// Append `s` to `out`, then ensure the section ends with one blank line so
/// the next prompt section is visually separated. No-op on spacing if `s`
/// already ends with "\n\n".
fn push_section(out: &mut String, s: &str) {
    out.push_str(s);
    if !out.ends_with("\n\n") {
        out.push_str("\n\n");
    }
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

    // Seed Emma's canonical session-policy snapshot WRITE-IF-ABSENT (same
    // contract as the duo path). Emma has no project, so the blueprint is just
    // general-policy.yaml + the global Tool-Gate list. Best-effort.
    match crate::policy::session_policy::read_session_policy(&paths.data_dir, "emma") {
        Ok(Some(_)) => {}
        Ok(None) => match crate::policy::Policy::resolve_at_root(&paths.data_dir, None, None, None) {
            Ok(seed) => {
                let tool_gate = crate::policy::tool_gate::load(&paths.data_dir);
                let sp = crate::policy::SessionPolicy {
                    policy: seed,
                    tool_gate,
                };
                if let Err(err) =
                    crate::policy::session_policy::write_session_policy(&paths.data_dir, "emma", &sp)
                {
                    tracing::warn!(%err, "failed to seed emma session-policy snapshot");
                }
            }
            Err(err) => tracing::warn!(%err, "resolving blueprint policy to seed emma snapshot failed"),
        },
        Err(err) => {
            tracing::warn!(%err, "reading emma session-policy snapshot failed; not re-seeding")
        }
    }

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
        None,  // no project_root
        signaling_addr,
        mcp_temp.path(),
        None, // no working dir
        None, // emma: fresh claude session every app start; resume not wired
    )
    .await?;

    // Pull event_rx out so we can drive the persistence pump.
    let mut agent_handle = agent;
    let event_rx = std::mem::replace(&mut agent_handle.event_rx, tokio::sync::mpsc::channel(1).1);
    let storage_clone = storage.clone();
    let bridge_clone = Arc::clone(&bridge);
    tokio::spawn(async move {
        pump_emma_agent(event_rx, storage_clone, bridge_clone).await;
    });

    info!("emma solo session spawned");

    Ok(EmmaHandle {
        agent: agent_handle,
        _mcp_temp: mcp_temp,
    })
}

/// Persist-only pump for Emma's solo agent. No peer forwarding, no IPAV
/// buffering — just stream events into the messages table. Fires
/// `MessagePersisted` events on the bridge so external `wait_for_change`
/// callers wake up without polling.
async fn pump_emma_agent(
    mut event_rx: mpsc::Receiver<AgentEvent>,
    storage: Storage,
    bridge: Arc<SignalingBridge>,
) {
    while let Some(event) = event_rx.recv().await {
        match event {
            AgentEvent::Text(text) => {
                match storage
                    .insert_message("emma", Author::Emma, MessageKind::Text, &text)
                    .await
                {
                    Ok(id) => bridge.notify_message_persisted("emma".into(), id),
                    Err(e) => warn!(?e, "persisting emma text"),
                }
            }
            AgentEvent::ToolUse { id, name, input } => {
                let payload = serde_json::json!({
                    "tool_use_id": id,
                    "name": name,
                    "input": input,
                });
                match storage
                    .insert_message(
                        "emma",
                        Author::Emma,
                        MessageKind::ToolUse,
                        &payload.to_string(),
                    )
                    .await
                {
                    Ok(id) => bridge.notify_message_persisted("emma".into(), id),
                    Err(e) => warn!(?e, "persisting emma tool_use"),
                }
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
                match storage
                    .insert_message(
                        "emma",
                        Author::Emma,
                        MessageKind::ToolResult,
                        &payload.to_string(),
                    )
                    .await
                {
                    Ok(id) => bridge.notify_message_persisted("emma".into(), id),
                    Err(e) => warn!(?e, "persisting emma tool_result"),
                }
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
    fn rain_gets_no_user_mcps_brian_gets_inherited() {
        // EYES enforcement: Rain must not have any external MCP servers
        // beyond the bot-hq-signaling one added by mcp_config_json. Brian
        // (HANDS) keeps whatever the user has in ~/.claude.json.
        // Mocking the file isn't worth it — we just verify Rain's map is
        // empty and Brian's matches what load_user_mcp_servers returns.
        let rain = user_mcp_servers_for_agent("rain");
        assert!(rain.is_empty(), "Rain must spawn with no external MCPs");
        let brian = user_mcp_servers_for_agent("brian");
        let expected_brian = load_user_mcp_servers(&default_user_settings_paths());
        assert_eq!(brian, expected_brian);
        let emma = user_mcp_servers_for_agent("emma");
        assert_eq!(emma, expected_brian);
    }

    #[test]
    fn prompt_starts_with_hardcoded_role() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let prompt = read_system_prompt(&paths, "brian", None, None).unwrap();
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
        let prompt = read_system_prompt(&paths, "brian", None, None).unwrap();
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

        let prompt = read_system_prompt(&paths, "brian", Some("foo"), None).unwrap();
        assert!(!prompt.contains("FOO_CONVENTIONS_M1"));
        assert!(!prompt.contains("FOO_NOTES_M1"));
        assert!(!prompt.contains("FOO_DECISIONS_M1"));
    }

    #[test]
    fn prompt_points_at_cl_index_first() {
        // Regression: layer 1b used to tell agents to Read conventions.md +
        // notes.md directly. After the CL index landed (commit e13e8e4),
        // the canonical entry point is cl_index_search. If this assertion
        // ever fails, layer 1b has drifted back to the old "blind Read"
        // workflow.
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let prompt = read_system_prompt(&paths, "brian", None, None).unwrap();
        assert!(prompt.contains("cl_index_search"));
        assert!(prompt.contains("Index-first"));
        // Regression: when the user mentions a bare filename (tasks.md,
        // eod.md), agents should head to _globals before falling back to
        // ask_user_choice or broad Glob sweeps.
        assert!(prompt.contains("Bare-filename heuristic"));
        assert!(prompt.contains("_globals"));
    }

    #[test]
    fn missing_optional_slots_are_fine() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        // No custom-general-rules.md content, nothing in agents/<name>/ —
        // should still produce a prompt with at minimum the hardcoded role
        // and the hardcoded universal rules.
        std::fs::remove_file(tmp.path().join("custom-general-rules.md")).ok();
        let prompt = read_system_prompt(&paths, "rain", Some("nonexistent"), None).unwrap();
        assert!(prompt.contains("EYES"));
        assert!(prompt.contains("Commit hygiene"));
    }

    #[test]
    fn prompt_always_contains_hardcoded_general_rules() {
        // Load-bearing test: even on a freshly-init'd data dir with the
        // user's custom file deleted, the universal rules must be present
        // (commit hygiene, push gate, IPAV, prod safety).
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        std::fs::remove_file(tmp.path().join("custom-general-rules.md")).ok();
        let prompt = read_system_prompt(&paths, "brian", None, None).unwrap();
        assert!(prompt.contains("Commit hygiene"), "missing commit-hygiene section");
        assert!(prompt.contains("`git push` is governed by the session's push gate"), "missing push gate");
        assert!(prompt.contains("IPAV discipline"), "missing IPAV section");
        assert!(prompt.contains("Production data access"), "missing prod-safety section");
    }

    #[test]
    fn custom_general_rules_appends_to_hardcoded() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        std::fs::write(
            tmp.path().join("custom-general-rules.md"),
            "MY_ORG_RULE_X7P: always prefer ripgrep over grep.\n",
        )
        .unwrap();
        let prompt = read_system_prompt(&paths, "brian", None, None).unwrap();
        // Both layers present.
        assert!(prompt.contains("Commit hygiene"));
        assert!(prompt.contains("MY_ORG_RULE_X7P"));
        // Custom additions come AFTER the hardcoded core.
        let core_pos = prompt.find("Commit hygiene").unwrap();
        let custom_pos = prompt.find("MY_ORG_RULE_X7P").unwrap();
        assert!(custom_pos > core_pos, "custom rules should append after hardcoded core");
    }
}
