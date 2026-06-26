//! Session lifecycle: open + close.
//!
//! `open_session` is the load-bearing entry: persists the row, reads the
//! system prompt from CL, spawns Brian + Rain, kicks off the duo event pumps,
//! and registers the session in `AppState`.

use crate::agents::{
    spawn_supervised_agent, AgentHandle, OutgoingUserMessage, RetryPolicy, SpawnConfig,
};
use crate::core::broadcast::with_phase_envelope;
use crate::core::duo::{pump_agent, DuoConfig};
use crate::core::ipav::{IpavPhase, IpavState};
use crate::paths::Paths;
use crate::signaling::{
    default_user_settings_paths, load_user_mcp_servers, mcp_config_json, SignalingBridge,
};
use crate::storage::{AgentConfig, Author, ClIndexEntry, Session, Storage};
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
    /// Run the duo (true) or solo-Brian (false). Defaults to true.
    pub rain_enabled: bool,
    /// Saved-model ids for each agent (None = fall back to per-agent config).
    pub brian_model_id: Option<String>,
    pub rain_model_id: Option<String>,
}

impl OpenSessionRequest {
    /// The historical duo default: Rain on, models resolved from agent config.
    pub fn duo(title: impl Into<String>, working_repo_path: Option<PathBuf>) -> Self {
        Self {
            title: title.into(),
            working_repo_path,
            rain_enabled: true,
            brian_model_id: None,
            rain_model_id: None,
        }
    }
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
    /// None when this session runs solo-Brian (Rain disabled at create).
    pub rain: Option<AgentHandle>,
    /// Shared "duo is awaiting user input" flag. Set by the bridge when any
    /// user-blocking MCP tool fires; checked by `router::route_forward`
    /// before it forwards Brian↔Rain chunks; cleared by
    /// `core::AppState::broadcast` when the user replies.
    pub awaiting: Arc<std::sync::atomic::AtomicBool>,
    /// Shared count of consecutive peer-forwards with no intervening user
    /// message — the L2 volley hard-cap (interrupt redesign). The router
    /// increments it on each forward; `AppState::broadcast` resets it to 0 on the
    /// user's next message; past the router's hard cap the volley breaks. Unlike
    /// `awaiting` it is NOT bridge-registered — no MCP tool touches it.
    pub user_silent_forwards: Arc<std::sync::atomic::AtomicU32>,
    /// Per-session duo-activity tracker (interrupt redesign, Batch 2) — drives
    /// the chat-input lock. Shared with both pumps (which clear `busy` on
    /// `TurnComplete`) and the dispatch paths in `AppState` (set `busy` on send,
    /// `cancelling` on cancel). Reads the same `awaiting` Arc above for the
    /// `AwaitingUser` state.
    pub activity: Arc<crate::core::ActivityTracker>,
    /// Shared "HANDS is mid-atomic-tool" flag (interrupt redesign, Batch 3.1
    /// Part 1) — read by `cancel_session_turn` to DEFER the kill until a
    /// `git commit`/`git push`/migration finishes, so a cancel never leaves the
    /// working tree half-written. Session-level (both pumps hold the Arc; only
    /// HANDS sets it).
    pub in_atomic_tool: Arc<std::sync::atomic::AtomicBool>,
    /// Set by `broadcast` when a user message arrives, so an in-flight cancel
    /// escalation skips its SIGKILL (the user superseded the cancel). Reset by
    /// `cancel_session_turn`. Shared with `interrupt_then_escalate`.
    pub cancel_superseded: Arc<std::sync::atomic::AtomicBool>,
    /// Handle-side control for the duo peer-forward router (`None` = solo). Lets
    /// `broadcast` reset the router's convergence streak on each user message.
    pub router: Option<crate::core::RouterControl>,
    /// Keeps the mcp-config temp files alive for the lifetime of the session.
    _mcp_temp: TempDir,
}

impl SessionHandle {
    /// Fan a wire message to both agents' stdin. Send errors are ignored: a
    /// closed input channel means the subprocess is already gone, which this
    /// caller can't remediate.
    pub async fn send_to_both(&self, msg: crate::agents::OutgoingUserMessage) {
        let _ = self.brian.input_tx.send(msg.clone()).await;
        self.activity.set_busy(crate::storage::Author::Brian, true);
        if let Some(rain) = &self.rain {
            let _ = rain.input_tx.send(msg).await;
            self.activity.set_busy(crate::storage::Author::Rain, true);
        }
    }

    /// True once either agent's retry supervisor has terminated — a permanent
    /// API error (e.g. `400`) or an exhausted retry budget drops the
    /// supervisor's input receiver, which closes this sender. The handle then
    /// lingers in the session map but can no longer drive the duo, so callers
    /// (`ensure_session_started`) evict + re-spawn it instead of treating it as
    /// live. Stays `false` during a healthy run AND during a transient-retry
    /// backoff (the supervisor still holds the receiver then), so a recovering
    /// agent is never wrongly evicted.
    pub fn is_stale(&self) -> bool {
        self.brian.input_tx.is_closed()
            || self.rain.as_ref().is_some_and(|r| r.input_tx.is_closed())
    }
}

