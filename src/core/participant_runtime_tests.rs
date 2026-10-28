use super::*;
use crate::agents::{AgentEvent, OutgoingUserMessage};
use crate::core::{
    sequencer::{SequencerCommand, SequencerDeps},
    session::SessionAgent,
};
use crate::storage::{Model, Participant, Storage};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, oneshot};

#[path = "helper_native_tests.rs"]
mod helper_native_tests;
#[cfg(unix)]
#[path = "direct_command_tests.rs"]
mod direct_command_tests;

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
                    let (mut handle, seat) = fake_agent(&cfg.session_id, &cfg.agent_name);
                    handle.effort_control = crate::agents::effort::EffortControl::codex(Arc::new(std::sync::Mutex::new(crate::agents::effort::EffortPair {
                        effort: cfg.overrides.effort.clone().unwrap_or_else(|| "medium".into()), ultracode: cfg.overrides.ultracode.unwrap_or(false),
                    })));
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
        let delegated = f.state.storage.post_to_channel("s1", "system", None, "system_notice", "Delegated message from controller of driver-session: DELEGATED-DIRECTION", None).await.unwrap().message_id();
        if !adding {
            f.state
                .storage
                .commit_delivery(f.rows[0].id, &[(old, None), (delegated, None)])
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
        assert_eq!(f.state.storage.cursor_for(target).await.unwrap(), delegated);
        let old_receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM participant_deliveries WHERE participant_id = ? AND message_id IN (?, ?)")
            .bind(target).bind(old).bind(delegated).fetch_one(f.state.storage.pool()).await.unwrap();
        assert_eq!(
            old_receipts,
            if adding { 0 } else { 2 },
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
        assert!(fresh.prompt.contains("DELEGATED-DIRECTION"), "a model change must retain recent delegated instructions separately from human words");
        assert!(fresh.prompt.contains("Channel excerpt (historical source, not necessarily human)"));
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

async fn driver_fixture(disk: bool) -> Fixture {
    let f = Fixture::with_database(false, disk).await;
    let activity = f.state.sessions.lock().await["s1"].activity.clone();
    f.state.bridge.register_session_activity("s1".into(), Arc::downgrade(&activity)).await;
    sqlx::query("UPDATE roles SET capabilities = json_insert(capabilities, '$[#]', 'control_bot_hq') WHERE id = ?")
        .bind(f.rows[0].role_id).execute(f.state.storage.pool()).await.unwrap();
    sqlx::query("UPDATE session_participants SET capabilities = json_insert(capabilities, '$[#]', 'control_bot_hq') WHERE id = ?")
        .bind(f.rows[0].id).execute(f.state.storage.pool()).await.unwrap();
    f
}

async fn driver_rpc(state: &Arc<AppState>, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let response = reqwest::Client::new()
        .post(format!("http://{}/sessions/s1/hands/mcp", state.signaling_addr))
        .header("x-bot-hq-mcp-token", "token-hands")
        .header("content-type", "application/json")
        .body(serde_json::json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"app_control","arguments":args}}).to_string())
        .send().await?.error_for_status()?.text().await?;
    let response: serde_json::Value = serde_json::from_str(&response)?;
    anyhow::ensure!(response.get("error").is_none(), "{response}");
    let text = response["result"]["content"][0]["text"].as_str().context("missing tool result")?;
    anyhow::ensure!(response["result"]["isError"] != true, "{text}");
    Ok(serde_json::from_str(text)?)
}

async fn helper_rpc(state: &Arc<AppState>, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let body=reqwest::Client::new().post(format!("http://{}/sessions/s1/hands/mcp",state.signaling_addr))
        .header("x-bot-hq-mcp-token","token-hands").header("content-type","application/json")
        .body(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delegate_tasks","arguments":args}}).to_string())
        .send().await?.text().await?;
    let reply: serde_json::Value=serde_json::from_str(&body)?;
    anyhow::ensure!(reply.get("error").is_none(),"{reply}");
    let text=reply["result"]["content"][0]["text"].as_str().context("missing helper reply")?;
    anyhow::ensure!(reply["result"]["isError"]!=true,"{text}");
    Ok(serde_json::from_str(text)?)
}

async fn helper_fixture() -> (Fixture,mpsc::UnboundedReceiver<(String,Seat)>) {
    helper_fixture_with_database(false).await
}
async fn helper_fixture_with_database(disk: bool) -> (Fixture,mpsc::UnboundedReceiver<(String,Seat)>) {
    let mut f=Fixture::with_database(false,disk).await;
    sqlx::query("UPDATE roles SET capabilities=json_insert(capabilities,'$[#]','delegate_tasks') WHERE id=?").bind(f.rows[0].role_id).execute(f.state.storage.pool()).await.unwrap();
    sqlx::query("UPDATE session_participants SET capabilities=json_insert(capabilities,'$[#]','delegate_tasks') WHERE id=?").bind(f.rows[0].id).execute(f.state.storage.pool()).await.unwrap();
    let (tx,rx)=mpsc::unbounded_channel();
    f.state.helpers.set_spawner(Arc::new(move |cfg,brief| {
        let tx=tx.clone(); Box::pin(async move {
            let (handle,seat)=fake_agent(&cfg.session_id,&cfg.agent_name);
            tx.send((brief,seat)).unwrap(); Ok(handle)
        })
    }));
    f.state.storage.post_to_channel("s1","user",None,"text","Investigate using helpers.",None).await.unwrap();
    f.ring.send(SequencerCommand::UserMessage {mentions:vec![],restarts_rotation:true}).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3),f.seats[0].input.recv()).await.unwrap().unwrap();
    (f,rx)
}

fn helper_start(id: &str) -> serde_json::Value {
    serde_json::json!({"action":"start","request_id":id,"tasks":[{"goal":"Inspect fixture","files":["source.rs"],"expected_evidence":"path:line","model_id":"old"}],"timeout_secs":30})
}

#[tokio::test]
async fn helpers_require_an_explicit_effort_when_the_parent_is_unconfirmed() {
    let (f, mut spawned) = helper_fixture().await;
    f.state.storage.set_observed_spawn_knobs(f.rows[0].id, None).await.unwrap();
    let request = helper_start("unconfirmed-parent");
    assert!(helper_rpc(&f.state, request.clone()).await.unwrap_err().to_string().contains("Parent effort is unconfirmed"));
    assert!(spawned.try_recv().is_err());
    assert!(f.state.storage.helper_batch("s1", f.rows[0].id, "unconfirmed-parent").await.unwrap().is_none());
    let mut explicit = request; explicit["tasks"][0]["effort"] = "high".into();
    helper_rpc(&f.state, explicit).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), spawned.recv()).await.unwrap().unwrap();
    f.end().await;
}

#[tokio::test]
async fn candidate_effort_metadata_comes_from_observation_and_unknown_is_not_confirmed() {
    for known in [true, false] {
        let f = Fixture::new(false).await;
        let (mut services, mut spawned) = services(false);
        let base = services.spawn.clone();
        services.spawn = Arc::new(move |cfg| {
            let base = base.clone();
            Box::pin(async move {
                let mut handle = base(cfg).await?;
                handle.effort_control = if known {
                    crate::agents::effort::EffortControl::codex(Arc::new(std::sync::Mutex::new(crate::agents::effort::EffortPair { effort: "max".into(), ultracode: false })))
                } else { Default::default() };
                Ok(handle)
            })
        });
        f.start("switch", f.switch(), services).await;
        assert_eq!(f.done("switch").await.status, "applied");
        let row = f.state.storage.participant_by_id(f.rows[0].id).await.unwrap().unwrap();
        assert_eq!(row.spawn_knobs_recorded, known);
        assert_eq!(row.effort_at_spawn.as_deref(), if known {Some("max")} else {None});
        assert!(row.effort.is_none(), "do not change the user's inherited choice");
        spawned.recv().await.unwrap();
        f.end().await;
    }
}

#[tokio::test]
async fn helpers_start_once_collect_transcripts_and_do_not_join_the_roster() {
    let (f,mut spawned)=helper_fixture().await;
    let request=helper_start("one");
    let first=helper_rpc(&f.state,request.clone()).await.unwrap();
    let (brief,seat)=tokio::time::timeout(Duration::from_secs(3),spawned.recv()).await.unwrap().unwrap();
    assert!(brief.contains("Inspect fixture"));
    let retry=helper_rpc(&f.state,request.clone()).await.unwrap(); assert_eq!(first["batch_id"],retry["batch_id"]);
    assert!(spawned.try_recv().is_err());
    assert!(helper_rpc(&f.state,helper_start("second")).await.unwrap_err().to_string().contains("active helper"));
    let mut conflict=request.clone();conflict["tasks"][0]["goal"]="Changed".into();
    assert!(helper_rpc(&f.state,conflict).await.unwrap_err().to_string().contains("different batch"));
    seat.events.send(AgentEvent::Text("source.rs:1 evidence".into())).await.unwrap();
    seat.events.send(AgentEvent::TurnComplete {stop_reason:None,subtype:Some("success".into()),is_error:false,api_error_status:None,context:crate::agents::spawn::ContextReport::none(crate::agents::spawn::ContextVerdict::NoUsage)}).await.unwrap();
    let result=helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"one","wait_secs":5})).await.unwrap();
    assert_eq!(result["status"],"completed"); assert_eq!(result["result"]["helpers"][0]["cleanup_confirmed"],true);
    assert!(result["result"]["helpers"][0].get("transcript").is_none());
    let doc=f.state.storage.session_document_by_slug("s1","helpers").await.unwrap().unwrap();
    assert!(doc.body.contains("source.rs:1 evidence") && doc.body.contains("Inspect fixture"));
    assert_eq!(f.state.storage.participants_for_session("s1").await.unwrap().len(),2);
    assert_eq!(helper_rpc(&f.state,request).await.unwrap()["batch_id"],first["batch_id"]);
    assert!(spawned.try_recv().is_err());
    f.end().await;
}

