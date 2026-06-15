use anyhow::Result;
use bot_hq::core::AppState as CoreAppState;
use bot_hq::paths::{LockGuard, Paths};
use bot_hq::plugins::Heartbeat;
use bot_hq::plugins::PluginRegistry;
use bot_hq::policy::{hooks, ViolationsLog};
use bot_hq::signaling::{start_external_server, start_signaling_server, SignalingBridge};
use bot_hq::storage::Storage;
use bot_hq::tauri_events;
use bot_hq::tauri_events::types::AgentMessage;
use serde_json::Value;
use std::sync::Arc;
use tauri::Emitter;
use tokio::runtime::Builder;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Default RUST_BACKTRACE=full so a Rust panic anywhere prints a full
    // backtrace to stderr. Without this the panic dies as a bare `abort()`
    // and we lose the panic site. User can pin a different level by
    // exporting RUST_BACKTRACE before launch.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        // SAFETY: single-threaded main-thread setup before any other threads spawn.
        unsafe {
            std::env::set_var("RUST_BACKTRACE", "full");
        }
    }

    // Chain a panic hook that SIGKILLs every registered claude-code child
    // BEFORE the original hook prints the panic + unwind reaches the FFI
    // barrier and aborts. Without this, a panic leaves brian/rain
    // orphaned to launchd (the ghost-Brian incident).
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        bot_hq::agents::spawn::reap_all_children();
        original_hook(info);
    }));

    // CLI subcommand dispatch — runs BEFORE GUI init so git hooks don't
    // pay the GUI startup cost. Hooks invoke us hundreds of milliseconds
    // per commit; the GUI takes seconds.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "policy-check" {
        return run_policy_check_cli(&args[2..]);
    }
    if args.len() >= 2 && args[1] == "install-hooks" {
        return run_install_hooks_cli(&args[2..]);
    }
    // Regenerate the frontend TypeScript bindings without launching the GUI
    // (dev/CI). The GUI also exports these on startup; this is the headless path.
    if args.len() >= 2 && args[1] == "export-bindings" {
        let builder = bot_hq::tauri_specta_gen::builder();
        builder
            .export(
                bot_hq::tauri_specta_gen::typescript_config(),
                "frontend/src/lib/bindings.ts",
            )
            .map_err(|e| anyhow::anyhow!("tauri-specta export failed: {e}"))?;
        println!("bindings exported to frontend/src/lib/bindings.ts");
        return Ok(());
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

    // Tokio runtime on dedicated worker threads. Tauri owns the OS main
    // thread; all async I/O (storage, agents, HTTP, bridge subscriber)
    // runs on this runtime.
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let rt = runtime.handle().clone();

    let (core, storage_arc, bridge_arc): (Arc<CoreAppState>, Arc<Storage>, Arc<SignalingBridge>) =
        runtime.block_on(async {
            let storage = Storage::open(&paths.db_path).await?;
            // Boot-time tray reconciliation: withdraw pending rows left on closed or
            // orphaned sessions (cruft from a close under a pre-fix binary). Keeps
            // the notification bell honest without waiting on a one-shot migration.
            match storage.withdraw_pending_tray_for_closed_or_orphaned().await {
                Ok(n) if n > 0 => {
                    tracing::info!(withdrawn = n, "swept stale pending tray rows at startup")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "startup tray sweep failed"),
            }
            let violations = ViolationsLog::new(&paths.data_dir);
            // Wipe any stale per-session policy snapshots — a leftover file would
            // leak a prior session's resolved policy into a fresh session that
            // should re-seed from the current blueprints.
            if let Err(e) =
                bot_hq::policy::session_policy::purge_all_session_policies(&paths.data_dir)
            {
                tracing::warn!(?e, "purge_all_session_policies failed at startup");
            }
            let bridge = SignalingBridge::with_policy(violations, paths.data_dir.clone());
            bridge.set_storage(storage.clone()).await;
            if let Err(e) = cl_startup_init(&storage, &bridge, &paths.data_dir).await {
                tracing::warn!(?e, "cl startup init failed — index may be partial");
            }
            let mut server = start_signaling_server(bridge.clone()).await?;
            tracing::info!(addr = %server.local_addr, "signaling server up");
            // Persist the bound address so the git pre-push hook (a separate
            // subprocess) can POST `/hooks/pre-push` to surface a per-push approval
            // prompt under `push_gate=ask`. Non-fatal; the hook fail-closes if the
            // file is absent. Registered on the server so it's removed on clean exit.
            if let Err(e) = paths.write_signaling_addr(server.local_addr) {
                tracing::warn!(?e, "failed to persist signaling addr for the pre-push hook");
            }
            server.set_addr_file(paths.signaling_addr_path.clone());

            // Local normalizing proxy for agents on a non-first-party gateway
            // (Rain → DeepSeek): strips request-build-time `role:"system"`
            // injections that strict gateways 400 on. Soft-fail — if it can't
            // bind, those agents hit their gateway directly and the rest of
            // bot-hq is unaffected. Started before any agent spawns so the addr
            // is installed by the time `build_command` reads it.
            match bot_hq::agents::llm_proxy::start_llm_proxy().await {
                Ok(proxy) => {
                    tracing::info!(addr = %proxy.local_addr, "llm normalizing proxy up");
                    bot_hq::agents::llm_proxy::install_global(proxy);
                }
                Err(e) => tracing::warn!(
                    ?e,
                    "llm proxy failed to start — agents on custom gateways will hit them directly"
                ),
            }
            let storage_arc = Arc::new(storage.clone());
            let bridge_arc = bridge.clone();
            let core = Arc::new(CoreAppState::new(paths.clone(), storage, server).await);
            Ok::<_, anyhow::Error>((core, storage_arc, bridge_arc))
        })?;

    // External MCP server — driver tools surface. Soft-fail on port
    // conflict or when explicitly disabled.
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
                tracing::warn!(
                    ?e,
                    port,
                    "external MCP port unavailable — skipping startup; internal MCP is unaffected"
                );
            }
        }
    });

    // Shutdown-signal handler. When killed from outside (SIGTERM from
    // launchd, SIGINT from terminal, SIGHUP on session disconnect), Tauri's
    // main-thread event loop never returns, so the panic-hook + signal-task
    // PID reapers are the only ways the claude-code children get killed.
    #[cfg(unix)]
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
                tracing::warn!("no signal handlers installed; shutdown won't trigger child reap via signal");
                std::future::pending::<()>().await;
            }
        }
        tracing::warn!("shutdown signal received; reaping children");
        bot_hq::agents::spawn::reap_all_children();
        std::process::exit(0);
    });

    // Windows twin: ctrl_c ≈ SIGINT, ctrl_close ≈ SIGHUP (console window
    // closed), ctrl_shutdown ≈ SIGTERM (logoff/OS shutdown). Windows has no
    // kill-children-on-parent-exit semantics, so the reap walk matters just
    // as much here as on unix.
    #[cfg(windows)]
    rt.spawn(async {
        use tokio::signal::windows;
        let mut ctrl_c = windows::ctrl_c().ok();
        let mut ctrl_close = windows::ctrl_close().ok();
        let mut ctrl_shutdown = windows::ctrl_shutdown().ok();
        tokio::select! {
            _ = async { if let Some(s) = ctrl_c.as_mut() { s.recv().await; } }, if ctrl_c.is_some() => {}
            _ = async { if let Some(s) = ctrl_close.as_mut() { s.recv().await; } }, if ctrl_close.is_some() => {}
            _ = async { if let Some(s) = ctrl_shutdown.as_mut() { s.recv().await; } }, if ctrl_shutdown.is_some() => {}
            else => {
                tracing::warn!("no signal handlers installed; shutdown won't trigger child reap via signal");
                std::future::pending::<()>().await;
            }
        }
        tracing::warn!("shutdown signal received; reaping children");
        bot_hq::agents::spawn::reap_all_children();
        std::process::exit(0);
    });

    // Export TypeScript bindings for the frontend at startup. Writes
    // `frontend/src/lib/bindings.ts` so the React side sees current
    // command signatures. Guarded on the target dir already existing:
    // the export CREATES missing intermediate dirs, so an unguarded call
    // litters `frontend/` into whatever CWD the app was launched from
    // (e.g. `~/frontend/` for a release app started from a terminal).
    // Repo-root launches keep the documented auto-regen; everywhere else
    // skips. Headless regen: `bot-hq export-bindings` (CLI branch above).
    let specta_builder = bot_hq::tauri_specta_gen::builder();
    if std::path::Path::new("frontend/src/lib").is_dir() {
        if let Err(e) = specta_builder.export(
            bot_hq::tauri_specta_gen::typescript_config(),
            "frontend/src/lib/bindings.ts",
        ) {
            tracing::warn!(
                ?e,
                "tauri-specta bindings export failed (frontend may have stale types)"
            );
        }
    } else {
        tracing::debug!("skipping bindings export (frontend/src/lib not present in cwd)");
    }

    // Plugin registry — scans `<data_dir>/plugins/` and owns the heartbeat
    // state. Constructed eagerly so we can pass it to Tauri's `.manage()` AND
    // share the Heartbeat with the setup-time sweep loop. Capability JSONs
    // land under `<data_dir>/capabilities/` so Tauri's compile-time glob can
    // pick them up on subsequent builds.
    let registry = Arc::new(PluginRegistry::new(
        paths.data_dir.clone(),
        paths.data_dir.join("capabilities"),
    )?);

    // Hand off to Tauri. Tauri owns the OS main thread.
    let storage_for_subscriber = Arc::clone(&storage_arc);
    let bridge_for_subscriber = Arc::clone(&bridge_arc);
    let rt_for_setup = rt.clone();
    let core_for_setup = Arc::clone(&core);
    let registry_for_setup = Arc::clone(&registry);
    let bridge_for_fs = Arc::clone(&bridge_arc);
    let paths_for_fs = paths.clone();

    tauri::Builder::default()
        // Opener plugin — the update banner's "Download" button opens the
        // GitHub release page in the system browser via `openUrl`.
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::clone(&storage_arc))
        .manage(Arc::clone(&bridge_arc))
        .manage(Arc::clone(&core))
        .manage(Arc::clone(&registry))
        .invoke_handler(specta_builder.invoke_handler())
        .setup(move |app| {
            // Tauri's setup runs on the OS main thread outside any Tokio
            // runtime context. spawn_subscriber + BatchEmitter::new both call
            // `tokio::spawn` internally (thread-local lookup), so we have to
            // enter the runtime for the duration of those calls. The spawned
            // tasks themselves are bound to the runtime once registered.
            let _rt_guard = rt_for_setup.enter();
            // Stash the AppHandle on CoreAppState AND on the bridge so MCP
            // tools (screenshot, webview automation) can reach the webview.
            // CoreAppState is for the external MCP path; the bridge copy is
            // for the internal MCP (per-agent jsonrpc.rs), which doesn't see
            // CoreAppState. Set-once — ignore the Err on duplicate.
            let handle = app.handle().clone();
            let _ = core_for_setup.app_handle.set(handle.clone());
            bridge_for_subscriber.set_app_handle(handle);
            // Wire the bridge subscriber: SignalingEvent stream → Tauri emit.
            let app_handle_for_msgs = app.handle().clone();
            let app_handle_for_events = app.handle().clone();
            tauri_events::spawn_subscriber(
                bridge_for_subscriber,
                storage_for_subscriber,
                move |msgs: Vec<AgentMessage>| {
                    if let Err(e) = app_handle_for_msgs
                        .emit(AgentMessage::EVENT_NAME_BATCH, &msgs)
                    {
                        tracing::warn!(?e, "emit agent.messages.batch failed");
                    }
                },
                move |name: &str, payload: Value| {
                    if let Err(e) = app_handle_for_events.emit(name, &payload) {
                        tracing::warn!(?e, event = name, "emit event failed");
                    }
                },
            );
            // Filesystem watcher → CL/EOD freshness. Watches the Context Library
            // dir; on a debounced change it re-syncs the index for the affected
            // scope and emits `cl:changed` so the UI refetches the now-current
            // index. Best-effort — a failure here just leaves CL views on their
            // existing poll. (Inside the rt guard above, so its tokio::spawn works.)
            let app_handle_for_fs = app.handle().clone();
            if let Err(e) = tauri_events::spawn_fs_watcher(
                paths_for_fs,
                bridge_for_fs,
                move |name: &str, payload: Value| {
                    if let Err(e) = app_handle_for_fs.emit(name, &payload) {
                        tracing::warn!(?e, event = name, "emit fs event failed");
                    }
                },
            ) {
                tracing::warn!(?e, "fs watcher failed to start; CL/EOD views fall back to polling");
            }
            // SessionCloseRequest handler. The agent-facing `close_session`
            // MCP tool only broadcasts a SignalingEvent::SessionCloseRequest;
            // nothing consumed it before (bridge_subscriber deliberately skips
            // it, and the comment claiming "AppState handles it" was false), so
            // close_session was a silent no-op — the subprocess kept running
            // and the row never got `closed_at`. This task is that missing
            // consumer: it routes the event to core.close_session, which kills
            // the subprocesses, marks the row closed/archived, and wipes the
            // session's permission grants.
            // Control-event consumer for the agent-facing `close_session` /
            // `advance_phase` MCP tools (they only broadcast a SignalingEvent;
            // bridge_subscriber deliberately skips SessionCloseRequest and only
            // emits the frontend chip for AgentAdvancePhase, so without this the
            // backend close/advance never happens). The slow work (close kills
            // subprocesses) runs on a SEPARATE serial worker fed by an unbounded
            // queue — the broadcast recv loop only matches + hands off, so it
            // never blocks. A blocking handler used to let a MessagePersisted
            // flood lag the shared channel and silently DROP a close/advance.
            let core_for_worker = Arc::clone(&core_for_setup);
            let mut close_rx = core_for_setup.subscribe_signaling();
            let (ctrl_tx, mut ctrl_rx) =
                tokio::sync::mpsc::unbounded_channel::<bot_hq::signaling::SignalingEvent>();
            tokio::spawn(async move {
                use bot_hq::signaling::SignalingEvent;
                while let Some(ev) = ctrl_rx.recv().await {
                    match ev {
                        SignalingEvent::SessionCloseRequest { session_id, archive, .. } => {
                            if let Err(e) =
                                core_for_worker.close_session(&session_id, archive).await
                            {
                                tracing::warn!(?e, %session_id, "close_session via MCP event failed");
                            }
                        }
                        SignalingEvent::AgentAdvancePhase { session_id, target, .. } => {
                            match bot_hq::core::ipav::IpavPhase::parse(&target) {
                                Some(phase) => {
                                    if let Err(e) =
                                        core_for_worker.advance_phase(&session_id, phase).await
                                    {
                                        tracing::warn!(?e, %session_id, %target, "advance_phase via MCP event failed");
                                    }
                                }
                                None => {
                                    tracing::warn!(%target, "advance_phase via MCP event: unparseable target");
                                }
                            }
                        }
                        _ => {}
                    }
                }
            });
            tokio::spawn(async move {
                use bot_hq::signaling::SignalingEvent;
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match close_rx.recv().await {
                        Ok(
                            ev @ (SignalingEvent::SessionCloseRequest { .. }
                            | SignalingEvent::AgentAdvancePhase { .. }),
                        ) => {
                            // Unbounded hand-off → never blocks the broadcast drain.
                            let _ = ctrl_tx.send(ev);
                        }
                        Ok(_) => {}
                        Err(RecvError::Lagged(n)) => {
                            tracing::warn!(skipped = n, "control subscriber lagged");
                        }
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            // Plugin heartbeat sweep loop. Ticks every PING_INTERVAL and
            // emits `plugin:crashed` for any iframe that crossed the
            // miss-limit this tick. The frontend tears down the iframe in
            // response. Skip mode on missed ticks: a backed-up runtime
            // shouldn't double-sweep and double-emit crash events.
            let app_handle_for_plugins = app.handle().clone();
            let heartbeat_for_sweep = Arc::clone(&registry_for_setup.heartbeat);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Heartbeat::ping_interval());
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let crashed = heartbeat_for_sweep.sweep();
                    for plugin_id in crashed {
                        if let Err(e) = app_handle_for_plugins.emit(
                            tauri_events::types::PLUGIN_CRASHED,
                            serde_json::json!({ "plugin_id": plugin_id }),
                        ) {
                            tracing::warn!(?e, plugin_id = %plugin_id, "emit plugin:crashed failed");
                        }
                    }
                }
            });
            tracing::info!("Tauri setup complete; webview launching");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    // After Tauri returns (window closed), drop everything in order.
    drop(core);
    drop(runtime);
    Ok(())
}

