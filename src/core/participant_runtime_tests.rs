use super::*;
use crate::agents::{AgentEvent, OutgoingUserMessage};
use crate::core::{
    sequencer::{SequencerCommand, SequencerDeps},
    session::SessionAgent,
};
use crate::storage::{Model, Participant, Storage};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, oneshot};

struct Seat {
    input: mpsc::Receiver<OutgoingUserMessage>,
    events: mpsc::Sender<AgentEvent>,
    stopped: watch::Receiver<bool>,
}

fn fake_agent(session: &str, slug: &str) -> (AgentHandle, Seat) {
    let (events, rx) = mpsc::channel(64);
    let (input, inputs) = mpsc::channel(64);
    let (control, _controls) = mpsc::channel(8);
    let (kill, killed) = oneshot::channel();
    let (receipt, observer) = watch::channel(None);
    let (dead, stopped) = watch::channel(false);
    let end = events.clone();
    tokio::spawn(async move {
        let _ = killed.await;
        dead.send_replace(true);
        receipt.send_replace(Some(Ok(())));
        let _ = end.send(AgentEvent::Exited("fixture stopped".into())).await;
    });
    (
        AgentHandle::from_parts(slug.into(), session, rx, input, control, kill)
            .with_stop_observer(observer),
        Seat {
            input: inputs,
            events,
            stopped,
        },
    )
}

struct Spawned {
    config: AgentConfig,
    resume: Option<String>,
    prompt: String,
    seat: Seat,
}

fn services(fail: bool) -> (SpawnServices, mpsc::UnboundedReceiver<Spawned>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        SpawnServices {
            spawn: Arc::new(move |cfg| {
                let tx = tx.clone();
                Box::pin(async move {
                    if fail && cfg.config.model_name == "new" {
                        anyhow::bail!("fixture refused candidate");
                    }
                    let (handle, seat) = fake_agent(&cfg.session_id, &cfg.agent_name);
                    tx.send(Spawned {
                        prompt: std::fs::read_to_string(&cfg.system_prompt_path).unwrap(),
                        config: cfg.config,
                        resume: cfg.resume_session_id,
                        seat,
                    })
                    .unwrap();
                    Ok(handle)
                })
            }),
            auth: Arc::new(|_| {
                Box::pin(async {
                    crate::agents::spawn::AuthProbe::LoggedIn {
                        auth_method: Some("claude.ai".into()),
                        email: Some("new@example.test".into()),
                        org_id: Some("org".into()),
                        subscription_type: Some("max".into()),
                    }
                })
            }),
            codex: Arc::new(|_| {
                Box::pin(async {
                    crate::agents::codex::probe::CodexProbe {
                        auth: crate::agents::spawn::AuthProbe::LoggedIn {
                            auth_method: Some("chatgpt".into()),
                            email: Some("codex@example.test".into()),
                            org_id: Some("codex-org".into()),
                            subscription_type: Some("pro".into()),
                        },
                        hooks: vec![],
                        version: "test-version".into(),
                        models: vec![],
                    }
                })
            }),
        },
        rx,
    )
}