/// EYES advisories ac2c2f31 / 1bea6602 (2026-10-27): a helper reasons at its
/// PARENT's recorded effort unless the task picks one, so delegation never
/// quietly lowers the effort behind the parent's output; and its brief demands
/// the exact search behind every claim of absence, because citations catch
/// wrong claims, not misses.
#[tokio::test]
async fn helpers_inherit_the_parents_effort_and_are_told_to_show_their_searches() {
    let mut f=Fixture::with_database(false,false).await;
    sqlx::query("UPDATE roles SET capabilities=json_insert(capabilities,'$[#]','delegate_tasks') WHERE id=?").bind(f.rows[0].role_id).execute(f.state.storage.pool()).await.unwrap();
    sqlx::query("UPDATE session_participants SET capabilities=json_insert(capabilities,'$[#]','delegate_tasks'), effort_at_spawn='max' WHERE id=?").bind(f.rows[0].id).execute(f.state.storage.pool()).await.unwrap();
    let (tx,mut spawned)=mpsc::unbounded_channel();
    f.state.helpers.set_spawner(Arc::new(move |cfg,brief| {
        let tx=tx.clone(); Box::pin(async move {
            let (handle,seat)=fake_agent(&cfg.session_id,&cfg.agent_name);
            let instructions=std::fs::read_to_string(&cfg.system_prompt_path).unwrap();
            tx.send((cfg.overrides.effort.clone(),instructions,brief,seat)).unwrap(); Ok(handle)
        })
    }));
    f.state.storage.post_to_channel("s1","user",None,"text","Investigate using helpers.",None).await.unwrap();
    f.ring.send(SequencerCommand::UserMessage {mentions:vec![],restarts_rotation:true}).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3),f.seats[0].input.recv()).await.unwrap().unwrap();
    // An effort outside the CLI's set is the caller's error, before any spawn.
    let bad=serde_json::json!({"action":"start","request_id":"bad","tasks":[{"goal":"g","files":[],"expected_evidence":"e","model_id":"old","effort":"ultra"}],"timeout_secs":30});
    assert!(helper_rpc(&f.state,bad).await.unwrap_err().to_string().contains("effort must be one of"));
    // Two helpers in one batch: the first inherits `max`, the second picked `low`.
    let request=serde_json::json!({"action":"start","request_id":"effort","tasks":[
        {"goal":"inherit","files":["source.rs"],"expected_evidence":"path:line","model_id":"old"},
        {"goal":"picked","files":["source.rs"],"expected_evidence":"path:line","model_id":"old","effort":"low"}],"timeout_secs":30});
    helper_rpc(&f.state,request).await.unwrap();
    let mut seen=std::collections::BTreeMap::new(); let mut seats=vec![];
    for _ in 0..2 {
        let (effort,instructions,brief,seat)=tokio::time::timeout(Duration::from_secs(3),spawned.recv()).await.unwrap().unwrap();
        assert!(instructions.contains("state the exact search behind it") && instructions.contains("a claim without its search is an opinion"), "{instructions}");
        let goal=serde_json::from_str::<serde_json::Value>(&brief).unwrap()["goal"].as_str().unwrap().to_string();
        seen.insert(goal,effort); seats.push(seat);
    }
    assert_eq!(seen.get("inherit").cloned().flatten().as_deref(),Some("max"),"the parent's recorded effort is the default: {seen:?}");
    assert_eq!(seen.get("picked").cloned().flatten().as_deref(),Some("low"),"a task's own pick wins: {seen:?}");
    for seat in &seats {
        seat.events.send(AgentEvent::Text("source.rs:1 evidence".into())).await.unwrap();
        seat.events.send(AgentEvent::TurnComplete {stop_reason:None,subtype:Some("success".into()),is_error:false,api_error_status:None,context:crate::agents::spawn::ContextReport::none(crate::agents::spawn::ContextVerdict::NoUsage)}).await.unwrap();
    }
    let result=helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"effort","wait_secs":5})).await.unwrap();
    assert_eq!(result["status"],"completed");
    let efforts: Vec<_>=result["result"]["helpers"].as_array().unwrap().iter().map(|h|h["effort"].as_str().map(str::to_string)).collect();
    assert!(efforts.contains(&Some("max".into())) && efforts.contains(&Some("low".into())), "the helpers document records each helper's effort: {efforts:?}");
    f.end().await;
}

/// eyes-2 blocker 813b097d: a batch recorded before `HelperTask.effort`
/// existed hashed `[[{goal,files,expected_evidence,model_id}],timeout]`; a
/// retry of the identical request must still find it (and spawn nothing),
/// while a request that now names an effort is different work.
#[tokio::test]
async fn a_retry_of_a_batch_recorded_before_the_effort_field_still_finds_it() {
    use sha2::{Digest, Sha256};
    let (f,mut spawned)=helper_fixture().await;
    let legacy=r#"[[{"goal":"Inspect fixture","files":["source.rs"],"expected_evidence":"path:line","model_id":"old"}],30]"#;
    let fingerprint=format!("{:x}",Sha256::digest(legacy.as_bytes()));
    sqlx::query("INSERT INTO session_helper_batches(id,session_id,participant_id,request_id,fingerprint,owner_incarnation,owner_epoch,request_json,status,result_json,created_at,finished_at) VALUES('legacy-batch','s1',?,'legacy',?,'old-incarnation',1,?,'completed',?,'2026-10-26T15:00:00.000Z','2026-10-26T15:04:00.000Z')")
        .bind(f.rows[0].id).bind(&fingerprint).bind(legacy).bind(r#"{"parent":"hands","helpers":[{"status":"completed","output":"legacy finding"}]}"#)
        .execute(f.state.storage.pool()).await.unwrap();
    let replay=helper_rpc(&f.state,helper_start("legacy")).await.unwrap();
    assert_eq!((replay["batch_id"].as_str(),replay["status"].as_str()),(Some("legacy-batch"),Some("completed")),"{replay}");
    assert_eq!(replay["result"]["helpers"][0]["output"],"legacy finding");
    assert!(spawned.try_recv().is_err(),"a retry never spawns");
    let mut with_effort=helper_start("legacy"); with_effort["tasks"][0]["effort"]="low".into();
    assert!(helper_rpc(&f.state,with_effort).await.unwrap_err().to_string().contains("different batch"));
    f.end().await;
}

#[tokio::test]
async fn helpers_preserve_final_findings_after_long_commentary_and_file_results() {
    let (f,mut spawned)=helper_fixture().await;
    helper_rpc(&f.state,helper_start("long-answer")).await.unwrap();
    let (_,seat)=tokio::time::timeout(Duration::from_secs(3),spawned.recv()).await.unwrap().unwrap();
    seat.events.send(AgentEvent::Text("EARLY_COMMENTARY ".repeat(2000))).await.unwrap();
    seat.events.send(AgentEvent::ToolUse {id:"read".into(),name:"Read".into(),input:serde_json::json!({"file_path":"src/source.rs"})}).await.unwrap();
    seat.events.send(AgentEvent::ToolResult {tool_use_id:"read".into(),content:"BULKY_FILE_CONTENT ".repeat(20_000),is_error:false}).await.unwrap();
    seat.events.send(AgentEvent::Text("Checking the call site. ".repeat(700))).await.unwrap();
    seat.events.send(AgentEvent::ToolUse {id:"grep".into(),name:"Grep".into(),input:serde_json::json!({"pattern":"call_site"})}).await.unwrap();
    seat.events.send(AgentEvent::ToolResult {tool_use_id:"grep".into(),content:"source.rs:42".into(),is_error:false}).await.unwrap();
    let finding="src/source.rs:42 — final finding with the actionable correction.";
    seat.events.send(AgentEvent::Text(finding.into())).await.unwrap();
    seat.events.send(AgentEvent::TurnComplete {stop_reason:None,subtype:Some("success".into()),is_error:false,api_error_status:None,context:crate::agents::spawn::ContextReport::none(crate::agents::spawn::ContextVerdict::NoUsage)}).await.unwrap();
    let result=helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"long-answer","wait_secs":5})).await.unwrap();
    assert_eq!(result["result"]["helpers"][0]["output"],finding);
    let doc=f.state.storage.session_document_by_slug("s1","helpers").await.unwrap().unwrap();
    assert!(doc.body.contains(finding));
    assert!(!doc.body.contains(&"BULKY_FILE_CONTENT ".repeat(100)));
    f.end().await;
}

#[tokio::test]
async fn helper_result_is_paged_and_readable_by_a_reviewer_without_launch_permission() {
    let (f,mut spawned)=helper_fixture().await;
    let started=helper_rpc(&f.state,helper_start("readable")).await.unwrap();
    let (_,seat)=tokio::time::timeout(Duration::from_secs(3),spawned.recv()).await.unwrap().unwrap();
    let answer=format!("FINAL_START {} FINAL_END","evidence ".repeat(3000));
    seat.events.send(AgentEvent::Text(answer.clone())).await.unwrap();
    seat.events.send(AgentEvent::TurnComplete {stop_reason:None,subtype:Some("success".into()),is_error:false,api_error_status:None,context:crate::agents::spawn::ContextReport::none(crate::agents::spawn::ContextVerdict::NoUsage)}).await.unwrap();
    let status=helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"readable","wait_secs":5})).await.unwrap();
    assert_eq!(status["result"]["helpers"][0]["output_preview_truncated"],true);
    let batch=started["batch_id"].as_str().unwrap();let mut offset=0;let mut hash=None;let mut text=String::new();
    loop {
        let raw=reqwest::Client::new().post(format!("http://{}/sessions/s1/eyes/mcp",f.state.signaling_addr))
            .header("x-bot-hq-mcp-token","token-eyes").header("content-type","application/json")
            .body(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"helper_result","arguments":{"batch_id":batch,"offset_bytes":offset}}}).to_string())
            .send().await.unwrap().text().await.unwrap();
        let reply: serde_json::Value=serde_json::from_str(&raw).unwrap();assert!(reply.get("error").is_none(),"{reply}");
        assert_ne!(reply["result"]["isError"],true,"{reply}");
        let page: serde_json::Value=serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        if let Some(hash)=&hash {assert_eq!(&page["sha256"],hash);} else {hash=Some(page["sha256"].clone());}
        assert!(page["content"].as_str().unwrap().len()<=16*1024);text.push_str(page["content"].as_str().unwrap());
        let Some(next)=page["next_offset_bytes"].as_u64() else {break};assert!(next>offset);offset=next;
    }
    let saved: serde_json::Value=serde_json::from_str(&text).unwrap();assert_eq!(saved["helpers"][0]["output"],answer);
    assert!(f.state.storage.helper_result("another-session",Some(batch),None,0).await.is_err());
    let index=f.state.storage.helper_result("s1",None,None,0).await.unwrap();assert_eq!(index["batches"][0]["batch_id"],batch);
    assert!(f.state.storage.helper_actor("s1","eyes").await.is_err(),"reviewer still cannot launch helpers");
    f.end().await;
}

#[tokio::test]
async fn helpers_stop_on_parent_interrupt_retirement_and_turn_end() {
    for mode in ["interrupt","retire","complete"] {
        let (f,mut spawned)=helper_fixture().await;
        helper_rpc(&f.state,helper_start("cancelled")).await.unwrap();
        let (_,mut seat)=tokio::time::timeout(Duration::from_secs(3),spawned.recv()).await.unwrap().unwrap();
        let owner={let sessions=f.state.sessions.lock().await;let agent=&sessions["s1"].participants[0];
            let epoch=agent.turn_epoch.as_ref().unwrap().load(Ordering::Acquire);
            if mode=="interrupt" {agent.handle.interrupt_at("test-pause",epoch);}
            if mode=="complete" {agent.handle.incarnation().end_helper_epoch(epoch);}
            agent.handle.incarnation()};
        if mode=="retire" {owner.retire().await;}
        let result=helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"cancelled","wait_secs":5})).await.unwrap();
        assert_eq!(result["status"],"cancelled","{mode}: {result}");
        if !*seat.stopped.borrow() {tokio::time::timeout(Duration::from_secs(2),seat.stopped.changed()).await.unwrap().unwrap();}
        assert!(*seat.stopped.borrow());
        f.end().await;
    }
}

#[tokio::test]
async fn codex_helper_eligibility_and_configuration_follow_the_selected_row() {
    let (f,_) = helper_fixture().await;
    sqlx::query("UPDATE models SET runtime='codex',provider='openai' WHERE id='old'").execute(f.state.storage.pool()).await.unwrap();
    let models=helper_rpc(&f.state,serde_json::json!({"action":"models"})).await.unwrap();
    let eligible=models["models"].as_array().unwrap().iter().any(|m|m["id"]=="old");
    assert_eq!(eligible,cfg!(unix));
    let (tx,mut seen)=mpsc::unbounded_channel();
    f.state.helpers.set_spawner(Arc::new(move |cfg, _| {
        let tx=tx.clone();Box::pin(async move {
            let (handle,seat)=fake_agent(&cfg.session_id,&cfg.agent_name);
            tx.send((cfg,seat)).unwrap();Ok(handle)
        })
    }));
    let mut start=helper_start("codex-config");start["tasks"][0].as_object_mut().unwrap().remove("model_id");
    if cfg!(unix) {
        helper_rpc(&f.state,start).await.unwrap();
        let (cfg,seat)=tokio::time::timeout(Duration::from_secs(3),seen.recv()).await.unwrap().unwrap();
        assert_eq!(cfg.config.runtime,"codex");assert_eq!(cfg.config.model_name,"old");
        assert_eq!(cfg.account_dir.as_deref(),Some(f._temp.path().join("old").to_str().unwrap()));
        let codex=cfg.codex.expect("selected Codex connector must be configured before spawn");
        assert!(codex.mcp_token.is_empty() && codex.mcp_url.is_empty() && codex.hook_trust.is_empty());
        assert!(cfg.mcp_config_path.is_none() && cfg.resume_session_id.is_none());
        drop(seat);
        helper_rpc(&f.state,serde_json::json!({"action":"cancel","request_id":"codex-config"})).await.unwrap();
        assert_eq!(helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"codex-config","wait_secs":5})).await.unwrap()["status"],"cancelled");
    } else {
        assert!(helper_rpc(&f.state,start).await.is_err());
        assert!(seen.try_recv().is_err());
    }
    f.end().await;
}

