//! Roles tab — the user-owned role templates a session's participants are
//! invited from.
//!
//! rc3 decision **D8**: there is no Agents tab in the end state. The Roles tab
//! owns the default model (`roles.default_model_id`) and the New Session dialog
//! overrides it per participant (`session_participants.model_id`). Both columns
//! ship in migration 0044, so this layer adds no schema.
//!
//! Thin over [`Storage`], per the module contract in [`super`] — with one
//! exception that is deliberate and lives HERE rather than in storage:
//! capability slugs are checked against [`Capability`], and the resulting set
//! against [`CapabilitySet::validate`]. `storage` carries no dependency on
//! `agents` (verified: nothing under `src/storage/` names `crate::agents`
//! outside a doc link), and this command layer is the Roles tab's only door, so
//! it is the last place a bad grant can be stopped before it becomes a row.

use crate::agents::capability::{Capability, CapabilitySet};
use crate::storage::{Role, RoleDraft, Storage, PARTICIPATION_MODES};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

/// A role as the Roles tab reads it.
///
/// `capabilities` is a `Vec<String>`, not the raw JSON column: the tab renders
/// a checklist, and handing it a string would put a JSON parser in the
/// frontend for a value the backend already has to parse to validate.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct RoleView {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    pub description_prompt: Option<String>,
    pub capabilities: Vec<String>,
    /// `active` | `on_mention` — see `storage::PARTICIPATION_MODES`.
    pub participation_mode: String,
    /// D8's model control. `None` = the role names no default and the invite
    /// has to choose one.
    pub default_model_id: Option<String>,
    /// Seeded by bot-hq. Still editable — the flag exists so the tab can offer
    /// "restore defaults", not to lock the row (migration 0044).
    ///
    /// **Permanently `false` since migration 0048**, which set it to 0 on every
    /// row to state that bot-hq ships no roles; `create_role` hardcodes 0 and
    /// `update_role` never writes it. Nothing may branch on it — use
    /// [`Self::has_builtin_prose`] for "does this role have a default to
    /// restore", which is the question the tab was really asking.
    pub builtin: bool,
    /// True when clearing `description_prompt` restores built-in prose rather
    /// than leaving the role with no instruction of its own.
    ///
    /// Answered in Rust, not by a slug list in TypeScript, for the same reason
    /// [`CapabilityView`] is: the set of roles the binary carries prose for
    /// lives in `agents::prompts`, and a copy in the frontend drifts silently
    /// the first time it changes.
    pub has_builtin_prose: bool,
    pub archived: bool,
}

impl TryFrom<Role> for RoleView {
    type Error = AppError;

    /// **Fails loudly on a capabilities column that is not a JSON string
    /// array**, rather than rendering it as an empty checklist.
    ///
    /// The empty rendering is the trap: a role with no capabilities is a LEGAL
    /// configuration — a role that only watches — so a malformed column
    /// shown as "grants nothing" is indistinguishable from a role the user
    /// deliberately narrowed. Worse, saving that form back would write the
    /// misreading in as fact.
    ///
    /// Reachable only from outside this layer's writes: `create_role` and
    /// `update_role` both go through `Storage::create_role` /
    /// `Storage::update_role`, which validate the shape, and 0044 seeds its two
    /// rows through SQLite's `json()`. So an error here means something wrote
    /// the column out of band.
    fn try_from(role: Role) -> Result<Self, AppError> {
        let capabilities: Vec<String> = serde_json::from_str(&role.capabilities).map_err(|e| {
            AppError::DbError(format!(
                "role {} ({}) has a malformed capabilities column: {e}",
                role.id, role.slug
            ))
        })?;
        let has_builtin_prose =
            !crate::agents::prompts::builtin_prose_for_role(&role.slug).is_empty();
        Ok(Self {
            id: role.id,
            slug: role.slug,
            display_name: role.display_name,
            description_prompt: role.description_prompt,
            capabilities,
            participation_mode: role.participation_mode,
            default_model_id: role.default_model_id,
            builtin: role.builtin,
            has_builtin_prose,
            archived: role.archived,
        })
    }
}

