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
    let mut child = match spawn(req) {
        Ok(child) => child,
        Err(e) => return unreadable(format!("{e:#}")),
    };
    let pid = child.id();
    let outcome = tokio::time::timeout(PROBE_TIMEOUT, run(&mut child, req)).await;
    reap(&mut child, pid).await;
    match outcome {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => unreadable(format!("{e:#}")),
        Err(_) => unreadable(format!("the Codex probe did not finish within {PROBE_TIMEOUT:?}")),
    }
}

fn unreadable(why: String) -> CodexProbe {
    CodexProbe { auth: AuthProbe::Unreadable(why), hooks: Vec::new(), version: String::new(), models: Vec::new() }
}

/// The short-lived app-server, the leader of its own process group: the
/// installed `codex` is npm's node wrapper and the native binary runs under
/// it, so the group is what [`reap`] can kill once the wrapper is gone.
fn spawn(req: &CodexProbeRequest) -> Result<tokio::process::Child> {
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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.as_std_mut().process_group(0);
    }
    cmd.spawn().with_context(|| format!("launching `{bin} app-server` for the probe"))
}

/// Everything the probe started, on every exit path: the group the wrapper
/// leads (where a reparented native binary still is), the live tree under
/// the pid, then the child itself. Each Test click once left one app-server
/// running on init (measured 2026-10-21: seven of them).
async fn reap(child: &mut tokio::process::Child, pid: Option<u32>) {
    if let Some(pid) = pid {
        #[cfg(unix)]
        // SAFETY: kill(2) is thread-safe; -pid targets the group the child leads.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        crate::process::kill_process_tree(pid);
    }
    let _ = child.kill().await;
}

async fn run(child: &mut tokio::process::Child, req: &CodexProbeRequest) -> Result<CodexProbe> {
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
    Ok(CodexProbe { auth: auth_from_account_read(&account), hooks: listed_hooks(&listed), version, models })
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

    /// The installed `codex` is npm's node wrapper, which runs the native
    /// binary as a grandchild. Killing the wrapper alone left one app-server
    /// per Test click running on init (seven of them, measured 2026-10-21),
    /// so the probe reaps its whole process group, on every exit path.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_probe_reaps_the_grandchild_the_wrapper_started() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("grandchild.pid");
        let fake = dir.path().join("codex");
        // Answers `initialize`, then exits with the grandchild still running —
        // the wrapper is gone when the probe fails on `account/read`.
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nsleep 300 >/dev/null 2>&1 </dev/null &\necho $! > '{}'\nread -r _req\nprintf '%s\\n' '{{\"id\":1,\"result\":{{\"userAgent\":\"fake/0.0.0\"}}}}'\n",
                mark.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let probe = probe_home(&CodexProbeRequest { bin: Some(fake.display().to_string()), home: dir.path().display().to_string(), cwd: None }).await;
        assert!(matches!(&probe.auth, AuthProbe::Unreadable(_)), "{probe:?}");
        let grandchild: i32 = std::fs::read_to_string(&mark).unwrap().trim().parse().unwrap();
        // A SIGKILLed child of a dead parent is reaped by init at once; a live one answers kill(0).
        tokio::time::sleep(Duration::from_millis(300)).await;
        let alive = unsafe { libc::kill(grandchild, 0) } == 0;
        assert!(!alive, "the grandchild {grandchild} outlived the probe");
    }

    /// A binary that is not there is `Unreadable`, never logged out.
    #[tokio::test]
    async fn a_missing_codex_binary_is_unreadable() {
        let probe = probe_home(&CodexProbeRequest { bin: Some("/nonexistent/codex".into()), home: "/tmp/nowhere".into(), cwd: None }).await;
        assert!(matches!(&probe.auth, AuthProbe::Unreadable(why) if why.contains("launching")), "{probe:?}");
        assert!(probe.hooks.is_empty());
    }
}
