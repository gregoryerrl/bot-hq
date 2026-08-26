//! One-time starter presets for the Tool Gate and the general policy (1.0.1).
//!
//! Mirrors the roles offer (0072): migration 0077 seeds `gate_preset_offer` /
//! `policy_preset_offer` = 'pending' on FRESH installs; [`backfill_offers`]
//! covers installs that predate 0077 but never wrote the config file; the
//! Settings cards render only on the literal 'pending'; the resolve commands
//! (`tauri_cmd::tool_gate` / `tauri_cmd::policy`) write these starters and
//! stamp the key. The starters are deliberately BASIC — the destructive-command
//! class a stock Claude Code install permission-asks on, not any personal
//! workflow doctrine. The resolve commands NEVER overwrite an existing file.

use crate::policy::tool_gate::{GateMode, GatedKeyword};
use crate::storage::Storage;
use std::path::Path;

/// The starter Tool Gate list: destructive / irreversible commands only.
///
/// Curation notes, so a future edit doesn't regress them:
/// - `rm -r` subsumes `rm -rf` under substring matching — don't list both.
/// - `sudo ` carries a trailing space: the matcher is a substring check and
///   bare `sudo` would match `pseudo`.
/// - NO `git push --force`: `force_push: blocked` in the starter policy fires
///   FIRST (hooks check it before the gate), so a keyword here would park an
///   approval the hook then denies — approve-then-refuse is worse than either
///   guard alone.
pub fn starter_gate_keywords() -> Vec<GatedKeyword> {
    [
        "rm -r",
        "sudo ",
        "chmod 777",
        "dd if=",
        "mkfs",
        "git reset --hard",
        "git clean -f",
    ]
    .into_iter()
    .map(|k| GatedKeyword {
        keyword: k.to_string(),
        mode: GateMode::Gate,
    })
    .collect()
}

/// The starter `config/general-policy.yaml`. Safe basics only; every key
/// commented so the file teaches its own format. `forbidden_in_commits` ships
/// EMPTY — it is the one key that hard-blocks commits, and stock Claude Code
/// setups ADD the very trailer a ban would refuse; the example is there to
/// uncomment, not imposed.
pub const STARTER_GENERAL_POLICY_YAML: &str = "\
# Cross-project defaults — the base every project overlays. A per-project
# policy.yaml (Context Library → projects/<p>/policy.yaml) REPLACES any list
# it sets here. These apply to every project that doesn't set its own.

# Phrases the pre-commit hook refuses in commit messages and staged diffs.
# Empty by default — this key HARD-BLOCKS commits, so adopt deliberately.
# Example (uncomment to refuse AI tool names in commits; attribution
# trailers work here too):
#   forbidden_in_commits:
#     - Claude
#     - GPT
forbidden_in_commits: []

# `ask` parks every `git push` for your Approve/Reject and PAUSES the session
# until you answer; `auto` lets pushes through. NOTE: a project policy.yaml
# cannot relax this back to `auto` — the overlay treats `auto` as unset; for
# a frictionless project, flip the per-session toggle in the gear tab.
push_gate: ask

# `git push --force` refused outright. Set `allowed` if you rebase branches.
force_push: blocked

# Free-form style note injected into agent prompts (not enforced):
# commit_style: imperative-mood
";