/// One row of the Roles tab's capability checklist.
///
/// The tab ASKS for this list rather than carrying a copy: a slug list
/// hardcoded in TypeScript drifts the first time a capability is added to
/// [`Capability`], and the drift is silent — the new grant just never appears
/// as a box, so no role can be given it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct CapabilityView {
    pub slug: String,
    pub label: String,
    pub description: String,
    /// Section heading, so 17 checkboxes render as five short groups.
    pub group: String,
    /// Slugs this one is incoherent without — [`Capability::requires`]. Sent so
    /// the checklist can say so BEFORE a save, rather than only through the
    /// `Validation` error [`validated_capabilities`] returns after one.
    pub requires: Vec<String>,
}

impl From<Capability> for CapabilityView {
    fn from(cap: Capability) -> Self {
        Self {
            slug: cap.slug().to_string(),
            label: cap.label().to_string(),
            description: cap.description().to_string(),
            group: cap.group().to_string(),
            requires: cap.requires().iter().map(|c| c.slug().to_string()).collect(),
        }
    }
}

/// The capability checklist, in render order.
///
/// Infallible — it reads a compile-time table, touches no storage, and so takes
/// no `State`. It is still a command rather than a constant in the frontend for
/// the reason on [`CapabilityView`].
#[tauri::command]
#[specta::specta]
pub fn list_capabilities() -> Vec<CapabilityView> {
    Capability::ALL.iter().copied().map(Into::into).collect()
}

/// What the Roles tab submits, for both create and edit.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct RoleDraftInput {
    pub display_name: String,
    /// `None` on create derives the slug from `display_name`; `None` on update
    /// leaves the existing slug ALONE. See [`RoleDraft::slug`] — a rename that
    /// re-derived the slug would break `ensure_session_roster`'s two literal
    /// `WHERE slug = 'hands' / 'eyes'` lookups.
    pub slug: Option<String>,
    pub description_prompt: Option<String>,
    pub capabilities: Vec<String>,
    pub participation_mode: String,
    pub default_model_id: Option<String>,
}

impl From<RoleDraftInput> for RoleDraft {
    fn from(input: RoleDraftInput) -> Self {
        Self {
            display_name: input.display_name,
            slug: input.slug,
            description_prompt: input.description_prompt,
            // Serialising a `Vec<String>` cannot fail, so the `unwrap_or` is a
            // formality rather than a swallowed error — and `"[]"` is the
            // column's own DEFAULT, so even the impossible branch stores a
            // legal value rather than a string SQLite would reject.
            capabilities: serde_json::to_string(&input.capabilities)
                .unwrap_or_else(|_| "[]".to_string()),
            participation_mode: input.participation_mode,
            default_model_id: input.default_model_id,
        }
    }
}

/// Reject grants that name nothing, and sets that cannot work.
///
/// Two distinct failures, both silent without this:
///
/// 1. **An unknown slug.** `CapabilitySet::from_slugs` is a `filter_map` over
///    `Capability::parse`, so `"edit_file"` (singular, a plausible typo) is
///    dropped on the floor. The role saves, the tab redraws it with the box
///    unticked, and the user reads that as the checkbox not sticking.
/// 2. **An incoherent set.** `GatedBash` without `RunBash` is a role that can
///    route a command for approval and then not run it. The design's own words:
///    `validate()` "refuses it rather than letting the UI ship a role that
///    cannot work".
///
/// Advisory [`CapabilitySet::warnings`] are NOT applied here. They flag legal
/// configurations (self-review, a silent worker) that the design says the tab
/// renders; turning advice into a refusal would make a documented-legal role
/// unsaveable.
fn validated_capabilities(slugs: &[String]) -> Result<(), AppError> {
    let unknown: Vec<&str> = slugs
        .iter()
        .map(String::as_str)
        .filter(|slug| Capability::parse(slug).is_none())
        .collect();
    if !unknown.is_empty() {
        return Err(AppError::Validation(format!(
            "unknown capabilit{}: {}",
            if unknown.len() == 1 { "y" } else { "ies" },
            unknown.join(", ")
        )));
    }
    let refs: Vec<&str> = slugs.iter().map(String::as_str).collect();
    CapabilitySet::from_slugs(&refs)
        .validate()
        .map_err(|errs| AppError::Validation(errs.join("; ")))
}

