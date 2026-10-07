//! Models registry + app settings (default model) commands. Backs the
//! Settings → Models subtab and the session-create model pickers.

use crate::storage::{Model, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::Emitter;

/// Frontend-facing shape of a saved model. `auth_token` is exposed (the desktop
/// UI is local + trusted, like the AgentCard token field).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ModelView {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub model_name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Context window in tokens, or `null` when unknown.
    ///
    /// The meter still takes its denominator from the CLI's own `contextWindow`
    /// report; this value is what the pump compares that report AGAINST, so a
    /// model the CLI does not recognise (200k by default) is called out in the
    /// channel instead of silently compacting every few turns.
    pub context_window: Option<i64>,
    /// claude-code settings merged into every participant's `--settings` at
    /// spawn — a JSON object as text, or `null`. `{"modelOverrides":{"claude-fable-5":
    /// "claude-fable-5-1"}}` is the shape that gives a model id newer than the
    /// installed CLI's catalog its real window.
    pub cli_settings: Option<String>,
    /// The Claude config dir (`CLAUDE_CONFIG_DIR`) a participant on this row is
    /// spawned with — which subscription it bills when the row has no gateway
    /// credential. `null`/blank = the CLI's default `~/.claude`. An absolute
    /// path; one signed-in account per dir (0090).
    pub claude_config_dir: Option<String>,
}

/// The stored form of a model row's Claude config dir, or a validation error
/// the dialog shows inline. Blank is the default dir. Otherwise it must be an
/// absolute path — the CLI keys its credential on the exact string, so `~`
/// (which the CLI would not expand) and a relative path are refused rather
/// than guessed at — and never the default dir spelled out: claude-code
/// treats an explicit `~/.claude` as a CUSTOM dir with its own Keychain item,
/// which would force a re-login of the account that already lives there (spec
/// §4). A trailing slash is stripped; nothing else is rewritten, so a
/// symlinked alias stays the distinct slot the CLI sees it as.
pub(crate) fn validate_config_dir(
    raw: Option<&str>,
    home: Option<&std::path::Path>,
) -> Result<Option<String>, AppError> {
    let Some(dir) = crate::storage::normalize_config_dir(raw) else {
        return Ok(None);
    };
    // The examples are this platform's: a Windows path needs its drive.
    let (home_example, dir_example) = if cfg!(windows) {
        (r"C:\Users\<you>\…", r"C:\Users\<you>\.claude-acct-2")
    } else {
        ("/Users/<you>/…", "/Users/<you>/.claude-acct-2")
    };
    if dir.starts_with('~') {
        return Err(AppError::Validation(format!(
            "Claude config dir must be an absolute path — the CLI does not expand `~`; \
             write {home_example} in full"
        )));
    }
    if !std::path::Path::new(&dir).is_absolute() {
        return Err(AppError::Validation(format!(
            "Claude config dir must be an absolute path (e.g. {dir_example})"
        )));
    }
    if let Some(home) = home {
        if std::path::Path::new(&dir) == home.join(".claude") {
            return Err(AppError::Validation(
                "That is the CLI's default dir — leave the field blank for it. Naming it \
                 explicitly makes claude-code treat it as a separate account and ask for a \
                 new login."
                    .into(),
            ));
        }
    }
    Ok(Some(dir))
}

/// `cli_settings` must be a JSON object (or absent). Anything else would be
/// merged into `--settings` as garbage, and claude-code silently ignores a
/// malformed `--settings` in `-p` mode — the whole hook block would vanish
/// with it. Empty / whitespace-only text is normalised to `None`.
fn validate_cli_settings(raw: Option<String>) -> Result<Option<String>, AppError> {
    let Some(text) = raw else { return Ok(None) };
    if text.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Object(_)) => Ok(Some(text)),
        Ok(_) => Err(AppError::Validation(
            "CLI settings must be a JSON object, e.g. {\"modelOverrides\":{…}}".into(),
        )),
        Err(e) => Err(AppError::Validation(format!("CLI settings is not valid JSON: {e}"))),
    }
}

impl From<Model> for ModelView {
    fn from(m: Model) -> Self {
        Self {
            id: m.id,
            display_name: m.display_name,
            provider: m.provider,
            model_name: m.model_name,
            base_url: m.base_url,
            auth_token: m.auth_token,
            created_at: m.created_at,
            updated_at: m.updated_at,
            context_window: m.context_window,
            cli_settings: m.cli_settings,
            claude_config_dir: m.claude_config_dir,
        }
    }
}

