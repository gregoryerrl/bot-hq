//! Models registry + app settings (default model) commands. Backs the
//! Settings → Models subtab and the session-create model pickers.

use crate::agents::runtime::Runtime;
use crate::storage::{Model, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::Emitter;

/// Frontend-facing shape of a saved model. `auth_token` is exposed (the desktop
/// UI is local + trusted, like the AgentCard token field).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(deny_unknown_fields)]
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
    /// Spawn hands it to the CLI as `CLAUDE_CODE_MAX_CONTEXT_TOKENS`, which the
    /// CLI takes as the window of a model id it does not recognise as Claude's
    /// own (a gateway model); for a Claude id the CLI keeps its own catalog.
    /// The meter still takes its denominator from the CLI's own `contextWindow`
    /// report; the pump compares that report against this value and calls a
    /// disagreement (or a gateway model with no value here) out in the channel
    /// instead of letting the model silently compact every few turns.
    pub context_window: Option<i64>,
    /// claude-code settings merged into every participant's `--settings` at
    /// spawn — a JSON object as text, or `null`. `{"modelOverrides":{"claude-fable-5":
    /// "claude-fable-5-1"}}` is the shape that gives a CLAUDE id newer than the
    /// installed CLI's catalog its real window; a gateway model mapped this way
    /// gets the Claude model's request profile instead, which its gateway
    /// rejects — its window comes from `context_window`.
    pub cli_settings: Option<String>,
    /// The Claude config dir (`CLAUDE_CONFIG_DIR`) a participant on this row is
    /// spawned with — which subscription it bills when the row has no gateway
    /// credential. `null`/blank = the CLI's default `~/.claude`. An absolute
    /// path; one signed-in account per dir (0090). For a `codex` row this is
    /// the Codex home (`CODEX_HOME`) and is REQUIRED — bot-hq never defaults
    /// to `~/.codex`, and Save refuses a dir a row of the other runtime uses.
    pub claude_config_dir: Option<String>,
    /// The connector a participant on this row runs through: `claude_code`
    /// (claude-code) or `codex` (OpenAI's Codex CLI), 0102. Save refuses any
    /// other value; blank is stored as `claude_code`.
    pub runtime: String,
    #[serde(default)]
    pub favorite: bool,
}

