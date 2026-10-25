//! Delegated configuration uses the desktop validators and storage writers.
use super::*;
use crate::policy::{self, GatedKeyword, session_policy::SessionPolicy};
use crate::tauri_cmd::{models::{ModelView, normalize_model, validate_dir_against_rows}, roles::{RoleDraftInput, RoleView, validated_draft}};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Component, Path};

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ConfigScope {
    Global,
    Project { project: String },
    Session { session_id: String },
}

impl ConfigScope {
    fn session(&self) -> Option<&str> {
        match self { Self::Session { session_id } => Some(session_id), _ => None }
    }
    fn validate(&self) -> Result<()> {
        let name = match self { Self::Global => return Ok(()), Self::Project { project } => project, Self::Session { session_id } => session_id };
        let mut parts = Path::new(name).components();
        anyhow::ensure!(!name.contains('\\') && matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none(), "scope name must be one path component");
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdminRequest {
    Models { before_id: Option<String> },
    Model { id: String },
    SaveModel { model: Value, #[serde(default)] clear_auth_token: bool },
    DeleteModel { id: String },
    Roles { before_id: Option<i64>, #[serde(default)] include_archived: bool },
    Role { id: i64, #[serde(default)] offset_bytes: usize },
    SaveRole { id: Option<i64>, draft: Value, expected_sha256: Option<String> },
    ArchiveRole { id: i64, archived: bool },
    Capabilities,
    Setting { key: String },
    SetSetting { key: String, value: String },
    Policy { scope: ConfigScope },
    SetPolicy { scope: ConfigScope, #[serde(deserialize_with = "strict_policy")] policy: Value },
    ToolGate { scope: ConfigScope },
    SetToolGate { scope: ConfigScope, keywords: Vec<GatedKeyword> },
}

fn strict_policy<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Value, D::Error> {
    use serde::de::Error;
    let value = Value::deserialize(deserializer)?;
    let object = value.as_object().ok_or_else(||D::Error::custom("policy must be an object"))?;
    for key in object.keys() {
        if key == "tool_blocklist" {
            return Err(D::Error::custom("tool_blocklist is retired; use set_tool_gate"));
        }
        if !policy::POLICY_KNOWN_KEYS.contains(&key.as_str()) {
            return Err(D::Error::custom(format!("unknown policy field {key}")));
        }
    }
    Ok(value)
}

fn overlay_present(mut stored: Value, patch: Value) -> Result<Value> {
    let patch = patch.as_object().context("saved fields must be an object")?;
    let fields = stored.as_object_mut().context("stored fields must be an object")?;
    for (key, value) in patch {
        if value.is_object() && fields.get(key).is_some_and(Value::is_object) {
            let nested = fields.remove(key).unwrap();
            fields.insert(key.clone(), overlay_present(nested, value.clone())?);
        } else {
            fields.insert(key.clone(), value.clone());
        }
    }
    Ok(stored)
}

fn generic_setting(key: &str) -> Result<()> {
    anyhow::ensure!(matches!(key, "worktree_default" | "adherence_nudges" | "promptcodes"),
        "this setting requires its dedicated application control (such as Diagnostics or a preset installer); generic settings are worktree_default, adherence_nudges and promptcodes");
    Ok(())
}

fn role_hash(role: &crate::storage::Role) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&RoleView::try_from(role.clone())?)?)))
}

impl AdminRequest {
    pub(super) fn identity(&self) -> (&'static str, Option<&str>, bool) {
        match self {
            Self::Models { .. } => ("models", None, false), Self::Model { .. } => ("model", None, false),
            Self::SaveModel { .. } => ("save_model", None, true), Self::DeleteModel { .. } => ("delete_model", None, true),
            Self::Roles { .. } => ("roles", None, false), Self::Role { .. } => ("role", None, false),
            Self::SaveRole { .. } => ("save_role", None, true), Self::ArchiveRole { .. } => ("archive_role", None, true),
            Self::Capabilities => ("capabilities", None, false), Self::Setting { .. } => ("setting", None, false), Self::SetSetting { .. } => ("set_setting", None, true),
            Self::Policy { scope } => ("policy", scope.session(), false), Self::SetPolicy { scope, .. } => ("set_policy", scope.session(), true),
            Self::ToolGate { scope } => ("tool_gate", scope.session(), false), Self::SetToolGate { scope, .. } => ("set_tool_gate", scope.session(), true),
        }
    }
}

impl SignalingBridge {
    async fn admin_policy_path(&self, storage: &Storage, scope: &ConfigScope) -> Result<PathBuf> {
        scope.validate()?;
        let dd = self.data_dir().context("data directory is unavailable")?;
        Ok(match scope {
            ConfigScope::Global => policy::general_policy_path(dd),
            ConfigScope::Project { project } => storage.cl_path_for_project(dd, project).await?.join("policy.yaml"),
            ConfigScope::Session { session_id } => {
                anyhow::ensure!(storage.get_session(session_id).await?.is_some(), "session does not exist");
                policy::session_policy::session_policy_path(dd, session_id)
            }
        })
    }