#[tokio::test]
#[cfg(unix)]
#[ignore = "spends one native Codex helper turn through real HTTP/manager; select CODEX_HOME/MODEL probe vars"]
async fn native_codex_child_runs_through_the_real_helper_manager() {
    let (f,_) = helper_fixture().await;
    f.state.helpers.clear_spawner();
    let model=std::env::var("BOT_HQ_HELPER_CODEX_MODEL").unwrap();
    let home=std::env::var("BOT_HQ_HELPER_CODEX_HOME").unwrap();
    sqlx::query("UPDATE models SET runtime='codex',provider='openai',model_name=?,claude_config_dir=?,context_window=NULL WHERE id='old'")
        .bind(&model).bind(&home).execute(f.state.storage.pool()).await.unwrap();
    std::fs::write(f._temp.path().join("source.rs"),"// CODEX_MANAGER_PELICAN\n").unwrap();
    let mut request=helper_start("native-codex-manager");
    request["tasks"][0]["goal"]="Read source.rs and quote line 1 with its path.".into();
    request["timeout_secs"]=120.into();
    let outcome=tokio::time::timeout(Duration::from_secs(140),async {
        let first=helper_rpc(&f.state,request.clone()).await?;
        let retry=helper_rpc(&f.state,request).await?;
        anyhow::ensure!(first["batch_id"]==retry["batch_id"],"retry changed the native batch");
        loop {
            let status=helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"native-codex-manager","wait_secs":45})).await?;
            if !matches!(status["status"].as_str(),Some("starting"|"running"|"cancelling")) {break;}
        }
        let row=f.state.storage.helper_batch("s1",f.rows[0].id,"native-codex-manager").await?.context("native batch missing")?;
        let result: serde_json::Value=serde_json::from_str(row.result_json.as_deref().context("native result missing")?)?;
        if let Ok(path)=std::env::var("BOT_HQ_HELPER_PROBE_LOG") {std::fs::write(path,serde_json::to_vec_pretty(&result)?)?;}
        anyhow::ensure!(row.status=="completed","native helper failed: {result}");
        anyhow::ensure!(result["helpers"][0]["runtime"]=="codex","wrong helper connector");
        anyhow::ensure!(result["helpers"][0]["cleanup_confirmed"]==true,"native cleanup unconfirmed");
        anyhow::ensure!(result["helpers"][0]["output"].as_str().is_some_and(|s|s.contains("CODEX_MANAGER_PELICAN")),"native finding missing: {result}");
        anyhow::ensure!(result["helpers"][0]["transcript"].as_str().is_some_and(|s|s.contains("repo_read")),"real repository tool was not used");
        let count: i64=sqlx::query_scalar("SELECT COUNT(*) FROM session_helper_batches WHERE session_id='s1'").fetch_one(f.state.storage.pool()).await?;
        anyhow::ensure!(count==1,"native retry created another batch");
        let doc=f.state.storage.session_document_by_slug("s1","helpers").await?.context("helper document missing")?;
        anyhow::ensure!(doc.body.contains("CODEX_MANAGER_PELICAN"),"native finding not retained for review");
        Ok::<_,anyhow::Error>(())
    }).await;
    f.end().await;
    outcome.expect("native manager probe timed out").expect("native manager probe failed");
    eprintln!("native Codex child: actual HTTP claim/manager/connector/result/document; one batch, real repo_read, cleanup confirmed");
}

#[tokio::test]
async fn helpers_enforce_grants_model_selection_and_wait_bounds() {
    let (f,mut spawned)=helper_fixture().await;
    assert!(!helper_rpc(&f.state,serde_json::json!({"action":"models"})).await.unwrap()["models"].as_array().unwrap().is_empty());
    let mut bad=helper_start("bad");bad["tasks"][0]["model_id"]="missing".into();assert!(helper_rpc(&f.state,bad).await.is_err());
    assert!(helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"missing","wait_secs":46})).await.unwrap_err().to_string().contains("45"));
    sqlx::query("UPDATE roles SET capabilities='[]' WHERE id=?").bind(f.rows[0].role_id).execute(f.state.storage.pool()).await.unwrap();
    assert!(helper_rpc(&f.state,helper_start("revoked")).await.unwrap_err().to_string().contains("revoked"));
    assert!(spawned.try_recv().is_err());
    f.end().await;
}

#[tokio::test]
async fn helpers_queued_start_loses_to_pause_or_revocation() {
    for revoke in [false,true] {
        let (f,mut spawned)=helper_fixture_with_database(true).await;
        let reached=Arc::new(tokio::sync::Notify::new());f.state.helpers.observe_claim(reached.clone());
        let mut held=f.state.storage.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
        let state=f.state.clone();let start=tokio::spawn(async move{helper_rpc(&state,helper_start("queued")).await});
        tokio::time::timeout(Duration::from_secs(3),reached.notified()).await.unwrap();
        if revoke {sqlx::query("UPDATE roles SET capabilities='[]' WHERE id=?").bind(f.rows[0].role_id).execute(&mut *held).await.unwrap();}
        else {let activity=f.state.sessions.lock().await["s1"].activity.clone();activity.set_paused(true);activity.set_paused(false);}
        held.commit().await.unwrap();
        assert!(start.await.unwrap().is_err());assert!(spawned.try_recv().is_err());
        assert!(f.state.storage.helper_batch("s1",f.rows[0].id,"queued").await.unwrap().is_none());
        f.end().await;
    }
}

#[tokio::test]
async fn helper_start_survives_lost_http_reply_and_retry_then_stops_with_parent() {
    let (f, mut spawned) = helper_fixture_with_database(true).await;
    let reached = Arc::new(tokio::sync::Notify::new());
    f.state.helpers.observe_claim(reached.clone());
    let held = f.state.storage.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
    let request = helper_start("lost-reply");
    let url = format!("http://{}/sessions/s1/hands/mcp", f.state.signaling_addr);
    let body = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"delegate_tasks","arguments":request}});
    let client = tokio::spawn(async move {
        reqwest::Client::new().post(url)
            .header("x-bot-hq-mcp-token", "token-hands")
            .header("content-type", "application/json").body(body.to_string())
            .timeout(Duration::from_millis(500)).send().await
    });
    tokio::time::timeout(Duration::from_secs(3), reached.notified()).await.unwrap();
    assert!(client.await.unwrap().unwrap_err().is_timeout(), "the start reply must actually be lost");
    held.rollback().await.unwrap();
    let (_, mut seat) = tokio::time::timeout(Duration::from_secs(3), spawned.recv()).await.unwrap().unwrap();
    let row = f.state.storage.helper_batch("s1", f.rows[0].id, "lost-reply").await.unwrap().unwrap();
    let retried = helper_rpc(&f.state, request).await.unwrap();
    assert_eq!(retried["batch_id"], row.id);
    assert!(tokio::time::timeout(Duration::from_millis(50), spawned.recv()).await.is_err(), "retry launched a second helper");
    {
        let sessions = f.state.sessions.lock().await;
        let parent = &sessions["s1"].participants[0];
        parent.handle.incarnation().end_helper_epoch(parent.turn_epoch.as_ref().unwrap().load(Ordering::Acquire));
    }
    let status = helper_rpc(&f.state, serde_json::json!({"action":"status","request_id":"lost-reply","wait_secs":5})).await.unwrap();
    assert_eq!(status["status"], "cancelled");
    assert_eq!(status["result"]["helpers"][0]["cleanup_confirmed"], true);
    if !*seat.stopped.borrow() {tokio::time::timeout(Duration::from_secs(2), seat.stopped.changed()).await.unwrap().unwrap();}
    assert!(*seat.stopped.borrow(), "lost HTTP reply must not orphan the child");
    f.end().await;
}

#[tokio::test]
async fn helpers_cancel_and_expire_without_replaying_the_request() {
    for timeout in [false,true] {
        let (f,mut spawned)=helper_fixture().await;
        let mut request=helper_start("ending");if timeout {request["timeout_secs"]=1.into();}
        let started=helper_rpc(&f.state,request.clone()).await.unwrap();
        let (_,seat)=tokio::time::timeout(Duration::from_secs(3),spawned.recv()).await.unwrap().unwrap();
        if !timeout {helper_rpc(&f.state,serde_json::json!({"action":"cancel","request_id":"ending"})).await.unwrap();}
        let row=helper_rpc(&f.state,serde_json::json!({"action":"status","request_id":"ending","wait_secs":5})).await.unwrap();
        assert_eq!(row["status"],"cancelled");assert!(*seat.stopped.borrow());
        assert_eq!(helper_rpc(&f.state,request).await.unwrap()["batch_id"],started["batch_id"]);
        assert!(spawned.try_recv().is_err());f.end().await;
    }
}

async fn decision_args(f: &Fixture, id: &str, picked: &str) -> serde_json::Value {
    let choice = driver_rpc(&f.state, serde_json::json!({"action":"choice","choice_id":id})).await.unwrap();
    serde_json::json!({"action":"answer","choice_id":id,"picked":picked,"expected_choice_sha256":choice["result"]["sha256"],"confirm_stale":true})
}

#[tokio::test]
async fn driver_answers_through_http_without_impersonating_human_or_running_twice() {
    let f = driver_fixture(false).await;
    let s = &f.state.storage;
    s.insert_tray_entry("s1", "question", "hands", crate::storage::QuestionKind::Choice, "Next step?",
        Some(&["Wait".into(), "Continue".into()]), None, None).await.unwrap();
    let args = decision_args(&f, "question", "Check the failing test first").await;
    let answer = driver_rpc(&f.state, args.clone()).await.unwrap();
    assert_eq!(answer["result"]["status"], "recorded");
    let id = answer["result"]["message_id"].as_i64().unwrap();
    let (origin, kind, body): (String, String, String) = sqlx::query_as("SELECT origin, kind, content FROM messages WHERE id = ?")
        .bind(id).fetch_one(s.pool()).await.unwrap();
    assert_eq!((origin.as_str(),kind.as_str()), ("system","system_notice"));
    assert!(body.contains("Delegated answer by hands of s1"));
    assert!(body.contains("the delegated controller answering"));
    assert!(!body.contains("the user answering"));
    let human: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_user_words WHERE message_id = ?").bind(id).fetch_one(s.pool()).await.unwrap();
    assert_eq!(human, 0);
    let recorded: String = sqlx::query_scalar("SELECT decision_actor_json FROM session_tray WHERE choice_id = 'question'").fetch_one(s.pool()).await.unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&recorded).unwrap()["participant_id"], f.rows[0].id);
    assert!(driver_rpc(&f.state, args).await.is_err());

    let ran = f._temp.path().join("once.txt");
    let command = format!("printf x >> '{}'", ran.display());
    s.insert_tray_entry("s1", "command", "hands", crate::storage::QuestionKind::Approval, &command,
        Some(&["Approve".into(), "Reject".into()]), None, Some(&command)).await.unwrap();
    let args = decision_args(&f, "command", "Approve").await;
    driver_rpc(&f.state, args.clone()).await.unwrap();
    assert_eq!(std::fs::read_to_string(&ran).unwrap(), "x");
    assert!(driver_rpc(&f.state, args).await.is_err());
    assert_eq!(std::fs::read_to_string(&ran).unwrap(), "x");
    // The untouched human API still records real human authorship.
    s.insert_tray_entry("s1", "human", "hands", crate::storage::QuestionKind::Choice, "Human?", Some(&["Yes".into()]), None, None).await.unwrap();
    f.state.resolve_choice("human", "Yes".into(), false).await.unwrap();
    let human: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_user_words WHERE session_id = 's1'").fetch_one(s.pool()).await.unwrap();
    assert_eq!(human, 1);
    s.insert_tray_entry("s1", "changed", "hands", crate::storage::QuestionKind::Choice, "Original?", Some(&["Yes".into()]), None, None).await.unwrap();
    let stale = decision_args(&f, "changed", "Yes").await;
    sqlx::query("UPDATE session_tray SET prompt = 'Revised question' WHERE choice_id = 'changed'").execute(s.pool()).await.unwrap();
    assert!(driver_rpc(&f.state, stale).await.unwrap_err().to_string().contains("changed since"));
    assert_eq!(s.get_tray_entry("changed").await.unwrap().unwrap().status, "pending");
    f.end().await;
}