impl From<ModelView> for Model {
    fn from(v: ModelView) -> Self {
        Self {
            id: v.id,
            display_name: v.display_name,
            provider: v.provider,
            model_name: v.model_name,
            base_url: v.base_url,
            auth_token: v.auth_token,
            created_at: v.created_at,
            updated_at: v.updated_at,
            context_window: v.context_window,
            cli_settings: v.cli_settings,
            claude_config_dir: v.claude_config_dir,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_models(
    storage: tauri::State<'_, Arc<Storage>>,
) -> Result<Vec<ModelView>, AppError> {
    storage
        .list_models()
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_model(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    model: ModelView,
) -> Result<(), AppError> {
    let mut m: Model = model.into();
    m.cli_settings = validate_cli_settings(m.cli_settings.take())?;
    m.claude_config_dir =
        validate_config_dir(m.claude_config_dir.as_deref(), crate::paths::home_dir().ok().as_deref())?;
    storage
        .upsert_model(&m)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    let _ = app.emit(crate::tauri_events::types::MODEL_CHANGED, ());
    Ok(())
}

/// The Model dialog's one-time commands for a second account's config dir, in
/// this machine's shell (`agents::account_setup`). The dir is checked by the
/// same rules as Save first, so a `~` path or the default dir spelled out gets
/// Save's refusal, never a command that signs the account in somewhere else
/// before Save is pressed (a quoted `~` is a literal folder name).
#[tauri::command]
#[specta::specta]
pub async fn account_setup_commands(
    dir: String,
) -> Result<crate::agents::account_setup::AccountSetupCommands, AppError> {
    let home = crate::paths::home_dir()
        .map_err(|e| AppError::Internal(format!("can't find your home folder: {e:#}")))?;
    account_setup_commands_for(&dir, &home)
}

fn account_setup_commands_for(
    dir: &str,
    home: &std::path::Path,
) -> Result<crate::agents::account_setup::AccountSetupCommands, AppError> {
    use crate::agents::account_setup::{commands, Shell};
    let Some(dir) = validate_config_dir(Some(dir), Some(home))? else {
        return Err(AppError::Validation("Type the Claude config dir first".into()));
    };
    let default_dir = home.join(".claude");
    Ok(commands(Shell::host(), &dir, &default_dir.to_string_lossy()))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_model(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), AppError> {
    storage
        .delete_model(&id)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    let _ = app.emit(crate::tauri_events::types::MODEL_CHANGED, ());
    Ok(())
}

/// Every account's usage-limit mark (0091), for the Models list and the New
/// Session dialog. Advisory: the UI says "limited until …" on the rows that
/// bill that account and nothing more — never which other account to use.
#[tauri::command]
#[specta::specta]
pub async fn list_account_marks(
    storage: tauri::State<'_, Arc<Storage>>,
) -> Result<Vec<crate::storage::AccountMark>, AppError> {
    storage
        .list_account_marks()
        .await
        .map_err(|e| AppError::DbError(e.to_string()))
}

/// The user's manual clear — extra usage was enabled, the reset passed, or
/// they simply know better than the mark.
#[tauri::command]
#[specta::specta]
pub async fn clear_account_mark(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    config_dir: String,
    org_id: String,
    model_name: String,
) -> Result<bool, AppError> {
    let cleared = storage
        .clear_account_mark(&config_dir, &org_id, &model_name)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    let _ = app.emit(crate::tauri_events::types::MODEL_CHANGED, ());
    Ok(cleared)
}

#[tauri::command]
#[specta::specta]
pub async fn get_app_setting(
    storage: tauri::State<'_, Arc<Storage>>,
    key: String,
) -> Result<Option<String>, AppError> {
    storage
        .get_setting(&key)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn set_app_setting(
    storage: tauri::State<'_, Arc<Storage>>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    storage
        .set_setting(&key, &value)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn view_roundtrips_through_model() {
        let view = ModelView {
            id: "m1".into(),
            display_name: "Opus".into(),
            provider: "anthropic".into(),
            model_name: "claude-opus-4-8".into(),
            base_url: Some("https://example/anthropic".into()),
            auth_token: Some("sk".into()),
            created_at: "2026-06-03T00:00:00.000Z".into(),
            updated_at: "2026-06-03T00:00:00.000Z".into(),
            context_window: Some(200_000),
            cli_settings: None,
            claude_config_dir: None,
        };
        let back: ModelView = Model::from(view.clone()).into();
        assert_eq!(back, view);
    }

    /// 0090: the dialog's config dir is an absolute path or nothing. `~` is
    /// refused (the CLI would not expand it and would key a credential on the
    /// literal string), so is a relative path, and so is the default dir
    /// spelled out (an explicit `~/.claude` is a CUSTOM dir to claude-code —
    /// a separate Keychain item and a forced re-login). A trailing slash is
    /// stripped; blank is the default.
    ///
    /// The home is absolute on THIS platform: `/Users/me` has a root but no
    /// drive, which Windows does not count as absolute — the windows CI job
    /// failed on exactly that (run 37559284918). Each refusal is matched to
    /// the rule that made it, so a fixture refused for the wrong reason cannot
    /// pass: under the unix home, the default-dir rule had never run on
    /// Windows.
    #[test]
    fn validate_config_dir_accepts_absolute_paths_only() {
        let home = if cfg!(windows) {
            std::path::Path::new(r"C:\Users\me")
        } else {
            std::path::Path::new("/Users/me")
        };
        let acct2 = home.join(".claude-acct-2");
        let acct2 = acct2.to_str().unwrap();
        let default_dir = home.join(".claude");
        let default_dir = default_dir.to_str().unwrap();
        let refusal = |dir: &str, home: Option<&std::path::Path>| match validate_config_dir(Some(dir), home) {
            Err(AppError::Validation(msg)) => msg,
            other => panic!("{dir:?} must be refused, got {other:?}"),
        };

        assert_eq!(validate_config_dir(None, Some(home)).unwrap(), None);
        assert_eq!(validate_config_dir(Some("  "), Some(home)).unwrap(), None);
        assert_eq!(
            validate_config_dir(Some(&format!("{acct2}/")), Some(home))
                .unwrap()
                .as_deref(),
            Some(acct2)
        );
        assert!(refusal("~/.claude-acct-2", Some(home)).contains("does not expand `~`"));
        let mut relative = vec![".claude-acct-2", "acct/2"];
        // Rooted but drive-less: relative to the current drive on Windows.
        if cfg!(windows) {
            relative.push("/Users/me/.claude-acct-2");
        }
        for dir in relative {
            let msg = refusal(dir, Some(home));
            assert!(msg.contains("must be an absolute path"), "{dir:?}: {msg}");
        }
        for dir in [default_dir.to_string(), format!("{default_dir}/")] {
            let msg = refusal(&dir, Some(home));
            assert!(msg.contains("the CLI's default dir"), "{dir:?}: {msg}");
        }
        // Without a known home the default-dir check cannot run; the absolute
        // rule still does.
        assert!(validate_config_dir(Some(default_dir), None).is_ok());
        assert!(refusal("relative", None).contains("must be an absolute path"));
    }

    /// The dialog's commands pass Save's rules first: a `~` path or the default
    /// dir spelled out gets the refusal, not a command (a quoted `~` would
    /// create a folder literally named `~` wherever the terminal stands, and
    /// sign the account in there).
    #[test]
    fn account_setup_commands_refuse_what_save_refuses() {
        use crate::agents::account_setup::Shell;
        let home = if cfg!(windows) {
            std::path::Path::new(r"C:\Users\me")
        } else {
            std::path::Path::new("/Users/me")
        };
        let refused = |dir: &str| match account_setup_commands_for(dir, home) {
            Err(AppError::Validation(msg)) => msg,
            other => panic!("{dir:?} must be refused, got {other:?}"),
        };
        assert!(refused("~/.claude-acct-2").contains("does not expand `~`"));
        assert!(refused(home.join(".claude").to_str().unwrap()).contains("the CLI's default dir"));
        assert!(refused("  ").contains("Type the Claude config dir first"));

        let acct2 = home.join(".claude-acct-2");
        let ok = account_setup_commands_for(&format!("{}/", acct2.display()), home).unwrap();
        assert_eq!(ok.shell, Shell::host());
        let quoted = crate::agents::account_setup::quote(ok.shell, acct2.to_str().unwrap());
        assert!(ok.setup.contains(&quoted), "the trailing slash is stripped: {}", ok.setup);
        let default_dir = crate::agents::account_setup::quote(ok.shell, home.join(".claude").to_str().unwrap());
        assert!(ok.share.contains(&default_dir), "{}", ok.share);
    }

    #[test]
    fn cli_settings_accepts_an_object_and_nothing_else() {
        let obj = r#"{"modelOverrides":{"claude-fable-5":"claude-fable-5-1"}}"#;
        assert_eq!(
            validate_cli_settings(Some(obj.to_string())).unwrap(),
            Some(obj.to_string())
        );
        assert_eq!(validate_cli_settings(None).unwrap(), None);
        assert_eq!(validate_cli_settings(Some("  \n".into())).unwrap(), None);
        // A JSON array or scalar is valid JSON and still refused: only an
        // object can be merged into `--settings`.
        assert!(validate_cli_settings(Some("[1,2]".into())).is_err());
        assert!(validate_cli_settings(Some("\"modelOverrides\"".into())).is_err());
        assert!(validate_cli_settings(Some("{not json".into())).is_err());
    }
}