/// The checks a draft runs before it reaches storage.
///
/// The participation mode and the blank name are re-checked here as well as in
/// storage, and that duplication is on purpose: storage's copy is the invariant
/// (nothing may write an unschedulable mode, whoever writes it), and this one is
/// the MESSAGE. Per [`AppError`]'s own doc the frontend routes `Validation` to
/// a field highlight; the identical refusal reaching the user from storage
/// arrives as the `DbError` these command bodies map anyhow errors to, which is
/// a toast carrying an anyhow chain.
fn validated_draft(input: &RoleDraftInput) -> Result<(), AppError> {
    if input.display_name.trim().is_empty() {
        return Err(AppError::Validation("a role needs a display name".into()));
    }
    if !PARTICIPATION_MODES.contains(&input.participation_mode.as_str()) {
        return Err(AppError::Validation(format!(
            "unknown participation mode {:?} — expected one of {}",
            input.participation_mode,
            PARTICIPATION_MODES.join(", ")
        )));
    }
    validated_capabilities(&input.capabilities)
}

/// The body of [`list_roles`], split out so the flag's two branches are
/// testable.
///
/// `tauri::State` cannot be constructed in a unit test, so a `#[tauri::command]`
/// body is unreachable from `cargo test`. That is why `models.rs` (and, until
/// D8, `agent_configs.rs`) test their view conversions and the storage calls beneath
/// them and never the bodies — a convention, not a rule. The branch this holds
/// is worth stepping outside it for: read the flag backwards and the role picker
/// offers roles the user removed, which looks like the archive silently failing.
async fn load_roles(storage: &Storage, include_archived: bool) -> Result<Vec<RoleView>, AppError> {
    let roles = if include_archived {
        storage.list_roles_including_archived().await
    } else {
        storage.list_roles().await
    }
    .map_err(|e| AppError::DbError(e.to_string()))?;
    // `collect` into a `Result`, so one malformed row fails the call rather
    // than silently shortening the list — see [`RoleView::try_from`].
    roles.into_iter().map(RoleView::try_from).collect()
}