fn init_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,bot_hq=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// `bot-hq policy-check <subcommand>` — used by git hooks installed in
/// working repos. Exits with the appropriate status code (0 = clean, 1 = block).
fn run_policy_check_cli(args: &[String]) -> Result<()> {
    use std::process::ExitCode;
    let exit_code = hooks::run_cli(args).unwrap_or_else(|e| {
        eprintln!("bot-hq policy-check: {e}");
        // Soft-fail: don't break the user's git workflow on internal errors.
        0
    });
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    let _ = ExitCode::SUCCESS;
    Ok(())
}

/// `bot-hq install-hooks --repo <P> --data-dir <D> [--project <Q>]` — manual
/// install path for CI / dev tooling. Normal session-spawn installs
/// automatically.
fn run_install_hooks_cli(args: &[String]) -> Result<()> {
    let mut repo: Option<std::path::PathBuf> = None;
    let mut data_dir: Option<std::path::PathBuf> = None;
    let mut project: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" => {
                repo = Some(std::path::PathBuf::from(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("--repo needs value"))?,
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
                project = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("--project needs value"))?
                        .clone(),
                );
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

async fn cl_startup_init(
    storage: &Storage,
    bridge: &Arc<SignalingBridge>,
    data_dir: &std::path::Path,
) -> Result<()> {
    let projects_dir = Paths::for_data_dir(data_dir.to_path_buf()).cl_projects_dir();
    if projects_dir.is_dir() {
        for entry in std::fs::read_dir(&projects_dir)?.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if !n.starts_with('.') => n,
                _ => continue,
            };
            storage.upsert_project(name, name, None, None, None).await?;
        }
    }

    let projects = storage.list_projects().await?;
    for p in projects {
        if let Err(e) = bridge.cl_rescan(&p.name).await {
            tracing::warn!(?e, project = %p.name, "cl_rescan failed");
        }
    }
    Ok(())
}

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