    pub(super) async fn control_admin(&self, storage: &Storage, request: AdminRequest, claim: Option<&crate::signaling::ControlClaim>, audit: i64) -> Result<Value> {
        use AdminRequest::*;
        if !request.identity().2 { return self.read_admin(storage, request).await; }
        let claim = claim.context("delegated configuration requires a write claim")?;
        let operation = request.identity().0;
        let target = request.identity().1.unwrap_or(&claim.actor.session_id).to_string();
        // Resolve filesystem destinations before the writer claim. Session
        // snapshots retain the OTHER half (policy or keyword list) at save.
        let path = match &request {
            SetPolicy { scope, .. } | SetToolGate { scope, .. } => Some(self.admin_policy_path(storage, scope).await?),
            _ => None,
        };
        let inherited = match &request {
            SetToolGate { scope: ConfigScope::Session { session_id }, .. } | SetPolicy { scope: ConfigScope::Session { session_id }, .. } => Some(self.resolve_policy_for(session_id).await?),
            _ => None,
        };
        let mut tx = self.begin_session_control(claim, &target).await?;
        let mut file_write = false;
        let (result, object) = match request {
            SaveModel { model: patch, clear_auth_token } => {
                let id = patch.get("id").and_then(Value::as_str).context("model id is required")?;
                let before = Storage::get_model_on(&mut tx, id).await?;
                let supplied_token = patch.get("auth_token").is_some_and(|v| !v.is_null());
                anyhow::ensure!(!clear_auth_token || !supplied_token, "supply a token or clear it, not both");
                let fields: Vec<_> = patch.as_object().context("model must be an object")?.keys().cloned().collect();
                let baseline = match &before {
                    Some(model) => { let mut view = ModelView::from(model.clone()); view.auth_token = None; serde_json::to_value(view)? },
                    None => json!({"created_at":"","updated_at":"","runtime":"claude_code"}),
                };
                let view: ModelView = serde_json::from_value(overlay_present(baseline, patch)?)?;
                let mut model = normalize_model(view.into())?;
                anyhow::ensure!(!model.id.is_empty() && !model.display_name.trim().is_empty() && !model.model_name.trim().is_empty(), "model id, display name and model name are required");
                let rows = Storage::list_models_on(&mut tx).await?;
                validate_dir_against_rows(&model.id, model.runtime(), model.claude_config_dir.as_deref(), &rows)?;
                if model.auth_token.is_none() && !clear_auth_token {
                    if let Some(old) = &before {
                        anyhow::ensure!(old.auth_token.is_none() || (old.base_url == model.base_url && old.provider == model.provider && old.runtime() == model.runtime()),
                            "changing the endpoint, provider or runtime requires re-supplying the token or clear_auth_token=true");
                        model.auth_token = old.auth_token.clone();
                    }
                }
                Storage::upsert_model_on(&mut tx, &model).await?;
                (json!({"id":model.id,"saved":true}), json!({"resource":"model","id":model.id,"provided_fields":fields}))
            }
            DeleteModel { id } => (json!({"deleted":Storage::delete_model_on(&mut tx, &id).await? > 0}), json!({"resource":"model","id":id})),
            SaveRole { id, draft: patch, expected_sha256 } => {
                let baseline = match id {
                    Some(id) => {
                        let old = Storage::role_by_id_on(&mut tx, id).await?.context("role does not exist")?;
                        if let Some(expected) = &expected_sha256 { anyhow::ensure!(*expected == role_hash(&old)?, "role changed since it was read; read it again before saving"); }
                        json!({"display_name":old.display_name,"slug":null,"description_prompt":old.description_prompt,
                            "capabilities":serde_json::from_str::<Value>(&old.capabilities)?,"participation_mode":old.participation_mode,"default_model_id":old.default_model_id})
                    }
                    None => { anyhow::ensure!(expected_sha256.is_none(), "expected_sha256 requires an existing role id"); json!({}) }
                };
                let draft: RoleDraftInput = serde_json::from_value(overlay_present(baseline, patch)?)?;
                validated_draft(&draft)?;
                let role = match id {
                    Some(id) => Storage::update_role_on(&mut tx, id, &draft.into()).await?,
                    None => Storage::create_role_on(&mut tx, &draft.into()).await?,
                };
                (json!({"id":role.id,"slug":role.slug,"saved":true}), json!({"resource":"role","id":role.id,"slug":role.slug,"capabilities":serde_json::from_str::<Value>(&role.capabilities)?}))
            }
            ArchiveRole { id, archived } => {
                Storage::set_role_archived_on(&mut tx, id, archived).await?;
                let role = Storage::role_by_id_on(&mut tx, id).await?.context("role disappeared")?;
                (json!({"id":id,"archived":archived}), json!({"resource":"role","id":id,"slug":role.slug,"archived":archived}))
            }
            SetSetting { key, value } => {
                generic_setting(&key)?;
                anyhow::ensure!(value.len() <= 64_000, "setting value is too large");
                if key == "promptcodes" { anyhow::ensure!(serde_json::from_str::<Value>(&value)?.is_array(), "promptcodes must be a JSON list"); }
                else { anyhow::ensure!(matches!(value.as_str(), "0" | "1"), "this boolean setting must be 0 or 1"); }
                Storage::set_setting_on(&mut tx, &key, &value).await?;
                (json!({"key":key,"saved":true}), json!({"resource":"setting","key":key}))
            }
            SetPolicy { scope, policy: patch } => {
                let dd = self.data_dir().context("data directory is unavailable")?;
                let current = match &scope {
                    ConfigScope::Session { session_id } => policy::session_policy::read_session_policy(dd, session_id)?.map(|s|s.policy).unwrap_or_else(||inherited.unwrap()),
                    _ => policy::read_policy_file(path.as_ref().unwrap())?,
                };
                let policy: policy::Policy = serde_json::from_value(overlay_present(serde_json::to_value(current)?, patch)?)?;
                claim.check(self)?;
                match &scope {
                    ConfigScope::Session { session_id } => {
                        let tool_gate = policy::session_policy::read_session_policy(dd, session_id)?.map(|s|s.tool_gate).unwrap_or_else(|| policy::tool_gate::load(dd));
                        policy::session_policy::write_session_policy(dd, session_id, &SessionPolicy { policy, tool_gate })?;
                    }
                    _ => {
                        let path = path.as_ref().unwrap();
                        policy::write_policy_file(path, &policy)?;
                        policy::audit::record_policy_write(dd, path)?;
                    }
                }
                file_write = true; (json!({"saved":true}), json!({"resource":"policy","scope":scope}))
            }
            SetToolGate { scope, keywords } => {
                let dd = self.data_dir().context("data directory is unavailable")?;
                let keywords: Vec<_> = keywords.into_iter().filter(|k| !k.keyword.trim().is_empty()).collect();
                let object = json!({"resource":"tool_gate","scope":scope,"keyword_count":keywords.len()});
                claim.check(self)?;
                match scope {
                    ConfigScope::Global => policy::tool_gate::save(dd, &keywords)?,
                    ConfigScope::Session { session_id } => {
                        let policy = policy::session_policy::read_session_policy(dd, &session_id)?.map(|s|s.policy).unwrap_or_else(||inherited.unwrap());
                        policy::session_policy::write_session_policy(dd, &session_id, &SessionPolicy { policy, tool_gate:keywords })?;
                    }
                    ConfigScope::Project { .. } => anyhow::bail!("Tool Gate has global and session scopes, not a project scope"),
                }
                file_write = true; (json!({"saved":true}), object)
            }
            _ => anyhow::bail!("not an administrative write"),
        };
        let detail = serde_json::to_string(&object)?;
        sqlx::query("UPDATE application_control_audit SET detail = ? WHERE id = ? AND outcome = 'started'")
            .bind(&detail).bind(audit).execute(&mut *tx).await?;
        // SQL changes can still roll back on Pause. Files use the synchronous
        // atomic writer: authority is checked immediately before that write,
        // while the role's writer lock remains held. A later Pause does not
        // retroactively undo a completed configuration save.
        if !file_write { claim.check(self)?; }
        tx.commit().await?;
        let _ = self.event_tx.send(SignalingEvent::ConfigurationChanged);
        crate::core::post_system_notice(storage, Some(self), &target, crate::storage::MessageKind::SystemNotice,
            format!("Configuration action {operation} applied by delegated controller {} of {}. Target: {detail}", claim.actor.slug, claim.actor.session_id), None).await;
        Ok(result)
    }