#[tokio::test]
async fn startup_snapshot_stops_at_its_frontier_and_later_rows_arrive_once() {
    for adding in [false, true] {
        let f = Fixture::new(false).await;
        let old = f
            .state
            .storage
            .post_to_channel("s1", "user", None, "text", "HISTORICAL-DIRECTION", None)
            .await
            .unwrap()
            .message_id();
        f.state
            .storage
            .record_user_words("s1", old, "HISTORICAL-DIRECTION")
            .await
            .unwrap();
        if !adding {
            f.state
                .storage
                .commit_delivery(f.rows[0].id, &[(old, None)])
                .await
                .unwrap();
            let unread = f
                .state
                .storage
                .post_to_channel("s1", "user", None, "text", "UNREAD-BEFORE-SWITCH", None)
                .await
                .unwrap()
                .message_id();
            f.state
                .storage
                .record_user_words("s1", unread, "UNREAD-BEFORE-SWITCH")
                .await
                .unwrap();
        }
        let (mut services, mut spawned) = services(false);
        let normal_auth = services.auth.clone();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (e, r) = (entered.clone(), release.clone());
        services.auth = Arc::new(move |dir| {
            let (e, r, probe) = (e.clone(), r.clone(), normal_auth.clone());
            Box::pin(async move {
                e.notify_one();
                r.notified().await;
                probe(dir).await
            })
        });
        let request = if adding {
            ParticipantChangeRequest::Add {
                role_id: f.rows[0].role_id.unwrap(),
                model_id: "new".into(),
                label: None,
            }
        } else {
            f.switch()
        };
        f.start("snapshot", request, services).await;
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        let target = if adding {
            f.state
                .storage
                .participants_for_session("s1")
                .await
                .unwrap()
                .into_iter()
                .max_by_key(|p| p.id)
                .unwrap()
                .id
        } else {
            f.rows[0].id
        };
        assert_eq!(f.state.storage.cursor_for(target).await.unwrap(), old);
        let old_receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM participant_deliveries WHERE participant_id = ? AND message_id = ?")
            .bind(target).bind(old).fetch_one(f.state.storage.pool()).await.unwrap();
        assert_eq!(
            old_receipts,
            if adding { 0 } else { 1 },
            "joining never manufactures delivery receipts"
        );
        let late = f
            .state
            .storage
            .post_to_channel("s1", "user", None, "text", "DIRECTION-DURING-STARTUP", None)
            .await
            .unwrap()
            .message_id();
        f.state
            .storage
            .record_user_words("s1", late, "DIRECTION-DURING-STARTUP")
            .await
            .unwrap();
        f.ring
            .send(SequencerCommand::UserMessage {
                mentions: vec![target],
                restarts_rotation: true,
            })
            .await
            .unwrap();
        release.notify_one();
        assert_eq!(f.done("snapshot").await.status, "applied");
        let mut fresh = spawned.recv().await.unwrap();
        assert!(fresh.prompt.contains("HISTORICAL-DIRECTION"));
        assert!(!fresh.prompt.contains("DIRECTION-DURING-STARTUP"));
        assert!(
            !fresh.prompt.contains("UNREAD-BEFORE-SWITCH"),
            "the snapshot must use the captured delivered cursor, not the newest channel row"
        );
        let wire = tokio::time::timeout(Duration::from_secs(2), fresh.seat.input.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(wire.message.content.contains("DIRECTION-DURING-STARTUP"));
        if !adding {
            assert!(wire.message.content.contains("UNREAD-BEFORE-SWITCH"));
        }
        assert!(
            !wire.message.content.contains("HISTORICAL-DIRECTION"),
            "old history is in the snapshot, not replayed as fresh input"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), fresh.seat.input.recv())
                .await
                .is_err()
        );
        f.end().await;
    }
}

struct Fixture {
    state: Arc<AppState>,
    rows: Vec<Participant>,
    seats: Vec<Seat>,
    ring: mpsc::Sender<SequencerCommand>,
    _temp: tempfile::TempDir,
}

impl Fixture {
    async fn new(codex: bool) -> Self {
        Self::with_database(codex, false).await
    }