#[tokio::test]
async fn driver_live_hook_answer_records_actor_before_unblocking_the_waiter() {
    let f = driver_fixture(false).await;
    let bridge = f.state.bridge.clone();
    let mut events = bridge.subscribe();
    let waiter = tokio::spawn(async move {
        bridge.request_approval("s1".into(), "hands".into(), "Hook decision".into(), vec!["Approve".into(), "Reject".into()], crate::signaling::ApprovalContext {
            kind: crate::policy::ViolationKind::GenericApproval, action: "fixture decision".into(), detail: None, command: None,
        }).await
    });
    let choice_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop { if let crate::signaling::SignalingEvent::PendingChoice(row) = events.recv().await.unwrap() { break row.choice_id; } }
    }).await.unwrap();
    let args = decision_args(&f, &choice_id, "Approve").await;
    let result = driver_rpc(&f.state, args).await.unwrap();
    assert_eq!(result["result"]["status"], "delivered");
    assert_eq!(waiter.await.unwrap().unwrap(), "Approve");
    let bodies: Vec<String> = sqlx::query_scalar("SELECT content FROM messages WHERE session_id = 's1' AND origin = 'system'")
        .fetch_all(f.state.storage.pool()).await.unwrap();
    assert!(bodies.iter().any(|b|b.contains("answered by hands of s1: Approve")));
    let human: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_user_words WHERE session_id = 's1'").fetch_one(f.state.storage.pool()).await.unwrap();
    assert_eq!(human, 0);
    let actor: Option<String> = sqlx::query_scalar("SELECT decision_actor_json FROM session_tray WHERE choice_id = ?").bind(choice_id).fetch_one(f.state.storage.pool()).await.unwrap();
    assert!(actor.is_some());
    f.end().await;
}

#[tokio::test]
async fn driver_send_waits_for_the_live_turn_and_reports_real_delivery() {
    let mut f = driver_fixture(false).await;
    f.state.storage.post_to_channel("s1", "user", None, "text", "Start existing work", None).await.unwrap();
    f.ring.send(SequencerCommand::UserMessage { mentions: vec![], restarts_rotation: true }).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), f.seats[0].input.recv()).await.unwrap().unwrap();
    let epoch = f.state.sessions.lock().await["s1"].participants[0].turn_epoch.as_ref().unwrap().load(Ordering::Acquire);
    let args = serde_json::json!({"action":"send","session_id":"s1","request_id":"send-once","text":"@eyes Review this after the current turn"});
    let sent = driver_rpc(&f.state, args.clone()).await.unwrap();
    let id = sent["result"]["message_id"].as_i64().unwrap();
    assert_eq!(driver_rpc(&f.state, args).await.unwrap()["result"]["message_id"], id);
    driver_rpc(&f.state, serde_json::json!({"action":"resume","session_id":"s1"})).await.unwrap();
    assert!(f.seats[1].input.try_recv().is_err(), "an active turn must not be preempted by a delegated message");
    assert!(!*f.seats[0].stopped.borrow());
    f.ring.send(SequencerCommand::TurnComplete { participant_id:f.rows[0].id, epoch, ending:crate::core::sequencer::TurnEnding::Spoke }).await.unwrap();
    let page = tokio::time::timeout(Duration::from_secs(2), f.seats[1].input.recv()).await.unwrap().unwrap();
    assert!(page.message.content.contains("Delegated message from hands of s1"));
    assert!(page.message.content.contains("Review this after the current turn"));
    let status = driver_rpc(&f.state, serde_json::json!({"action":"message_status","session_id":"s1","message_id":id})).await.unwrap();
    assert!(status["result"]["participants"].as_array().unwrap().iter().any(|r|r["participant_id"] == f.rows[1].id && r["delivered_at"].is_string()));
    let human: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_user_words WHERE message_id = ?").bind(id).fetch_one(f.state.storage.pool()).await.unwrap();
    assert_eq!(human, 0);
    f.end().await;
}

#[tokio::test]
async fn driver_send_preserves_pause_human_stage_and_retry_tombstones() {
    let f = driver_fixture(false).await;
    let s = &f.state.storage;
    s.create_session("target", "paused target", None).await.unwrap();
    s.ensure_session_roster("target", 1).await.unwrap();
    let activity = crate::core::ActivityTracker::new("target", Arc::new(std::sync::atomic::AtomicBool::new(false)), f.state.bridge.clone(), vec![]);
    activity.set_paused(true);
    f.state.bridge.register_session_activity("target".into(), Arc::downgrade(&activity)).await;
    f.state.stage_user_response("target", "Human draft must survive", vec![]).await.unwrap();
    let args = serde_json::json!({"action":"send","session_id":"target","request_id":"paused-send","text":"Direction from the controller"});
    let sent = driver_rpc(&f.state, args.clone()).await.unwrap();
    let id = sent["result"]["message_id"].as_i64().unwrap();
    assert!(activity.is_paused());
    assert_eq!(sent["result"]["wake_requested"], false);
    assert_eq!(f.state.staged_response("target").await.unwrap().0, "Human draft must survive");
    assert_eq!(driver_rpc(&f.state, args.clone()).await.unwrap()["result"]["message_id"], id);
    let mut changed = args.clone(); changed["text"] = "different instruction".into();
    assert!(driver_rpc(&f.state, changed).await.unwrap_err().to_string().contains("different message"));
    let body: (String, String) = sqlx::query_as("SELECT origin, content FROM messages WHERE id = ? AND session_id = 'target'").bind(id).fetch_one(s.pool()).await.unwrap();
    assert_eq!(body.0, "system"); assert!(body.1.contains("hands of s1"));
    sqlx::query("DELETE FROM messages WHERE id = ?").bind(id).execute(s.pool()).await.unwrap();
    assert!(driver_rpc(&f.state, args).await.unwrap_err().to_string().contains("no longer retained"));
    s.close_session("target", false).await.unwrap();
    assert!(driver_rpc(&f.state, serde_json::json!({"action":"send","session_id":"target","request_id":"after-close","text":"Do not reopen implicitly"})).await.is_err());
    f.end().await;
}

#[tokio::test]
async fn driver_admin_partial_policy_save_preserves_unmentioned_rules() {
    let f = driver_fixture(false).await;
    let scope = serde_json::json!({"kind":"global"});
    admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":scope,"policy":{"push_gate":"ask","forbidden_in_commits":["retain-this-rule"]}})).await.unwrap();
    admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":scope,"policy":{"push_gate":"auto"}})).await.unwrap();
    let policy = admin_rpc(&f.state, serde_json::json!({"operation":"policy","scope":scope})).await.unwrap();
    assert_eq!(policy["forbidden_in_commits"], serde_json::json!(["retain-this-rule"]));
    admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":scope,"policy":{"forbidden_in_commits":[]}})).await.unwrap();
    let cleared = admin_rpc(&f.state, serde_json::json!({"operation":"policy","scope":scope})).await.unwrap();
    assert_eq!(cleared["forbidden_in_commits"], serde_json::json!([]));
    assert_eq!(cleared["push_gate"], "auto");
    let connection = serde_json::json!({"engine":"postgres","host":"old.example.test","database":"reports","user":"reader","sslmode":"verify-full","password_var":"FIXTURE_DB_PASSWORD"});
    admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":scope,"policy":{"prod_read":connection}})).await.unwrap();
    admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":scope,"policy":{"prod_read":{"host":"new.example.test"}}})).await.unwrap();
    let current = admin_rpc(&f.state, serde_json::json!({"operation":"policy","scope":scope})).await.unwrap();
    for field in ["database","user","sslmode","password_var"] { assert_eq!(current["prod_read"][field], connection[field]); }
    assert_eq!(current["prod_read"]["host"], "new.example.test");
    admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":scope,"policy":{"prod_read":null}})).await.unwrap();
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"policy","scope":scope})).await.unwrap()["prod_read"].is_null());
    f.end().await;
}

#[tokio::test]
async fn driver_admin_partial_role_save_preserves_prose_and_default_model() {
    let f = driver_fixture(false).await;
    let role = f.state.storage.create_role(&crate::storage::RoleDraft {
        display_name: "Keep my instructions".into(), description_prompt: Some("User-authored instructions must survive".into()),
        default_model_id: Some("old".into()), capabilities: "[\"read_channel\"]".into(), participation_mode: "active".into(), ..Default::default()
    }).await.unwrap();
    admin_rpc(&f.state, serde_json::json!({"operation":"save_role","id":role.id,"draft":{
        "display_name":role.display_name,"capabilities":["read_channel","post_channel"],"participation_mode":"active"
    }})).await.unwrap();
    let after = f.state.storage.role_by_id(role.id).await.unwrap().unwrap();
    assert_eq!(after.description_prompt, role.description_prompt);
    assert_eq!(after.default_model_id, role.default_model_id);
    let read = admin_rpc(&f.state, serde_json::json!({"operation":"role","id":role.id})).await.unwrap();
    admin_rpc(&f.state, serde_json::json!({"operation":"save_role","id":role.id,"expected_sha256":read["sha256"],"draft":{}})).await.unwrap();
    let read = admin_rpc(&f.state, serde_json::json!({"operation":"role","id":role.id})).await.unwrap();
    sqlx::query("UPDATE roles SET description_prompt = 'New desktop edit' WHERE id = ?").bind(role.id).execute(f.state.storage.pool()).await.unwrap();
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"save_role","id":role.id,"expected_sha256":read["sha256"],"draft":{"display_name":"Stale save"}})).await.unwrap_err().to_string().contains("changed since"));
    admin_rpc(&f.state, serde_json::json!({"operation":"save_role","id":role.id,"draft":{"description_prompt":null,"default_model_id":null}})).await.unwrap();
    let cleared = f.state.storage.role_by_id(role.id).await.unwrap().unwrap();
    assert!(cleared.description_prompt.is_none()); assert!(cleared.default_model_id.is_none());
    assert_eq!(cleared.display_name, role.display_name);
    f.end().await;
}

#[tokio::test]
async fn driver_favorites_use_the_shared_setter_and_preserve_inventory_cursors() {
    let f = driver_fixture(false).await;
    let changed = admin_rpc(&f.state, serde_json::json!({"operation":"set_model_favorite","id":"new","favorite":true})).await.unwrap();
    assert_eq!(changed["favorite"], true);
    assert!(f.state.storage.get_model("new").await.unwrap().unwrap().favorite);
    let listed = admin_rpc(&f.state, serde_json::json!({"operation":"models"})).await.unwrap();
    let rows = listed["models"].as_array().unwrap();
    let ids: Vec<_> = rows.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]), "inventory keeps its existing ID cursor order");
    assert_eq!(rows.iter().find(|m| m["id"] == "new").unwrap()["favorite"], true);
    let page = admin_rpc(&f.state, serde_json::json!({"operation":"models","before_id":"old"})).await.unwrap();
    assert!(page["models"].as_array().unwrap().iter().all(|m| m["id"].as_str().unwrap() < "old"));
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"new","display_name":"Renamed"}})).await.unwrap();
    assert!(f.state.storage.get_model("new").await.unwrap().unwrap().favorite);
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"set_model_favorite","id":"missing","favorite":true})).await.is_err());
    f.end().await;
}

