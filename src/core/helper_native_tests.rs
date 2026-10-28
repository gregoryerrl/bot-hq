//! Opt-in native MCP transport checks against an isolated AppState/turn fixture.
//! The parent CLI is real; the held ring seat is a recorder. Long helpers use a
//! deterministic 65-second child. The cross-runtime test uses a real Claude child.
use super::*;
use crate::agents::codex::client::{CodexClient, Incoming};
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

type Trace = Vec<(f64, AgentEvent)>;

fn scrub(cmd: &mut tokio::process::Command) {
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("BOT_HQ_") || name.starts_with("CODEX_") || name == "CLAUDECODE" {
            cmd.env_remove(key);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.as_std_mut().process_group(0);
    }
}

async fn stop(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        crate::process::kill_process_tree(pid);
        #[cfg(unix)]
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

fn drain_errors(child: &mut tokio::process::Child) -> tokio::task::JoinHandle<String> {
    let stderr = child.stderr.take().unwrap();
    tokio::spawn(async move {
        let mut bytes = vec![];
        // Drain all bytes so a noisy CLI cannot block; retain only a bounded tail.
        let mut input = BufReader::new(stderr);
        let mut buffer = [0; 4096];
        while let Ok(n) = input.read(&mut buffer).await {
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..n]);
            if bytes.len() > 16_384 {
                bytes.drain(..bytes.len() - 16_384);
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

async fn claude_parent(f: &Fixture, brief: &str, trace: &mut Trace) -> anyhow::Result<()> {
    claude_probe(f, brief, trace, None).await
}

// Some(false): a long direct gate call. Some(true): interrupt it while the
// native client stays alive, so killing the test CLI cannot hide a leak.
async fn claude_probe(f: &Fixture, brief: &str, trace: &mut Trace, gate: Option<bool>) -> anyhow::Result<()> {
    let model = std::env::var("BOT_HQ_HELPER_LIVE_MODEL")?;
    let account = std::env::var("BOT_HQ_HELPER_LIVE_ACCOUNT")?;
    let mcp = if gate.is_some() {
        serde_json::from_str(&crate::signaling::mcp_config_json(f.state.signaling_addr,
            "s1", "hands", Some("token-hands"), &serde_json::Map::new()))?
    } else { json!({"mcpServers":{"fixture":{"type":"http",
        "url":format!("http://{}/sessions/s1/hands/mcp", f.state.signaling_addr),
        "headers":{"x-bot-hq-mcp-token":"token-hands"}}}}) };
    let mut cmd = crate::process::tokio_command("claude");
    scrub(&mut cmd);
    crate::agents::spawn::apply_account_env(&mut cmd, Some(&account));
    cmd.current_dir(f._temp.path())
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
        ])
        .args([
            "--model",
            &model,
            "--tools",
            "ToolSearch",
            "--allowedTools",
            "ToolSearch mcp__fixture__* mcp__bot-hq-signaling__*",
        ])
        .args([
            "--permission-mode",
            "dontAsk",
            "--setting-sources",
            "",
            "--strict-mcp-config",
        ])
        .args([
            "--mcp-config",
            &mcp.to_string(),
            "--no-chrome",
            "--disable-slash-commands",
            "--no-session-persistence",
        ])
        .args(["--settings", "{\"crossSessionInbound\":\"refuse\"}"])
        .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
        .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .env_remove("MCP_TOOL_TIMEOUT"); // Exercise the native default, not an enlarged ceiling.
    let mut child = cmd.spawn()?;
    let errors = drain_errors(&mut child);
    let started = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(180), async {
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(
                (json!({"type":"user","message":{"role":"user","content":brief}}).to_string()
                    + "\n")
                    .as_bytes(),
            )
            .await?;
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut carry = None;
        let interrupt = gate_interrupt_due(f, gate);
        tokio::pin!(interrupt);
        let mut interrupted = false;
        loop {
            let line = tokio::select! {
                line = lines.next_line() => line?,
                _ = &mut interrupt, if !interrupted => {
                    interrupted = true;
                    stdin.write_all((serde_json::to_string(&crate::agents::protocol::ControlRequest::interrupt("gate-probe"))? + "\n").as_bytes()).await?;
                    f.state.cancel_and_escalate("s1",crate::storage::now_utc()).await?;
                    eprintln!("native Claude interrupt sent at {:.2}s", started.elapsed().as_secs_f64());
                    observe_gate_cancellation(f).await?;
                    continue;
                }
            };
            let Some(line) = line else { break; };
            let Ok(event) = serde_json::from_str::<crate::agents::protocol::StreamEvent>(&line)
            else {
                continue;
            };
            for event in crate::agents::events::translate(event, &mut carry) {
                let done = matches!(event, AgentEvent::TurnComplete { .. });
                trace.push((started.elapsed().as_secs_f64(), event));
                if done {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        }
        anyhow::bail!("Claude stream ended without completion")
    })
    .await;
    stop(&mut child).await;
    let errors = errors.await?;
    result
        .context("native Claude parent exceeded 180 seconds")?
        .with_context(|| format!("Claude stderr: {errors}"))
}

async fn codex_parent(f: &Fixture, brief: &str, trace: &mut Trace, normal_discovery: bool) -> anyhow::Result<()> {
    codex_probe(f, brief, trace, normal_discovery, None).await
}

async fn codex_probe(f: &Fixture, brief: &str, trace: &mut Trace, normal_discovery: bool, gate: Option<bool>) -> anyhow::Result<()> {
    let source_home = std::env::var("BOT_HQ_HELPER_CODEX_HOME")?;
    // Direct-command probes get their own ephemeral home. Only credentials
    // are copied; shared hooks, catalogs and conversation files are untouched.
    let isolated_home = if gate.is_some() { Some(tempfile::tempdir()?) } else { None };
    let home = if let Some(dir) = isolated_home.as_ref() {
        let auth = dir.path().join("auth.json");
        std::fs::copy(std::path::Path::new(&source_home).join("auth.json"), &auth)?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&auth, std::fs::Permissions::from_mode(0o600))?;
        }
        dir.path().to_string_lossy().into_owned()
    } else { source_home };
    let model = std::env::var("BOT_HQ_HELPER_CODEX_MODEL")?;
    let mut cmd = if normal_discovery {
        // Take MCP exposure/auth from the actual production command so this
        // acceptance test fails when that join regresses, not only its copy.
        let codex = crate::agents::codex::spawn::CodexSpawn {
            bin:None, mcp_url:format!("http://{}/sessions/s1/hands/mcp",f.state.signaling_addr),
            mcp_token:"token-hands".into(),hook_trust:vec![],hook_exe:None,model_catalog:None,
        };
        let cfg = crate::agents::SpawnConfig {
            agent_name:"hands".into(),session_id:"s1".into(),project:None,
            config:crate::storage::AgentConfig {agent_name:"hands".into(),provider:"openai".into(),
                model_name:model.clone(),base_url:None,auth_token:None,updated_at:String::new(),
                context_window:None,cli_settings:None,claude_config_dir:Some(home.clone()),runtime:"codex".into()},
            system_prompt_path:f._temp.path().join("unused-prompt"),mcp_config_path:None,
            working_dir:Some(f._temp.path().into()),claude_bin:None,resume_session_id:None,
            data_dir:f._temp.path().join("data"),capabilities:crate::agents::ResolvedCapabilities::Known(
                crate::agents::CapabilitySet::from_slugs(&["read_channel","delegate_tasks"])),
            overrides:Default::default(),blank_cli_attribution:true,account_dir:Some(home.clone()),
            account_email:None,account_org:None,codex:Some(codex.clone()),forwarded_mcp:Vec::new(),
        };
        crate::agents::codex::spawn::build_codex_command(&cfg,&codex)
    } else {
        let mut cmd=crate::process::tokio_command("codex");
        cmd.args(["app-server","--listen","stdio://"]);
        for flag in crate::agents::codex::spawn::DISABLED_FEATURES {cmd.args(["--disable",flag]);}
        cmd
    };
    scrub(&mut cmd);
    for key in crate::agents::codex::spawn::CODEX_AUTH_ENV_SCRUB {
        cmd.env_remove(key);
    }
    cmd.env("CODEX_HOME", &home)
        .env("BOT_HQ_MCP_TOKEN","token-hands")
        .current_dir(f._temp.path());
    for config in [
        "agents.enabled=false",
        "features.hooks=false",
        "features.shell_tool=false",
        "features.view_image=false",
        "project_doc_max_bytes=0",
        "memories.generate_memories=false",
        "memories.use_memories=false",
        "web_search=\"disabled\"",
    ] {
        cmd.args(["-c", config]);
    }
    cmd.args(["-c", if normal_discovery {"features.code_mode.enabled=true"} else {"features.code_mode.enabled=false"}]);
    let mut child = cmd.spawn()?;
    let errors = drain_errors(&mut child);
    let (client, mut incoming) =
        CodexClient::start(child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let started = Instant::now();
    let mut raw_gate_events = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(180), async {
        client.request("initialize", json!({"clientInfo":{"name":"bot-hq-helper-mcp-probe","version":"1"},"capabilities":{"experimentalApi":true}})).await?;
        client.notify("initialized", json!({})).await?;
        let cwd = f._temp.path().canonicalize()?;
        let config = client.request("config/read", json!({"includeLayers":false,"cwd":cwd})).await?;
        let mut servers = serde_json::Map::new();
        if let Some(configured) = config["config"]["mcp_servers"].as_object() {
            for (name, value) in configured {
                let mut disabled = value.as_object().context("invalid inherited server")?.clone();
                disabled.retain(|_,value| !value.is_null());
                if !normal_discovery || name!="bot_hq" {disabled.insert("enabled".into(), json!(false));}
                servers.insert(name.clone(), Value::Object(disabled));
            }
        }
        // No tool_timeout_sec: measure the connector's ordinary timeout.
        if !normal_discovery {servers.insert("bot_hq".into(), json!({"url":format!("http://{}/sessions/s1/hands/mcp",f.state.signaling_addr),
            "http_headers":{"x-bot-hq-mcp-token":"token-hands"},"enabled":true,"required":true,
            "default_tools_approval_mode":"approve"}));}
        let thread = client.request("thread/start", json!({"model":model,"cwd":cwd,"ephemeral":true,
            "approvalPolicy":"never","sandbox":"read-only","baseInstructions":"Execute the MCP acceptance brief precisely. Use only the fixture MCP tools; no files, shell, network browsing or other sessions.",
            "config":{"mcp_servers":servers}})).await?;
        let id = thread["thread"]["id"].as_str().context("no native thread")?;
        let inventory = client.request("mcpServerStatus/list",json!({"threadId":id,"limit":100})).await?;
        let expected_tool = if gate.is_some() {"action_gate"} else {"delegate_tasks"};
        anyhow::ensure!(inventory["data"].as_array().is_some_and(|servers| servers.iter().any(|s|
            s["name"]=="bot_hq" && s["tools"].as_object().is_some_and(|tools|tools.contains_key(expected_tool)))),
            "fixture MCP tool was not registered: {inventory}");
        let turn = client.request("turn/start", json!({"threadId":id,"input":[{"type":"text","text":brief}],"effort":"low"})).await?;
        let turn_id = turn["turn"]["id"].as_str().context("no native turn")?;
        let mut translator = crate::agents::codex::events::Translator::new(model,"Sign in to the chosen probe account.");
        let interrupt = gate_interrupt_due(f, gate);
        tokio::pin!(interrupt);
        let mut interrupted = false;
        loop {
            let incoming = tokio::select! {
                incoming = incoming.recv() => incoming,
                _ = &mut interrupt, if !interrupted => {
                    interrupted = true;
                    client.request("turn/interrupt", json!({"threadId":id,"turnId":turn_id})).await?;
                    f.state.cancel_and_escalate("s1",crate::storage::now_utc()).await?;
                    eprintln!("native Codex interrupt sent at {:.2}s", started.elapsed().as_secs_f64());
                    observe_gate_cancellation(f).await?;
                    continue;
                }
            };
            let Some(incoming) = incoming else { break; };
            if gate.is_some() { raw_gate_events.push(format!("{:.3} {incoming:?}", started.elapsed().as_secs_f64())); }
            anyhow::ensure!(incoming != Incoming::Closed,"Codex stream ended without completion");
            for event in translator.translate(&incoming) {
                let done = matches!(event, AgentEvent::TurnComplete { .. });
                trace.push((started.elapsed().as_secs_f64(), event));
                if done { return Ok::<_,anyhow::Error>(()); }
            }
        }
        anyhow::bail!("Codex stream ended without completion")
    }).await;
    stop(&mut child).await;
    if let (Some(pause), Ok(prefix)) = (gate, std::env::var("BOT_HQ_HELPER_PROBE_LOG")) {
        std::fs::write(format!("{prefix}-codex-gate-{pause}-raw.log"), raw_gate_events.join("\n"))?;
    }
    let errors = errors.await?;
    result
        .context("native Codex parent exceeded 180 seconds")?
        .with_context(|| format!("Codex stderr: {errors}"))
}

async fn gate_interrupt_due(f: &Fixture, gate: Option<bool>) {
    if gate != Some(true) { return std::future::pending().await; }
    while !f._temp.path().join("gate-grandchild").exists() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    tokio::time::sleep(Duration::from_secs(10)).await;
}

pub(super) fn gate_pids(f: &Fixture) -> Vec<u32> {
    ["gate-leader", "gate-child", "gate-grandchild"].into_iter()
        .filter_map(|name| std::fs::read_to_string(f._temp.path().join(name)).ok()?.trim().parse().ok())
        .collect()
}

#[cfg(not(unix))]
async fn observe_gate_cancellation(_: &Fixture) -> anyhow::Result<()> {
    anyhow::bail!("native command-group probe is Unix-only")
}

#[cfg(unix)]
pub(super) async fn observe_gate_cancellation(f: &Fixture) -> anyhow::Result<()> {
    let pids = gate_pids(f);
    anyhow::ensure!(pids.len() == 3, "gate must have started all three processes");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    while tokio::time::Instant::now() < deadline {
        if pids.iter().all(|pid| crate::process::is_gone(*pid)) {
            eprintln!("gate command processes stopped before fixture-client teardown: {pids:?}");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("native interrupt left gate processes running: {:?}",
        pids.into_iter().filter(|pid| !crate::process::is_gone(*pid)).collect::<Vec<_>>())
}

#[cfg(unix)]
pub(super) async fn gate_fixture() -> Fixture {
    let (f,_) = helper_fixture().await;
    for table in ["roles", "session_participants"] {
        sqlx::query(&format!("UPDATE {table} SET capabilities='[\"read_channel\",\"gated_bash\",\"edit_files\"]' WHERE slug='hands'"))
            .execute(f.state.storage.pool()).await.unwrap();
    }
    std::fs::write(f._temp.path().join("gate-command.sh"),
        "echo $$ > gate-child\nsleep 70 &\necho $! > gate-grandchild\nwait\necho GATE_NATIVE_PELICAN >> gate-executions\necho GATE_NATIVE_PELICAN\n").unwrap();
    f
}

#[tokio::test]
#[cfg(unix)]
async fn dropping_the_http_gate_request_cleans_up_its_command_group() {
    let f = gate_fixture().await;
    let url = format!("http://{}/sessions/s1/hands/mcp", f.state.signaling_addr);
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"action_gate","arguments":{"command":"echo $$ > gate-leader; sh gate-command.sh; :", "timeout_secs":100}}});
    let call = tokio::spawn(async move {
        reqwest::Client::new().post(url).header("x-bot-hq-mcp-token", "token-hands")
            .header("content-type", "application/json").body(request.to_string()).send().await
    });
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        while gate_pids(&f).len()!=3 { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await;
    call.abort();
    let _ = call.await;
    let cleanup = observe_gate_cancellation(&f).await;
    for pid in gate_pids(&f) { if !crate::process::is_gone(pid) { crate::process::kill_process_tree(pid); } }
    f.end().await;
    assert!(started.is_ok(), "HTTP command never started");
    assert!(cleanup.is_ok(), "{cleanup:?}");
}

#[cfg(unix)]
async fn native_gate(runtime: &str, pause: bool) {
    let f = gate_fixture().await;
    let request = json!({"command":"echo $$ > gate-leader; sh gate-command.sh; echo GATE_SHELL_FINISHED", "timeout_secs":100});
    let brief = format!("Native MCP acceptance against disposable bot-hq storage. Discover and call action_gate EXACTLY ONCE with these arguments: {request}. Use the bot-hq MCP tool, not a shell tool. In code mode use functions.exec with tools.<discovered action_gate name> and collect its result with bounded waits as needed. The command intentionally takes 70 seconds. Never retry it after an error. Report the actual tool result and finish; do not use any other tool except MCP discovery and code-cell waits. No permissions or real projects are involved.");
    let mut trace = vec![];
    let result = if runtime == "claude" {
        claude_probe(&f, &brief, &mut trace, Some(pause)).await
    } else {
        codex_probe(&f, &brief, &mut trace, true, Some(pause)).await
    };
    let executions = std::fs::read_to_string(f._temp.path().join("gate-executions")).unwrap_or_default();
    let notices: Vec<(String,String)> = sqlx::query_as("SELECT origin,content FROM messages WHERE session_id='s1' AND kind='system_notice' AND content LIKE 'Direct command result%'")
        .fetch_all(f.state.storage.pool()).await.unwrap();
    // Cleanup happens even on a failing probe, after observing cancellation
    // with the CLI still alive. No host/app PID is involved.
    for pid in gate_pids(&f) { if !crate::process::is_gone(pid) { crate::process::kill_process_tree(pid); } }
    f.end().await;
    if let Ok(prefix) = std::env::var("BOT_HQ_HELPER_PROBE_LOG") {
        std::fs::write(format!("{prefix}-{runtime}-gate-{pause}.log"), format!("result={result:?}\nexecutions={executions:?}\n{trace:#?}")).unwrap();
    }
    assert!(result.is_ok(), "native {runtime}: {result:?}\n{trace:#?}");
    assert_eq!(notices.len(),1,"each direct outcome must survive a lost client reply: {notices:?}");
    assert_eq!(notices[0].0,"system");
    if pause { assert!(notices[0].1.contains("outcome unknown"),"{notices:?}"); }
    let calls: Vec<_> = trace.iter().filter_map(|(at,event)| match event {
        AgentEvent::ToolUse{id,name,..} if name.ends_with("action_gate") => Some((at,id)), _ => None
    }).collect();
    assert_eq!(calls.len(), 1, "must execute one real MCP call: {trace:#?}");
    if pause { assert!(executions.is_empty()); return; }
    let (start,id) = calls[0];
    let (end,error,content) = trace.iter().find_map(|(at,event)| match event {
        AgentEvent::ToolResult{tool_use_id,is_error,content} if tool_use_id==id => Some((at,is_error,content)), _ => None
    }).expect("the native MCP call must have a result");
    assert!(!error && content.contains("GATE_NATIVE_PELICAN"), "{content}");
    assert_eq!(executions.lines().count(), 1);
    assert!(end-start >= 70. && end-start < 120., "call duration {}s", end-start);
    let waits = trace.iter().filter(|(_,e)| matches!(e,AgentEvent::ToolUse{name,..} if name.contains("wait"))).count();
    eprintln!("native {runtime} direct action_gate: {:.2}s, one execution, {waits} wait calls", end-start);
}

#[tokio::test]
#[cfg(unix)]
#[ignore = "spends a Claude turn; set BOT_HQ_HELPER_LIVE_MODEL and BOT_HQ_HELPER_LIVE_ACCOUNT"]
async fn claude_direct_gate_crosses_sixty_seconds() { native_gate("claude", false).await; }

#[tokio::test]
#[cfg(unix)]
#[ignore = "spends a Codex turn; set BOT_HQ_HELPER_CODEX_HOME and BOT_HQ_HELPER_CODEX_MODEL"]
async fn codex_direct_gate_crosses_sixty_seconds() { native_gate("codex", false).await; }

#[tokio::test]
#[cfg(unix)]
#[ignore = "spends a Claude turn; native interrupt while a direct gate is running"]
async fn claude_pause_cancels_direct_gate() { native_gate("claude", true).await; }

#[tokio::test]
#[cfg(unix)]
#[ignore = "spends a Codex turn; native interrupt through the production code-mode route"]
async fn codex_pause_cancels_direct_gate() { native_gate("codex", true).await; }

async fn native_batch(runtime: &str, real_helper: bool) {
    let (f, _) = helper_fixture().await;
    // Every tool is scoped to this disposable storage, and the native caller
    // has only the grant needed by this probe, never a real-session identity.
    for table in ["roles", "session_participants"] {
        sqlx::query(&format!("UPDATE {table} SET capabilities='[\"read_channel\",\"delegate_tasks\"]' WHERE slug='hands'"))
            .execute(f.state.storage.pool()).await.unwrap();
    }
    std::fs::write(
        f._temp.path().join("source.rs"),
        "// HELPER_NATIVE_PELICAN\n",
    )
    .unwrap();
    let count = Arc::new(AtomicU64::new(0));
    let total = count.clone();
    f.state.helpers.set_spawner(Arc::new(move |cfg,brief| {
        total.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if real_helper {return crate::agents::spawn::spawn_claude_helper(cfg,brief).await;}
            let (handle, mut seat) = fake_agent(&cfg.session_id,&cfg.agent_name);
            tokio::spawn(async move {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(65)) => {
                        let _ = seat.events.send(AgentEvent::Text("source.rs:1 HELPER_NATIVE_PELICAN".into())).await;
                        let _ = seat.events.send(AgentEvent::TurnComplete {stop_reason:None,subtype:Some("success".into()),is_error:false,api_error_status:None,
                            context:crate::agents::spawn::ContextReport::none(crate::agents::spawn::ContextVerdict::NoUsage)}).await;
                    }
                    _ = seat.stopped.changed() => {}
                }
            });
            Ok(handle)
        })
    }));
    if real_helper {
        sqlx::query("UPDATE models SET provider='anthropic',model_name=?,claude_config_dir=?,context_window=NULL WHERE id='old'")
            .bind(std::env::var("BOT_HQ_HELPER_LIVE_MODEL").unwrap())
            .bind(std::env::var("BOT_HQ_HELPER_LIVE_ACCOUNT").unwrap())
            .execute(f.state.storage.pool()).await.unwrap();
    }
    let request = json!({"action":"start","request_id":"native","tasks":[{"goal":"Read source.rs and quote its first line with a path:line citation.",
        "files":["source.rs"],"expected_evidence":"source.rs:1 followed by its exact contents","model_id":"old"}],"timeout_secs":140});
    let brief = format!("This is a live MCP acceptance test against a disposable bot-hq fixture. Discover delegate_tasks if needed. Call it with these exact arguments: {request}. Repeat that IDENTICAL start once to verify idempotency. Then call delegate_tasks with action=status, request_id=native, wait_secs=45 repeatedly until terminal. Running/starting is expected; the deterministic helper may take 65 seconds. Do not cancel or finish your turn while it is running. Do not call any other tool except MCP discovery. Once completed, report the returned helper finding verbatim. No need to ask permission.");
    let mut trace = vec![];
    let result = if runtime == "claude" {
        claude_parent(&f, &brief, &mut trace).await
    } else {
        codex_parent(&f, &brief, &mut trace, real_helper).await
    };
    if let Ok(prefix) = std::env::var("BOT_HQ_HELPER_PROBE_LOG") {
        std::fs::write(
            format!(
                "{prefix}-{runtime}-{}.log",
                if real_helper { "real" } else { "long" }
            ),
            format!("result={result:?}\n{trace:#?}"),
        )
        .unwrap();
    }
    let row = f
        .state
        .storage
        .helper_batch("s1", f.rows[0].id, "native")
        .await
        .unwrap();
    let doc = f
        .state
        .storage
        .session_document_by_slug("s1", "helpers")
        .await
        .unwrap();
    // Always clean up before asserting, including failed native attempts.
    f.end().await;
    assert!(result.is_ok(), "native {runtime}: {result:?}\n{trace:#?}");
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "start retries must share one child"
    );
    let row = row.expect("native parent must actually call start");
    assert_eq!(row.status, "completed", "{row:?}\n{trace:#?}");
    let saved: Value = serde_json::from_str(row.result_json.as_deref().unwrap()).unwrap();
    assert_eq!(saved["helpers"][0]["cleanup_confirmed"], true);
    assert!(saved["helpers"][0]["output"]
        .as_str()
        .unwrap()
        .contains("HELPER_NATIVE_PELICAN"));
    assert!(doc.unwrap().body.contains("HELPER_NATIVE_PELICAN"));
    let calls: Vec<_> = trace
        .iter()
        .filter_map(|(t, e)| match e {
            AgentEvent::ToolUse { id, name, input } if name.ends_with("delegate_tasks") => {
                Some((t, id, input))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        calls
            .iter()
            .filter(|(_, _, input)| input["action"] == "start")
            .count(),
        2
    );
    let statuses: Vec<_> = calls
        .iter()
        .filter(|(_, _, input)| input["action"] == "status")
        .collect();
    assert!(
        statuses.len() >= if real_helper { 1 } else { 2 },
        "must observe the long-running batch across calls"
    );
    for (start, id, _) in &calls {
        let (end, error, content) = trace
            .iter()
            .find_map(|(t, e)| match e {
                AgentEvent::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } if tool_use_id == *id => Some((t, is_error, content)),
                _ => None,
            })
            .expect("every native MCP call needs a result");
        assert!(!error, "{content}");
        assert!(
            end - **start < 60.,
            "individual MCP call exceeded the native timeout"
        );
    }
    assert!(trace.iter().any(|(_,e)| matches!(e,AgentEvent::ToolResult {content,is_error:false,..} if content.contains("HELPER_NATIVE_PELICAN"))),"native parent must receive the findings, not just start the work");
    assert!(trace.iter().any(|(_, e)| matches!(
        e,
        AgentEvent::TurnComplete {
            is_error: false,
            ..
        }
    )));
    if !real_helper {
        assert!(trace.last().unwrap().0 >= 65.);
    }
    if real_helper {
        assert!(saved["helpers"][0]["transcript"]
            .as_str()
            .unwrap()
            .contains("Read"));
    }
    eprintln!("{runtime} parent / {} helper: one launch, {} status calls, {:.1}s native turn, findings received and cleanup confirmed",if real_helper {"native Claude"}else{"65-second fixture"},statuses.len(),trace.last().unwrap().0);
}

#[tokio::test]
#[ignore = "spends a Claude turn; set BOT_HQ_HELPER_LIVE_MODEL and BOT_HQ_HELPER_LIVE_ACCOUNT"]
async fn claude_mcp_waits_across_sixty_second_helper() {
    native_batch("claude", false).await;
}

#[tokio::test]
#[ignore = "spends a Codex turn; set BOT_HQ_HELPER_CODEX_HOME and BOT_HQ_HELPER_CODEX_MODEL"]
async fn codex_mcp_waits_across_sixty_second_helper() {
    native_batch("codex", false).await;
}

#[tokio::test]
#[ignore = "spends a Codex parent and Claude helper turn; set both runtime model/account pairs"]
async fn codex_parent_reads_real_claude_helper_result() {
    native_batch("codex", true).await;
}
