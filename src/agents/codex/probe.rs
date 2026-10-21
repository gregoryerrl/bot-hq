//! The pre-spawn probe of a Codex home: one short-lived `codex app-server`
//! asked three things — `initialize` (the version), `account/read` (who is
//! signed in, and how), `hooks/list` (whether bot-hq's gate hook is there and
//! what hash Codex gives it) — then killed. The connector's `claude auth
//! status --json`: no model call, about a second, and the only way to learn
//! the trust hash before the real spawn can grant it.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;

use super::client::CodexClient;
use super::home::ListedHook;
use super::spawn::{initialize, listed_hooks, CODEX_AUTH_ENV_SCRUB};
use crate::agents::spawn::AuthProbe;

/// What `account/read` says a ChatGPT sign-in is (`account.type`).
pub const SUBSCRIPTION_ACCOUNT_TYPE: &str = "chatgpt";

/// What the probe asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexProbeRequest {
    /// The `codex` binary (for tests). Defaults to `"codex"`.
    pub bin: Option<String>,
    /// The Codex home (`CODEX_HOME`).
    pub home: String,
    /// The working dir `hooks/list` is asked for (project hooks live there).
    pub cwd: Option<String>,
}

/// What it found.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexProbe {
    pub auth: AuthProbe,
    pub hooks: Vec<ListedHook>,
    pub version: String,
    /// The model ids `model/list` offers the signed-in account (empty when
    /// the call failed or the home is logged out).
    pub models: Vec<String>,
}

/// How long the probe may take end to end.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// `account/read`'s answer as an [`AuthProbe`]: a `chatgpt` account is the
/// subscription sign-in (`auth_method` = its type, the identity its email and
/// `workspaceRouting.chatgptAccountId` — the organisation-id analog the limit
/// mark is keyed on), an `apiKey` account is a sign-in that bills the API,
/// and a null account is logged out.
pub fn auth_from_account_read(result: &Value) -> AuthProbe {
    let account = &result["account"];
    if account.is_null() {
        return AuthProbe::LoggedOut;
    }
    let Some(kind) = account["type"].as_str() else {
        return AuthProbe::Unreadable("account/read carried no account type".into());
    };
    AuthProbe::LoggedIn {
        auth_method: Some(kind.to_string()),
        email: account["email"].as_str().map(str::to_string),
        org_id: result["workspaceRouting"]["chatgptAccountId"].as_str().map(str::to_string),
        subscription_type: account["planType"].as_str().map(str::to_string),
    }
}

/// The ids in a `model/list` answer.
pub fn model_ids(result: &Value) -> Vec<String> {
    result["data"]
        .as_array()
        .map(|ms| ms.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Run the probe. Every failure is [`AuthProbe::Unreadable`] with the reason
/// (never "logged out": a spawn then goes ahead and Codex reports its own
/// error), with no hooks and no version.
pub async fn probe_home(req: &CodexProbeRequest) -> CodexProbe {
    match tokio::time::timeout(PROBE_TIMEOUT, run(req)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => unreadable(format!("{e:#}")),
        Err(_) => unreadable(format!("the Codex probe did not finish within {PROBE_TIMEOUT:?}")),
    }
}

fn unreadable(why: String) -> CodexProbe {
    CodexProbe { auth: AuthProbe::Unreadable(why), hooks: Vec::new(), version: String::new(), models: Vec::new() }
}

async fn run(req: &CodexProbeRequest) -> Result<CodexProbe> {
    let bin = req.bin.clone().unwrap_or_else(|| "codex".to_string());
    let mut cmd = crate::process::tokio_command(&bin);
    crate::appimage_env::scrub_tokio(&mut cmd);
    for name in CODEX_AUTH_ENV_SCRUB {
        cmd.env_remove(name);
    }
    cmd.arg("app-server")
        .arg("--listen")
        .arg("stdio://")
        .arg("--disable")
        .arg("apps")
        .arg("--disable")
        .arg("plugins")
        .env("CODEX_HOME", &req.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn().with_context(|| format!("launching `{bin} app-server` for the probe"))?;
    let stdin = child.stdin.take().context("probe: missing stdin")?;
    let stdout = child.stdout.take().context("probe: missing stdout")?;
    let (mut client, _incoming) = CodexClient::start(stdout, stdin);
    client.timeout = Duration::from_secs(15);
    let version = initialize(&client).await?;
    let account = client.request("account/read", json!({ "refreshToken": false })).await?;
    let listed = client
        .request("hooks/list", json!({ "cwds": req.cwd.iter().collect::<Vec<_>>() }))
        .await
        .unwrap_or(Value::Null);
    let models = client
        .request("model/list", json!({ "limit": 50 }))
        .await
        .map(|v| model_ids(&v))
        .unwrap_or_default();
    let probe = CodexProbe { auth: auth_from_account_read(&account), hooks: listed_hooks(&listed), version, models };
    let _ = child.kill().await;
    Ok(probe)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes measured on 0.163.0 (s-de77e2f0): a ChatGPT sign-in, an
    /// API-key sign-in, a logged-out home, and an answer with no type.
    #[test]
    fn account_read_maps_onto_the_auth_probe() {
        let chatgpt = json!({"account": {"type": "chatgpt", "email": "me@example.com", "planType": "pro"}, "requiresOpenaiAuth": true, "workspaceRouting": {"chatgptAccountId": "26dba616-92c0", "backendOrigin": "https://chatgpt.com"}});
        assert_eq!(
            auth_from_account_read(&chatgpt),
            AuthProbe::LoggedIn {
                auth_method: Some("chatgpt".into()),
                email: Some("me@example.com".into()),
                org_id: Some("26dba616-92c0".into()),
                subscription_type: Some("pro".into()),
            }
        );
        let api = json!({"account": {"type": "apiKey"}, "requiresOpenaiAuth": true});
        assert!(matches!(auth_from_account_read(&api), AuthProbe::LoggedIn { auth_method: Some(m), email: None, .. } if m == "apiKey"));
        assert_eq!(auth_from_account_read(&json!({"account": null, "requiresOpenaiAuth": true})), AuthProbe::LoggedOut);
        assert!(matches!(auth_from_account_read(&json!({"account": {"email": "x"}})), AuthProbe::Unreadable(_)));
        assert_eq!(model_ids(&json!({"data": [{"id": "gpt-6.1-sol", "isDefault": true}, {"id": "gpt-6-astra"}]})), ["gpt-6.1-sol", "gpt-6-astra"]);
        assert!(model_ids(&json!({})).is_empty());
    }

    /// A binary that is not there is `Unreadable`, never logged out.
    #[tokio::test]
    async fn a_missing_codex_binary_is_unreadable() {
        let probe = probe_home(&CodexProbeRequest { bin: Some("/nonexistent/codex".into()), home: "/tmp/nowhere".into(), cwd: None }).await;
        assert!(matches!(&probe.auth, AuthProbe::Unreadable(why) if why.contains("launching")), "{probe:?}");
        assert!(probe.hooks.is_empty());
    }
}
