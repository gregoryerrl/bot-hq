use anyhow::Result;
use bot_hq::core::AppState as CoreAppState;
use bot_hq::paths::{LockGuard, Paths};
use bot_hq::plugins::Heartbeat;
use bot_hq::plugins::PluginRegistry;
use bot_hq::policy::{hooks, ViolationsLog};
use bot_hq::signaling::{start_signaling_server, SignalingBridge};
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

    // Load .env if present (best-effort; ignored if missing). Runs BEFORE
    // logging is initialised so a `RUST_LOG` set there reaches `EnvFilter`.
    if let Ok(env_path) = std::env::current_dir().map(|p| p.join(".env")) {
        let _ = load_env_file(&env_path);
    }

    let paths = Paths::from_env()?;
    let init_outcome = paths.init()?;
    // Logging comes up only now: the file sink needs `<data_dir>/.local/logs/`
    // to exist. Anything that fails above propagates via `?` and is printed by
    // main, so nothing diagnostic is lost by the wait. `_log_guard` must live
    // until the process exits — see `init_logging`.
    let _log_guard = init_logging(&paths);
    tracing::info!("bot-hq starting");
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
            // Boot orphan sweep (2026-08-15): a restart over a mid-turn session
            // kills the turn without a stop — the box reopens bannerless and
            // the watchdog needs its whole grace to notice. Sessions whose
            // last recorded state was busy/cancelling get the restart halt so
            // they land inside the every-stop-is-a-HALT model immediately.
            match storage.halt_orphaned_busy_sessions().await {
                Ok(n) if n > 0 => {
                    tracing::info!(halted = n, "restart-orphaned sessions wear the halt banner")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "boot orphan sweep failed"),
            }
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
            // GC: drop resolved tray rows older than GC_RETENTION_DAYS so session_tray
            // stays bounded — resolved rows are never read again (the in-chat
            // tray + counters only surface pending).
            match storage.purge_resolved_tray(GC_RETENTION_DAYS).await {
                Ok(n) if n > 0 => {
                    tracing::info!(purged = n, "purged old resolved tray rows at startup")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "startup tray GC failed"),
            }
            // Same GC posture for the activity timeline: small per session, but
            // unbounded over time without this.
            match storage.purge_activity_events(GC_RETENTION_DAYS).await {
                Ok(n) if n > 0 => {
                    tracing::info!(purged = n, "purged old activity events at startup")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "startup activity-event GC failed"),
            }
            // And for the five other append-only telemetry tables (round 10):
            // deliveries grow N× `messages`, the rest once per turn / tool call
            // / Stop; none is read past this horizon. `messages` is not swept —
            // its retention is the user's parked decision.
            for (what, purged) in [
                (
                    "participant deliveries",
                    storage.purge_participant_deliveries(GC_RETENTION_DAYS).await,
                ),
                ("context readings", storage.purge_context_readings(GC_RETENTION_DAYS).await),
                ("retrieval events", storage.purge_retrieval_events(GC_RETENTION_DAYS).await),
                ("cancel events", storage.purge_cancel_events(GC_RETENTION_DAYS).await),
                ("CL reads", storage.purge_cl_reads(GC_RETENTION_DAYS).await),
            ] {
                match purged {
                    Ok(n) if n > 0 => tracing::info!(purged = n, what, "startup GC"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(?e, what, "startup GC failed"),
                }
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

            // Local normalizing proxy for participants whose model row has a
            // custom `base_url` (a non-first-party, Anthropic-compatible
            // gateway): strips request-build-time `role:"system"`
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
    // + enabled-cache state. Constructed eagerly so we can pass it to
    // Tauri's `.manage()` AND share it with the `bhq-plugin://` scheme
    // handler and the setup-time sweep loop.
    let registry = Arc::new(PluginRegistry::new(paths.data_dir.clone()));

    // Seed the enabled-plugin cache + re-register enabled plugins with the
    // heartbeat. Both otherwise only happen on install/enable, so a restart
    // would leave the `bhq-plugin://` handler refusing every plugin and the
    // sweep loop watching nothing.
    runtime.block_on(async {
        match storage_arc.list_plugins().await {
            Ok(rows) => {
                // Consent-frozen per-plugin caches, seeded for ALL installed
                // rows (enable/disable never re-reads the DB): serve root
                // (linked installs point at the user's source dir), granted
                // capabilities (parsed from the STORED manifest — the disk
                // manifest is never the grant authority), and CSP headers.
                for row in &rows {
                    let root = if row.linked {
                        std::path::PathBuf::from(&row.dir_path)
                    } else {
                        registry.plugin_dir(&row.id)
                    };
                    registry.set_serve_root(&row.id, Some(root));
                    match bot_hq::plugins::PluginManifest::parse(&row.manifest_json) {
                        Ok(m) => registry
                            .set_granted_caps(&row.id, Some(m.requested_capabilities)),
                        Err(e) => tracing::warn!(
                            ?e,
                            plugin_id = %row.id,
                            "unparseable stored manifest; plugin gets no capability grants"
                        ),
                    }
                    if let Some(csp_json) = &row.csp_json {
                        match serde_json::from_str::<bot_hq::plugins::CspExtraOrigins>(csp_json) {
                            Ok(extra) => registry.set_csp_header(
                                &row.id,
                                Some(bot_hq::plugins::serve::build_plugin_csp(Some(&extra))),
                            ),
                            Err(e) => tracing::warn!(
                                ?e,
                                plugin_id = %row.id,
                                "invalid csp_json grant; serving default CSP"
                            ),
                        }
                    }
                }
                let enabled: std::collections::HashSet<String> = rows
                    .into_iter()
                    .filter(|r| r.enabled)
                    .map(|r| r.id)
                    .collect();
                for id in &enabled {
                    registry.heartbeat.register(id);
                }
                registry.set_enabled_ids(enabled);
            }
            Err(e) => tracing::warn!(?e, "plugin enabled-cache boot seed failed"),
        }
    });

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
        // Dialog plugin — native folder picker for the New-project / working-repo
        // path fields (replaces blind text-entry of paths).
        .plugin(tauri_plugin_dialog::init())
        // Plugin-bundle serving: `bhq-plugin://<id>/<path>` resolves to
        // `<data_dir>/plugins/<id>/<path>` for INSTALLED + ENABLED plugins
        // only. Registered once at Builder time — install/enable needs no
        // app restart because the handler re-reads the registry's enabled
        // cache per request. Resolution + traversal guards live in
        // `plugins::serve` (pure, unit-tested); this closure is http glue.
        .register_uri_scheme_protocol("bhq-plugin", {
            let registry = Arc::clone(&registry);
            move |_ctx, request| {
                use bot_hq::plugins::serve::{self, ServeError};
                let uri = request.uri();
                let outcome = serve::parse_plugin_request(uri.host(), uri.path())
                    .and_then(|(id, rel)| {
                        // Root comes from the registry's serve-root cache —
                        // normal installs resolve inside data_dir, linked
                        // installs at the user's source dir, and the guards
                        // in resolve_with_root treat either as the boundary.
                        serve::resolve_with_root(
                            registry.serve_root_for(id).as_deref(),
                            registry.is_enabled(id),
                            id,
                            rel,
                        )
                        .map(|resolved| (id.to_string(), resolved))
                    })
                    .and_then(|(id, (path, mime))| {
                        std::fs::read(&path)
                            .map(|body| (id, body, mime))
                            .map_err(|_| ServeError::NotFound)
                    });
                match outcome {
                    Ok((plugin_id, body, mime)) => tauri::http::Response::builder()
                        .status(200)
                        .header("Content-Type", mime)
                        // The consent-frozen per-plugin header, or the
                        // strict default for plugins without a grant.
                        .header(
                            "Content-Security-Policy",
                            registry
                                .csp_header_for(&plugin_id)
                                .unwrap_or_else(|| serve::PLUGIN_CSP.to_string()),
                        )
                        .header("Cache-Control", "no-store")
                        .body(body)
                        .unwrap_or_else(|_| plugin_asset_error(500)),
                    Err(err) => {
                        tracing::debug!(?err, uri = %uri, "bhq-plugin asset refused");
                        plugin_asset_error(match err {
                            ServeError::Disabled => 403,
                            ServeError::BadRequest => 400,
                            _ => 404,
                        })
                    }
                }
            }
        })
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
            // CoreAppState's copy serves the app layer (`session:created`,
            // the close path); the bridge copy is for the MCP tools (per-agent
            // jsonrpc.rs), which don't see CoreAppState. Set-once — ignore the
            // Err on duplicate.
            let handle = app.handle().clone();
            let _ = core_for_setup.app_handle.set(handle.clone());
            bridge_for_subscriber.set_app_handle(handle);
            // Share the per-session PTY registry with the bridge so the
            // terminal_exec / terminal_read MCP tools reach the same
            // terminals the Terminal subtab renders.
            bridge_for_subscriber.set_terminal_registry(Arc::clone(&core_for_setup.terminals));
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
            // Filesystem watcher → CL freshness. Watches the Context Library
            // dir; on a debounced change it re-syncs the index for the affected
            // scope and emits `cl:changed` so the UI refetches the now-current
            // index. Best-effort — a failure here just leaves CL views on their
            // existing poll. (Inside the rt guard above, so its tokio::spawn works.)
            let app_handle_for_fs = app.handle().clone();
            match tauri_events::spawn_fs_watcher(
                paths_for_fs,
                bridge_for_fs,
                move |name: &str, payload: Value| {
                    if let Err(e) = app_handle_for_fs.emit(name, &payload) {
                        tracing::warn!(?e, event = name, "emit fs event failed");
                    }
                },
            ) {
                // Stash the handle so the session spawn/close paths can register
                // working repos for live A-tab diffs (Phase 3).
                Ok(handle) => {
                    // Seed watches for already-enabled plugins' served dirs so
                    // `plugin:assets_changed` fires from boot (install/enable
                    // register later ones through the lifecycle commands).
                    for id in registry_for_setup.enabled_ids() {
                        if let Some(root) = registry_for_setup.serve_root_for(&id) {
                            handle.add_plugin_dir(&id, root);
                        }
                    }
                    let _ = core_for_setup.fs_watcher.set(handle);
                }
                Err(e) => {
                    tracing::warn!(?e, "fs watcher failed to start; CL + A-tab fall back to polling");
                }
            }
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
                                core_for_worker
                                    .close_session(
                                        &session_id,
                                        archive,
                                        bot_hq::core::close_learnings::ClosePath::Agent,
                                    )
                                    .await
                            {
                                tracing::warn!(?e, %session_id, "close_session via MCP event failed");
                            }
                        }
                        SignalingEvent::HaltAcked { session_id, agent } => {
                            // rc3 D35: the declarer said it is waiting — stop
                            // its residual generation so the halt is a halt.
                            // Keyed on the halt tool's RESULT reaching the
                            // declarer's own stream (round 8, A1b), not on the
                            // AwaitingUser state change: fired from the state
                            // change, the interrupt raced the tool ack and the
                            // agent's transcript showed its own halt as
                            // rejected. By the time the result is in the
                            // stream there is nothing left to race.
                            core_for_worker.halt_declared(&session_id, &agent).await;
                        }
                        SignalingEvent::StagedDeliveryDue { session_id } => {
                            // The ring reached a boundary with a stage
                            // pending: deliver it through the one send path.
                            core_for_worker.deliver_staged(&session_id).await;
                        }
                        SignalingEvent::AgentAdvancePhase { session_id, target, .. } => {
                            match bot_hq::core::ipav::IpavPhase::parse(&target) {
                                Some(phase) => {
                                    if let Err(e) =
                                        core_for_worker
                                            .advance_phase(
                                                &session_id,
                                                phase,
                                                bot_hq::core::state::PhaseAdvanceSource::Agent,
                                            )
                                            .await
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
                            | SignalingEvent::AgentAdvancePhase { .. }
                            | SignalingEvent::HaltAcked { .. }
                            | SignalingEvent::StagedDeliveryDue { .. }),
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

/// Bare status-only response for refused / failed `bhq-plugin://` asset
/// requests. No body — nothing plugin-controlled is reflected back.
fn plugin_asset_error(status: u16) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .body(Vec::new())
        .expect("static status-only response")
}

/// Boot-time GC horizon for the append-only runtime tables — `session_tray`
/// resolved rows, `activity_events`, and (round 10) `participant_deliveries`,
/// `context_readings`, `retrieval_events`, `cancel_events`, `cl_reads`
/// (`storage/gc.rs`): rows older than this many days are purged at startup.
/// One number, spelled once (round 9).
const GC_RETENTION_DAYS: i64 = 90;

/// How many daily log files to keep. Bounded from the start: this data home
/// used to carry one append-only, unrotated sink (`native-accounting.jsonl`,
/// written by the loop rc3 D9 deleted) and a second unbounded one was not
/// worth the diagnostics — a rule that outlived its example.
const LOG_FILES_KEPT: usize = 14;

/// Install the tracing subscriber: stdout (unchanged) PLUS a rolling daily file
/// under `<data_dir>/.local/logs/`.
///
/// There was no file sink at all before this. `tracing_subscriber::fmt()` writes
/// to stdout, and a `.app` launched from Finder has no terminal attached — so
/// every `warn!` the host emitted was discarded. That is not a small gap: two
/// migrations exist purely to work around it. `0040_cancel_events` records what
/// three `info!`/`warn!` lines already said, because "21 Stops across 13 sessions
/// left zero forensic trace"; `0041_forward_events` does the same for dropped
/// peer-forwards, whose early-returns were "a bare `debug!`".
///
/// **Returns a guard that must stay alive for the process's lifetime.** The
/// non-blocking writer flushes on a worker thread; dropping the guard stops it,
/// which silently discards buffered lines — the standard way this gets shipped
/// looking fine and logging nothing.
///
/// Called AFTER `Paths::init` (the directory has to exist), which also means a
/// `RUST_LOG` set in `.env` now takes effect — the `.env` load used to happen
/// after the filter was already built.
fn init_logging(paths: &Paths) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,bot_hq=debug"));
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("bot-hq")
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(&paths.logs_dir)
        .expect("building the rolling log appender");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                // No colour codes in a file, and absolute timestamps so a line
                // can be lined up against a message row or a git commit.
                .with_ansi(false)
                .with_writer(file_writer),
        )
        .init();
    guard
}