pub async fn open_session(
    req: OpenSessionRequest,
    paths: &Paths,
    storage: Storage,
    bridge: Arc<SignalingBridge>,
    signaling_addr: SocketAddr,
) -> Result<SessionHandle> {
    let id = Uuid::new_v4().to_string();
    let mut session = storage
        .create_session(
            &id,
            &req.title,
            req.working_repo_path.as_ref().and_then(|p| p.to_str()),
        )
        .await
        .context("creating session row")?;

    // Persist the create-dialog choices on the row BEFORE spawn so
    // spawn_session_handle (and any later respawn) reads them. Mirror onto the
    // in-memory struct so we don't need a re-fetch.
    storage
        .set_session_spawn_config(
            &id,
            req.rain_enabled,
            req.brian_model_id.as_deref(),
            req.rain_model_id.as_deref(),
        )
        .await
        .context("recording session spawn config")?;
    session.rain_enabled = if req.rain_enabled { 1 } else { 0 };
    session.brian_model_id = req.brian_model_id;
    session.rain_model_id = req.rain_model_id;

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

/// Spawn subprocesses for a session row that ALREADY EXISTS in storage.
/// Idempotency check belongs to the caller — this
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
    spawn_session_handle(
        session,
        working_repo_path,
        paths,
        storage,
        bridge,
        signaling_addr,
    )
    .await
}

/// Shared spawn logic for both fresh and existing sessions: spawn Brian + Rain,
/// kick the duo pumps, return the handle.
/// Resolve a session's project from its repo paths. A registered project
/// whose `working_repo_path` matches wins (matched against the BASE repo
/// first — a worktree session's path ends in the repo basename, not
/// necessarily the project name); the path basename stays as the fallback
/// for unregistered repos. Repo-less sessions resolve to `None` (general
/// policy applies by inheritance).
/// How a session's project name was derived from its repo path — surfaced in
/// the gear tab (policy-origin badge) so the user can see WHY a session
/// inherited a given policy. The 2026-06-11 "why the full forbidden list?"
/// surprise was an unregistered repo silently resolving to general policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ProjectProvenance {
    /// Matched a registered project's working-repo path (canonical compare).
    Registered,
    /// No registered match — fell back to the repo's path basename.
    Inferred,
    /// Repo-less session — no project; general policy applies by inheritance.
    None,
}

pub(crate) async fn resolve_session_project(
    storage: &Storage,
    base_repo_path: Option<&str>,
    working_repo_path: Option<&Path>,
) -> (Option<String>, ProjectProvenance) {
    let repo: &Path = match base_repo_path.map(Path::new).or(working_repo_path) {
        Some(p) => p,
        None => return (None, ProjectProvenance::None),
    };
    let basename = repo
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string);
    match storage.project_by_repo_path(repo).await {
        Ok(Some(name)) => {
            if basename.as_deref() != Some(name.as_str()) {
                info!(
                    project = %name,
                    repo = %repo.display(),
                    "project resolved from registered working repo (basename differs)"
                );
            }
            (Some(name), ProjectProvenance::Registered)
        }
        Ok(None) => (basename, ProjectProvenance::Inferred),
        Err(err) => {
            warn!(%err, repo = %repo.display(), "project lookup failed — using path basename");
            (basename, ProjectProvenance::Inferred)
        }
    }
}

