//! The Codex connector against the REAL `codex` binary and a signed-in,
//! bot-hq-managed home — one turn, billed to the account. Ignored by
//! default; run it on purpose:
//!
//! ```text
//! BOT_HQ_CODEX_LIVE_HOME=/Users/you/.codex-bot-hq cargo test --test codex_live_test -- --ignored --nocapture
//! ```
//!
//! What it proves that no fake can: the hooks file bot-hq writes is listed and
//! trusted through `-c hooks.state`, the gate hook runs the built `bot-hq`
//! binary and parks a gated command on the live signaling server, bot-hq's
//! MCP server answers the Codex client with the secret in a header, a
//! real-size (150 KB+) developer-instructions text rides `thread/start` to
//! its last line, and the events come back in the vocabulary the pump reads.

use bot_hq::agents::codex::home::{hook_key, write_hooks_json, RenderedHook};
use bot_hq::agents::codex::probe::{probe_home, CodexProbeRequest};
use bot_hq::agents::codex::spawn::{spawn_codex_agent, CodexSpawn};
use bot_hq::agents::spawn::{AgentEvent, SpawnConfig};
use bot_hq::agents::{CapabilitySet, ResolvedCapabilities};
use bot_hq::signaling::{start_signaling_server, SignalingBridge};
use bot_hq::storage::{AgentConfig, Storage, MAX_SESSION_PARTICIPANTS};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
#[ignore = "spends one Codex turn on a real account; set BOT_HQ_CODEX_LIVE_HOME to run"]
async fn a_codex_participant_runs_one_gated_turn_for_real() {
    let Ok(home) = std::env::var("BOT_HQ_CODEX_LIVE_HOME") else {
        eprintln!("BOT_HQ_CODEX_LIVE_HOME is not set; nothing to run against");
        return;
    };
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_bot-hq"));
    let data = tempfile::tempdir().unwrap();
    let data_dir = data.path().to_path_buf();
    std::fs::create_dir_all(data_dir.join(".local")).unwrap();
    std::fs::create_dir_all(data_dir.join("config")).unwrap();
    std::fs::write(data_dir.join("config/tool-gate.json"), r#"[{"keyword":"gated-probe","mode":"gate"}]"#).unwrap();

    // The live app's shape, in-process: storage with a roster, the bridge, the
    // signaling server, the addr + hook-token files the hook binary reads.
    let storage = Storage::memory().await.unwrap();
    storage.create_session("s-live", "codex live", None).await.unwrap();
    storage.ensure_session_roster("s-live", MAX_SESSION_PARTICIPANTS).await.unwrap();
    let bridge = SignalingBridge::new();
    bridge.set_storage(storage.clone()).await;
    let hook_token = "live-hook-token";
    bridge.set_hook_token(hook_token);
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    std::fs::write(data_dir.join(".local/signaling-addr"), format!("{}\n", server.local_addr)).unwrap();
    std::fs::write(data_dir.join(".local/hook-token"), format!("{hook_token}\n")).unwrap();
    let mcp_token = "live-mcp-secret";
    bridge.register_mcp_token("s-live", "hands", mcp_token);

    // The pre-spawn probe: the hooks file first, then the trust hash.
    write_hooks_json(Path::new(&home), &exe, &data_dir).unwrap();
    let rendered = RenderedHook::new(Path::new(&home), &exe, &data_dir);
    let probe = probe_home(&CodexProbeRequest { bin: None, home: home.clone(), cwd: Some("/tmp".into()) }).await;
    eprintln!("probe: auth={:?} version={} models={:?} hooks={}", probe.auth, probe.version, probe.models, probe.hooks.len());
    let trust = probe.hooks.iter().find_map(|h| rendered.trust_for(h)).expect("bot-hq's hook is listed in the home");
    assert_eq!(trust.key, hook_key(Path::new(&home)));

    // A prompt the size of a real role prompt (this session's measured
    // 96–167 KB), with the instructions at the START and a codeword at the
    // END the model must quote: that proves the whole text rode
    // `thread/start` (EYES N2), not merely its first lines.
    let codeword = "PELICAN-4471";
    let filler: String = (0..1400)
        .map(|i| format!("Background note {i}: the Context Library is advisory and reality wins; verify against the live code before trusting an atom.\n"))
        .collect();
    let prompt = format!(
        "You are a probe inside bot-hq. Be terse. Do exactly these steps in order and report each in one line: \
         Step 1: call the MCP tool cl_index_search with project \"bot-hq\" and query \"conventions\"; report how many rows came back. \
         Step 2: run the shell command: echo gated-probe ; report exactly what happened. \
         Step 3: run the shell command: echo plain-probe ; report its output. \
         Step 4: quote the codeword that appears at the very end of these instructions, exactly. Then stop.\n\n\
         {filler}\n\
         The codeword is {codeword}."
    );
    assert!(prompt.len() > 150_000, "a real-size prompt: {} bytes", prompt.len());
    let prompt_path = data_dir.join("prompt.txt");
    std::fs::write(&prompt_path, &prompt).unwrap();
    let cfg = SpawnConfig {
        agent_name: "hands".into(),
        config: AgentConfig {
            agent_name: "hands".into(),
            provider: "openai".into(),
            model_name: "gpt-6.1-sol".into(),
            base_url: None,
            auth_token: None,
            updated_at: String::new(),
            context_window: Some(475_000),
            cli_settings: None,
            claude_config_dir: Some(home.clone()),
            runtime: "codex".into(),
        },
        system_prompt_path: prompt_path,
        mcp_config_path: None,
        working_dir: Some(PathBuf::from("/tmp")),
        claude_bin: None,
        session_id: "s-live".into(),
        resume_session_id: None,
        project: None,
        data_dir: data_dir.clone(),
        capabilities: ResolvedCapabilities::Known(CapabilitySet::preset_hands()),
        overrides: Default::default(),
        blank_cli_attribution: true,
        account_dir: Some(home.clone()),
        account_email: None,
        account_org: None,
        codex: Some(CodexSpawn {
            bin: None,
            mcp_url: format!("http://{}/sessions/s-live/hands/mcp", server.local_addr),
            mcp_token: mcp_token.into(),
            hook_trust: vec![trust],
            hook_exe: Some(exe.clone()),
            model_catalog: None,
        }),
    };
    let mut handle = spawn_codex_agent(cfg).await.expect("the participant starts");
    // One page, the way the sequencer delivers one: a persisted row of this
    // session, which the receipt-gated input accepts.
    let row = storage
        .post_to_channel("s-live", "user", None, "text", "Go.", None)
        .await
        .unwrap();
    assert!(handle.input().deliver_batch(&[row]).await, "the page is taken");

    let mut saw_init = false;
    let mut tool_uses = Vec::new();
    let mut texts = Vec::new();
    let mut notices = Vec::new();
    let mut completed = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, handle.event_rx.recv()).await {
        eprintln!("event: {ev:?}");
        match ev {
            AgentEvent::Init { session_id, .. } => {
                saw_init = session_id.is_some();
            }
            AgentEvent::ToolUse { name, input, .. } => tool_uses.push((name, input)),
            AgentEvent::Text(t) => texts.push(t),
            AgentEvent::Notice(n) => notices.push(n),
            AgentEvent::TurnComplete { is_error, context, .. } => {
                completed = Some((is_error, context));
                break;
            }
            _ => {}
        }
    }
    handle.kill();
    let (is_error, context) = completed.expect("the turn completed within the deadline");
    assert!(saw_init, "the thread id came back as Init");
    assert!(!is_error, "the turn failed: {notices:?}");
    assert!(tool_uses.iter().any(|(n, _)| n == "mcp__bot-hq-signaling__cl_index_search"), "the MCP tool was called through bot-hq's server: {tool_uses:?}");
    assert!(tool_uses.iter().any(|(n, i)| n == "Bash" && i["command"].as_str().is_some_and(|c| c.contains("plain-probe"))), "the plain command ran: {tool_uses:?}");
    assert!(!tool_uses.iter().any(|(n, i)| n == "Bash" && i["command"].as_str().is_some_and(|c| c.contains("gated-probe"))), "the gated command never became a command item: {tool_uses:?}");
    let final_text = texts.join("\n");
    assert!(final_text.to_lowercase().contains("park") || final_text.to_lowercase().contains("block"), "the model saw the gate's refusal: {final_text}");
    assert!(final_text.contains(codeword), "the END of the {}-byte prompt reached the model: {final_text}", prompt.len());
    let tray = storage.tray_entries_for_session("s-live").await.unwrap();
    assert!(!tray.is_empty(), "the gated command was parked on the live server");
    let usage = context.usable().expect("a context reading from tokenUsage");
    assert_eq!(usage.context_window, 475_000, "the window Codex reports");
    assert!(usage.used_tokens > 1_000 && usage.used_tokens < 200_000, "a model call's prompt size, not a running total: {}", usage.used_tokens);
    eprintln!("final text:\n{final_text}\ncontext: {usage:?}");
}
