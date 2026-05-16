use anyhow::Result;
use bot_hq::core::AppState as CoreAppState;
use bot_hq::paths::{InitOutcome, LockGuard, Paths};
use bot_hq::policy::{hooks, ViolationsLog};
use bot_hq::signaling::{start_external_server, start_signaling_server, SignalingBridge};
use bot_hq::storage::Storage;
use bot_hq::ui::install_view_model;
use bot_hq::{AppState, AppWindow};
use slint::ComponentHandle;
use std::sync::Arc;
use tokio::runtime::Builder;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // CLI subcommand dispatch — runs BEFORE GUI init so git hooks don't
    // pay the Slint/tokio startup cost. Hooks invoke us hundreds of
    // milliseconds per commit; the GUI takes seconds.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "policy-check" {
        return run_policy_check_cli(&args[2..]);
    }
    if args.len() >= 2 && args[1] == "install-hooks" {
        return run_install_hooks_cli(&args[2..]);
    }

    init_logging();
    tracing::info!("bot-hq starting");

    // Load .env if present (best-effort; ignored if missing).
    if let Ok(env_path) = std::env::current_dir().map(|p| p.join(".env")) {
        let _ = load_env_file(&env_path);
    }

    let paths = Paths::from_env()?;
    let init_outcome = paths.init()?;
    tracing::info!(data_dir = %paths.data_dir.display(), outcome = ?init_outcome, "data dir ready");

    let _lock = LockGuard::acquire(&paths.lock_path)?;

    // Tokio runtime on a dedicated thread pool. The Slint event loop owns the
    // OS main thread; all async I/O (storage, agents, HTTP) runs here.
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let rt = runtime.handle().clone();

    let core: Arc<CoreAppState> = runtime.block_on(async {
        let storage = Storage::open(&paths.db_path).await?;
        let violations = ViolationsLog::new(&paths.data_dir);
        let bridge = SignalingBridge::with_policy(violations, paths.data_dir.clone());
        let server = start_signaling_server(bridge).await?;
        tracing::info!(addr = %server.local_addr, "signaling server up");
        Ok::<_, anyhow::Error>(Arc::new(
            CoreAppState::new(paths.clone(), storage, server).await,
        ))
    })?;

    // Spawn Emma's solo agent (NOT the duo — Emma is a singleton helper, not a
    // bilateral pair) so her chat is responsive on first message. Failure is
    // non-fatal — Emma chat stays dormant until the user fixes the env (e.g.,
    // installs/auths claude-code CLI).
    runtime.block_on(async {
        if let Err(e) = core.ensure_emma_started().await {
            tracing::warn!(
                error = ?e,
                "failed to spawn emma — chat will be inactive until restart"
            );
        }
    });

    // External MCP server — lets another agent (Claude Code, etc.) drive
    // bot-hq from outside. Soft-fails on port conflict + when disabled. The
    // returned handle is stored on AppState so it lives until shutdown.
    runtime.block_on(async {
        if std::env::var("BOT_HQ_EXTERNAL_MCP_DISABLED").is_ok() {
            tracing::info!("external MCP server disabled via BOT_HQ_EXTERNAL_MCP_DISABLED");
            return;
        }
        let port: u16 = std::env::var("BOT_HQ_EXTERNAL_MCP_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7892);
        let token = match paths.read_mcp_token() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(?e, "external MCP: failed to read token, skipping startup");
                return;
            }
        };
        match start_external_server(Arc::clone(&core), port, token).await {
            Ok(server) => {
                tracing::info!(addr = %server.local_addr, "external MCP server up");
                core.external_server.lock().await.replace(server);
            }
            Err(e) => {
                tracing::warn!(?e, port, "external MCP port unavailable — skipping startup; internal MCP is unaffected");
            }
        }
    });

    let window = AppWindow::new()?;
    let state = window.global::<AppState>();
    apply_init_outcome(&state, &paths, &init_outcome);

    runtime.block_on(install_view_model(&window, Arc::clone(&core), rt))?;

    window.run()?;

    // After window closes, drop everything in an orderly fashion.
    drop(core);
    drop(runtime);
    Ok(())
}

fn apply_init_outcome(state: &AppState, paths: &Paths, outcome: &InitOutcome) {
    let text = match outcome {
        InitOutcome::FirstRun => format!(
            "Initialized fresh Context Library at {}",
            paths.data_dir.display()
        ),
        InitOutcome::Repaired { repaired_slots } => {
            format!("Repaired missing CL slot(s): {}", repaired_slots.join(", "))
        }
        InitOutcome::Existing => String::new(),
    };
    state.set_status_text(text.into());
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,bot_hq=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Handle `bot-hq policy-check <subcommand> [--data-dir P] [--project Q] ...`.
/// Used by git hooks installed in working repos. Exits with the appropriate
/// status code (0 = clean / allow, 1 = block).
fn run_policy_check_cli(args: &[String]) -> Result<()> {
    use std::process::ExitCode;
    let exit_code = hooks::run_cli(args)
        .unwrap_or_else(|e| {
            eprintln!("bot-hq policy-check: {e}");
            // Soft-fail: don't break the user's git workflow on internal
            // errors. The hook prints the error; user can investigate.
            // Returning 0 means git allows the operation to proceed.
            0
        });
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    let _ = ExitCode::SUCCESS;
    Ok(())
}

/// Handle `bot-hq install-hooks --repo <P> --data-dir <D> [--project <Q>]`.
/// Useful for manual install in a working repo (CI scripts, dev tooling).
/// Normal session-spawn flow installs hooks automatically.
fn run_install_hooks_cli(args: &[String]) -> Result<()> {
    let mut repo: Option<std::path::PathBuf> = None;
    let mut data_dir: Option<std::path::PathBuf> = None;
    let mut project: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" => {
                repo = Some(std::path::PathBuf::from(
                    args.get(i + 1).ok_or_else(|| anyhow::anyhow!("--repo needs value"))?,
                ));
                i += 2;
            }
            "--data-dir" => {
                data_dir = Some(std::path::PathBuf::from(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("--data-dir needs value"))?,
                ));
                i += 2;
            }
            "--project" => {
                project = Some(args.get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--project needs value"))?
                    .clone());
                i += 2;
            }
            unknown => return Err(anyhow::anyhow!("unknown flag {unknown}")),
        }
    }
    let repo = repo.ok_or_else(|| anyhow::anyhow!("--repo is required"))?;
    let data_dir = data_dir.ok_or_else(|| anyhow::anyhow!("--data-dir is required"))?;
    let report = hooks::install_hooks(&repo, &data_dir, project.as_deref())?;
    if report.not_a_git_repo {
        println!("not a git repo: {}", repo.display());
        return Ok(());
    }
    println!(
        "hooks: installed={:?} updated={:?} sidecar={:?} unchanged={:?}",
        report.installed, report.updated, report.sidecar, report.unchanged
    );
    Ok(())
}

/// Minimal .env loader. Mutates the process env. Lines that aren't `KEY=VALUE`
/// are skipped silently. Existing env vars take precedence.
fn load_env_file(path: &std::path::Path) -> std::io::Result<()> {
    let body = std::fs::read_to_string(path)?;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if std::env::var_os(key).is_none() {
                // SAFETY: single-threaded main-thread setup before any other threads spawn.
                unsafe {
                    std::env::set_var(key, value);
                }
            }
        }
    }
    Ok(())
}