    async fn with_database(codex: bool, disk: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let storage = if disk {
            let s = Storage::open(&temp.path().join("fixture.db"))
                .await
                .unwrap();
            s.install_role_preset().await.unwrap();
            sqlx::query("UPDATE roles SET archived = 1 WHERE slug = 'agent'")
                .execute(s.pool())
                .await
                .unwrap();
            s
        } else {
            Storage::memory().await.unwrap()
        };
        let storage = storage.with_handoff_dir(temp.path().join("data/.local/handoffs"));
        for id in ["old", "new"] {
            storage
                .upsert_model(&Model {
                    id: id.into(),
                    display_name: id.into(),
                    provider: "fixture".into(),
                    model_name: id.into(),
                    base_url: None,
                    auth_token: None,
                    created_at: String::new(),
                    updated_at: String::new(),
                    context_window: Some(100_000),
                    cli_settings: None,
                    claude_config_dir: Some(temp.path().join(id).to_string_lossy().into_owned()),
                    runtime: if codex && id == "new" {
                        "codex"
                    } else {
                        "claude_code"
                    }
                    .into(),
                })
                .await
                .unwrap();
        }
        storage
            .create_session(
                "s1",
                "runtime fixture",
                Some(&temp.path().to_string_lossy()),
            )
            .await
            .unwrap();
        storage.ensure_session_roster("s1", 2).await.unwrap();
        sqlx::query("UPDATE session_participants SET model_id = 'old', claude_session_id = 'thread-' || slug WHERE session_id = 's1'")
            .execute(storage.pool()).await.unwrap();
        let rows = storage.participants_for_session("s1").await.unwrap();
        let data_dir = temp.path().join("data");
        crate::policy::tool_gate::save(
            &data_dir,
            &[crate::policy::tool_gate::GatedKeyword {
                keyword: "gh".into(),
                mode: crate::policy::tool_gate::GateMode::AutoAllow,
            }],
        )
        .unwrap();
        let bridge = crate::signaling::SignalingBridge::with_policy(
            crate::policy::ViolationsLog::new(&data_dir),
            data_dir,
        );
        bridge.set_storage(storage.clone()).await;
        let server = crate::signaling::start_signaling_server(bridge.clone())
            .await
            .unwrap();
        let state = Arc::new(
            AppState::new(
                crate::paths::Paths::for_data_dir(temp.path().join("data")),
                storage.clone(),
                server,
            )
            .await,
        );
        let (mut handle, _) = crate::core::session::stub_session_for_tests("s1", &bridge).await;
        handle.participants.clear();
        let mut inputs = std::collections::HashMap::new();
        let mut epochs = std::collections::HashMap::new();
        let mut seats = vec![];
        for row in &rows {
            let (agent, seat) = fake_agent("s1", &row.slug);
            let config = crate::core::session::frozen_model_config(
                &row.slug,
                &storage.get_model("old").await.unwrap().unwrap(),
            );
            let epoch = Arc::new(AtomicU64::new(0));
            inputs.insert(row.id, agent.input().clone());
            epochs.insert(row.id, epoch.clone());
            let token = format!("token-{}", row.slug);
            bridge.register_mcp_token("s1", &row.slug, &token);
            handle.participants.push(SessionAgent {
                participant_id: Some(row.id),
                slug: row.slug.clone(),
                turn_position: row.turn_position,
                capabilities: crate::agents::ResolvedCapabilities::Known(
                    crate::agents::CapabilitySet::from_json(&row.capabilities).unwrap(),
                ),
                config_snapshot: Some(config),
                _spawn_temp: None,
                mcp_token: Some(token),
                system_prompt_path: temp.path().join("old-prompt"),
                handle: agent,
                turn_epoch: Some(epoch),
            });
            seats.push(seat);
        }
        refresh_runtime_roster(&mut handle, &storage, &bridge, &rows);
        let (ring, rx) = mpsc::channel(64);
        bridge
            .register_session_sequencer("s1".into(), ring.clone())
            .await;
        let deps = SequencerDeps {
            in_atomic_tool: handle.in_atomic_tool.clone(),
            session_id: Arc::from("s1"),
            storage: storage.clone(),
            inputs,
            epochs,
            data_dir: None,
            bridge: Some(bridge.clone()),
            activity: Some(handle.activity.clone()),
            boots: false,
            orienting: Some(handle.orienting.clone()),
        };
        tokio::spawn(crate::core::sequencer::run_sequencer(deps, rx));
        state.sessions.lock().await.insert("s1".into(), handle);
        Self {
            state,
            rows,
            seats,
            ring,
            _temp: temp,
        }
    }
    async fn start(
        &self,
        id: &str,
        request: ParticipantChangeRequest,
        services: SpawnServices,
    ) -> ParticipantChange {
        let revision = self.state.storage.roster_revision("s1").await.unwrap();
        self.state
            .start_participant_change("s1", id, revision, request, services)
            .await
            .unwrap()
    }
    async fn done(&self, id: &str) -> ParticipantChange {
        let run = self
            .state
            .sessions
            .lock()
            .await
            .get("s1")
            .unwrap()
            .change_run
            .clone()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), run.wait_done())
            .await
            .expect("worker must finish")
            .unwrap();
        self.state
            .storage
            .participant_change("s1", id)
            .await
            .unwrap()
            .unwrap()
    }
    fn switch(&self) -> ParticipantChangeRequest {
        ParticipantChangeRequest::SwitchModel {
            participant_id: self.rows[0].id,
            model_id: "new".into(),
        }
    }
    async fn end(mut self) {
        let mut handle = self.state.sessions.lock().await.remove("s1").unwrap();
        self.state
            .quiesce_session_handle(&mut handle)
            .await
            .unwrap();
        self.seats.clear();
    }
}