/// `bot-hq policy-check <subcommand>` — used by git hooks installed in
/// working repos. Exits with the appropriate status code (0 = clean, 1 = block).
fn run_policy_check_cli(args: &[String]) -> Result<()> {
    let exit_code = hooks::run_cli(args).unwrap_or_else(|e| {
        eprintln!("bot-hq policy-check: {e}");
        // Soft-fail: don't break the user's git workflow on internal errors.
        0
    });
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
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

#[cfg(test)]
mod tests {
    use super::LOG_FILES_KEPT;

    /// The rolling-appender CONFIG is ours even though the machinery isn't: a
    /// wrong prefix/suffix or a builder error would leave the app logging to
    /// nowhere exactly as it did before the sink existed, and no other test
    /// would notice — every CLI subcommand returns before `init_logging`, and a
    /// full launch needs the single-instance lock.
    #[test]
    fn the_rolling_appender_actually_writes_a_file() {
        use std::io::Write;
        let tmp = tempfile::TempDir::new().unwrap();
        let appender = tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("bot-hq")
            .filename_suffix("log")
            .max_log_files(LOG_FILES_KEPT)
            .build(tmp.path())
            .expect("appender builds");
        let (writer, guard) = tracing_appender::non_blocking(appender);
        {
            let mut w = writer;
            writeln!(w, "hello from the sink").unwrap();
        }
        // Dropping the guard flushes the worker thread — the same reason main
        // must hold it for the process's lifetime.
        drop(guard);

        let written: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(written.len(), 1, "expected one log file, got {written:?}");
        assert!(
            written[0].starts_with("bot-hq") && written[0].ends_with("log"),
            "unexpected log filename: {}",
            written[0]
        );
        let body = std::fs::read_to_string(tmp.path().join(&written[0])).unwrap();
        assert!(body.contains("hello from the sink"), "log file was empty");
    }
}