/// The connector a dialog value names, or a validation error the dialog
/// shows inline. Blank is claude-code (a row the dialog never touched); any
/// value this build does not know is refused by name, so a typo cannot be
/// stored and later read back as claude-code by the fail-safe column read.
pub(crate) fn validate_runtime(raw: &str) -> Result<Runtime, AppError> {
    if raw.is_empty() {
        return Ok(Runtime::ClaudeCode);
    }
    Runtime::parse(raw).ok_or_else(|| {
        AppError::Validation(format!(
            "Unknown runtime {raw:?} — a model runs through claude_code or codex"
        ))
    })
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
    runtime: Runtime,
) -> Result<Option<String>, AppError> {
    let Some(dir) = crate::storage::normalize_config_dir(raw) else {
        return Ok(None);
    };
    // What the field is called and which CLI default it must not name: the
    // Codex home is the same kind of slot (one signed-in account per
    // `CODEX_HOME`), with `~/.codex` as the default the CLI uses unprompted.
    let (field, default_leaf, example_leaf) = match runtime {
        Runtime::ClaudeCode => ("Claude config dir", ".claude", ".claude-acct-2"),
        Runtime::Codex => ("Codex home", ".codex", ".codex-bot-hq"),
    };
    // The examples are this platform's: a Windows path needs its drive.
    let (home_example, dir_example) = if cfg!(windows) {
        (r"C:\Users\<you>\…".to_string(), format!(r"C:\Users\<you>\{example_leaf}"))
    } else {
        ("/Users/<you>/…".to_string(), format!("/Users/<you>/{example_leaf}"))
    };
    if dir.starts_with('~') {
        return Err(AppError::Validation(format!(
            "{field} must be an absolute path — the CLI does not expand `~`; \
             write {home_example} in full"
        )));
    }
    if !std::path::Path::new(&dir).is_absolute() {
        return Err(AppError::Validation(format!(
            "{field} must be an absolute path (e.g. {dir_example})"
        )));
    }
    if let Some(home) = home {
        // The OTHER CLI's default dir is the other runtime's by definition
        // (EYES c4f6809d): `~/.claude` as a Codex home would get bot-hq's
        // hooks file, a 0700 chmod and a Codex sign-in; `~/.codex` as a
        // Claude dir would get claude's files. Neither is stored on any row,
        // so the row-level rule cannot see them.
        let (other_leaf, other_name) = match runtime {
            Runtime::ClaudeCode => (".codex", "Codex's default home"),
            Runtime::Codex => (".claude", "claude-code's default config dir"),
        };
        if std::path::Path::new(&dir) == home.join(other_leaf) {
            return Err(AppError::Validation(format!(
                "That is {other_name} (~/{other_leaf}), which belongs to the other runtime — a {field} \
                 needs a path of its own (e.g. {dir_example})"
            )));
        }
        if std::path::Path::new(&dir) == home.join(default_leaf) {
            return Err(AppError::Validation(match runtime {
                Runtime::ClaudeCode => "That is the CLI's default dir — leave the field blank for it. \
                     Naming it explicitly makes claude-code treat it as a separate account and ask \
                     for a new login."
                    .into(),
                Runtime::Codex => format!(
                    "That is Codex's default home (~/{default_leaf}) — bot-hq writes its hooks \
                     into the Codex home it is given, so give it a home of its own \
                     (e.g. {dir_example})"
                ),
            }));
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

/// The cross-runtime dir rule (EYES d928bf99): a dir is one connector's
/// credential slot and config home — bot-hq writes its hooks file into a
/// Codex home, claude-code writes its own files into a Claude dir — so a dir
/// a row of the OTHER runtime uses is refused, by that row's name. Rows of
/// the same runtime may share one (one account, several models). A Codex
/// row with no home is refused: bot-hq never defaults to `~/.codex`.
pub(crate) fn validate_dir_against_rows(
    id: &str,
    runtime: Runtime,
    dir: Option<&str>,
    rows: &[Model],
) -> Result<(), AppError> {
    let dir = crate::storage::normalize_config_dir(dir);
    let Some(dir) = dir else {
        return match runtime {
            Runtime::Codex => Err(AppError::Validation(
                "A Codex row needs a Codex home of its own (an absolute path, never ~/.codex): bot-hq \
                 writes its gate hook there and the ChatGPT account signed in there is the one it bills."
                    .into(),
            )),
            Runtime::ClaudeCode => Ok(()),
        };
    };
    // The row's OWN saved state counts too: editing a Claude row's Runtime to
    // Codex while keeping its dir is the very case the rule exists for.
    let same = |a: &str| crate::storage::normalize_config_dir(Some(a)).as_deref() == Some(dir.as_str());
    if let Some(other) = rows
        .iter()
        .filter(|r| r.runtime() != runtime)
        .find(|r| r.claude_config_dir.as_deref().is_some_and(same))
    {
        let (theirs, mine) = match other.runtime() {
            Runtime::ClaudeCode => ("Claude config dir", "Codex home"),
            Runtime::Codex => ("Codex home", "Claude config dir"),
        };
        let whose = if other.id == id { "this row's own saved".to_string() } else { format!("the saved model \"{}\" ({})'s", other.display_name, other.id) };
        return Err(AppError::Validation(format!(
            "{dir} is {whose} {theirs}; a dir belongs to one runtime — give this row's {mine} a path of \
             its own.",
        )));
    }
    Ok(())
}

impl From<Model> for ModelView {
    fn from(m: Model) -> Self {
        Self {
            favorite: m.favorite,
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
            runtime: m.runtime,
        }
    }
}

impl From<ModelView> for Model {
    fn from(v: ModelView) -> Self {
        Self {
            favorite: v.favorite,
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
            runtime: v.runtime,
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
pub async fn set_model_favorite(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    id: String,
    favorite: bool,
) -> Result<(), AppError> {
    if !storage.set_model_favorite(&id, favorite).await.map_err(|e| AppError::DbError(e.to_string()))? {
        return Err(AppError::NotFound(format!("model {id}")));
    }
    let _ = app.emit(crate::tauri_events::types::MODEL_CHANGED, ());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_model(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    model: ModelView,
) -> Result<(), AppError> {
    let m = normalize_model(model.into())?;
    let runtime = m.runtime();
    let rows = storage
        .list_models()
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    validate_dir_against_rows(&m.id, runtime, m.claude_config_dir.as_deref(), &rows)?;
    storage
        .upsert_model(&m)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    let _ = app.emit(crate::tauri_events::types::MODEL_CHANGED, ());
    Ok(())
}

/// Validation shared by the desktop editor and delegated administration.
pub(crate) fn normalize_model(mut m: Model) -> Result<Model, AppError> {
    let runtime = validate_runtime(&m.runtime)?;
    m.runtime = runtime.as_str().to_string();
    m.cli_settings = validate_cli_settings(m.cli_settings.take())?;
    m.claude_config_dir = validate_config_dir(
        m.claude_config_dir.as_deref(),
        crate::paths::home_dir().ok().as_deref(),
        runtime,
    )?;
    Ok(m)
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
    runtime: String,
) -> Result<crate::agents::account_setup::AccountSetupCommands, AppError> {
    let home = crate::paths::home_dir()
        .map_err(|e| AppError::Internal(format!("can't find your home folder: {e:#}")))?;
    account_setup_commands_for(&dir, &home, validate_runtime(&runtime)?)
}

fn account_setup_commands_for(
    dir: &str,
    home: &std::path::Path,
    runtime: Runtime,
) -> Result<crate::agents::account_setup::AccountSetupCommands, AppError> {
    use crate::agents::account_setup::{codex_commands, commands, Shell};
    let Some(dir) = validate_config_dir(Some(dir), Some(home), runtime)? else {
        let field = match runtime {
            Runtime::ClaudeCode => "Claude config dir",
            Runtime::Codex => "Codex home",
        };
        return Err(AppError::Validation(format!("Type the {field} first")));
    };
    Ok(match runtime {
        Runtime::ClaudeCode => {
            let default_dir = home.join(".claude");
            commands(Shell::host(), &dir, &default_dir.to_string_lossy())
        }
        Runtime::Codex => codex_commands(Shell::host(), &dir),
    })
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
            favorite: false,
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
            runtime: "claude_code".into(),
        };
        let back: ModelView = Model::from(view.clone()).into();
        assert_eq!(back, view);
    }

    /// 0102: Save accepts the two connectors by their column value and refuses
    /// anything else by name — a typo must not be stored and then read back as
    /// claude-code by the fail-safe column read.
    #[test]
    fn save_accepts_the_two_runtimes_and_refuses_the_rest() {
        use crate::agents::runtime::Runtime;
        assert_eq!(validate_runtime("claude_code").unwrap(), Runtime::ClaudeCode);
        assert_eq!(validate_runtime("codex").unwrap(), Runtime::Codex);
        // Blank is a row the dialog never touched: claude-code, as before 0102.
        assert_eq!(validate_runtime("").unwrap(), Runtime::ClaudeCode);
        match validate_runtime("gemini") {
            Err(AppError::Validation(msg)) => assert!(msg.contains("gemini"), "{msg}"),
            other => panic!("must be refused, got {other:?}"),
        }
    }

    /// The Codex home follows the same rules as the Claude config dir — absolute,
    /// no `~`, never the CLI's own default spelled out — but the default it
    /// refuses is `~/.codex`, and the wording names the Codex home, not Claude.
    #[test]
    fn validate_config_dir_knows_the_codex_default_home() {
        use crate::agents::runtime::Runtime;
        let home = if cfg!(windows) {
            std::path::Path::new(r"C:\Users\me")
        } else {
            std::path::Path::new("/Users/me")
        };
        let codex_default = home.join(".codex");
        let codex_default = codex_default.to_str().unwrap();
        let managed = home.join(".codex-bot-hq");
        let managed = managed.to_str().unwrap();
        let refusal = |dir: &str, rt: Runtime| match validate_config_dir(Some(dir), Some(home), rt) {
            Err(AppError::Validation(msg)) => msg,
            other => panic!("{dir:?} must be refused, got {other:?}"),
        };
        assert_eq!(
            validate_config_dir(Some(managed), Some(home), Runtime::Codex).unwrap().as_deref(),
            Some(managed)
        );
        let msg = refusal(codex_default, Runtime::Codex);
        assert!(msg.contains("default"), "{msg}");
        assert!(msg.contains("Codex"), "names the Codex home: {msg}");
        assert!(!msg.contains("Claude"), "must not talk about Claude: {msg}");
        let msg = refusal("~/.codex-bot-hq", Runtime::Codex);
        assert!(msg.contains("Codex home"), "{msg}");
        // The OTHER CLI's default dir is refused too (EYES c4f6809d): account
        // 1's `~/.claude` can never be a Codex home, nor `~/.codex` a Claude dir.
        let claude_default = home.join(".claude");
        let msg = refusal(claude_default.to_str().unwrap(), Runtime::Codex);
        assert!(msg.contains("other runtime") && msg.contains(".claude"), "{msg}");
        let msg = match validate_config_dir(Some(codex_default), Some(home), Runtime::ClaudeCode) {
            Err(AppError::Validation(msg)) => msg,
            other => panic!("~/.codex must be refused as a Claude dir, got {other:?}"),
        };
        assert!(msg.contains("other runtime") && msg.contains(".codex"), "{msg}");
    }

    /// EYES d928bf99 (a) + N3: Save refuses a dir that a row of the OTHER
    /// runtime uses (a Claude dir as a Codex home would get bot-hq's hooks
    /// file and a Codex sign-in; a Codex home as a Claude dir would get
    /// claude's files), a row's own dir and same-runtime twins stay fine, and
    /// a Codex row with no home is refused outright — bot-hq never defaults
    /// to `~/.codex`.
    #[test]
    fn save_refuses_a_dir_the_other_runtime_uses_and_a_homeless_codex_row() {
        use crate::agents::runtime::Runtime;
        let row = |id: &str, runtime: &str, dir: Option<&str>| Model {
            favorite: false,
            id: id.into(),
            display_name: id.into(),
            provider: "x".into(),
            model_name: "m".into(),
            base_url: None,
            auth_token: None,
            created_at: String::new(),
            updated_at: String::new(),
            context_window: None,
            cli_settings: None,
            claude_config_dir: dir.map(str::to_string),
            runtime: runtime.into(),
        };
        let rows = vec![
            row("c1", "claude_code", Some("/Users/me/.claude-acct-2")),
            row("c2", "claude_code", None),
            row("x1", "codex", Some("/Users/me/.codex-bot-hq")),
        ];
        let refused = |id: &str, runtime: Runtime, dir: Option<&str>| match validate_dir_against_rows(id, runtime, dir, &rows) {
            Err(AppError::Validation(msg)) => msg,
            other => panic!("{id} {runtime} {dir:?} must be refused, got {other:?}"),
        };
        // A new Codex row on account 2's Claude dir.
        let msg = refused("new", Runtime::Codex, Some("/Users/me/.claude-acct-2"));
        assert!(msg.contains("c1") && msg.contains("Claude"), "{msg}");
        // The claude row c1 edited to Codex, keeping its dir.
        let msg = refused("c1", Runtime::Codex, Some("/Users/me/.claude-acct-2/"));
        assert!(msg.contains("Claude"), "{msg}");
        // A Claude row on the Codex home.
        let msg = refused("new", Runtime::ClaudeCode, Some("/Users/me/.codex-bot-hq"));
        assert!(msg.contains("x1") && msg.contains("Codex"), "{msg}");
        // A Codex row with no home.
        let msg = refused("new", Runtime::Codex, None);
        assert!(msg.contains("Codex home"), "{msg}");
        // Fine: the row's own dir, a same-runtime twin, a fresh dir, a blank Claude dir.
        assert!(validate_dir_against_rows("x1", Runtime::Codex, Some("/Users/me/.codex-bot-hq"), &rows).is_ok());
        assert!(validate_dir_against_rows("new", Runtime::Codex, Some("/Users/me/.codex-bot-hq"), &rows).is_ok(), "two Codex rows may share a home (one account, two models)");
        assert!(validate_dir_against_rows("new", Runtime::ClaudeCode, Some("/Users/me/.claude-acct-2"), &rows).is_ok());
        assert!(validate_dir_against_rows("new", Runtime::ClaudeCode, None, &rows).is_ok());
        assert!(validate_dir_against_rows("new", Runtime::Codex, Some("/Users/me/.codex-other"), &rows).is_ok());
    }

    /// A Codex row's dialog command signs the account in to its home with
    /// `CODEX_HOME`, and shares nothing: a bot-hq-managed home carries no user
    /// config to link, and the hooks bot-hq writes there are its own.
    #[test]
    fn account_setup_commands_for_a_codex_row_sign_in_with_codex_home() {
        use crate::agents::account_setup::Shell;
        use crate::agents::runtime::Runtime;
        let home = if cfg!(windows) {
            std::path::Path::new(r"C:\Users\me")
        } else {
            std::path::Path::new("/Users/me")
        };
        let managed = home.join(".codex-bot-hq");
        let ok = account_setup_commands_for(managed.to_str().unwrap(), home, Runtime::Codex).unwrap();
        assert_eq!(ok.shell, Shell::host());
        let quoted = crate::agents::account_setup::quote(ok.shell, managed.to_str().unwrap());
        assert!(ok.setup.contains("CODEX_HOME"), "{}", ok.setup);
        assert!(ok.setup.contains("codex login"), "{}", ok.setup);
        assert!(ok.setup.contains(&quoted), "{}", ok.setup);
        assert!(!ok.setup.contains("claude"), "{}", ok.setup);
        assert_eq!(ok.share, "", "nothing to share into a managed Codex home");
        match account_setup_commands_for(home.join(".codex").to_str().unwrap(), home, Runtime::Codex) {
            Err(AppError::Validation(msg)) => assert!(msg.contains("default"), "{msg}"),
            other => panic!("the default home must be refused, got {other:?}"),
        }
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
        use crate::agents::runtime::Runtime;
        let home = if cfg!(windows) {
            std::path::Path::new(r"C:\Users\me")
        } else {
            std::path::Path::new("/Users/me")
        };
        let acct2 = home.join(".claude-acct-2");
        let acct2 = acct2.to_str().unwrap();
        let default_dir = home.join(".claude");
        let default_dir = default_dir.to_str().unwrap();
        let refusal = |dir: &str, home: Option<&std::path::Path>| match validate_config_dir(Some(dir), home, Runtime::ClaudeCode) {
            Err(AppError::Validation(msg)) => msg,
            other => panic!("{dir:?} must be refused, got {other:?}"),
        };

        assert_eq!(validate_config_dir(None, Some(home), Runtime::ClaudeCode).unwrap(), None);
        assert_eq!(validate_config_dir(Some("  "), Some(home), Runtime::ClaudeCode).unwrap(), None);
        assert_eq!(
            validate_config_dir(Some(&format!("{acct2}/")), Some(home), Runtime::ClaudeCode)
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
        assert!(validate_config_dir(Some(default_dir), None, Runtime::ClaudeCode).is_ok());
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
        use crate::agents::runtime::Runtime;
        let refused = |dir: &str| match account_setup_commands_for(dir, home, Runtime::ClaudeCode) {
            Err(AppError::Validation(msg)) => msg,
            other => panic!("{dir:?} must be refused, got {other:?}"),
        };
        assert!(refused("~/.claude-acct-2").contains("does not expand `~`"));
        assert!(refused(home.join(".claude").to_str().unwrap()).contains("the CLI's default dir"));
        assert!(refused("  ").contains("Type the Claude config dir first"));

        let acct2 = home.join(".claude-acct-2");
        let ok = account_setup_commands_for(&format!("{}/", acct2.display()), home, Runtime::ClaudeCode).unwrap();
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