#[tokio::test]
async fn blocked_publication_does_not_hold_other_sessions_controls() {
    let f = Fixture::with_database(false, true).await;
    let (other, _input) =
        crate::core::session::stub_session_for_tests("other", &f.state.bridge).await;
    f.state.sessions.lock().await.insert("other".into(), other);
    let (mut services, mut spawned) = services(false);
    let normal = services.spawn.clone();
    let ready = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (r, go) = (ready.clone(), release.clone());
    services.spawn = Arc::new(move |config| {
        let normal = normal.clone();
        let r = r.clone();
        let go = go.clone();
        Box::pin(async move {
            let result = normal(config).await;
            r.notify_one();
            go.notified().await;
            result
        })
    });
    f.start("switch", f.switch(), services).await;
    tokio::time::timeout(Duration::from_secs(2), ready.notified())
        .await
        .unwrap();
    // WAL allows the worker's ownership/roster reads but blocks its publication
    // writer. This is real SQLite contention, not a sleep in a mocked setter.
    let transaction = f
        .state
        .storage
        .pool()
        .begin_with("BEGIN IMMEDIATE")
        .await
        .unwrap();
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let pool = f.state.storage.pool();
            if pool.size() as usize - pool.num_idle() >= 2 {
                tokio::time::sleep(Duration::from_millis(20)).await;
                if pool.size() as usize - pool.num_idle() >= 2 {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("worker must reach the blocked writer");
    let response =
        tokio::time::timeout(Duration::from_millis(200), f.state.current_phase("other")).await;
    // Release before asserting so failure still permits owned cleanup.
    transaction.rollback().await.unwrap();
    assert!(
        response.is_ok(),
        "another session's control must not wait for this publication transaction"
    );
    assert_eq!(f.done("switch").await.status, "applied");
    let _candidate = spawned.recv().await.unwrap();
    f.end().await;
}

#[tokio::test]
async fn a_closing_owner_can_cancel_a_worker_waiting_for_its_lifecycle_barrier() {
    let f = Fixture::new(false).await;
    let held = f.state.lifecycle_lock("s1").lock_owned().await;
    let (run, _completion) = ChangeRun::new("closing".into());
    let state = f.state.clone();
    let waiting_run = run.clone();
    let mut waiting =
        tokio::spawn(async move { state.publication_guard("s1", &waiting_run).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut waiting)
            .await
            .is_err()
    );
    run.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .expect("the closer must not deadlock while joining its worker")
        .unwrap();
    assert!(result.is_err());
    drop(held);
    f.end().await;
}

#[tokio::test]
async fn worker_switches_both_connectors_without_restarting_peers_and_receives_next_turn() {
    for codex in [false, true] {
        let f = Fixture::new(codex).await;
        let (services, mut spawned) = services(false);
        let mut changes = f.state.bridge.subscribe();
        let peer_handoff = crate::agents::handoff::doc_slug(&f.rows[1].slug);
        f.state.storage.upsert_session_document("s1", &peer_handoff, "Keep this handoff", None).await.unwrap();
        let before_handoff = f.state.storage.session_document_by_slug("s1", &peer_handoff).await.unwrap().unwrap();
        f.state.storage.write_handoff_due("s1", &f.rows[1].slug, "Still owed");
        let original_peer = f.state.sessions.lock().await["s1"].participants[1]
            .handle
            .incarnation()
            .id()
            .to_string();
        f.start("switch", f.switch(), services).await;
        assert_eq!(f.done("switch").await.status, "applied");
        let mut changed = vec![];
        while let Ok(event) = changes.try_recv() {
            if let crate::signaling::SignalingEvent::ParticipantChanged { session_id, agent } = event {
                assert_eq!(session_id, "s1"); changed.push(agent);
            }
        }
        assert_eq!(changed, vec![f.rows[0].slug.clone()]);
        let controls = crate::tauri_cmd::participant_changes::controls(&f.state.storage, "s1").await.unwrap();
        assert_eq!(controls.latest_change.unwrap().status, "applied");
        assert_eq!(controls.selections.iter().find(|s| s.participant_id == f.rows[0].id).unwrap().model_id.as_deref(), Some("new"));
        let dir = f._temp.path().join("data/.local/handoffs");
        let mirror = std::fs::read_to_string(crate::agents::handoff::file_path(&dir, "s1", &f.rows[1].slug).unwrap()).unwrap();
        assert!(mirror.contains("\"model\":\"new\""), "unchanged peers must see the new roster: {mirror}");
        assert!(mirror.contains("Keep this handoff"));
        assert_eq!(crate::agents::handoff::written_at(&mirror), Some(before_handoff.updated_at.as_str()));
        assert!(crate::agents::handoff::due_path(&dir, "s1", &f.rows[1].slug).unwrap().exists(), "refresh is not an authored handoff");
        assert!(mirror.len() <= crate::agents::handoff::FILE_BUDGET_BYTES);
        let mut fresh = spawned.recv().await.unwrap();
        assert_eq!(fresh.config.model_name, "new");
        assert_eq!(fresh.resume, None);
        assert!(*f.seats[0].stopped.borrow());
        assert!(!*f.seats[1].stopped.borrow());
        assert!(!f.state.bridge.mcp_token_matches(
            "s1",
            &f.rows[0].slug,
            Some(&format!("token-{}", f.rows[0].slug))
        ));
        let sessions = f.state.sessions.lock().await;
        let now = &sessions["s1"];
        assert_eq!(now.participants[1].handle.incarnation().id(), original_peer);
        assert_eq!(
            now.participants[0]
                .config_snapshot
                .as_ref()
                .unwrap()
                .runtime,
            if codex { "codex" } else { "claude_code" }
        );
        let epoch = now.participants[0].turn_epoch.as_ref().unwrap().clone();
        drop(sessions);
        f.state
            .storage
            .post_to_channel("s1", "user", None, "text", "continue after switch", None)
            .await
            .unwrap();
        f.ring
            .send(SequencerCommand::UserMessage {
                mentions: vec![],
                restarts_rotation: true,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), fresh.seat.input.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(epoch.load(Ordering::Acquire) > 0);
        fresh
            .seat
            .events
            .send(AgentEvent::Init {
                session_id: Some("fresh-thread".into()),
                api_key_source: None,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if f.state
                    .storage
                    .participant_by_id(f.rows[0].id)
                    .await
                    .unwrap()
                    .unwrap()
                    .claude_session_id
                    .as_deref()
                    == Some("fresh-thread")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            f.state
                .storage
                .participant_by_id(f.rows[1].id)
                .await
                .unwrap()
                .unwrap()
                .claude_session_id,
            f.rows[1].claude_session_id
        );
        f.end().await;
    }
}

#[tokio::test]
async fn worker_waits_for_the_live_turn_and_duplicate_clicks_share_one_spawn() {
    let mut f = Fixture::new(false).await;
    f.state
        .storage
        .post_to_channel("s1", "user", None, "text", "work", None)
        .await
        .unwrap();
    f.ring
        .send(SequencerCommand::UserMessage {
            mentions: vec![],
            restarts_rotation: true,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), f.seats[0].input.recv())
        .await
        .unwrap()
        .unwrap();
    let epoch = f.state.sessions.lock().await["s1"].participants[0]
        .turn_epoch
        .as_ref()
        .unwrap()
        .load(Ordering::Acquire);
    let (services, mut spawned) = services(false);
    let request = f.start("switch", f.switch(), services.clone()).await;
    f.state
        .start_participant_change(
            "s1",
            "switch",
            request.expected_revision,
            f.switch(),
            services,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!*f.seats[0].stopped.borrow());
    assert!(spawned.try_recv().is_err());
    f.ring
        .send(SequencerCommand::TurnComplete {
            participant_id: f.rows[0].id,
            epoch,
            ending: crate::core::sequencer::TurnEnding::Spoke,
        })
        .await
        .unwrap();
    assert_eq!(f.done("switch").await.status, "applied");
    assert!(spawned.try_recv().is_ok());
    assert!(spawned.try_recv().is_err());
    f.end().await;
}

#[tokio::test]
async fn failed_candidate_restores_only_its_old_runtime_and_saved_model() {
    let f = Fixture::new(false).await;
    let (services, mut spawned) = services(true);
    f.start("switch", f.switch(), services).await;
    assert_eq!(f.done("switch").await.status, "failed");
    let recovered = spawned.recv().await.unwrap();
    assert_eq!(recovered.config.model_name, "old");
    assert_eq!(recovered.resume, f.rows[0].claude_session_id);
    assert!(!*f.seats[1].stopped.borrow());
    let row = f
        .state
        .storage
        .participant_by_id(f.rows[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.model_id.as_deref(), Some("old"));
    let sessions = f.state.sessions.lock().await;
    let actor = &sessions["s1"].participants[0];
    assert!(f
        .state
        .bridge
        .mcp_token_matches("s1", &actor.slug, actor.mcp_token.as_deref()));
    assert!(actor.handle.incarnation().is_active());
    drop(sessions);
    f.end().await;
}

#[tokio::test]
async fn add_then_remove_updates_reviewers_without_stopping_peers() {
    let f = Fixture::new(false).await;
    let (services, mut spawned) = services(false);
    let role = f.state.storage.role_by_slug("eyes").await.unwrap().unwrap();
    f.start(
        "add",
        ParticipantChangeRequest::Add {
            role_id: role.id,
            model_id: "new".into(),
            label: None,
        },
        services.clone(),
    )
    .await;
    assert_eq!(f.done("add").await.status, "applied");
    let fresh = spawned.recv().await.unwrap();
    let rows = f
        .state
        .storage
        .participants_for_session("s1")
        .await
        .unwrap();
    let added = rows.iter().max_by_key(|p| p.id).unwrap();
    assert!(f.state.bridge.session_reviewers("s1").contains(&added.slug));
    f.start(
        "remove",
        ParticipantChangeRequest::Remove {
            participant_id: added.id,
        },
        services,
    )
    .await;
    assert_eq!(f.done("remove").await.status, "applied");
    assert!(*fresh.seat.stopped.borrow());
    assert!(!f.state.bridge.session_reviewers("s1").contains(&added.slug));
    assert!(
        !f.state
            .storage
            .participant_by_id(added.id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    assert!(!*f.seats[0].stopped.borrow());
    assert!(!*f.seats[1].stopped.borrow());
    f.end().await;
}

#[tokio::test]
async fn close_during_probe_waits_for_worker_and_never_publishes_its_late_candidate() {
    let f = Fixture::new(false).await;
    let body = f
        .state
        .storage
        .post_to_channel(
            "s1",
            "system",
            None,
            "system_notice",
            "pending publication",
            None,
        )
        .await
        .unwrap()
        .message_id();
    f.state
        .storage
        .insert_queued_gate_with_approval(
            "s1",
            "auto",
            &f.rows[0].slug,
            "fixture",
            "publish fixture",
            false,
        )
        .await
        .unwrap();
    f.state
        .storage
        .set_tray_body_row("auto", body)
        .await
        .unwrap();
    assert!(f
        .state
        .storage
        .mark_auto_gate_reviewed("auto", body)
        .await
        .unwrap());
    let (mut services, mut spawned) = services(false);
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (e, r) = (entered.clone(), release.clone());
    services.auth = Arc::new(move |_| {
        let (e, r) = (e.clone(), r.clone());
        Box::pin(async move {
            e.notify_one();
            r.notified().await;
            crate::agents::spawn::AuthProbe::Unreadable("fixture".into())
        })
    });
    f.start("switch", f.switch(), services).await;
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    assert!(
        !f.state
            .storage
            .claim_auto_gate_run("auto", body)
            .await
            .unwrap(),
        "a roster in preparation cannot authorize a run"
    );
    f.state
        .storage
        .invalidate_auto_gate_review("auto", body)
        .await
        .unwrap();
    assert!(
        !f.state
            .storage
            .mark_auto_gate_reviewed("auto", body)
            .await
            .unwrap(),
        "a late review cannot authorize tentative membership"
    );
    // Exercise the actual close path, including removal from the runtime map.
    let state = f.state.clone();
    let closing = tokio::spawn(async move {
        state
            .close_session("s1", false, crate::core::close_learnings::ClosePath::Agent)
            .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    if closing.is_finished() {
        panic!(
            "cleanup ended before the late worker: {:?}",
            closing.await.unwrap()
        );
    }
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(3), closing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(spawned.try_recv().is_err());
    assert!(f
        .state
        .storage
        .get_session("s1")
        .await
        .unwrap()
        .unwrap()
        .closed_at
        .is_some());
    assert_eq!(
        f.state
            .storage
            .participant_change("s1", "switch")
            .await
            .unwrap()
            .unwrap()
            .status,
        "failed"
    );
    assert_eq!(
        f.state
            .storage
            .participant_by_id(f.rows[0].id)
            .await
            .unwrap()
            .unwrap()
            .model_id
            .as_deref(),
        Some("old")
    );
    assert!(!f.state.sessions.lock().await.contains_key("s1"));
}

#[tokio::test]
async fn automatic_publication_runs_after_adding_or_removing_a_reviewer() {
    for remove in [false, true] {
        let f = Fixture::new(false).await;
        let (services, mut spawned) = services(false);
        let request = if remove {
            ParticipantChangeRequest::Remove {
                participant_id: f.rows[1].id,
            }
        } else {
            ParticipantChangeRequest::Add {
                role_id: f.rows[1].role_id.unwrap(),
                model_id: "new".into(),
                label: None,
            }
        };
        f.start("roster", request, services).await;
        assert_eq!(f.done("roster").await.status, "applied");
        let _added = if remove {
            None
        } else {
            Some(spawned.recv().await.unwrap())
        };
        let body = f._temp.path().join("body.md");
        let marker = f._temp.path().join("published.txt");
        std::fs::write(&body, "fixture publication").unwrap();
        let quote =
            |p: &std::path::Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
        let command = format!(
            "gh() {{ cat {} >> {}; }}; gh issue create --title fixture --body-file {}",
            quote(&body),
            quote(&marker),
            quote(&body)
        );
        let reply = f
            .state
            .bridge
            .action_gate("s1".into(), f.rows[0].slug.clone(), command.clone(), false)
            .await
            .unwrap();
        if !remove {
            assert!(
                !marker.exists(),
                "configured reviewers still read automatic publications"
            );
            let id = f
                .state
                .storage
                .queued_gate_for_command("s1", &command)
                .await
                .unwrap()
                .unwrap();
            let row = f.state.storage.get_tray_entry(&id).await.unwrap().unwrap();
            f.state
                .storage
                .commit_delivery(f.rows[1].id, &[(row.body_row_id.unwrap(), None)])
                .await
                .unwrap();
            f.state
                .bridge
                .settle_queued_outward("s1", f.rows[1].id)
                .await;
            assert_eq!(f.state.bridge.run_reviewed_auto_publications().await, 1);
        }
        assert!(
            marker.exists(),
            "remove={remove}; {reply}; rows={:?}",
            f.state
                .storage
                .messages_for_session("s1", None)
                .await
                .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "fixture publication"
        );
        assert_eq!(
            f.state.bridge.run_reviewed_auto_publications().await,
            0,
            "only one execution"
        );
        f.end().await;
    }
}

#[tokio::test]
async fn removing_a_requester_keeps_its_user_gate_answerable() {
    let f = Fixture::new(false).await;
    f.state
        .storage
        .insert_tray_entry(
            "s1",
            "pending",
            &f.rows[1].slug,
            crate::storage::QuestionKind::Approval,
            "fixture approval",
            Some(&["Approve".into(), "Reject".into()]),
            None,
            Some("printf fixture"),
        )
        .await
        .unwrap();
    let (services, _spawned) = services(false);
    f.start(
        "remove",
        ParticipantChangeRequest::Remove {
            participant_id: f.rows[1].id,
        },
        services,
    )
    .await;
    assert_eq!(f.done("remove").await.status, "applied");
    assert_eq!(
        f.state
            .storage
            .get_tray_entry("pending")
            .await
            .unwrap()
            .unwrap()
            .status,
        "pending"
    );
    f.state
        .bridge
        .resolve_choice("pending", "Reject — keep the history".into())
        .await
        .unwrap();
    assert_eq!(
        f.state
            .storage
            .get_tray_entry("pending")
            .await
            .unwrap()
            .unwrap()
            .picked_option
            .as_deref(),
        Some("Reject — keep the history")
    );
    f.end().await;
}

#[tokio::test]
async fn a_pending_change_can_be_cancelled_without_stopping_the_current_turn() {
    let mut f = Fixture::new(false).await;
    f.state
        .storage
        .post_to_channel("s1", "user", None, "text", "work", None)
        .await
        .unwrap();
    f.ring
        .send(SequencerCommand::UserMessage {
            mentions: vec![],
            restarts_rotation: true,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), f.seats[0].input.recv())
        .await
        .unwrap()
        .unwrap();
    let (services, mut spawned) = services(false);
    f.start("cancel", f.switch(), services).await;
    assert!(f
        .state
        .cancel_participant_change("s1", "cancel")
        .await
        .unwrap());
    assert_eq!(f.done("cancel").await.status, "cancelled");
    assert!(!*f.seats[0].stopped.borrow());
    assert!(!*f.seats[1].stopped.borrow());
    assert!(spawned.try_recv().is_err());
    assert!(f.state.storage.session_halt("s1").await.unwrap().is_none());
    f.end().await;
}

#[tokio::test]
async fn worker_drains_old_watchdog_callbacks_before_retiring_the_target() {
    let f = Fixture::new(false).await;
    let old_scope = f.state.sessions.lock().await["s1"]
        .watchdog_roster
        .lock()
        .unwrap()
        .scope
        .clone();
    let callback = old_scope.enter().await.unwrap();
    let (services, mut spawned) = services(false);
    f.start("switch", f.switch(), services).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !old_scope.is_active(),
        "future old callbacks are already excluded"
    );
    assert!(
        !*f.seats[0].stopped.borrow(),
        "the admitted callback must finish first"
    );
    drop(callback);
    assert_eq!(f.done("switch").await.status, "applied");
    let _fresh = spawned.recv().await.unwrap();
    let sessions = f.state.sessions.lock().await;
    let roster = sessions["s1"].watchdog_roster.lock().unwrap();
    assert!(roster.scope.is_active());
    assert!(!Arc::ptr_eq(&old_scope, &roster.scope));
    assert!(!roster.changing);
    assert!(roster
        .agents
        .iter()
        .any(|(slug, live)| slug == &f.rows[0].slug && live.upgrade().is_some()));
    drop(roster);
    drop(sessions);
    f.end().await;
}

#[tokio::test]
async fn switching_a_reviewer_reposts_a_previously_read_body_to_its_fresh_context() {
    let f = Fixture::new(false).await;
    let s = &f.state.storage;
    let body = s
        .post_to_channel(
            "s1",
            "system",
            None,
            "system_notice",
            "EXACT PENDING BODY",
            None,
        )
        .await
        .unwrap()
        .message_id();
    s.insert_queued_gate_with_approval(
        "s1",
        "automatic",
        &f.rows[0].slug,
        "fixture",
        "publish fixture",
        false,
    )
    .await
    .unwrap();
    s.set_tray_body_row("automatic", body).await.unwrap();
    s.commit_delivery(f.rows[1].id, &[(body, None)])
        .await
        .unwrap();
    assert!(s.mark_auto_gate_reviewed("automatic", body).await.unwrap());
    let (services, mut spawned) = services(false);
    f.start(
        "switch",
        ParticipantChangeRequest::SwitchModel {
            participant_id: f.rows[1].id,
            model_id: "new".into(),
        },
        services,
    )
    .await;
    assert_eq!(f.done("switch").await.status, "applied");
    let mut fresh = spawned.recv().await.unwrap();
    let row = s.get_tray_entry("automatic").await.unwrap().unwrap();
    assert!(row.body_row_id.unwrap() > body);
    assert!(row.reviewed_at.is_none());
    assert!(!s.claim_auto_gate_run("automatic", body).await.unwrap());
    assert!(!s
        .claim_auto_gate_run("automatic", row.body_row_id.unwrap())
        .await
        .unwrap());
    let page = tokio::time::timeout(Duration::from_secs(2), fresh.seat.input.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(page.message.content.contains("EXACT PENDING BODY"));
    assert!(page
        .message
        .content
        .contains("review this outstanding decision again"));
    f.end().await;
}