#[tokio::test]
async fn driver_admin_partial_model_save_preserves_account_endpoint_and_window() {
    let f = driver_fixture(false).await;
    let mut model = f.state.storage.get_model("old").await.unwrap().unwrap();
    model.base_url = Some("https://original.example.test".into());
    model.auth_token = Some("stored-fixture-secret".into());
    model.claude_config_dir = Some(f._temp.path().join("account-slot").to_string_lossy().to_string());
    model.context_window = Some(1_050_000);
    model.cli_settings = Some("{\"modelOverrides\":{\"old\":\"new\"}}".into());
    f.state.storage.upsert_model(&model).await.unwrap();
    let mut patch = serde_json::to_value(crate::tauri_cmd::models::ModelView::from(model.clone())).unwrap();
    for key in ["base_url","auth_token","claude_config_dir","context_window","cli_settings"] { patch.as_object_mut().unwrap().remove(key); }
    patch["display_name"] = "Rename only".into();
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":patch})).await.unwrap();
    let after = f.state.storage.get_model("old").await.unwrap().unwrap();
    assert_eq!(after.base_url, model.base_url);
    assert_eq!(after.claude_config_dir, model.claude_config_dir);
    assert_eq!(after.context_window, model.context_window);
    assert_eq!(after.cli_settings, model.cli_settings);
    assert_eq!(after.auth_token, model.auth_token);
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"old","context_window":null,"cli_settings":null,"claude_config_dir":null,"auth_token":model.auth_token}})).await.unwrap();
    let cleared = f.state.storage.get_model("old").await.unwrap().unwrap();
    assert!(cleared.context_window.is_none()); assert!(cleared.cli_settings.is_none()); assert!(cleared.claude_config_dir.is_none());
    assert_eq!(cleared.base_url, model.base_url); assert_eq!(cleared.auth_token, model.auth_token);
    f.end().await;
}

#[tokio::test]
async fn driver_admin_endpoint_changes_require_explicit_credential_intent() {
    let f = driver_fixture(false).await;
    let mut old = f.state.storage.get_model("old").await.unwrap().unwrap();
    old.auth_token = Some("private-original-fixture-token".into());
    old.base_url = Some("https://original.example.test".into());
    f.state.storage.upsert_model(&old).await.unwrap();
    for patch in [serde_json::json!({"id":"old","base_url":"https://different.example.test"}),
        serde_json::json!({"id":"old","base_url":"https://different.example.test","auth_token":null}),
        serde_json::json!({"id":"old","provider":"different-provider"}),
        serde_json::json!({"id":"old","runtime":"codex","claude_config_dir":f._temp.path().join("codex-slot")})] {
        let error = admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":patch})).await.unwrap_err();
        assert!(error.to_string().contains("re-supplying"), "{error}");
        let kept = f.state.storage.get_model("old").await.unwrap().unwrap();
        assert_eq!(kept.auth_token, old.auth_token); assert_eq!(kept.base_url, old.base_url);
    }
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"old","base_url":"https://different.example.test","auth_token":"explicit-new-fixture-token"}})).await.unwrap();
    assert_eq!(f.state.storage.get_model("old").await.unwrap().unwrap().auth_token.as_deref(), Some("explicit-new-fixture-token"));
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"old","base_url":null},"clear_auth_token":true})).await.unwrap();
    assert!(f.state.storage.get_model("old").await.unwrap().unwrap().auth_token.is_none());
    f.end().await;
}

#[tokio::test]
async fn driver_admin_cli_settings_changes_require_explicit_credential_intent() {
    let f = driver_fixture(false).await;
    let mut old = f.state.storage.get_model("old").await.unwrap().unwrap();
    old.auth_token = Some("private-original-fixture-token".into());
    old.cli_settings = Some("{\"modelOverrides\":{\"old\":\"new\"}}".into());
    f.state.storage.upsert_model(&old).await.unwrap();
    for settings in [
        serde_json::json!("{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://different.example.test\"}}"),
        serde_json::json!("{\"apiKeyHelper\":\"fixture-helper\"}"),
        serde_json::json!("{\"permissions\":{\"defaultMode\":\"bypassPermissions\"}}"),
        serde_json::Value::Null,
    ] {
        let error = admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"old","cli_settings":settings,"auth_token":null}})).await.unwrap_err();
        assert!(error.to_string().contains("re-supplying"), "{error}");
        let kept = f.state.storage.get_model("old").await.unwrap().unwrap();
        assert_eq!(kept.auth_token, old.auth_token);
        assert_eq!(kept.cli_settings, old.cli_settings);
    }
    // Formatting alone changes no settings and must not require a secret.
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"old","cli_settings":"{ \"modelOverrides\": { \"old\": \"new\" } }"}})).await.unwrap();
    assert_eq!(f.state.storage.get_model("old").await.unwrap().unwrap().auth_token, old.auth_token);
    let settings = "{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://different.example.test\"}}";
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"old","cli_settings":settings,"auth_token":"explicit-new-fixture-token"}})).await.unwrap();
    let changed = f.state.storage.get_model("old").await.unwrap().unwrap();
    assert_eq!(changed.cli_settings.as_deref(), Some(settings));
    assert_eq!(changed.auth_token.as_deref(), Some("explicit-new-fixture-token"));
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"old","cli_settings":null},"clear_auth_token":true})).await.unwrap();
    let cleared = f.state.storage.get_model("old").await.unwrap().unwrap();
    assert!(cleared.auth_token.is_none()); assert!(cleared.cli_settings.is_none());
    // The delegated editor retains full settings parity for tokenless rows.
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":{"id":"old","cli_settings":settings}})).await.unwrap();
    assert_eq!(f.state.storage.get_model("old").await.unwrap().unwrap().cli_settings.as_deref(), Some(settings));
    f.end().await;
}

#[tokio::test]
async fn driver_admin_rejects_internal_settings_and_records_the_changed_object() {
    let f = driver_fixture(false).await;
    for key in ["telemetry_enabled","telemetry_endpoint","telemetry_install_id","role_preset_offer","policy_preset_offer","tool_gate_preset_offer"] {
        let before = f.state.storage.get_setting(key).await.unwrap();
        let error = admin_rpc(&f.state, serde_json::json!({"operation":"set_setting","key":key,"value":"0"})).await.unwrap_err();
        assert!(error.to_string().contains("dedicated application control"));
        assert_eq!(f.state.storage.get_setting(key).await.unwrap(), before);
        assert!(admin_rpc(&f.state, serde_json::json!({"operation":"setting","key":key})).await.is_err());
    }
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"set_setting","key":"adherence_nudges","value":"false"})).await.is_err());
    let result = driver_rpc(&f.state, serde_json::json!({"action":"admin","request":{"operation":"save_role","draft":{"display_name":"Audited controller","capabilities":["read_channel","control_bot_hq"],"participation_mode":"active"}}})).await.unwrap();
    let detail: String = sqlx::query_scalar("SELECT detail FROM application_control_audit WHERE id = ?").bind(result["audit_id"].as_i64().unwrap()).fetch_one(f.state.storage.pool()).await.unwrap();
    let target: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert_eq!(target["resource"], "role"); assert_eq!(target["id"], result["result"]["id"]);
    assert_eq!(target["slug"], result["result"]["slug"]);
    assert!(target["capabilities"].as_array().unwrap().iter().any(|v|v == "control_bot_hq"));
    let notice: String = sqlx::query_scalar("SELECT content FROM messages WHERE session_id = 's1' AND content LIKE 'Configuration action save_role%' ORDER BY id DESC LIMIT 1").fetch_one(f.state.storage.pool()).await.unwrap();
    assert!(notice.contains(&detail));
    f.end().await;
}

#[tokio::test]
async fn driver_admin_validates_edits_preserves_credentials_and_can_revoke_itself() {
    let f = driver_fixture(false).await;
    let s = &f.state.storage;
    for request in [serde_json::json!({"operation":"models"}), serde_json::json!({"operation":"roles"}), serde_json::json!({"operation":"capabilities"}), serde_json::json!({"operation":"role","id":f.rows[0].role_id})] {
        admin_rpc(&f.state, request).await.unwrap();
    }
    sqlx::query("UPDATE models SET auth_token = 'fixture-credential' WHERE id = 'old'").execute(s.pool()).await.unwrap();
    let read = admin_rpc(&f.state, serde_json::json!({"operation":"model","id":"old"})).await.unwrap();
    assert_eq!(read["auth_configured"], true); assert!(read["model"]["auth_token"].is_null());
    let mut model = read["model"].clone(); model["display_name"] = "Renamed".into();
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":model})).await.unwrap();
    assert_eq!(s.get_model("old").await.unwrap().unwrap().auth_token.as_deref(), Some("fixture-credential"));
    admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":model,"clear_auth_token":true})).await.unwrap();
    assert!(s.get_model("old").await.unwrap().unwrap().auth_token.is_none());
    let mut bad = model.clone(); bad["runtime"] = "mistyped".into();
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":bad})).await.is_err());
    let mut bad = model; bad["claude_config_dir"] = "relative/slot".into();
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"save_model","model":bad})).await.is_err());
    let draft = serde_json::json!({"display_name":"Support", "capabilities":["read_channel"],"participation_mode":"active"});
    let saved = admin_rpc(&f.state, serde_json::json!({"operation":"save_role","draft":draft})).await.unwrap();
    let id = saved["id"].as_i64().unwrap();
    admin_rpc(&f.state, serde_json::json!({"operation":"archive_role","id":id,"archived":true})).await.unwrap();
    assert!(s.role_by_id(id).await.unwrap().unwrap().archived);
    let mut bad = draft; bad["capabilities"] = serde_json::json!(["invented_capability"]);
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"save_role","draft":bad})).await.is_err());
    admin_rpc(&f.state, serde_json::json!({"operation":"set_setting","key":"worktree_default","value":"0"})).await.unwrap();
    assert_eq!(admin_rpc(&f.state, serde_json::json!({"operation":"setting","key":"worktree_default"})).await.unwrap()["value"], "0");
    let role = s.role_by_id(f.rows[0].role_id.unwrap()).await.unwrap().unwrap();
    let draft = serde_json::json!({"display_name":role.display_name,"slug":role.slug,"description_prompt":role.description_prompt,"capabilities":["read_channel"],"participation_mode":role.participation_mode,"default_model_id":role.default_model_id});
    admin_rpc(&f.state, serde_json::json!({"operation":"save_role","id":role.id,"draft":draft})).await.unwrap();
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"models"})).await.is_err(), "current-role revocation must affect the next call");
    f.end().await;
}

async fn admin_rpc(state: &Arc<AppState>, request: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    driver_rpc(state, serde_json::json!({"action":"admin","request":request})).await.map(|r|r["result"].clone())
}

