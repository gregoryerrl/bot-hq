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
    let model = std::env::var("BOT_HQ_HELPER_LIVE_MODEL")?;
    let account = std::env::var("BOT_HQ_HELPER_LIVE_ACCOUNT")?;
    let mcp = json!({"mcpServers":{"fixture":{"type":"http",
        "url":format!("http://{}/sessions/s1/hands/mcp", f.state.signaling_addr),
        "headers":{"x-bot-hq-mcp-token":"token-hands"}}}});
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
            "ToolSearch mcp__fixture__*",
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
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                (json!({"type":"user","message":{"role":"user","content":brief}}).to_string()
                    + "\n")
                    .as_bytes(),
            )
            .await?;
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut carry = None;
        while let Some(line) = lines.next_line().await? {
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

async fn codex_parent(f: &Fixture, brief: &str, trace: &mut Trace) -> anyhow::Result<()> {
    let home = std::env::var("BOT_HQ_HELPER_CODEX_HOME")?;
    let model = std::env::var("BOT_HQ_HELPER_CODEX_MODEL")?;
    let mut cmd = crate::process::tokio_command("codex");
    scrub(&mut cmd);
    for key in crate::agents::codex::spawn::CODEX_AUTH_ENV_SCRUB {
        cmd.env_remove(key);
    }
    cmd.env("CODEX_HOME", &home)
        .current_dir(f._temp.path())
        .args(["app-server", "--listen", "stdio://"]);
    for flag in crate::agents::codex::spawn::DISABLED_FEATURES {
        cmd.args(["--disable", flag]);
    }
    for config in [
        "agents.enabled=false",
        "features.hooks=false",
        "features.shell_tool=false",
        "features.code_mode.enabled=false",
        "features.view_image=false",
        "project_doc_max_bytes=0",
        "memories.generate_memories=false",
        "memories.use_memories=false",
        "web_search=\"disabled\"",
    ] {
        cmd.args(["-c", config]);
    }
    let mut child = cmd.spawn()?;
    let errors = drain_errors(&mut child);
    let (client, mut incoming) =
        CodexClient::start(child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let started = Instant::now();
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
                disabled.insert("enabled".into(), json!(false));
                servers.insert(name.clone(), Value::Object(disabled));
            }
        }
        // No tool_timeout_sec: measure the connector's ordinary timeout.
        servers.insert("bot_hq".into(), json!({"url":format!("http://{}/sessions/s1/hands/mcp",f.state.signaling_addr),
            "http_headers":{"x-bot-hq-mcp-token":"token-hands"},"enabled":true,"required":true,
            "default_tools_approval_mode":"approve"}));
        let thread = client.request("thread/start", json!({"model":model,"cwd":cwd,"ephemeral":true,
            "approvalPolicy":"never","sandbox":"read-only","baseInstructions":"Execute the MCP acceptance brief precisely. Use only the fixture MCP tools; no files, shell, network browsing or other sessions.",
            "config":{"mcp_servers":servers}})).await?;
        let id = thread["thread"]["id"].as_str().context("no native thread")?;
        let inventory = client.request("mcpServerStatus/list",json!({"threadId":id,"limit":100})).await?;
        anyhow::ensure!(inventory["data"].as_array().is_some_and(|servers| servers.iter().any(|s|
            s["name"]=="bot_hq" && s["tools"].as_object().is_some_and(|tools|tools.contains_key("delegate_tasks")))),
            "fixture MCP tool was not registered: {inventory}");
        client.request("turn/start", json!({"threadId":id,"input":[{"type":"text","text":brief}],"effort":"low"})).await?;
        let mut translator = crate::agents::codex::events::Translator::new(model,"Sign in to the chosen probe account.");
        while let Some(incoming) = incoming.recv().await {
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
    let errors = errors.await?;
    result
        .context("native Codex parent exceeded 180 seconds")?
        .with_context(|| format!("Codex stderr: {errors}"))
}

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
        codex_parent(&f, &brief, &mut trace).await
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
