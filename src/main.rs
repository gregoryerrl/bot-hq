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
    // Default RUST_BACKTRACE=full so a Rust panic anywhere (Slint FFI
    // callbacks, startup, async tasks) prints a full backtrace to stderr.
    // Without this the panic dies as a bare `abort()` in the crash report
    // and we lose the panic site. User can still pin a different level
    // (RUST_BACKTRACE=1 / =0) by exporting before launch.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        // SAFETY: single-threaded main-thread setup before any other threads spawn.
        unsafe {
            std::env::set_var("RUST_BACKTRACE", "full");
        }
    }

    // Chain a panic hook that SIGKILLs every registered claude-code child
    // BEFORE the original hook prints the panic + unwind reaches the C++
    // FFI barrier and aborts. Without this, a Slint-callback panic leaves
    // brian/rain/emma orphaned to launchd, where they keep editing files
    // (the ghost-Brian incident). Combined with the per-callback
    // catch_unwind, this is belt-and-suspenders.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        bot_hq::agents::spawn::reap_all_children();
        original_hook(info);
    }));

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
        // Bridge in-memory state is gone after a restart; any leftover
        // session-permission JSON files would let a fresh session inherit
        // grants it never earned. Wipe them all.
        if let Err(e) =
            bot_hq::policy::session_permissions::purge_all_session_permissions(&paths.data_dir)
        {
            tracing::warn!(?e, "purge_all_session_permissions failed at startup");
        }
        let bridge = SignalingBridge::with_policy(violations, paths.data_dir.clone());
        // Wire storage into the bridge so out-of-band choice resolutions (when
        // the agent's blocking tool call already timed out client-side) can be
        // surfaced as synthetic user messages.
        bridge.set_storage(storage.clone()).await;
        // Initialize the CL index: scan projects on disk, upsert project rows,
        // rescan each one so file metadata gets indexed. Also one-shot
        // import of legacy bcc-ad-manager files. All idempotent.
        if let Err(e) = cl_startup_init(&storage, &bridge, &paths.data_dir).await {
            tracing::warn!(?e, "cl startup init failed — index may be partial");
        }
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
    // Register the embedded Lucide icon font with the shared font
    // collection. Must happen AFTER AppWindow::new() because that's what
    // initializes the slint platform (the fontique collection lives on
    // the platform context). Failure is non-fatal — icons fall back to
    // .notdef boxes if registration trips, but the UI still works.
    {
        use slint::fontique_08::fontique;
        let bytes: &[u8] = include_bytes!("../assets/fonts/lucide.ttf");
        let blob = fontique::Blob::new(std::sync::Arc::new(bytes.to_vec()));
        let mut collection = slint::fontique_08::shared_collection();
        let fonts = collection.register_fonts(blob, None);
        tracing::info!(font_count = fonts.len(), "lucide.ttf registered");
    }
    let state = window.global::<AppState>();
    apply_init_outcome(&state, &paths, &init_outcome);

    // Shutdown-signal handler: when bot-hq is killed from outside
    // (SIGTERM from launchd, SIGINT from the terminal, SIGHUP on
    // session disconnect), the OS event loop never returns from
    // `window.run()` so the drop chain at the bottom of main never
    // fires. Without an explicit reap, the claude-code children orphan
    // to launchd — same incident class as the panic-abort case. Uses
    // tokio's signal API (self-pipe; the actual handler is async-
    // signal-safe and the work runs on a normal tokio worker).
    rt.spawn(async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).ok();
        let mut sigint = signal(SignalKind::interrupt()).ok();
        let mut sighup = signal(SignalKind::hangup()).ok();
        tokio::select! {
            _ = async { if let Some(s) = sigterm.as_mut() { s.recv().await; } }, if sigterm.is_some() => {}
            _ = async { if let Some(s) = sigint.as_mut() { s.recv().await; } }, if sigint.is_some() => {}
            _ = async { if let Some(s) = sighup.as_mut() { s.recv().await; } }, if sighup.is_some() => {}
            else => {
                // All three signal() registrations failed (non-Unix host, or
                // container without signal support). Park forever so the
                // select! doesn't panic — children will be reaped via the
                // panic-hook path or window-close drop chain instead.
                tracing::warn!("no signal handlers installed; shutdown won't trigger child reap via signal");
                std::future::pending::<()>().await;
            }
        }
        tracing::warn!("shutdown signal received; reaping children");
        bot_hq::agents::spawn::reap_all_children();
        std::process::exit(0);
    });

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