async fn spawn_session_handle(
    session: Session,
    working_repo_path: Option<PathBuf>,
    paths: &Paths,
    storage: Storage,
    bridge: Arc<SignalingBridge>,
    signaling_addr: SocketAddr,
) -> Result<SessionHandle> {
    let (project, _provenance) = resolve_session_project(
        &storage,
        session.base_repo_path.as_deref(),
        working_repo_path.as_deref(),
    )
    .await;

    // Register session→project with the bridge so policy-aware MCP tools can
    // resolve `<data_dir>/projects/<project>/policy.yaml` per-call.
    bridge
        .register_session(session.id.clone(), project.clone())
        .await;

    // Worktree-isolated session: materialize the worktree before anything
    // touches the path (hook install, HEAD capture, agent cwd). Idempotent —
    // respawn/restart re-enter here. On failure the session falls back to the
    // BASE repo and the row is converted to direct mode so row-readers
    // (action_gate) and the live handle can't disagree about where it runs.
    let working_repo_path = match (session.base_repo_path.as_ref(), working_repo_path) {
        (Some(base), Some(wt)) => {
            let base_pb = PathBuf::from(base);
            let wt_clone = wt.clone();
            let branch = crate::core::worktree::branch_for_session(&session.id);
            let ensured = tokio::task::spawn_blocking(move || {
                crate::core::worktree::ensure_worktree(&base_pb, &wt_clone, &branch)
            })
            .await
            .context("worktree ensure task panicked")?;
            match ensured {
                Ok(()) => {
                    info!(session_id = %session.id, worktree = %wt.display(), "session worktree ready");
                    Some(wt)
                }
                Err(err) => {
                    warn!(
                        %err,
                        session_id = %session.id,
                        base = %base,
                        "worktree ensure failed — falling back to the base repo (direct mode)"
                    );
                    if let Err(e) = storage.convert_session_to_direct(&session.id, base).await {
                        warn!(?e, session_id = %session.id, "convert_session_to_direct failed");
                    }
                    Some(PathBuf::from(base))
                }
            }
        }
        (_, wrp) => wrp,
    };

    // Resolve the project's on-disk CL root once. Honors `projects.cl_path`
    // (folder-view registration with non-default location) and falls back to
    // the convention `<data_dir>/projects/<name>/`. Used for both the policy
    // audit and the per-agent system prompt below.
    let project_root: Option<PathBuf> = match project.as_deref() {
        Some(p) => storage.cl_path_for_project(&paths.data_dir, p).await.ok(),
        None => None,
    };

    // Fetch the project's CL index rows (filenames + descriptions, most-
    // recently-updated first) so each agent's system prompt can carry a compact
    // "table of contents" primer (see `read_system_prompt`). This pre-warms the
    // cold start: an agent that skips `cl_index_search` on its first turn still
    // knows what project context EXISTS to pull. Bodies stay pull-only. Best-
    // effort; None for `_globals` / repo-less sessions.
    let cl_index: Option<Vec<ClIndexEntry>> = match project.as_deref() {
        Some(p) => storage.cl_index_search(Some(p), None).await.ok(),
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

    // Resolve each agent's spawn config from its chosen saved model (create
    // dialog), falling back to the per-agent config. Rain is skipped entirely
    // when the session runs solo-Brian.
    let rain_enabled = session.rain_enabled != 0;
    let brian_cfg =
        resolve_spawn_config(&storage, "brian", session.brian_model_id.as_deref()).await;
    let rain_cfg = if rain_enabled {
        Some(resolve_spawn_config(&storage, "rain", session.rain_model_id.as_deref()).await)
    } else {
        None
    };

    // Record the model names we're about to spawn with. Session header reads
    // these so it reflects the live (frozen-at-spawn) model, not the current
    // DB value, which can drift after a config swap. Rain's is NULL for a solo
    // session.
    let rain_model_name = rain_cfg.as_ref().map(|c| c.model_name.as_str());
    if let Err(e) = storage
        .set_session_spawn_models(&session.id, &brian_cfg.model_name, rain_model_name)
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
    // A1 (adherence): a FIRST spawn (no stored claude session id yet) gets the
    // one-shot CL-opener nudge below; a `--resume` reopen does not (restored).
    let is_first_spawn = session.brian_claude_session_id.is_none();
    // Per-session effort/ultracode picks (create dialog); overlaid over the
    // persistent per-agent override in build_command (session wins).
    let brian_effort = session.brian_effort.clone();
    let brian_ultracode = session.brian_ultracode;
    let rain_effort = session.rain_effort.clone();
    let rain_ultracode = session.rain_ultracode;

    let brian = spawn_agent_for(
        &session.id,
        "brian",
        brian_cfg,
        paths,
        &project,
        project_root.as_deref(),
        cl_index.as_deref(),
        signaling_addr,
        mcp_temp.path(),
        working_repo_path.clone(),
        brian_resume,
        brian_effort,
        brian_ultracode,
    )
    .await?;
    let rain = if let Some(rc) = rain_cfg {
        Some(
            spawn_agent_for(
                &session.id,
                "rain",
                rc,
                paths,
                &project,
                project_root.as_deref(),
                cl_index.as_deref(),
                signaling_addr,
                mcp_temp.path(),
                working_repo_path.clone(),
                rain_resume,
                rain_effort,
                rain_ultracode,
            )
            .await?,
        )
    } else {
        info!(session_id = %session.id, "solo-Brian session (Rain disabled)");
        None
    };

    let ipav = Arc::new(Mutex::new(IpavState::default()));
    let awaiting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // L2 volley hard-cap counter — incremented per peer-forward by the duo
    // pumps, reset on the user's next message in `broadcast`. Shared into both
    // DuoConfigs + the SessionHandle (for the reset); no bridge registration.
    let user_silent_forwards = Arc::new(std::sync::atomic::AtomicU32::new(0));
    // Register the flag with the bridge so user-blocking MCP tools can set it
    // synchronously (before the agent's next chunk volleys). The duo pumps
    // read the same Arc, so updates propagate to both pumps with no
    // additional plumbing.
    bridge
        .register_session_awaiting(session.id.clone(), Arc::clone(&awaiting))
        .await;

    // Shared "HANDS mid-atomic-tool" flag (interrupt redesign, Batch 3.1 Part
    // 1) — lets a cancel defer the kill until a git commit/push/migration
    // finishes. Session-level: both pumps hold the Arc, only HANDS sets it.
    let in_atomic_tool = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // A user message sent during a cancel's interrupt→SIGKILL escalation window
    // supersedes the cancel: `broadcast` sets this, and `interrupt_then_escalate`
    // skips its SIGKILL when set (the user's message + its own preempt-interrupt
    // already aborted the stuck turn — killing it would lose the fresh turn +
    // warm cache). `cancel_session_turn` resets it false when a new cancel begins.
    let cancel_superseded = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Per-session activity tracker (interrupt redesign, Batch 2) — drives the
    // chat-input lock. Shares the `awaiting` Arc (for the AwaitingUser state);
    // both pumps flip per-agent `busy`, the dispatch paths set busy on send.
    let activity =
        crate::core::ActivityTracker::new(session.id.clone(), Arc::clone(&awaiting), Arc::clone(&bridge));
    // Bug B: let the bridge reach this tracker so `set_session_awaiting` can emit
    // AwaitingUser the moment a question is parked (instead of waiting for the
    // agent's next TurnComplete set_busy). Weak — the tracker is owned here and by
    // the SessionHandle; a strong bridge ref would cycle. Mirrors the awaiting reg.
    bridge
        .register_session_activity(session.id.clone(), Arc::downgrade(&activity))
        .await;

    // Per-agent pumps need to be spawned BEFORE we move the handles, so we
    // pull the receivers + input senders here. The handles keep their other
    // fields (kill signal, etc.).
    let mut brian_handle = brian;
    let brian_events =
        std::mem::replace(&mut brian_handle.event_rx, tokio::sync::mpsc::channel(1).1);

    // Rain (optional): pull its receiver + input sender when present.
    let mut rain_handle = rain;
    let rain_input = rain_handle.as_ref().map(|r| r.input_tx.clone());
    let rain_events = rain_handle
        .as_mut()
        .map(|r| std::mem::replace(&mut r.event_rx, tokio::sync::mpsc::channel(1).1));

    // Brian's pump: peer is Rain's input when present, else None (solo).
    let storage_clone = storage.clone();
    let ipav_clone = Arc::clone(&ipav);
    let session_id_clone = session.id.clone();
    // Batch 7: per-agent liveness for the stall watchdog. The watchdog holds Weak
    // refs, so it self-terminates once the pumps drop their Arcs (session end).
    let brian_liveness = crate::core::watchdog::AgentLiveness::new();
    let mut watchdog_agents = vec![(Author::Brian, Arc::downgrade(&brian_liveness))];
    // Central peer-forward router (duo only). The single forward decision point +
    // the interleaved convergence stream; both pumps emit RouterCommand to it.
    // Lifecycle: when both pumps drop their router_tx clones (session end) the
    // command channel closes and run_router returns (like the watchdog — no
    // explicit teardown). The shared `awaiting`/`user_silent_forwards` Arcs are
    // cloned in, so the bridge's awaiting set + broadcast's counter reset are
    // visible here with no extra plumbing.
    // Shared across the user boundary: `broadcast` sets it on each user message;
    // the router consumes it to clear its convergence streak (so a pre-message
    // streak can't suppress the first post-message peer-forward).
    let convergence_reset = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Per-direction delivered-forward counters (diagnostics).
    let fwd_brian_to_rain = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let fwd_rain_to_brian = Arc::new(std::sync::atomic::AtomicU64::new(0));
    // Router liveness flag (true while the task runs; the router's AliveGuard flips
    // it false on exit/panic). The watchdog reads it via a Weak.
    let router_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (router_tx, router_control, router_watch) = match &rain_input {
        Some(rain_in) => {
            let (router_tx, router_rx) = tokio::sync::mpsc::channel(256);
            // O1: seed + register the session's open-blocking-findings count cache;
            // the router reads this Arc lock-free per forward instead of a
            // per-forward SELECT COUNT(*) + storage-lock.
            let open_blocking = bridge.register_open_blocking(session.id.clone()).await;
            let deps = crate::core::RouterDeps {
                awaiting: Arc::clone(&awaiting),
                user_silent_forwards: Arc::clone(&user_silent_forwards),
                convergence_reset: Arc::clone(&convergence_reset),
                fwd_brian_to_rain: Arc::clone(&fwd_brian_to_rain),
                fwd_rain_to_brian: Arc::clone(&fwd_rain_to_brian),
                alive: Arc::clone(&router_alive),
                activity: Some(Arc::clone(&activity)),
                open_blocking,
                ipav: Arc::clone(&ipav),
                brian_input: brian_handle.input_tx.clone(),
                rain_input: Some(rain_in.clone()),
            };
            let task = tokio::spawn(crate::core::run_router(deps, router_rx));
            // Seed the router-health dot "up" — also clears any stale `false` left
            // by a prior (pre-rebuild) router for this same session id.
            bridge.notify_router_health(session.id.clone(), true);
            let watch = crate::core::watchdog::RouterWatch {
                alive: Arc::downgrade(&router_alive),
                fwd_brian_to_rain: Arc::downgrade(&fwd_brian_to_rain),
                fwd_rain_to_brian: Arc::downgrade(&fwd_rain_to_brian),
            };
            (
                Some(router_tx),
                Some(crate::core::RouterControl {
                    convergence_reset,
                    fwd_brian_to_rain,
                    fwd_rain_to_brian,
                    alive: router_alive,
                    task,
                }),
                Some(watch),
            )
        }
        None => (None, None, None),
    };
    let brian_duo = DuoConfig {
        router_tx: router_tx.clone(),
        bridge: Some(Arc::clone(&bridge)),
        activity: Some(Arc::clone(&activity)),
        in_atomic_tool: Some(Arc::clone(&in_atomic_tool)),
        liveness: Some(Arc::clone(&brian_liveness)),
        // A3a: Brian's own stdin, so the pump can self-nudge him if he mutates
        // before the Apply phase.
        self_input_tx: Some(brian_handle.input_tx.clone()),
        ..DuoConfig::new(session_id_clone, Author::Brian)
    };
    tokio::spawn(async move {
        pump_agent(brian_duo, brian_events, storage_clone, ipav_clone).await;
    });

    // Rain's pump only runs in a duo session.
    if let Some(rain_events) = rain_events {
        let storage_clone = storage.clone();
        let ipav_clone = Arc::clone(&ipav);
        let session_id_clone = session.id.clone();
        let rain_liveness = crate::core::watchdog::AgentLiveness::new();
        watchdog_agents.push((Author::Rain, Arc::downgrade(&rain_liveness)));
        let rain_duo = DuoConfig {
            router_tx: router_tx.clone(),
            bridge: Some(Arc::clone(&bridge)),
            activity: Some(Arc::clone(&activity)),
            in_atomic_tool: Some(Arc::clone(&in_atomic_tool)),
            liveness: Some(Arc::clone(&rain_liveness)),
            ..DuoConfig::new(session_id_clone, Author::Rain)
        };
        tokio::spawn(async move {
            pump_agent(rain_duo, rain_events, storage_clone, ipav_clone).await;
        });
    }

    // Batch 7: spawn the per-session stall watchdog (solo + duo). It holds Weak
    // liveness refs, so it self-terminates once the pumps drop their Arcs.
    tokio::spawn(crate::core::watchdog::run_stall_watchdog(
        session.id.clone(),
        watchdog_agents,
        Arc::clone(&activity),
        Arc::clone(&bridge),
        router_watch,
    ));

    // A1 (adherence): one-shot session-start CL-opener nudge. Mechanically pages
    // the agent toward `cl_index_search` so a model that doesn't reliably follow
    // the prompt-side opener still gets nudged. Fires only on a FIRST spawn (not
    // a `--resume` reopen), only for a real project (skips `_globals`/repo-less),
    // and only when nudges are enabled. Delivered before the user's first task —
    // the agent opens the CL during the user's think-time, so the task lands
    // with conventions already loaded.
    if is_first_spawn && storage.adherence_nudges_enabled().await {
        if let Some(nudge) = cl_opener_nudge(project.as_deref()) {
            let wire = with_phase_envelope(IpavPhase::Investigate, &nudge);
            let _ = brian_handle
                .input_tx
                .send(OutgoingUserMessage::text(wire.clone()))
                .await;
            if let Some(r) = rain_handle.as_ref() {
                let _ = r.input_tx.send(OutgoingUserMessage::text(wire)).await;
            }
        }
    }

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
        user_silent_forwards,
        activity,
        in_atomic_tool,
        cancel_superseded,
        router: router_control,
        _mcp_temp: mcp_temp,
    })
}