    async fn read_admin(&self, storage: &Storage, request: AdminRequest) -> Result<Value> {
        use AdminRequest::*;
        Ok(match request {
            Models { before_id } => {
                let mut rows = storage.list_models().await?;
                rows.sort_by(|a,b| b.id.cmp(&a.id));
                let rows: Vec<_> = rows.into_iter().filter(|m|before_id.as_ref().is_none_or(|id|m.id < *id)).take(20).collect();
                let next = (rows.len() == 20).then(||rows.last().unwrap().id.clone());
                json!({"models":rows.into_iter().map(|m|json!({"id":m.id,"display_name":m.display_name,"provider":m.provider,"model_name":m.model_name,"runtime":m.runtime,"context_window":m.context_window,"auth_configured":m.auth_token.is_some()})).collect::<Vec<_>>(),"next_before_id":next})
            }
            Model { id } => {
                let model = storage.get_model(&id).await?.context("model does not exist")?;
                let configured = model.auth_token.is_some();
                let mut view = ModelView::from(model); view.auth_token = None;
                json!({"model":view,"auth_configured":configured})
            }
            Roles { before_id, include_archived } => {
                let mut rows = crate::tauri_cmd::roles::load_roles(storage, include_archived).await?;
                rows.sort_by_key(|r| std::cmp::Reverse(r.id));
                let rows: Vec<_> = rows.into_iter().filter(|r|before_id.is_none_or(|id|r.id < id)).take(20).collect();
                let next = (rows.len() == 20).then(||rows.last().unwrap().id);
                json!({"roles":rows.into_iter().map(|r|json!({"id":r.id,"slug":r.slug,"display_name":r.display_name,"participation_mode":r.participation_mode,"default_model_id":r.default_model_id,"archived":r.archived,"capabilities":r.capabilities})).collect::<Vec<_>>(),"next_before_id":next})
            }
            Role { id, offset_bytes } => {
                let view = RoleView::try_from(storage.role_by_id(id).await?.context("role does not exist")?)?;
                let body = serde_json::to_string(&view)?;
                anyhow::ensure!(offset_bytes <= body.len() && body.is_char_boundary(offset_bytes), "offset_bytes must be a UTF-8 boundary");
                let end = crate::text::floor_char_boundary(&body, (offset_bytes + 6000).min(body.len()));
                use sha2::{Digest, Sha256};
                json!({"id":id,"sha256":format!("{:x}",Sha256::digest(body.as_bytes())),"body":&body[offset_bytes..end],"offset_bytes":offset_bytes,"next_offset_bytes":(end < body.len()).then_some(end)})
            }
            Capabilities => serde_json::to_value(crate::tauri_cmd::roles::list_capabilities())?,
            Setting { key } => { generic_setting(&key)?; json!({"key":key,"value":storage.get_setting(&key).await?}) },
            Policy { scope } => {
                let path = self.admin_policy_path(storage, &scope).await?;
                let policy = match scope { ConfigScope::Session { session_id } => self.resolve_policy_for(&session_id).await?, _ => policy::read_policy_file(&path)? };
                serde_json::to_value(policy)?
            }
            ToolGate { scope } => {
                self.admin_policy_path(storage, &scope).await?;
                let dd = self.data_dir().context("data directory is unavailable")?;
                serde_json::to_value(match scope {
                    ConfigScope::Global => policy::tool_gate::load(dd),
                    ConfigScope::Session { session_id } => policy::session_policy::read_session_policy(dd, &session_id)?.map(|s|s.tool_gate).unwrap_or_else(||policy::tool_gate::load(dd)),
                    ConfigScope::Project { .. } => anyhow::bail!("Tool Gate has global and session scopes, not a project scope"),
                })?
            }
            _ => anyhow::bail!("not an administrative read"),
        })
    }
}