/// Path of the legacy bot-hq CL root we offer a one-shot import from.
/// Only `bcc-ad-manager` content is migrated; other legacy projects are
/// considered superseded by their entries in the new CL or deliberately
/// dropped.
const LEGACY_CL_ROOT: &str = ".bot-hq-legacy-2026-05-15";

/// Idempotent CL index initialization. Called once at startup, after storage
/// is opened and the bridge has been wired with storage + data_dir.
///
/// Three jobs:
///   1. Walk `<data_dir>/projects/<name>/` and register each subdirectory as
///      a project row (best-effort display_name).
///   2. Call `bridge.cl_rescan(<project>)` for every project + `_globals`,
///      which auto-adds new files to `cl_index` with description seeded
///      from the file's first H1.
///   3. One-shot legacy import: copy missing files from
///      `~/.bot-hq-legacy-2026-05-15/projects/bcc-ad-manager/` into
///      `<data_dir>/projects/bcc-ad-manager/`. Non-destructive (skips
///      files that already exist in the new CL).
async fn cl_startup_init(
    storage: &Storage,
    bridge: &Arc<SignalingBridge>,
    data_dir: &std::path::Path,
) -> Result<()> {
    let projects_dir = data_dir.join("projects");
    if projects_dir.is_dir() {
        for entry in std::fs::read_dir(&projects_dir)? .flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if !n.starts_with('.') => n,
                _ => continue,
            };
            storage
                .upsert_project(name, name, None, None, None)
                .await?;
        }
    }

    // Legacy bcc-ad-manager import — one-shot, non-destructive.
    if let Some(base) = directories::BaseDirs::new() {
        let legacy_bcc = base
            .home_dir()
            .join(LEGACY_CL_ROOT)
            .join("projects")
            .join("bcc-ad-manager");
        if legacy_bcc.is_dir() {
            let target = data_dir.join("projects").join("bcc-ad-manager");
            std::fs::create_dir_all(&target).ok();
            let imported = mirror_dir_non_destructive(&legacy_bcc, &target)?;
            if imported > 0 {
                tracing::info!(
                    imported,
                    "imported {} new files from legacy bcc-ad-manager CL",
                    imported
                );
                // Make sure the project row exists.
                storage
                    .upsert_project("bcc-ad-manager", "bcc-ad-manager", None, None, None)
                    .await?;
            }
        }
    }

    // Rescan every project (including _globals) so the index sees the
    // current filesystem.
    let projects = storage.list_projects().await?;
    for p in projects {
        if let Err(e) = bridge.cl_rescan(&p.name).await {
            tracing::warn!(?e, project = %p.name, "cl_rescan failed");
        }
    }
    Ok(())
}

/// Copy every file from `src` to `dst` (recursively), preserving relative
/// structure, BUT only when the target file doesn't already exist. Returns
/// the count of files actually copied. The user can prune duplicates later
/// via the UI; we never overwrite their newer content.
fn mirror_dir_non_destructive(
    src: &std::path::Path,
    dst: &std::path::Path,
) -> std::io::Result<usize> {
    let mut copied = 0usize;
    for entry in std::fs::read_dir(src)?.flatten() {
        let src_path = entry.path();
        let name = match src_path.file_name() {
            Some(n) => n.to_os_string(),
            None => continue,
        };
        // Skip hidden (.git, .local, etc.) — legacy CL had a lock file and
        // sqlite db in the root we don't want to drag over.
        if name.to_str().is_some_and(|n| n.starts_with('.')) {
            continue;
        }
        let dst_path = dst.join(&name);
        if src_path.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            copied += mirror_dir_non_destructive(&src_path, &dst_path)?;
        } else if !dst_path.exists() {
            std::fs::copy(&src_path, &dst_path)?;
            copied += 1;
        }
    }
    Ok(copied)
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
