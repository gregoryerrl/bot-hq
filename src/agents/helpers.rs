//! Restricted helper connectors. Helpers are not session participants and
//! receive no MCP identity or policy-changing tools. The owner supplies a
//! bounded brief and owns the returned process until cleanup is acknowledged.

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use tokio::process::Command;

use super::spawn::SpawnConfig;

pub const CLAUDE_READ_TOOLS: &str = "Read,Grep,Glob";

/// Build the actual helper command, also used by the opt-in native probe.
/// No ordinary participant flag or role/model permission override is copied.
pub fn claude_command(cfg: &SpawnConfig) -> Result<Command> {
    ensure!(cfg.codex.is_none() && cfg.config.runtime != "codex", "a Claude helper requires a Claude model");
    ensure!(cfg.resume_session_id.is_none(), "helpers always start a fresh conversation");
    let repo = cfg.working_dir.as_deref().context("a helper requires a repository directory")?;
    let repo = repo.canonicalize().context("resolving helper repository")?;
    ensure!(repo.is_dir(), "helper repository is not a directory");
    ensure!(cfg.system_prompt_path.is_file(), "helper instructions are missing");

    let mut settings = json!({
        "permissions": {"blockReadsOutsideWorkingDirectories": true},
        "crossSessionInbound": "refuse",
        "disableClaudeAiConnectors": true,
        "syncClaudeAiSkills": false,
        "syncClaudeAiPlugins": false,
        "attribution": {"commit": "", "pr": ""}
    });
    // Preserve the selected model's catalog mapping/window without importing
    // env, hooks, apiKeyHelper, permissions, plugins or other executable config.
    if let Some(raw) = &cfg.config.cli_settings {
        let value: Value = serde_json::from_str(raw).context("parsing helper model settings")?;
        ensure!(value.is_object(), "helper model settings must be an object");
        if let Some(mapping) = value.get("modelOverrides") {
            settings["modelOverrides"] = mapping.clone();
        }
    }
    let mut cmd = crate::process::tokio_command(cfg.claude_bin.as_deref().unwrap_or("claude"));
    crate::appimage_env::scrub_tokio(&mut cmd);
    super::spawn::apply_account_env(&mut cmd, cfg.account_dir.as_deref());
    // No inherited host identity, capability token or nested CLI session.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("BOT_HQ_") || name.starts_with("CODEX_") || name == "CLAUDECODE" {
            cmd.env_remove(key);
        }
    }
    for name in ["BOT_HQ_MCP_TOKEN", "BOT_HQ_SESSION_ID", "BOT_HQ_AGENT", "CLAUDECODE"] { cmd.env_remove(name); }
    cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"])
        .args(["--tools", CLAUDE_READ_TOOLS, "--allowedTools", CLAUDE_READ_TOOLS])
        .args(["--permission-mode", "dontAsk", "--setting-sources", ""])
        .args(["--mcp-config", "{\"mcpServers\":{}}", "--strict-mcp-config"])
        .args(["--safe-mode", "--restricted", "--no-chrome", "--disable-slash-commands", "--no-session-persistence"])
        .args(["--disallowedTools", "mcp__* Agent Workflow SendMessage ListAgents CronCreate CronList CronDelete RemoteTrigger Bash PowerShell Edit Write NotebookEdit"])
        .arg("--append-system-prompt-file").arg(&cfg.system_prompt_path)
        .arg("--settings").arg(settings.to_string())
        .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
        .env("CLAUDE_CODE_SILENT_TURN_REMINDER", "0")
        .env("ANTHROPIC_MODEL", &cfg.config.model_name)
        .current_dir(repo);
    if let Some(effort) = cfg.overrides.effort.as_deref().filter(|s| !s.is_empty()) {
        cmd.env("CLAUDE_CODE_EFFORT_LEVEL", effort);
    }
    if let Some(window) = cfg.config.context_window.filter(|w| *w > 0) {
        cmd.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", window.to_string());
    }
    if let Some(token) = cfg.config.auth_token.as_deref().filter(|s| !s.is_empty()) {
        cmd.env("ANTHROPIC_AUTH_TOKEN", token);
    }
    if let Some(base) = crate::agents::llm_proxy::proxied_base_url(cfg.config.base_url.as_deref(), crate::agents::llm_proxy::proxy_addr()) {
        cmd.env("ANTHROPIC_BASE_URL", base);
    }
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{CapabilitySet, ResolvedCapabilities};
    use std::path::Path;

    fn cfg(repo: &Path) -> SpawnConfig {
        let instructions = repo.join("helper-prompt.txt");
        std::fs::write(&instructions, "Read the requested source; cite paths and lines.").unwrap();
        SpawnConfig {
            agent_name: "helper-fixture".into(),
            config: crate::storage::AgentConfig { agent_name: "helper-fixture".into(), provider: "anthropic".into(),
                model_name: "fixture-model".into(), base_url: Some("https://fixture.example.test".into()), auth_token: Some("fixture-token".into()),
                updated_at: String::new(), context_window: Some(200_000), cli_settings: Some(r#"{"modelOverrides":{"fixture-model":"mapped"},"env":{"ANTHROPIC_BASE_URL":"https://wrong.example.test"},"apiKeyHelper":"wrong-helper","hooks":{},"permissions":{"defaultMode":"bypassPermissions"},"enabledPlugins":{"wrong":true}}"#.into()),
                claude_config_dir: None, runtime: "claude_code".into() },
            system_prompt_path: instructions, mcp_config_path: Some(repo.join("parent-mcp.json")),
            working_dir: Some(repo.into()), claude_bin: None, session_id: "parent-session".into(),
            resume_session_id: None, project: None, data_dir: repo.into(),
            capabilities: ResolvedCapabilities::Known(CapabilitySet::preset_hands()),
            overrides: Default::default(), blank_cli_attribution: false, account_dir: Some("/fixture-account".into()),
            account_email: None, account_org: None, codex: None,
        }
    }

    #[test]
    fn helper_command_does_not_inherit_parent_authority_or_settings() {
        let repo = tempfile::tempdir().unwrap();
        let cfg = cfg(repo.path());
        let cmd = claude_command(&cfg).unwrap();
        let argv: Vec<_> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        let value = |flag: &str| argv.windows(2).find(|w| w[0] == flag).unwrap()[1].clone();
        assert_eq!(value("--tools"), "Read,Grep,Glob");
        assert_eq!(value("--mcp-config"), "{\"mcpServers\":{}}");
        assert_eq!(value("--setting-sources"), "");
        assert_eq!(value("--permission-mode"), "dontAsk");
        for flag in ["--safe-mode", "--restricted", "--strict-mcp-config", "--no-chrome", "--disable-slash-commands", "--no-session-persistence"] { assert!(argv.iter().any(|s| s == flag)); }
        assert!(!argv.iter().any(|s| s == "--dangerously-skip-permissions" || s == "--resume"));
        let settings: Value = serde_json::from_str(&value("--settings")).unwrap();
        assert_eq!(settings["permissions"], json!({"blockReadsOutsideWorkingDirectories":true}));
        assert_eq!(settings["crossSessionInbound"], "refuse");
        assert_eq!(settings["modelOverrides"]["fixture-model"], "mapped");
        for key in ["env", "apiKeyHelper", "hooks", "enabledPlugins"] { assert!(settings.get(key).is_none()); }
        let env: std::collections::HashMap<_, _> = cmd.as_std().get_envs().map(|(k,v)|(k.to_string_lossy().to_string(),v.map(|s|s.to_string_lossy().to_string()))).collect();
        for key in ["BOT_HQ_MCP_TOKEN", "BOT_HQ_SESSION_ID", "BOT_HQ_AGENT", "CLAUDECODE"] { assert_eq!(env.get(key), Some(&None)); }
        assert_eq!(env["ANTHROPIC_AUTH_TOKEN"].as_deref(), Some("fixture-token"));
        assert_eq!(env["CLAUDE_CONFIG_DIR"].as_deref(), Some("/fixture-account"));
        assert_eq!(env["CLAUDE_CODE_DISABLE_CLAUDE_MDS"].as_deref(), Some("1"));
        assert_eq!(cmd.as_std().get_current_dir(), Some(repo.path().canonicalize().unwrap().as_path()));
    }

    #[test]
    fn helper_refuses_missing_scope_resume_and_wrong_runtime() {
        let repo = tempfile::tempdir().unwrap();
        let mut cfg = cfg(repo.path());
        cfg.resume_session_id = Some("parent-conversation".into());
        assert!(claude_command(&cfg).unwrap_err().to_string().contains("fresh"));
        cfg.resume_session_id = None;
        cfg.config.runtime = "codex".into();
        assert!(claude_command(&cfg).unwrap_err().to_string().contains("Claude model"));
        cfg.config.runtime = "claude_code".into();
        cfg.working_dir = None;
        assert!(claude_command(&cfg).unwrap_err().to_string().contains("repository"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn helper_spawn_uses_restricted_command_and_delivers_one_brief() {
        use std::os::unix::fs::PermissionsExt;
        use crate::agents::AgentEvent;
        let repo = tempfile::tempdir().unwrap();
        let mut cfg = cfg(repo.path());
        let script = repo.path().join("fake-claude");
        std::fs::write(&script, r#"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
assert args[args.index('--tools')+1] == 'Read,Grep,Glob'
assert '--safe-mode' in args and '--restricted' in args
assert not os.environ.get('BOT_HQ_SESSION_ID')
brief = json.loads(sys.stdin.readline())['message']['content']
print(json.dumps({'type':'assistant','message':{'id':'fixture-message','role':'assistant','content':[{'type':'text','text':brief}]}}),flush=True)
print(json.dumps({'type':'result','subtype':'success','is_error':False,'result':'done'}),flush=True)
"#).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        cfg.claude_bin = Some(script.to_string_lossy().to_string());
        let mut handle = crate::agents::spawn::spawn_claude_helper(cfg, "fixture brief".into()).await.unwrap();
        let mut texts = vec![];
        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while let Some(event) = handle.event_rx.recv().await {
                match event { AgentEvent::Text(text) => texts.push(text), AgentEvent::TurnComplete { is_error, .. } => return !is_error, _ => {} }
            }
            false
        }).await.unwrap();
        handle.stop_and_wait(std::time::Duration::from_secs(5)).await.unwrap();
        assert!(completed);
        assert_eq!(texts, vec!["fixture brief"]);
    }
}