#[tokio::test]
async fn driver_admin_policy_and_tool_gate_preserve_each_other_and_frozen_sessions() {
    let f = driver_fixture(false).await;
    let global = serde_json::json!({"kind":"global"});
    let session = serde_json::json!({"kind":"session","session_id":"s1"});
    let keywords = serde_json::json!([{"keyword":"gh","mode":"gate"}]);
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":session,"policy":{"pushgate":"auto"}})).await.is_err());
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":session,"policy":{"tool_blocklist":["ignored-before"]}})).await.unwrap_err().to_string().contains("retired"));
    admin_rpc(&f.state, serde_json::json!({"operation":"set_tool_gate","scope":global,"keywords":keywords})).await.unwrap();
    admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":session,"policy":{"push_gate":"ask","force_push":"blocked"}})).await.unwrap();
    assert_eq!(admin_rpc(&f.state, serde_json::json!({"operation":"tool_gate","scope":session})).await.unwrap(), keywords);
    admin_rpc(&f.state, serde_json::json!({"operation":"set_tool_gate","scope":session,"keywords":[{"keyword":"curl","mode":"auto_allow"}]})).await.unwrap();
    assert_eq!(admin_rpc(&f.state, serde_json::json!({"operation":"policy","scope":session})).await.unwrap()["push_gate"], "ask");
    admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":global,"policy":{"push_gate":"auto"}})).await.unwrap();
    assert_eq!(admin_rpc(&f.state, serde_json::json!({"operation":"policy","scope":session})).await.unwrap()["push_gate"], "ask");
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"set_policy","scope":{"kind":"project","project":"../escape"},"policy":{}})).await.is_err());
    assert!(admin_rpc(&f.state, serde_json::json!({"operation":"tool_gate","scope":{"kind":"session","session_id":"../escape"}})).await.is_err());
    let notices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = 's1' AND origin = 'system' AND content LIKE 'Configuration action %delegated controller hands of s1%'").fetch_one(f.state.storage.pool()).await.unwrap();
    assert_eq!(notices, 4);
    f.end().await;
}

#[tokio::test]
async fn driver_admin_queued_sql_and_file_writes_lose_to_pause_and_revocation() {
    for file in [false, true] {
        for revoke in [false, true] {
            let f = driver_fixture(true).await;
            let policy_path = crate::policy::general_policy_path(&f.state.paths.data_dir);
            let before = std::fs::read(&policy_path).ok();
            let setting_before = f.state.storage.get_setting("worktree_default").await.unwrap();
            let reached = Arc::new(tokio::sync::Notify::new());
            f.state.bridge.observe_control_tickets(reached.clone());
            let mut held = f.state.storage.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
            let state = f.state.clone();
            let request = tokio::spawn(async move { admin_rpc(&state, if file {
                serde_json::json!({"operation":"set_policy","scope":{"kind":"global"},"policy":{"push_gate":"auto"}})
            } else { serde_json::json!({"operation":"set_setting","key":"worktree_default","value":"0"}) }).await });
            tokio::time::timeout(Duration::from_secs(2), reached.notified()).await.unwrap();
            if revoke {
                sqlx::query("UPDATE roles SET capabilities = '[]' WHERE id = ?").bind(f.rows[0].role_id).execute(&mut *held).await.unwrap();
            } else {
                let activity = f.state.sessions.lock().await["s1"].activity.clone();
                activity.set_paused(true); activity.set_paused(false);
            }
            held.commit().await.unwrap();
            assert!(request.await.unwrap().is_err());
            assert_eq!(f.state.storage.get_setting("worktree_default").await.unwrap(), setting_before);
            assert_eq!(std::fs::read(&policy_path).ok(), before);
            f.end().await;
        }
    }
}

#[tokio::test]
async fn driver_roster_changes_keep_provenance_and_recheck_the_reserved_boundary() {
    for mode in ["apply", "revoke", "cancel"] {
        let revoke = mode == "revoke";
        let mut f = driver_fixture(false).await;
        f.state.storage.post_to_channel("s1", "user", None, "text", "Existing task", None).await.unwrap();
        f.ring.send(SequencerCommand::UserMessage { mentions:vec![], restarts_rotation:true }).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), f.seats[0].input.recv()).await.unwrap().unwrap();
        let epoch = f.state.sessions.lock().await["s1"].participants[0].turn_epoch.as_ref().unwrap().load(Ordering::Acquire);
        let revision = f.state.storage.roster_revision("s1").await.unwrap();
        let args = serde_json::json!({"action":"change_participant","session_id":"s1","request_id":"delegated-remove","revision":revision,"request":{"kind":"remove","participant_id":f.rows[1].id}});
        let response = driver_rpc(&f.state, args.clone()).await.unwrap();
        assert_eq!(response["result"]["status"], "pending");
        assert_eq!(driver_rpc(&f.state, args).await.unwrap()["result"]["id"], "delegated-remove");
        assert!(!*f.seats[1].stopped.borrow());
        let row = f.state.storage.participant_change("s1", "delegated-remove").await.unwrap().unwrap();
        assert_eq!(row.origin().unwrap(), ParticipantChangeOrigin::Agent { session_id:"s1".into(),participant_id:f.rows[0].id });
        if mode == "cancel" {
            assert_eq!(driver_rpc(&f.state, serde_json::json!({"action":"cancel_participant_change","session_id":"s1","request_id":"delegated-remove"})).await.unwrap()["result"]["cancelled"], true);
            let found: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE session_id = 's1' AND content LIKE 'Participant change delegated-remove was cancelled by delegated controller hands of s1%')").fetch_one(f.state.storage.pool()).await.unwrap();
            assert!(found);
        }
        if revoke { sqlx::query("UPDATE roles SET capabilities = '[]' WHERE id = ?").bind(f.rows[0].role_id).execute(f.state.storage.pool()).await.unwrap(); }
        f.ring.send(SequencerCommand::TurnComplete { participant_id:f.rows[0].id, epoch, ending:crate::core::sequencer::TurnEnding::Spoke }).await.unwrap();
        let result = f.done("delegated-remove").await;
        assert_eq!(result.status, if mode == "apply" {"applied"} else {"cancelled"});
        assert_eq!(*f.seats[1].stopped.borrow(), mode == "apply");
        if revoke {
            assert!(result.error.unwrap().contains("revoked"));
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let found: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE session_id = 's1' AND content LIKE 'Participant change delegated-remove was cancelled before applying:%revoked%')").fetch_one(f.state.storage.pool()).await.unwrap();
                    if found { break; } tokio::task::yield_now().await;
                }
            }).await.unwrap();
        }
        f.end().await;
    }
}

#[tokio::test]
async fn driver_lifecycle_closes_archives_and_reopens_through_native_paths() {
    let f = driver_fixture(false).await;
    let s = &f.state.storage;
    s.create_session("target", "target", None).await.unwrap();
    s.ensure_session_roster("target", 1).await.unwrap();
    sqlx::query("UPDATE session_participants SET capabilities = '[\"read_channel\"]' WHERE session_id = 'target'")
        .execute(s.pool()).await.unwrap();
    let bridge = f.state.bridge.clone();
    let (tx, mut inputs) = mpsc::unbounded_channel();
    let starts = Arc::new(AtomicU64::new(0)); let count = starts.clone();
    f.state.set_session_starter(Arc::new(move |id| {
        let (bridge, tx, count) = (bridge.clone(), tx.clone(), count.clone());
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            let (handle, input) = crate::core::session::stub_session_for_tests(&id, &bridge).await;
            bridge.register_session_activity(id, Arc::downgrade(&handle.activity)).await;
            tx.send(input).unwrap(); Ok(handle)
        })
    }));
    driver_rpc(&f.state, serde_json::json!({"action":"start","session_id":"target"})).await.unwrap();
    let _initial = inputs.recv().await.unwrap();
    let result = driver_rpc(&f.state, serde_json::json!({"action":"close","session_id":"target","archive":true})).await.unwrap();
    assert_eq!(result["result"]["status"], "closed");
    assert_eq!(result["result"]["archived"], true);
    assert!(!f.state.sessions.lock().await.contains_key("target"));
    let result = driver_rpc(&f.state, serde_json::json!({"action":"reopen","session_id":"target"})).await.unwrap();
    assert_eq!(result["result"]["status"], "reopened");
    assert_eq!(result["result"]["startup"]["status"], "started");
    let _reopened = inputs.recv().await.unwrap();
    let row = s.get_session("target").await.unwrap().unwrap();
    assert!(row.closed_at.is_none()); assert_eq!(row.archived, 0);
    assert!(s.session_halt("target").await.unwrap().is_some(), "reopen must preserve a meaningful hold");
    let result = driver_rpc(&f.state, serde_json::json!({"action":"reopen","session_id":"target"})).await.unwrap();
    assert_eq!(result["result"]["status"], "already_open");
    assert_eq!(result["result"]["startup"]["status"], "unchanged");
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    let notices: Vec<(String, String)> = sqlx::query_as("SELECT origin, content FROM messages WHERE session_id = 'target'")
        .fetch_all(s.pool()).await.unwrap();
    assert!(notices.iter().any(|(origin,text)| origin == "system" && text.contains("closed and archived by delegated controller hands of s1")));
    assert!(notices.iter().any(|(origin,text)| origin == "system" && text.contains("reopened by delegated controller hands of s1")));
    assert!(!notices.iter().any(|(_,text)| text.contains("REOPENED by the user")));
    let words: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_user_words WHERE session_id = 'target'").fetch_one(s.pool()).await.unwrap();
    assert_eq!(words, 0);
    let mut handle = f.state.sessions.lock().await.remove("target").unwrap();
    f.state.quiesce_session_handle(&mut handle).await.unwrap();
    f.end().await;
}

#[tokio::test]
async fn driver_lifecycle_reopen_reports_startup_failure_without_reclosing() {
    let f = driver_fixture(false).await;
    f.state.storage.create_session("target", "target", None).await.unwrap();
    f.state.storage.close_session("target", true).await.unwrap();
    f.state.set_session_starter(Arc::new(|_| Box::pin(async { anyhow::bail!("fixture could not restart") })));
    let result = driver_rpc(&f.state, serde_json::json!({"action":"reopen","session_id":"target"})).await.unwrap();
    assert_eq!(result["result"]["status"], "reopened");
    assert_eq!(result["result"]["startup"]["status"], "failed");
    assert!(f.state.storage.get_session("target").await.unwrap().unwrap().closed_at.is_none());
    assert!(f.state.storage.session_halt("target").await.unwrap().is_some());
    f.end().await;
}

#[tokio::test]
async fn driver_lifecycle_self_pause_and_close_are_audited_before_interrupt() {
    for close in [false, true] {
        let f = driver_fixture(false).await;
        let (activity, atomic) = {
            let sessions = f.state.sessions.lock().await;
            (sessions["s1"].activity.clone(), sessions["s1"].in_atomic_tool.clone())
        };
        if close {
            sqlx::query("UPDATE session_participants SET capabilities = '[\"read_channel\",\"control_bot_hq\"]' WHERE session_id = 's1'")
                .execute(f.state.storage.pool()).await.unwrap();
        } else { atomic.store(true, Ordering::Release); }
        let action = if close { "close" } else { "pause" };
        let result = driver_rpc(&f.state, serde_json::json!({"action":action,"session_id":"s1"})).await.unwrap();
        let outcome: (String, Option<String>) = sqlx::query_as("SELECT outcome, detail FROM application_control_audit WHERE id = ?")
            .bind(result["audit_id"].as_i64().unwrap()).fetch_one(f.state.storage.pool()).await.unwrap();
        assert_eq!(outcome.0, "ok");
        if close {
            assert_eq!(result["result"]["status"], "closed");
            assert!(outcome.1.unwrap().contains("Session row closed"));
            assert!(f.state.storage.get_session("s1").await.unwrap().unwrap().closed_at.is_some());
            assert!(!f.state.sessions.lock().await.contains_key("s1"));
            for seat in &f.seats { assert!(*seat.stopped.borrow()); }
        } else {
            assert_eq!(result["result"]["status"], "pause_requested");
            assert!(outcome.1.unwrap().contains("runtime interruption follows"));
            assert!(activity.is_paused());
            for seat in &f.seats { assert!(!*seat.stopped.borrow(), "atomic grace must precede interruption"); }
            atomic.store(false, Ordering::Release);
            tokio::time::timeout(Duration::from_secs(4), async {
                while f.state.storage.list_cancel_events(Some("s1")).await.unwrap().is_empty() { tokio::task::yield_now().await; }
            }).await.unwrap();
            f.end().await;
        }
    }
}