/// A1 (adherence): the one-shot session-start CL-opener nudge text for a
/// session targeting `project`, or `None` for a repo-less / `_globals` session
/// (no project conventions to page in). Distinct from the system-prompt CL
/// INDEX primer (layer 2b, `render_cl_primer`) — this is a runtime stdin nudge
/// delivered to each agent. Pure so it's unit-testable; the caller wraps it in
/// the phase envelope before sending.
fn cl_opener_nudge(project: Option<&str>) -> Option<String> {
    let name = project.filter(|p| !p.is_empty() && *p != "_globals")?;
    Some(format!(
        "🔔 Session start — project `{name}`. Before the user's first task, call \
         `cl_index_search(project=\"{name}\")` to load this project's conventions \
         (formatter, test commands, gates) — they live in the Context Library, not \
         the repo. Then wait for the user's task; take no other action yet."
    ))
}

#[allow(clippy::too_many_arguments)]
async fn spawn_agent_for(
    session_id: &str,
    agent_name: &str,
    config: AgentConfig,
    paths: &Paths,
    project: &Option<String>,
    project_root: Option<&Path>,
    cl_index: Option<&[ClIndexEntry]>,
    signaling_addr: SocketAddr,
    mcp_temp_dir: &std::path::Path,
    working_dir: Option<PathBuf>,
    resume_session_id: Option<String>,
    session_effort: Option<String>,
    session_ultracode: Option<bool>,
) -> Result<AgentHandle> {
    let system_prompt =
        read_system_prompt(paths, agent_name, project.as_deref(), project_root, cl_index)?;
    // The assembled prompt is multi-KB. Hand it to claude-code via a file
    // (`--append-system-prompt-file`) rather than an inline arg so the command
    // line stays under Windows' 32,767-char `CreateProcessW` limit. Co-located
    // with the mcp-config in the same per-agent temp dir (same lifecycle).
    let system_prompt_path = mcp_temp_dir.join(format!("{agent_name}-system-prompt.txt"));
    std::fs::write(&system_prompt_path, &system_prompt)
        .with_context(|| format!("writing system prompt to {}", system_prompt_path.display()))?;
    let mcp_config_path = mcp_temp_dir.join(format!("{agent_name}-mcp.json"));
    let mut user_servers = user_mcp_servers_for_agent(agent_name);
    // Apply per-agent MCP overrides (Settings → Claude Config): a server the
    // user disabled for this agent is dropped from its forwarded mcp-config.
    let agent_override = crate::claude_config::resolve_agent_overrides(
        &crate::claude_config::load_overrides(&paths.data_dir),
        agent_name,
    );
    for name in crate::claude_config::overrides::disabled_mcp(&agent_override) {
        user_servers.remove(&name);
    }
    let json = mcp_config_json(signaling_addr, session_id, agent_name, &user_servers);
    std::fs::write(&mcp_config_path, json)
        .with_context(|| format!("writing mcp-config to {}", mcp_config_path.display()))?;

    let spawn_cfg = SpawnConfig {
        agent_name: agent_name.to_string(),
        config,
        system_prompt_path,
        mcp_config_path: Some(mcp_config_path),
        working_dir,
        claude_bin: None,
        session_id: session_id.to_string(),
        resume_session_id,
        project: project.clone(),
        data_dir: paths.data_dir.clone(),
        session_effort,
        session_ultracode,
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
/// HANDS (Brian) gets the full merged set from the
/// user's claude-code config so they can drive browsers, talk to Discord,
/// etc.
pub fn user_mcp_servers_for_agent(agent_name: &str) -> serde_json::Map<String, serde_json::Value> {
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
///      bearing parts (push gates, CL workflow, IPAV, prod safety) can't
///      drift if a user edits a CL file.
///   4. **`<data_dir>/library/custom-general-rules.md`** — user-editable
///      additions to the universal rules (optional).
///   5. **`<data_dir>/library/agents/<name>/custom-instruction.md`** — per-agent
///      overrides (optional).
///   6. **Policy directive block** — rendered from policy.yaml, project-aware.
///
/// Project context BODIES (conventions / notes / decisions content) are NOT
/// injected here — agents pull those via `cl_index_search` + `Read` when
/// assigned a project task. What IS injected (when `cl_index` is provided) is a
/// compact CL *index primer*: the same `file_path — description` rows
/// `cl_index_search` returns, so an agent that skips the tool on a cold start
/// still knows what context exists to pull. This keeps spawn-time prompts
/// compact (table-of-contents, not the books) while pre-warming the map.
///
/// Missing optional files are logged at debug and skipped. Policy parse
/// errors propagate — broken YAML should surface loudly.
pub fn read_system_prompt(
    paths: &Paths,
    agent: &str,
    project: Option<&str>,
    project_root: Option<&Path>,
    cl_index: Option<&[ClIndexEntry]>,
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
    let (project_arg, project_line) = match project {
        Some(p) => (
            format!("\"{p}\""),
            format!(
                "**This session's project is `{p}`** — pass it as the \
                 `project` argument below.\n\n"
            ),
        ),
        None => ("\"_globals\"".to_string(), String::new()),
    };
    out.push_str(&format!(
        "## Context Library\n\n\
         {project_line}\
         Your Context Library lives at `{cl}`. Single source of truth — \
         other `~/.bot-hq*` paths are archives from prior installs, ignore \
         them.\n\n\
         **Index-first.** The CL is indexed in SQLite; each file has a \
         description so you can decide what's worth opening without burning \
         context on irrelevant files. Call \
         `cl_index_search(project=<your project>)` BEFORE reaching for \
         `Read` on any CL path. Pass \
         `\"_globals\"` for system-level / cross-project notes, your \
         session's project name for project-scoped notes, or omit `project` \
         to search everything. Folders carry their own descriptions in \
         `cl_folders` — `cl_folder_search(project=<your project>)` returns \
         folder-level summaries so you can scope a sweep before opening \
         individual files. Tool signatures for `cl_index_search`, \
         `cl_folder_search`, `cl_register_read`, `cl_rescan` are in the \
         General rules section below.\n\n\
         **Bare-filename heuristic.** If the user references a bare \
         filename (e.g. \"work on task 1 from tasks.md\", \"check scratch.md\") \
         and it's NOT in your working repo, do NOT keep `Glob`-searching \
         broader paths. Try `cl_index_search(project=\"_globals\", \
         query=<name>)` next — common cross-project files like `tasks.md` \
         and `scratch.md` live at the CL root and surface as `_globals` rows. \
         Only fall back to `ask_user_choice` if `_globals` also misses.\n\n\
         Per-project conventional files at `{cl}/projects/<project>/` \
         (the index covers everything under this path, not just these):\n\
         - `conventions.md` — repo, stack, commands, gates, commit rules\n\
         - `notes.md` — current state, recurring trouble, gotchas\n\
         - `decisions.md` — chronological log of prior decisions\n\
         - `policy.yaml` — machine-enforced gates (already rendered into \
         this prompt if the project has one)\n\n\
         Trust the index over a hardcoded filename list. Don't ask the user \
         for facts that live in the CL.\n\n",
        cl = paths.cl_dir.display()
    ));

    // 2b. Project CL index primer — the concrete table of contents for THIS
    // project (filenames + descriptions, most-recently-updated first). Only the
    // index rows `cl_index_search` already returns; bodies stay pull-only. This
    // pre-warms a cold start so an agent that skips `cl_index_search` on its
    // first turn still knows what project context exists to pull. Empty for
    // `_globals` / repo-less sessions.
    if let Some(entries) = cl_index {
        let primer = render_cl_primer(entries);
        if !primer.is_empty() {
            push_section(&mut out, &primer);
        }
    }

    // 3. Hardcoded universal rules — always present.
    push_section(&mut out, crate::agents::GENERAL_RULES);

    // 4 + 5. Optional user-editable slots: custom-general-rules.md applies to
    // all agents; agents/<name>/custom-instruction.md is per-agent.
    let slots = [
        paths.cl_dir.join("custom-general-rules.md"),
        paths
            .cl_dir
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

    // Interpolate the generic `<your project>` placeholder — used in the role
    // prompt, GENERAL_RULES, and the CL anchor above — with the resolved
    // project name, so every `cl_index_search(project=…)` example names the
    // real project instead of leaving the agent to guess (a wrong guess
    // silently returns nothing). Repo-less sessions default to `"_globals"`.
    out = out.replace("<your project>", &project_arg);
    Ok(out)
}

/// Number of CL index rows the spawn-time primer lists. The CL is deliberately
/// kept light (one-liner descriptions), so this cap is a guardrail against a
/// pathological project, not an expected truncation.
const CL_PRIMER_MAX_ROWS: usize = 12;
/// Per-row description cap so a body-snippet description (files with no H1) can't
/// bloat the prompt — the primer is a table of contents, not content.
const CL_PRIMER_DESC_MAX: usize = 100;

/// Render the project CL index as a compact "table of contents" primer:
/// `` - `file_path` — description `` lines in the order `cl_index_search`
/// returns them (most-recently-updated first). Only the index rows — never file
/// bodies. `policy.yaml` is skipped (already rendered as the policy block).
/// Returns "" when there's nothing useful to list.
fn render_cl_primer(entries: &[ClIndexEntry]) -> String {
    let mut lines = Vec::new();
    for e in entries.iter().filter(|e| e.file_path != "policy.yaml") {
        let desc = e.description.trim();
        if desc.is_empty() {
            lines.push(format!("- `{}`", e.file_path));
        } else {
            let desc = truncate_chars(desc, CL_PRIMER_DESC_MAX);
            lines.push(format!("- `{}` — {}", e.file_path, desc));
        }
        if lines.len() >= CL_PRIMER_MAX_ROWS {
            break;
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "## Project CL — files available (this project's index)\n\n\
         These are the CL index rows for this project (most-recently-updated \
         first) so you know what context EXISTS without a cold-start \
         `cl_index_search`. Bodies are NOT inlined — pull the ones you need \
         with `Read` (or re-run `cl_index_search` for the full, live list):\n\n\
         {}\n",
        lines.join("\n")
    )
}

/// Truncate to at most `max` chars (char-boundary safe), appending `…` when cut.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
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

/// Last-resort spawn config when an agent has neither a chosen saved model nor
/// a stored `agent_config` row (near-unreachable — agent configs seed in
/// migration 0001). Intentionally Anthropic for EVERY agent: at this tier we
/// hold no gateway credentials (`base_url`/`auth_token`), and Anthropic's
/// ambient auth is the only provider that works without them. Labeling a
/// non-Anthropic agent here (e.g. Rain on her DeepSeek gateway) would ship a
/// dead, unreachable config, so the universal Anthropic default is deliberate.
fn default_agent_config(name: &str) -> AgentConfig {
    AgentConfig {
        agent_name: name.to_string(),
        provider: "anthropic".into(),
        model_name: "claude-opus-4-7".into(),
        base_url: None,
        auth_token: None,
        updated_at: String::new(),
    }
}

/// Resolve the `AgentConfig` to spawn an agent with. Prefers an explicit
/// saved-model id (chosen in the create dialog, stored on the session row); a
/// missing/empty id or a deleted model falls back to the per-agent config, then
/// the hardcoded default. Keeps the legacy path intact for sessions created
/// before per-agent model selection existed (`*_model_id` is NULL there).
pub(crate) async fn resolve_spawn_config(
    storage: &Storage,
    agent_name: &str,
    model_id: Option<&str>,
) -> AgentConfig {
    if let Some(id) = model_id.filter(|s| !s.is_empty()) {
        if let Ok(Some(m)) = storage.get_model(id).await {
            return AgentConfig {
                agent_name: agent_name.to_string(),
                provider: m.provider,
                model_name: m.model_name,
                base_url: m.base_url,
                auth_token: m.auth_token,
                updated_at: m.updated_at,
            };
        }
        tracing::warn!(
            agent = agent_name,
            model_id = id,
            "chosen model not found; falling back to agent config"
        );
    }
    storage
        .get_agent_config(agent_name)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| default_agent_config(agent_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn resolve_project_prefers_registered_lookup_over_basename() {
        let s = Storage::memory().await.unwrap();
        s.upsert_project("acme", "acme", Some("/repos/acme-web"), None, None)
            .await
            .unwrap();
        // Registered repo with a non-matching basename → project name wins.
        let (p, prov) =
            resolve_session_project(&s, None, Some(Path::new("/repos/acme-web"))).await;
        assert_eq!(p.as_deref(), Some("acme"));
        assert_eq!(prov, ProjectProvenance::Registered);
    }

    #[tokio::test]
    async fn resolve_project_falls_back_to_basename() {
        let s = Storage::memory().await.unwrap();
        let (p, prov) =
            resolve_session_project(&s, None, Some(Path::new("/repos/loose-repo"))).await;
        assert_eq!(p.as_deref(), Some("loose-repo"));
        assert_eq!(prov, ProjectProvenance::Inferred);
    }

    #[tokio::test]
    async fn resolve_project_matches_base_repo_for_worktree_sessions() {
        let s = Storage::memory().await.unwrap();
        s.upsert_project("acme", "acme", Some("/repos/acme-web"), None, None)
            .await
            .unwrap();
        // Worktree session: working path is the worktree; base must drive
        // the lookup.
        let (p, prov) = resolve_session_project(
            &s,
            Some("/repos/acme-web"),
            Some(Path::new("/data/.local/worktrees/s-1/acme-web")),
        )
        .await;
        assert_eq!(p.as_deref(), Some("acme"));
        assert_eq!(prov, ProjectProvenance::Registered);
    }

    #[tokio::test]
    async fn resolve_project_none_without_repo() {
        let s = Storage::memory().await.unwrap();
        let (p, prov) = resolve_session_project(&s, None, None).await;
        assert_eq!(p, None);
        assert_eq!(prov, ProjectProvenance::None);
    }

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
    }

    #[test]
    fn prompt_starts_with_hardcoded_role() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let prompt = read_system_prompt(&paths, "brian", None, None, None).unwrap();
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
        let agent_dir = paths.cl_dir.join("agents/brian");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("custom-instruction.md"),
            "BRIAN_CUSTOM_PREFS_X9Q",
        )
        .unwrap();
        let prompt = read_system_prompt(&paths, "brian", None, None, None).unwrap();
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

        let prompt = read_system_prompt(&paths, "brian", Some("foo"), None, None).unwrap();
        assert!(!prompt.contains("FOO_CONVENTIONS_M1"));
        assert!(!prompt.contains("FOO_NOTES_M1"));
        assert!(!prompt.contains("FOO_DECISIONS_M1"));
    }

    fn cl_entry(file_path: &str, description: &str) -> ClIndexEntry {
        ClIndexEntry {
            id: 0,
            project_id: "foo".into(),
            file_path: file_path.into(),
            description: description.into(),
            tags: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn cl_primer_injects_index_rows_but_not_bodies() {
        // F-B: the CL index primer surfaces the table of contents (filenames +
        // descriptions) so an agent cold-starts knowing what to pull — but
        // NEVER file bodies (those stay pull-only via cl_index_search + Read).
        // policy.yaml is omitted (it's already rendered as the policy block).
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let entries = vec![
            cl_entry("conventions.md", "repo, stack, commands"),
            cl_entry("notes.md", "durable gotchas"),
            cl_entry("policy.yaml", "machine gates"),
        ];
        let prompt =
            read_system_prompt(&paths, "brian", Some("foo"), None, Some(&entries)).unwrap();
        assert!(prompt.contains("Project CL — files available"));
        assert!(prompt.contains("`conventions.md` — repo, stack, commands"));
        assert!(prompt.contains("`notes.md` — durable gotchas"));
        // policy.yaml filtered (already the policy block).
        assert!(!prompt.contains("`policy.yaml` — machine gates"));
    }

    #[test]
    fn cl_primer_absent_when_no_index_provided() {
        // No primer rows (repo-less / _globals) → no primer section. Keeps the
        // existing prompt shape for sessions without a project.
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let prompt = read_system_prompt(&paths, "brian", Some("foo"), None, None).unwrap();
        assert!(!prompt.contains("Project CL — files available"));
    }

    #[test]
    fn cl_opener_nudge_fires_for_real_project_only() {
        // A1: the session-start nudge pages the agent at cl_index_search for a
        // real project, and is absent for repo-less / _globals / empty sessions
        // (no project conventions to load).
        let nudge = cl_opener_nudge(Some("bot-hq")).expect("real project gets a nudge");
        assert!(nudge.contains("cl_index_search(project=\"bot-hq\")"));
        assert_eq!(cl_opener_nudge(None), None, "repo-less session: no nudge");
        assert_eq!(cl_opener_nudge(Some("_globals")), None, "_globals: no nudge");
        assert_eq!(cl_opener_nudge(Some("")), None, "empty project: no nudge");
    }

    #[test]
    fn render_cl_primer_skips_policy_and_caps_rows() {
        let mut entries = vec![cl_entry("policy.yaml", "gates")];
        for i in 0..20 {
            entries.push(cl_entry(&format!("f{i}.md"), "d"));
        }
        let out = render_cl_primer(&entries);
        assert!(!out.contains("policy.yaml"), "policy.yaml must be filtered");
        let rows = out.lines().filter(|l| l.starts_with("- `")).count();
        assert_eq!(rows, CL_PRIMER_MAX_ROWS, "row count must be capped");
    }

    #[test]
    fn render_cl_primer_empty_when_no_usable_rows() {
        assert_eq!(render_cl_primer(&[]), "");
        // Only policy.yaml present → filtered → nothing to render.
        assert_eq!(render_cl_primer(&[cl_entry("policy.yaml", "x")]), "");
    }

    #[test]
    fn render_cl_primer_truncates_long_description() {
        let long = "x".repeat(250);
        let out = render_cl_primer(&[cl_entry("notes.md", &long)]);
        assert!(out.contains('…'), "over-long description should be truncated");
        assert!(
            !out.contains(&"x".repeat(CL_PRIMER_DESC_MAX + 1)),
            "full over-long description must not appear in the primer"
        );
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
        let prompt = read_system_prompt(&paths, "brian", None, None, None).unwrap();
        assert!(prompt.contains("cl_index_search"));
        assert!(prompt.contains("Index-first"));
        // Regression: when the user mentions a bare filename (tasks.md,
        // scratch.md), agents should head to _globals before falling back to
        // ask_user_choice or broad Glob sweeps.
        assert!(prompt.contains("Bare-filename heuristic"));
        assert!(prompt.contains("_globals"));
    }

    #[test]
    fn cl_anchor_interpolates_resolved_project_name() {
        // Issue: the CL anchor used to print the literal placeholder
        // `cl_index_search(project=<your project>)`, so an agent had to GUESS
        // its project key — and a wrong guess silently returns nothing. The
        // resolved project name is now interpolated into the anchor and stated
        // explicitly, removing the silent wrong-scope failure mode.
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let prompt = read_system_prompt(&paths, "brian", Some("bot-hq"), None, None).unwrap();
        assert!(
            prompt.contains("cl_index_search(project=\"bot-hq\")"),
            "CL anchor must interpolate the resolved project name"
        );
        assert!(
            prompt.contains("This session's project is `bot-hq`"),
            "CL anchor must state the session's project explicitly"
        );
        assert!(
            !prompt.contains("project=<your project>"),
            "no literal placeholder should survive interpolation"
        );
        // Repo-less session (project None) falls back to the _globals example
        // rather than leaving a dangling placeholder.
        let prompt_none = read_system_prompt(&paths, "brian", None, None, None).unwrap();
        assert!(prompt_none.contains("cl_index_search(project=\"_globals\")"));
    }

    #[test]
    fn missing_optional_slots_are_fine() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        // No custom-general-rules.md content, nothing in agents/<name>/ —
        // should still produce a prompt with at minimum the hardcoded role
        // and the hardcoded universal rules.
        std::fs::remove_file(paths.cl_dir.join("custom-general-rules.md")).ok();
        let prompt = read_system_prompt(&paths, "rain", Some("nonexistent"), None, None).unwrap();
        assert!(prompt.contains("EYES"));
        assert!(prompt.contains("Working directory"));
    }

    #[test]
    fn prompt_always_contains_hardcoded_general_rules() {
        // Load-bearing test: even on a freshly-init'd data dir with the
        // user's custom file deleted, the universal rules must be present
        // (working directory, push gate, IPAV, prod safety).
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        std::fs::remove_file(paths.cl_dir.join("custom-general-rules.md")).ok();
        let prompt = read_system_prompt(&paths, "brian", None, None, None).unwrap();
        assert!(
            prompt.contains("Working directory"),
            "missing working-directory section"
        );
        assert!(
            prompt.contains("`git push` is governed by the session's push gate"),
            "missing push gate"
        );
        assert!(prompt.contains("IPAV discipline"), "missing IPAV section");
        assert!(
            prompt.contains("Production data access"),
            "missing prod-safety section"
        );
    }

    #[test]
    fn custom_general_rules_appends_to_hardcoded() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        std::fs::write(
            paths.cl_dir.join("custom-general-rules.md"),
            "MY_ORG_RULE_X7P: always prefer ripgrep over grep.\n",
        )
        .unwrap();
        let prompt = read_system_prompt(&paths, "brian", None, None, None).unwrap();
        // Both layers present.
        assert!(prompt.contains("Working directory"));
        assert!(prompt.contains("MY_ORG_RULE_X7P"));
        // Custom additions come AFTER the hardcoded core.
        let core_pos = prompt.find("Working directory").unwrap();
        let custom_pos = prompt.find("MY_ORG_RULE_X7P").unwrap();
        assert!(
            custom_pos > core_pos,
            "custom rules should append after hardcoded core"
        );
    }
}