/// `include_archived = false` is what a picker wants; `true` is what the tab's
/// own list wants, so an archived role can be brought back.
#[tauri::command]
#[specta::specta]
pub async fn list_roles(
    storage: tauri::State<'_, Arc<Storage>>,
    include_archived: bool,
) -> Result<Vec<RoleView>, AppError> {
    load_roles(&storage, include_archived).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_role(
    storage: tauri::State<'_, Arc<Storage>>,
    draft: RoleDraftInput,
) -> Result<RoleView, AppError> {
    validated_draft(&draft)?;
    let role = storage
        .create_role(&draft.into())
        .await
        .map_err(|e| AppError::DbError(format!("{e:#}")))?;
    // Returns the STORED row, so the tab learns the slug that was actually
    // allocated. It cannot compute that itself — a collision suffixes it.
    RoleView::try_from(role)
}

#[tauri::command]
#[specta::specta]
pub async fn update_role(
    storage: tauri::State<'_, Arc<Storage>>,
    id: i64,
    draft: RoleDraftInput,
) -> Result<RoleView, AppError> {
    validated_draft(&draft)?;
    let role = storage
        .update_role(id, &draft.into())
        .await
        .map_err(|e| AppError::DbError(format!("{e:#}")))?;
    RoleView::try_from(role)
}

/// Decision D8: removing a role is archival. `archived = false` restores.
///
/// Not named `delete_role`, and not a one-way door, because neither is what it
/// does — see migration 0047 for why a hard delete is refused by the FK the
/// moment a single session has used the role.
#[tauri::command]
#[specta::specta]
pub async fn archive_role(
    storage: tauri::State<'_, Arc<Storage>>,
    id: i64,
    archived: bool,
) -> Result<(), AppError> {
    storage
        .set_role_archived(id, archived)
        .await
        .map_err(|e| AppError::DbError(format!("{e:#}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(display_name: &str) -> RoleDraftInput {
        RoleDraftInput {
            display_name: display_name.to_string(),
            slug: None,
            description_prompt: None,
            capabilities: vec!["read_channel".into(), "post_channel".into()],
            participation_mode: "active".into(),
            default_model_id: None,
        }
    }

    #[test]
    fn the_checklist_offers_exactly_the_slugs_the_validator_accepts() {
        // The tab builds its draft from these rows and `validated_capabilities`
        // refuses anything it does not recognise, so the two lists have to be
        // the SAME list. A row here the validator rejects is a checkbox that
        // fails every save it is ticked for; a capability missing here is a
        // grant no role can be given, silently.
        let rows = list_capabilities();
        assert_eq!(rows.len(), Capability::ALL.len());
        for row in &rows {
            assert!(
                Capability::parse(&row.slug).is_some(),
                "the tab would offer {}, which the validator rejects",
                row.slug
            );
        }
        // And ticking every box is itself a saveable role: each capability's
        // dependencies are elsewhere in the same list, so "select all" can
        // never be the one combination the backend refuses.
        let slugs: Vec<String> = rows.iter().map(|r| r.slug.clone()).collect();
        assert!(
            validated_capabilities(&slugs).is_ok(),
            "ticking every offered box must save: {:?}",
            validated_capabilities(&slugs)
        );
    }

    #[test]
    fn every_checklist_row_carries_its_dependencies() {
        // `gated_bash` without `run_bash` is the refusal the user is most
        // likely to hit, so the checklist has to be able to say so before the
        // save rather than only through the error afterwards.
        let rows = list_capabilities();
        let gated = rows
            .iter()
            .find(|r| r.slug == "gated_bash")
            .expect("gated_bash must be offered");
        assert_eq!(gated.requires, ["run_bash"]);
        assert!(!gated.label.is_empty() && !gated.description.is_empty());
        assert_eq!(gated.group, "Execution");
        // A capability with no dependency sends an empty list, not a missing
        // field — the checklist reads `requires` unconditionally.
        let read = rows
            .iter()
            .find(|r| r.slug == "read_channel")
            .expect("read_channel must be offered");
        assert!(read.requires.is_empty());
    }

    #[test]
    fn the_view_round_trips_every_field_the_tab_can_set() {
        // The view is the ONLY channel between the Roles tab and the stored
        // role, so a field dropped here is a control the user can set and never
        // persist — the failure `AgentConfigView` was bitten by when `native`
        // was missing from it.
        let role = Role {
            id: 7,
            slug: "code-reviewer".into(),
            display_name: "Code Reviewer".into(),
            description_prompt: Some("be terse".into()),
            capabilities: r#"["read_channel","file_finding"]"#.into(),
            participation_mode: "on_mention".into(),
            default_model_id: Some("m1".into()),
            builtin: true,
            archived: true,
        };
        let view = RoleView::try_from(role).unwrap();
        assert_eq!(view.id, 7);
        assert_eq!(view.slug, "code-reviewer");
        assert_eq!(view.display_name, "Code Reviewer");
        assert_eq!(view.description_prompt.as_deref(), Some("be terse"));
        assert_eq!(view.capabilities, ["read_channel", "file_finding"]);
        assert_eq!(view.participation_mode, "on_mention");
        assert_eq!(view.default_model_id.as_deref(), Some("m1"));
        assert!(view.builtin);
        assert!(view.archived);
    }

    #[test]
    fn a_malformed_capabilities_column_is_an_error_not_an_empty_checklist() {
        // "No capabilities" is a legal role, so a malformed
        // column rendered as an empty list would be indistinguishable from a
        // deliberate one — and saving that form back would write the
        // misreading in as fact.
        let role = Role {
            id: 7,
            slug: "broken".into(),
            display_name: "Broken".into(),
            description_prompt: None,
            capabilities: "{}".into(),
            participation_mode: "active".into(),
            default_model_id: None,
            builtin: false,
            archived: false,
        };
        let err = RoleView::try_from(role).expect_err("must not decode");
        assert!(matches!(err, AppError::DbError(_)), "got {err}");
        assert!(err.to_string().contains("broken"), "name the row: {err}");
    }

    #[test]
    fn the_draft_carries_capabilities_across_as_the_json_the_column_holds() {
        let mut i = input("Reviewer");
        i.capabilities = vec!["read_channel".into(), "file_finding".into()];
        i.description_prompt = Some("be terse".into());
        i.participation_mode = "on_mention".into();
        i.default_model_id = Some("m1".into());
        let draft: RoleDraft = i.into();
        assert_eq!(draft.capabilities, r#"["read_channel","file_finding"]"#);
        // Every other field crosses too. `description_prompt` is the design's
        // "ONLY stored prose" — the role's whole identity layer — so a
        // conversion that dropped it would silently blank the field the user
        // just typed and save the blank.
        assert_eq!(draft.display_name, "Reviewer");
        assert_eq!(draft.description_prompt.as_deref(), Some("be terse"));
        assert_eq!(draft.participation_mode, "on_mention");
        assert_eq!(draft.default_model_id.as_deref(), Some("m1"));
        // An empty grant list is a legal role, and must serialise to the
        // column's own default rather than to `null`.
        let mut empty = input("Watcher");
        empty.capabilities = vec![];
        assert_eq!(RoleDraft::from(empty).capabilities, "[]");
    }

    #[test]
    fn the_draft_carries_the_slug_option_across_unchanged() {
        // `slug` is the one field whose `None` is not "unset" but an
        // INSTRUCTION — derive on create, leave alone on update — so dropping
        // it in this conversion has two different silent effects: a caller-typed
        // slug ignored on create, and, worse, an intended rename that saves as
        // a no-op with the command still reporting the old slug back as if it
        // had taken. (Added after this exact conversion survived being mutated
        // to `slug: None`, killing nothing.)
        let mut renamed = input("Executor");
        renamed.slug = Some("Executor One".into());
        assert_eq!(
            RoleDraft::from(renamed).slug.as_deref(),
            Some("Executor One"),
            "the caller's slug must reach storage, which is what normalises it"
        );
        // And `None` must stay `None` rather than being filled in here: only
        // storage knows whether this is a create (derive) or an update (keep).
        assert_eq!(RoleDraft::from(input("Executor")).slug, None);
    }

    #[tokio::test]
    async fn the_archived_flag_decides_which_roles_the_tab_is_handed() {
        let storage = Storage::memory().await.unwrap();
        let created = storage
            .create_role(&input("Code Reviewer").into())
            .await
            .unwrap();
        storage.set_role_archived(created.id, true).await.unwrap();

        // false: a picker's list. The archived role is gone.
        let live = load_roles(&storage, false).await.unwrap();
        assert!(live.iter().all(|r| !r.archived));
        assert!(!live.iter().any(|r| r.id == created.id));
        assert_eq!(live.len(), 2, "the two seeded roles remain");

        // true: the tab's own list, which needs the archived row to offer an
        // un-archive at all.
        let all = load_roles(&storage, true).await.unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.iter().any(|r| r.id == created.id && r.archived));
    }

    #[tokio::test]
    async fn one_malformed_row_fails_the_list_instead_of_vanishing_from_it() {
        // `collect` into a `Result`, not a `filter_map`. Skipping the bad row
        // would hand the tab a list that is quietly one role short — and the
        // role that disappeared is the one something is already wrong with, so
        // the user's first move would be to recreate it and collide on the slug.
        let storage = Storage::memory().await.unwrap();
        // Written straight past `create_role`'s shape check, which is the only
        // way this state is reachable at all.
        sqlx::query("UPDATE roles SET capabilities = '{}' WHERE slug = 'eyes'")
            .execute(storage.pool())
            .await
            .unwrap();
        let err = load_roles(&storage, false)
            .await
            .expect_err("a malformed row must not be skipped");
        assert!(matches!(err, AppError::DbError(_)), "got {err}");
        assert!(err.to_string().contains("eyes"), "name the row: {err}");
    }

    #[test]
    fn an_unknown_capability_slug_is_refused_rather_than_silently_dropped() {
        // `CapabilitySet::from_slugs` filter_maps over `Capability::parse`, so
        // a typo like this would otherwise save as a role with one fewer grant
        // and redraw with the box unticked — which reads as a UI bug.
        let mut i = input("Editor");
        i.capabilities = vec!["read_channel".into(), "edit_file".into()];
        let err = validated_draft(&i).expect_err("a typo must not save");
        assert!(matches!(err, AppError::Validation(_)), "got {err}");
        assert!(err.to_string().contains("edit_file"), "name it: {err}");

        // The correctly spelled slug saves.
        i.capabilities = vec!["read_channel".into(), "edit_files".into()];
        assert!(validated_draft(&i).is_ok());
    }

    #[test]
    fn an_incoherent_capability_set_is_refused() {
        // `GatedBash` without `RunBash`: a role that can route a command for
        // approval and then not run it. The design says `validate()` refuses
        // this rather than letting the tab ship a role that cannot work.
        let mut i = input("Gated");
        i.capabilities = vec!["gated_bash".into()];
        let err = validated_draft(&i).expect_err("must be refused");
        assert!(err.to_string().contains("run_bash"), "explain it: {err}");

        i.capabilities = vec!["gated_bash".into(), "run_bash".into()];
        assert!(validated_draft(&i).is_ok());
    }

    #[test]
    fn advisory_warnings_never_block_a_save() {
        // `CapabilitySet::warnings` flags self-review and the silent worker.
        // Both are LEGAL configurations the design says the tab renders, so
        // turning the advice into a refusal would make a documented-legal role
        // unsaveable.
        let mut i = input("Self Reviewer");
        i.capabilities = vec![
            "read_channel".into(),
            "post_channel".into(),
            "file_finding".into(),
            "disposition_finding".into(),
        ];
        let set = CapabilitySet::from_slugs(&["file_finding", "disposition_finding"]);
        assert!(!set.warnings().is_empty(), "this set must be warn-worthy");
        assert!(validated_draft(&i).is_ok(), "a warning is not a refusal");
    }

    #[test]
    fn a_blank_name_or_an_unschedulable_mode_is_refused_with_a_field_error() {
        // `Validation` rather than `DbError`: the frontend routes the two
        // differently — field highlight versus a toast.
        let mut blank = input("   ");
        blank.slug = Some("watcher".into());
        assert!(matches!(
            validated_draft(&blank).expect_err("blank name"),
            AppError::Validation(_)
        ));

        let mut mode = input("Watcher");
        mode.participation_mode = "Active".into();
        let err = validated_draft(&mode).expect_err("capitalised mode");
        assert!(matches!(err, AppError::Validation(_)), "got {err}");
        // The ring filters on the exact string "active", so this role's
        // participants would be enabled, listed, and never handed a turn.
        assert!(err.to_string().contains("on_mention"), "list them: {err}");
    }

    #[tokio::test]
    async fn create_update_and_archive_move_the_stored_row() {
        // The commands take `tauri::State`, which cannot be built in a unit
        // test, so this drives the same storage calls the bodies do. What it
        // pins is the CONTRACT the bodies rely on: create returns the allocated
        // slug, update keeps it, archive leaves the list.
        let storage = Arc::new(Storage::memory().await.unwrap());
        let created = storage
            .create_role(&input("Code Reviewer").into())
            .await
            .unwrap();
        let view = RoleView::try_from(created.clone()).unwrap();
        assert_eq!(view.slug, "code-reviewer");
        assert_eq!(view.capabilities, ["read_channel", "post_channel"]);

        let mut edit = input("Code Reviewer");
        edit.default_model_id = Some("m1".into());
        edit.participation_mode = "on_mention".into();
        let updated = storage.update_role(created.id, &edit.into()).await.unwrap();
        let view = RoleView::try_from(updated).unwrap();
        assert_eq!(view.slug, "code-reviewer", "an edit is not a rename");
        assert_eq!(view.default_model_id.as_deref(), Some("m1"));
        assert_eq!(view.participation_mode, "on_mention");

        storage.set_role_archived(created.id, true).await.unwrap();
        let live: Vec<String> = storage
            .list_roles()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.slug)
            .collect();
        assert!(!live.contains(&"code-reviewer".to_string()));
        let all = storage.list_roles_including_archived().await.unwrap();
        assert!(all.iter().any(|r| r.id == created.id && r.archived));
    }
}