#[tokio::test]
async fn driver_lifecycle_waiting_calls_lose_to_revocation_and_pause_generations() {
    for action in ["close", "reopen", "pause"] {
        for revoke in [false, true] {
            let f = driver_fixture(false).await;
            f.state.storage.create_session("target", "target", None).await.unwrap();
            if action == "reopen" { f.state.storage.close_session("target", false).await.unwrap(); }
            let activity = f.state.sessions.lock().await["s1"].activity.clone();
            let lock = f.state.lifecycle_lock("target"); let lifecycle = lock.lock().await;
            let sessions = f.state.sessions.lock().await;
            let state = f.state.clone();
            let request = tokio::spawn(async move { driver_rpc(&state, serde_json::json!({"action":action,"session_id":"target"})).await });
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM application_control_audit WHERE operation = ? AND outcome = 'started'")
                        .bind(action).fetch_one(f.state.storage.pool()).await.unwrap();
                    if count > 0 { break; } tokio::task::yield_now().await;
                }
            }).await.unwrap();
            if revoke {
                sqlx::query("UPDATE roles SET capabilities = '[]' WHERE id = ?").bind(f.rows[0].role_id)
                    .execute(f.state.storage.pool()).await.unwrap();
            } else { activity.set_paused(true); activity.set_paused(false); }
            drop(sessions); drop(lifecycle);
            assert!(request.await.unwrap().is_err(), "{action}, revoke={revoke}");
            assert_eq!(f.state.storage.get_session("target").await.unwrap().unwrap().closed_at.is_some(), action == "reopen");
            let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = 'target'")
                .fetch_one(f.state.storage.pool()).await.unwrap();
            assert_eq!(messages, 0);
            f.end().await;
        }
    }
}

#[tokio::test]
async fn driver_create_uses_dialog_picks_and_native_start_without_human_authorship() {
    let f = driver_fixture(false).await;
    let bridge = f.state.bridge.clone();
    let starts = Arc::new(AtomicU64::new(0));
    let count = starts.clone();
    let (tx, mut receivers) = mpsc::unbounded_channel();
    f.state.set_session_starter(Arc::new(move |id| {
        let (bridge, count, tx) = (bridge.clone(), count.clone(), tx.clone());
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            let (handle, input) = crate::core::session::stub_session_for_tests(&id, &bridge).await;
            bridge.register_session_activity(id, Arc::downgrade(&handle.activity)).await;
            tx.send(input).unwrap();
            Ok(handle)
        })
    }));
    let repo = f._temp.path().join("example");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let args = serde_json::json!({"action":"create", "session_id":"created", "title":"Delegated project work",
        "repo_path":repo, "project":"example", "use_worktree":true,
        "participants":[{"roleId":f.rows[0].role_id,"modelId":"new","label":"Builder","effort":"high"},
            {"roleId":f.rows[1].role_id,"modelId":"old","label":"Reviewer"}]});
    let result = driver_rpc(&f.state, args.clone()).await.unwrap();
    assert_eq!(result["result"]["status"], "created");
    assert_eq!(result["result"]["startup"]["status"], "started");
    assert_eq!(result["result"]["session"]["multi_participant"], true);
    assert_eq!(result["result"]["session"]["base_repo_path"], repo.to_str().unwrap());
    let working = result["result"]["session"]["working_repo_path"].as_str().unwrap();
    assert!(working.contains("worktrees/created/example"), "{working}");
    let _input = receivers.recv().await.unwrap();
    let rows = f.state.storage.participants_for_session("created").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].model_id.as_deref(), Some("new"));
    assert_eq!(rows[0].effort.as_deref(), Some("high"));
    assert_eq!(rows[0].label.as_deref(), Some("Builder"));
    assert_eq!(rows[1].role_id, f.rows[1].role_id);
    let notice: (String, String) = sqlx::query_as("SELECT origin, content FROM messages WHERE session_id = 'created' ORDER BY id LIMIT 1")
        .fetch_one(f.state.storage.pool()).await.unwrap();
    assert_eq!(notice.0, "system"); assert!(notice.1.contains("hands of s1"));
    let human: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_user_words WHERE session_id = 'created'")
        .fetch_one(f.state.storage.pool()).await.unwrap();
    assert_eq!(human, 0);
    assert!(driver_rpc(&f.state, args).await.is_err(), "an existing id must never be overwritten");
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let mut target = f.state.sessions.lock().await.remove("created").unwrap();
    f.state.quiesce_session_handle(&mut target).await.unwrap();
    f.end().await;
}

#[tokio::test]
async fn driver_create_reports_start_failure_and_rejects_invalid_picks_before_insert() {
    let f = driver_fixture(false).await;
    f.state.set_session_starter(Arc::new(|_| Box::pin(async { anyhow::bail!("fixture startup failure") })));
    let base = serde_json::json!({"action":"create", "session_id":"created", "title":"Work",
        "participants":[{"roleId":f.rows[0].role_id}]});
    for participants in [serde_json::json!([]), serde_json::json!([{"roleId":-1}]),
        serde_json::json!([{"roleId":f.rows[0].role_id,"modelId":"missing-model"}]),
        serde_json::json!([{"roleId":f.rows[0].role_id,"model_id":"new"}]),
        serde_json::Value::Array(vec![serde_json::json!({"roleId":f.rows[0].role_id});9])] {
        let mut invalid = base.clone(); invalid["participants"] = participants;
        assert!(driver_rpc(&f.state, invalid).await.is_err());
        assert!(f.state.storage.get_session("created").await.unwrap().is_none());
    }
    let mut unsafe_id = base.clone(); unsafe_id["session_id"] = "../escape".into();
    assert!(driver_rpc(&f.state, unsafe_id).await.is_err());
    let result = driver_rpc(&f.state, base).await.unwrap();
    assert_eq!(result["result"]["status"], "created");
    assert_eq!(result["result"]["startup"]["status"], "failed");
    assert!(result["result"]["startup"]["error"].as_str().unwrap().contains("fixture startup failure"));
    assert!(f.state.storage.get_session("created").await.unwrap().is_some());
    assert!(!f.state.sessions.lock().await.contains_key("created"));
    f.end().await;
}

#[tokio::test]
async fn driver_create_rechecks_the_claim_after_waiting_for_creation_ownership() {
    for revoked in [false, true] {
        let f = driver_fixture(false).await;
        f.state.set_session_starter(Arc::new(|_| Box::pin(async { panic!("a stale creation must never start") })));
        let lock = f.state.lifecycle_lock("created");
        let held = lock.lock().await;
        let state = f.state.clone();
        let role = f.rows[0].role_id;
        let request = tokio::spawn(async move { driver_rpc(&state, serde_json::json!({"action":"create",
            "session_id":"created", "title":"Work", "participants":[{"roleId":role}]})).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM application_control_audit WHERE operation = 'create' AND outcome = 'started'")
                    .fetch_one(f.state.storage.pool()).await.unwrap();
                if count > 0 { break; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        if revoked {
            sqlx::query("UPDATE roles SET capabilities = '[]' WHERE id = ?").bind(role)
                .execute(f.state.storage.pool()).await.unwrap();
        } else {
            let activity = f.state.sessions.lock().await["s1"].activity.clone();
            activity.set_paused(true); activity.set_paused(false);
        }
        drop(held);
        assert!(request.await.unwrap().is_err());
        assert!(f.state.storage.get_session("created").await.unwrap().is_none());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = 'created'")
            .fetch_one(f.state.storage.pool()).await.unwrap();
        assert_eq!(count, 0);
        f.end().await;
    }
}

#[tokio::test]
async fn driver_start_uses_the_native_install_path_and_resume_is_attributed() {
    let f = driver_fixture(false).await;
    f.state.storage.create_session("dormant", "dormant target", None).await.unwrap();
    f.state.storage.ensure_session_roster("dormant", 1).await.unwrap();
    let bridge = f.state.bridge.clone();
    let starts = Arc::new(AtomicU64::new(0));
    let count = starts.clone();
    let (tx, mut receivers) = mpsc::unbounded_channel();
    f.state.set_session_starter(Arc::new(move |id| {
        let (bridge, count, tx) = (bridge.clone(), count.clone(), tx.clone());
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            let (handle, input) = crate::core::session::stub_session_for_tests(&id, &bridge).await;
            bridge.register_session_activity(id, Arc::downgrade(&handle.activity)).await;
            tx.send(input).unwrap();
            Ok(handle)
        })
    }));
    let result = driver_rpc(&f.state, serde_json::json!({"action":"start","session_id":"dormant"})).await.unwrap();
    assert_eq!(result["result"]["status"], "started");
    let _input = receivers.recv().await.unwrap();
    driver_rpc(&f.state, serde_json::json!({"action":"start","session_id":"dormant"})).await.unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let activity = f.state.sessions.lock().await["dormant"].activity.clone();
    activity.set_paused(true);
    driver_rpc(&f.state, serde_json::json!({"action":"start","session_id":"dormant"})).await.unwrap();
    assert!(activity.is_paused(), "start must not silently resume a healthy paused target");
    driver_rpc(&f.state, serde_json::json!({"action":"resume","session_id":"dormant"})).await.unwrap();
    assert!(!activity.is_paused());
    let message: (String, String) = sqlx::query_as("SELECT origin, content FROM messages WHERE session_id = 'dormant' ORDER BY id DESC LIMIT 1").fetch_one(f.state.storage.pool()).await.unwrap();
    assert_eq!(message.0, "system"); assert!(message.1.contains("hands of s1"));
    let human: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_user_words WHERE session_id = 'dormant'").fetch_one(f.state.storage.pool()).await.unwrap();
    assert_eq!(human, 0);
    f.state.storage.create_session("closed", "closed target", None).await.unwrap();
    f.state.storage.close_session("closed", false).await.unwrap();
    assert!(driver_rpc(&f.state, serde_json::json!({"action":"start","session_id":"closed"})).await.is_err());
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let mut target = f.state.sessions.lock().await.remove("dormant").unwrap();
    f.state.quiesce_session_handle(&mut target).await.unwrap();
    f.end().await;
}

#[tokio::test]
async fn driver_start_rechecks_authority_after_waiting_for_the_lifecycle_lock() {
    let f = driver_fixture(false).await;
    f.state.storage.create_session("dormant", "target", None).await.unwrap();
    f.state.set_session_starter(Arc::new(|_|Box::pin(async { panic!("revoked controller must never reach spawn") })));
    let lock = f.state.lifecycle_lock("dormant");
    let held = lock.lock().await;
    let state = f.state.clone();
    let request = tokio::spawn(async move { driver_rpc(&state, serde_json::json!({"action":"start","session_id":"dormant"})).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !f.state.start_gate_is_held() { tokio::task::yield_now().await; }
    }).await.unwrap();
    sqlx::query("UPDATE roles SET capabilities = '[]' WHERE id = ?").bind(f.rows[0].role_id).execute(f.state.storage.pool()).await.unwrap();
    drop(held);
    assert!(request.await.unwrap().unwrap_err().to_string().contains("revoked"));
    assert!(!f.state.sessions.lock().await.contains_key("dormant"));
    f.end().await;
}

#[tokio::test]
async fn driver_queued_writes_lose_to_pause_revocation_and_runtime_replacement() {
    for action in ["answer", "send"] {
    for revoke in ["pause", "role", "token", "target_pause"] {
        let f = driver_fixture(true).await;
        f.state.storage.create_session("target", "other project", None).await.unwrap();
        let target_activity = crate::core::ActivityTracker::new("target", Arc::new(std::sync::atomic::AtomicBool::new(false)), f.state.bridge.clone(), vec![]);
        f.state.bridge.register_session_activity("target".into(), Arc::downgrade(&target_activity)).await;
        f.state.storage.insert_tray_entry("target", "race", "worker", crate::storage::QuestionKind::Choice, "Act?", Some(&["Yes".into()]), None, None).await.unwrap();
        let args = if action == "answer" { decision_args(&f, "race", "Yes").await }
            else { serde_json::json!({"action":"send","session_id":"target","request_id":"race-send","text":"Must not land after revocation"}) };
        let started = Arc::new(tokio::sync::Notify::new());
        f.state.bridge.observe_control_tickets(started.clone());
        let mut writer = f.state.storage.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
        let state = f.state.clone();
        let request = tokio::spawn(async move { driver_rpc(&state, args).await });
        tokio::time::timeout(Duration::from_secs(5), started.notified()).await.unwrap();
        assert!(!request.is_finished(), "the action must be queued behind our database writer");
        match revoke {
            "pause" => { let a = f.state.sessions.lock().await["s1"].activity.clone(); a.set_paused(true); a.set_paused(false); }
            "target_pause" => { target_activity.set_paused(true); target_activity.set_paused(false); }
            "role" => { sqlx::query("UPDATE roles SET capabilities = '[]' WHERE id = ?").bind(f.rows[0].role_id).execute(&mut *writer).await.unwrap(); }
            "token" => { f.state.bridge.register_mcp_token("s1", "hands", "replacement-token"); }
            _ => unreachable!(),
        }
        writer.commit().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), request).await.unwrap().unwrap();
        assert!(result.is_err(), "{action}/{revoke}: {result:?}");
        assert_eq!(f.state.storage.get_tray_entry("race").await.unwrap().unwrap().status, "pending");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = 'target'").fetch_one(f.state.storage.pool()).await.unwrap();
        assert_eq!(count, 0, "{revoke} must leave no invented answer");
        f.end().await;
    }
    }
}