/// Boot backfill for installs that predate migration 0077: a key that is
/// ABSENT arms only when its config file is also absent — an install that
/// configured gates or policy by hand never sees the offer (and its file is
/// never touched; only the resolve commands write files, and they refuse
/// existing ones). Idempotent: once a key holds any value, it is skipped.
pub async fn backfill_offers(storage: &Storage, data_dir: &Path) -> anyhow::Result<()> {
    let gate_file = crate::policy::tool_gate::config_path(data_dir);
    if storage.get_setting("gate_preset_offer").await?.is_none() && !gate_file.exists() {
        storage.set_setting("gate_preset_offer", "pending").await?;
    }
    let policy_file = crate::policy::general_policy_path(data_dir);
    if storage.get_setting("policy_preset_offer").await?.is_none() && !policy_file.exists() {
        storage.set_setting("policy_preset_offer", "pending").await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{ForcePushMode, Policy, PushGateMode};

    #[test]
    fn the_starter_yaml_parses_and_means_what_the_card_says() {
        // Round-trip through the REAL loader against a real data dir — a
        // starter that drifts from the Policy schema would otherwise brick
        // policy resolution for exactly the user who accepted the offer.
        let dir = tempfile::tempdir().unwrap();
        let path = crate::policy::general_policy_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, STARTER_GENERAL_POLICY_YAML).unwrap();
        let p = Policy::resolve(dir.path(), None, None).unwrap();
        assert_eq!(p.push_gate, PushGateMode::Ask);
        assert_eq!(p.force_push, ForcePushMode::Blocked);
        assert!(
            p.forbidden_in_commits.is_empty(),
            "the hard-blocking list must ship empty — the example is a comment"
        );
    }

    #[test]
    fn starter_keywords_round_trip_and_stay_basic() {
        let dir = tempfile::tempdir().unwrap();
        let kws = starter_gate_keywords();
        crate::policy::tool_gate::save(dir.path(), &kws).unwrap();
        assert_eq!(crate::policy::tool_gate::load(dir.path()), kws);
        // The curation rules above, pinned:
        assert!(kws.iter().all(|k| k.mode == GateMode::Gate));
        assert!(!kws.iter().any(|k| k.keyword == "rm -rf"), "rm -r subsumes it");
        assert!(!kws.iter().any(|k| k.keyword.contains("push --force")),
            "force-push is the policy's job; a gate here parks an approval the hook then denies");
        assert!(kws.iter().any(|k| k.keyword == "sudo "), "trailing space, or 'pseudo' matches");
    }

    #[tokio::test]
    async fn fresh_db_seeds_both_offers_via_0077() {
        // Storage::memory() runs the migrations against an empty DB — the
        // fresh-install path. Both keys must come out 'pending'.
        let s = Storage::memory().await.unwrap();
        assert_eq!(s.get_setting("gate_preset_offer").await.unwrap().as_deref(), Some("pending"));
        assert_eq!(s.get_setting("policy_preset_offer").await.unwrap().as_deref(), Some("pending"));
    }

    /// A DB that predates 0077: the keys are ABSENT (an upgrading install's
    /// migration guard skips seeding once sessions exist). memory() cannot
    /// replay that ordering, so simulate it by clearing what 0077 seeded.
    async fn pre_0077_storage() -> Storage {
        let s = Storage::memory().await.unwrap();
        sqlx::query("DELETE FROM app_settings WHERE key IN ('gate_preset_offer','policy_preset_offer')")
            .execute(s.pool())
            .await
            .unwrap();
        s
    }

    #[tokio::test]
    async fn backfill_arms_only_absent_key_with_absent_file() {
        let dir = tempfile::tempdir().unwrap();
        let s = pre_0077_storage().await;

        // Half-configured install: a hand-written tool-gate.json, no policy.
        crate::policy::tool_gate::save(dir.path(), &[]).unwrap();
        backfill_offers(&s, dir.path()).await.unwrap();
        assert_eq!(
            s.get_setting("gate_preset_offer").await.unwrap(),
            None,
            "a configured file must suppress its offer"
        );
        assert_eq!(
            s.get_setting("policy_preset_offer").await.unwrap().as_deref(),
            Some("pending")
        );

        // Idempotent, and a resolved key is never re-armed.
        s.set_setting("policy_preset_offer", "declined").await.unwrap();
        backfill_offers(&s, dir.path()).await.unwrap();
        assert_eq!(
            s.get_setting("policy_preset_offer").await.unwrap().as_deref(),
            Some("declined")
        );
    }
}