fn effort_request(id: i64) -> ParticipantChangeRequest {
    ParticipantChangeRequest::SetEffort { participant_id: id, effort: "low".into(), ultracode: false }
}

async fn install_effort_fixture(f: &Fixture, mode: &'static str) {
    use crate::agents::{effort::{ClaudeReplies, EffortControl}, protocol::ControlRequest};
    let replies = ClaudeReplies::new();
    replies.observe(&serde_json::json!({"type":"system","subtype":"init","per_turn_effort_active":true}));
    let (tx, mut rx) = mpsc::channel::<ControlRequest>(8);
    let control = EffortControl::claude(replies.clone(), tx, false);
    let activity = f.state.sessions.lock().await["s1"].activity.clone();
    let storage = f.state.storage.clone();
    let role = f.rows[0].role_id;
    tokio::spawn(async move {
        let mut actual = "high".to_string(); let mut writes = 0;
        while let Some(request) = rx.recv().await {
            let mut body = serde_json::json!({});
            if request.request.subtype == "apply_flag_settings" {
                writes += 1;
                actual = request.request.fields["settings"]["effortLevel"].as_str().unwrap().to_string();
                if writes == 1 {
                    if mode == "pause" { activity.set_paused(true); activity.set_paused(false); }
                    if mode == "revoke" { sqlx::query("UPDATE roles SET capabilities='[]' WHERE id=?").bind(role).execute(storage.pool()).await.unwrap(); }
                    if mode == "close" { storage.close_session("s1", false).await.unwrap(); }
                    if mode == "unknown" { replies.observe(&serde_json::json!({"type":"system","subtype":"per_turn_effort_changed","per_turn_effort_active":false})); }
                }
            } else {
                body = serde_json::json!({"applied":{"effort":actual,"ultracode":false,"ultracodeAvailable":true}});
            }
            replies.observe(&serde_json::json!({"type":"control_response","response":{"subtype":if mode=="unknown" && writes>0 && request.request.subtype=="get_settings" {"error"} else {"success"},"request_id":request.request_id,"response":body}}));
        }
    });
    f.state.sessions.lock().await.get_mut("s1").unwrap().participants[0].handle.effort_control = control;
    f.state.storage.set_spawn_knobs(f.rows[0].id, Some("high"), Some(false)).await.unwrap();
}

#[tokio::test]
async fn effort_change_keeps_runtime_identity_and_publishes_confirmed_and_relaunch_choices() {
    let f = Fixture::new(false).await; install_effort_fixture(&f, "normal").await;
    let mut events = f.state.bridge.subscribe();
    let before = f.state.storage.participant_by_id(f.rows[0].id).await.unwrap().unwrap();
    let identity = f.state.sessions.lock().await["s1"].participants[0].handle.incarnation().id().to_string();
    let revision = f.state.storage.roster_revision("s1").await.unwrap();
    let (services, mut spawned) = services(false);
    f.start("effort", effort_request(before.id), services).await;
    assert_eq!(f.done("effort").await.status, "applied");
    let mut notified = false;
    while let Ok(event) = events.try_recv() {
        if let crate::signaling::SignalingEvent::ParticipantChanged { agent, context_reset, .. } = event {
            assert_eq!(agent, f.rows[0].slug); assert!(!context_reset); notified = true;
        }
    }
    assert!(notified, "the actual publication event preserves the meter");
    assert!(spawned.try_recv().is_err(), "effort never spawns a replacement");
    let after = f.state.storage.participant_by_id(before.id).await.unwrap().unwrap();
    assert_eq!((after.effort.as_deref(), after.ultracode, after.effort_at_spawn.as_deref(), after.ultracode_at_spawn, after.spawn_knobs_recorded), (Some("low"), Some(false), Some("low"), Some(false), true));
    assert_eq!((after.claude_session_id, after.model_id, after.account_dir_at_spawn, after.capabilities), (before.claude_session_id, before.model_id, before.account_dir_at_spawn, before.capabilities));
    assert!(f.state.storage.roster_revision("s1").await.unwrap() > revision);
    assert_eq!(f.state.sessions.lock().await["s1"].participants[0].handle.incarnation().id(), identity);
    assert!(f.state.bridge.mcp_token_matches("s1", &f.rows[0].slug, Some("token-hands")));
    assert!(!*f.seats[0].stopped.borrow() && !*f.seats[1].stopped.borrow());
    let view = crate::tauri_cmd::participant_changes::controls_with_runtime(&f.state.storage, "s1", Some(&f.state)).await.unwrap();
    assert_eq!(view.selections[0].effort.as_ref().unwrap().current.as_ref().unwrap().effort, "low");
    let runtime = f.state.sessions.lock().await;
    assert_eq!(runtime["s1"].participants[0].handle.effort_control.retry_pair().unwrap().effort, "low");
    drop(runtime);
    f.end().await;
}

#[tokio::test]
async fn effort_changes_wait_for_the_live_turn_and_can_be_cancelled_before_sending() {
    for cancel in [false, true] {
        let mut f = Fixture::new(false).await; install_effort_fixture(&f, "normal").await;
        f.state.storage.post_to_channel("s1", "user", None, "text", "work", None).await.unwrap();
        f.ring.send(SequencerCommand::UserMessage { mentions:vec![], restarts_rotation:true }).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), f.seats[0].input.recv()).await.unwrap().unwrap();
        let epoch = f.state.sessions.lock().await["s1"].participants[0].turn_epoch.as_ref().unwrap().load(Ordering::Acquire);
        let (services, mut spawned) = services(false);
        let row = f.start("effort", effort_request(f.rows[0].id), services.clone()).await;
        let replay = f.state.start_participant_change("s1", "effort", row.expected_revision, effort_request(f.rows[0].id), services).await.unwrap();
        assert_eq!(replay.id, row.id);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(f.state.storage.participant_change("s1", "effort").await.unwrap().unwrap().status, "pending");
        assert_eq!(f.state.participant_effort_status("s1").await[0].1.current.as_ref().unwrap().effort, "high");
        if cancel { assert!(f.state.cancel_participant_change("s1", "effort").await.unwrap()); }
        f.ring.send(SequencerCommand::TurnComplete { participant_id:f.rows[0].id, epoch, ending:crate::core::sequencer::TurnEnding::Spoke }).await.unwrap();
        assert_eq!(f.done("effort").await.status, if cancel { "cancelled" } else { "applied" });
        assert!(spawned.try_recv().is_err());
        assert!(!*f.seats[0].stopped.borrow());
        assert_eq!(f.state.participant_effort_status("s1").await[0].1.current.as_ref().unwrap().effort, if cancel { "high" } else { "low" });
        f.end().await;
    }
}

#[tokio::test]
async fn effort_publication_loses_to_pause_and_driver_revocation_then_restores_readback() {
    for mode in ["pause", "revoke", "close"] {
        let f = driver_fixture(false).await; install_effort_fixture(&f, mode).await;
        let revision = f.state.storage.roster_revision("s1").await.unwrap();
        let result = driver_rpc(&f.state, serde_json::json!({"action":"change_participant","session_id":"s1","request_id":"effort","revision":revision,"request":{"kind":"set_effort","participant_id":f.rows[0].id,"effort":"low","ultracode":false}})).await.unwrap();
        assert!(result["result"].is_object(), "{result}");
        assert_eq!(f.done("effort").await.status, "failed");
        let after = f.state.storage.participant_by_id(f.rows[0].id).await.unwrap().unwrap();
        assert_eq!(after.effort_at_spawn.as_deref(), Some("high"));
        assert_ne!(after.effort.as_deref(), Some("low"));
        assert_eq!(f.state.participant_effort_status("s1").await[0].1.current.as_ref().unwrap().effort, "high");
        assert!(!*f.seats[0].stopped.borrow());
        f.end().await;
    }
}

#[tokio::test]
async fn unconfirmed_effort_stops_the_runtime_and_clears_the_confirmation_badge() {
    let f = Fixture::new(false).await; install_effort_fixture(&f, "unknown").await;
    let (services, mut spawned) = services(false);
    f.start("effort", effort_request(f.rows[0].id), services).await;
    assert_eq!(f.done("effort").await.status, "failed");
    let after = f.state.storage.participant_by_id(f.rows[0].id).await.unwrap().unwrap();
    assert!(!after.spawn_knobs_recorded);
    assert_ne!(after.effort.as_deref(), Some("low"));
    assert!(f.state.storage.session_halt("s1").await.unwrap().is_some());
    assert!(spawned.try_recv().is_err());
    tokio::time::timeout(Duration::from_secs(2), async { while !*f.seats[0].stopped.borrow() { tokio::task::yield_now().await; } }).await.unwrap();
    f.end().await;
}

#[tokio::test]
async fn driver_effort_change_and_session_read_share_the_confirmed_runtime_state() {
    let f = driver_fixture(false).await; install_effort_fixture(&f, "normal").await;
    let revision = f.state.storage.roster_revision("s1").await.unwrap();
    let request = serde_json::json!({"action":"change_participant","session_id":"s1","request_id":"effort","revision":revision,"request":{"kind":"set_effort","participant_id":f.rows[0].id,"effort":"low","ultracode":false}});
    driver_rpc(&f.state, request.clone()).await.unwrap();
    let row = f.done("effort").await;
    assert_eq!(row.status, "applied");
    assert_eq!(row.origin().unwrap(), ParticipantChangeOrigin::Agent { session_id:"s1".into(), participant_id:f.rows[0].id });
    driver_rpc(&f.state, request).await.unwrap(); // idempotent reply, no second apply
    let state = driver_rpc(&f.state, serde_json::json!({"action":"session","session_id":"s1"})).await.unwrap();
    assert_eq!(state["result"]["controls"]["selections"][0]["effort"]["current"]["effort"], "low");
    assert_eq!(state["result"]["controls"]["latest_change"]["status"], "applied");
    f.end().await;
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
                    favorite: false,
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
        // The fixture's existing fake processes report medium at startup.
        for row in storage.participants_for_session("s1").await.unwrap() {
            storage.set_spawn_knobs(row.id, Some("medium"), Some(false)).await.unwrap();
        }
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
        let state = AppState::new_shared(
            crate::paths::Paths::for_data_dir(temp.path().join("data")), storage.clone(), server,
        ).await;
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
            if let crate::signaling::SignalingEvent::ParticipantChanged { session_id, agent, .. } = event {
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
