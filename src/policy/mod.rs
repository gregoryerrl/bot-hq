//! Per-project policy: machine-readable subset of CL rules that the enforcement
//! layer (signaling bridge + UI dialogs) reads to decide which agent actions
//! need user approval, which words to grep out of commits, etc.
//!
//! Layout under `<data_dir>/`:
//!
//! ```text
//! general-policy.yaml                       (defaults — overlay base)
//! projects/<project>/policy.yaml            (per-project overrides)
//! .local/session-policies/<sid>.yaml        (per-session canonical snapshot)
//! ```
//!
//! Missing files are not errors. A project with no `policy.yaml` resolves to
//! [`Policy::default()`] (auto push, no forbidden words, no gates).
//!
//! Resolution: a session's policy is CANONICAL — once seeded at spawn from
//! general+project it is the sole source for that session (wired in
//! [`session_policy`]). Outside a session, project overlays general; lists are
//! *replaced* not merged (explicit per-project lists win).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod audit;
pub mod hooks;
pub mod session_policy;
pub mod tool_gate;
pub mod violations;

pub use audit::{audit_policy_files, audit_policy_files_at_root, MutationOutcome};
pub use hooks::{install_hooks, HookInstallReport};
pub use session_policy::SessionPolicy;
pub use tool_gate::{GateMode, GatedKeyword};
pub use violations::{ViolationKind, ViolationOutcome, ViolationsLog};

/// Resolved policy for a (general + per-project) overlay, or a session snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    /// Words/phrases that must not appear in commit messages or staged diffs.
    /// Pre-commit grep blocks the commit if any match.
    #[serde(default)]
    pub forbidden_in_commits: Vec<String>,

    /// `git push` gate. `auto` = pushes go through; `ask` = the pre-push hook
    /// blocks until the user enables pushes (the Session Settings toggle).
    #[serde(default)]
    pub push_gate: PushGateMode,

    /// Force-push gate. `blocked` = `git push --force`/`--force-with-lease`
    /// denied; `allowed` = permitted (still subject to `push_gate`).
    #[serde(default)]
    pub force_push: ForcePushMode,

    /// Bash commands that always require approval — `request_approval`
    /// kind="per_action", every invocation.
    #[serde(default)]
    pub per_action_approval: Vec<String>,

    /// Regex pattern branch names must match. Empty = no constraint.
    #[serde(default)]
    pub branch_pattern: String,

    /// Free-form commit style note (imperative, conventional, etc).
    /// Surfaced to the agent in its system prompt.
    #[serde(default)]
    pub commit_style: String,
}

/// `git push` gate. Set per tier (global/project/session); a session inherits
/// the resolved value at spawn then can flip it in the gear tab. No per-branch
/// memory — the user toggles `auto` to enable pushes for the session.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PushGateMode {
    /// No prompt — pushes go through.
    #[default]
    Auto,
    /// Pushes are gated — the pre-push hook blocks until the user flips the
    /// session push toggle to `auto`.
    Ask,
}

/// Force-push gate.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForcePushMode {
    /// `git push --force` / `--force-with-lease` denied.
    Blocked,
    /// Permissive default — no policy file = no enforcement. The user opts into
    /// blocking by writing `force_push: blocked` in policy.yaml.
    #[default]
    Allowed,
}

impl Policy {
    /// Load + resolve policy for `project` against `data_dir`.
    /// - Reads `<data_dir>/general-policy.yaml` as the base.
    /// - If `project` is `Some(p)`, overlays `<data_dir>/projects/<p>/policy.yaml`.
    /// - Either missing → contribute nothing (no error).
    /// - Parse errors return Err (loud — the user needs to know their YAML is broken).
    pub fn resolve(
        data_dir: &Path,
        project: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Self> {
        Self::resolve_at_root(data_dir, project, None, session_id)
    }

    /// Like [`resolve`] but accepts an explicit `project_root` override so
    /// callers that have already resolved a project's `cl_path` don't pay a
    /// second DB lookup. `None` for `project_root` reverts to the default
    /// convention (`<data_dir>/projects/<name>/`), which is what the CLI hook
    /// context uses (no storage handle available).
    ///
    /// When `session_id` is `Some` AND a canonical session-policy snapshot
    /// exists for it (seeded at spawn under
    /// `<data_dir>/.local/session-policies/<sid>.yaml`), that snapshot's
    /// [`Policy`] is returned VERBATIM — the general+project blueprints are NOT
    /// re-merged, because the snapshot (incl. any gear-tab user edits) is the
    /// sole source of truth for a live session. Fail-open: an unreadable /
    /// malformed snapshot is logged and we fall back to the general+project
    /// overlay so a glitchy file can't brick a session's policy resolution.
    pub fn resolve_at_root(
        data_dir: &Path,
        project: Option<&str>,
        project_root: Option<&Path>,
        session_id: Option<&str>,
    ) -> Result<Self> {
        if let Some(sid) = session_id {
            match session_policy::read_session_policy(data_dir, sid) {
                Ok(Some(sp)) => return Ok(sp.policy),
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    session_id = sid,
                    error = %e,
                    "session-policy unreadable; falling back to general+project"
                ),
            }
        }

        let general_path = data_dir.join("general-policy.yaml");
        let base = load_one(&general_path)?.unwrap_or_default();

        let overlay = match project {
            Some(p) => {
                let proj_path = match project_root {
                    Some(root) => root.join("policy.yaml"),
                    None => data_dir.join("projects").join(p).join("policy.yaml"),
                };
                load_one(&proj_path)?
            }
            None => None,
        };

        Ok(merge(base, overlay))
    }

    /// Returns true if `command` matches any prefix in `per_action_approval`.
    pub fn requires_per_action_approval(&self, command: &str) -> bool {
        let cmd = command.trim();
        self.per_action_approval
            .iter()
            .any(|prefix| cmd.starts_with(prefix.trim()))
    }

    /// Returns the first forbidden word found in `text`, if any.
    /// Case-sensitive — disguise words are typically branded names (bot-hq,
    /// Claude, Anthropic, …) where casing matters.
    pub fn first_forbidden_word(&self, text: &str) -> Option<&str> {
        self.forbidden_in_commits
            .iter()
            .find(|w| text.contains(w.as_str()))
            .map(String::as_str)
    }

    /// Render the system-prompt directive block. Agents see this prepended
    /// to their per-project context. Empty if the policy has no enforceable
    /// content (i.e., default policy).
    pub fn render_system_prompt_block(&self) -> String {
        if self.is_effectively_empty() {
            return String::new();
        }

        let mut out = String::from("## Enforcement policy (load-bearing)\n\n");
        out.push_str(
            "bot-hq enforces these rules at the tool-call boundary. The MCP \
             tools below are NOT optional: skipping them will trigger a denied \
             violation logged in `violations.jsonl`. Call them BEFORE the \
             corresponding bash command runs.\n\n",
        );

        if !self.forbidden_in_commits.is_empty() {
            out.push_str("### Commit-message disguise (pre-commit grep)\n\n");
            out.push_str(
                "Before every `git commit`, call \
                 `mcp__bot-hq-signaling__check_commit_message` with the proposed \
                 message. The tool returns either `ok` or `forbidden_word:<word>`. \
                 If a forbidden word is found, REWRITE the message — do not bypass.\n\n",
            );
            out.push_str("Forbidden words:\n");
            for w in &self.forbidden_in_commits {
                out.push_str(&format!("- `{w}`\n"));
            }
            out.push('\n');
        }

        if matches!(self.push_gate, PushGateMode::Ask) {
            out.push_str("### Push gate\n\n");
            out.push_str(
                "Pushes are GATED this session — the pre-push hook blocks \
                 `git push`. There is no per-push approval: the user enables \
                 pushes by flipping the push toggle to `auto` in Session \
                 Settings. If you need to push, ask the user to enable it.\n\n",
            );
        }

        if matches!(self.force_push, ForcePushMode::Blocked) {
            out.push_str("### Force-push\n\n");
            out.push_str(
                "Force-push is BLOCKED. Do not run `git push --force` / \
                 `--force-with-lease` under any circumstances.\n\n",
            );
        }

        if !self.per_action_approval.is_empty() {
            out.push_str("### Per-action approval (every time)\n\n");
            out.push_str(
                "Each of the following requires `request_approval` with \
                 kind=\"per_action\" — every single invocation, no remembered \
                 approval:\n\n",
            );
            for cmd in &self.per_action_approval {
                out.push_str(&format!("- `{cmd}`\n"));
            }
            out.push('\n');
        }

        if !self.branch_pattern.is_empty() {
            out.push_str(&format!(
                "### Branch naming\n\nBranches must match: `{}`\n\n",
                self.branch_pattern
            ));
        }

        if !self.commit_style.is_empty() {
            out.push_str(&format!("### Commit style\n\n{}\n\n", self.commit_style));
        }

        out
    }

    fn is_effectively_empty(&self) -> bool {
        self.forbidden_in_commits.is_empty()
            && matches!(self.push_gate, PushGateMode::Auto)
            && matches!(self.force_push, ForcePushMode::Allowed)
            && self.per_action_approval.is_empty()
            && self.branch_pattern.is_empty()
            && self.commit_style.is_empty()
    }
}

impl PushGateMode {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            PushGateMode::Auto => "auto",
            PushGateMode::Ask => "ask",
        }
    }
}

fn load_one(path: &Path) -> Result<Option<Policy>> {
    let body = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let parsed: Policy = serde_yaml::from_str(&body)
        .with_context(|| format!("parsing {} as YAML", path.display()))?;
    Ok(Some(parsed))
}

/// Overlay `overlay` onto `base`. Lists are replaced not merged when the
/// overlay sets them non-empty (so projects can carry their own exact list).
/// Scalar gates are replaced when the overlay sets a non-default value (so a
/// project that omits a gate inherits general's; a project can tighten to
/// `ask`/`blocked` but a default `auto`/`allowed` reads as "not set").
fn merge(base: Policy, overlay: Option<Policy>) -> Policy {
    let Some(o) = overlay else { return base };
    Policy {
        forbidden_in_commits: if o.forbidden_in_commits.is_empty() {
            base.forbidden_in_commits
        } else {
            o.forbidden_in_commits
        },
        push_gate: if matches!(o.push_gate, PushGateMode::Auto) {
            base.push_gate
        } else {
            o.push_gate
        },
        force_push: if matches!(o.force_push, ForcePushMode::Allowed) {
            base.force_push
        } else {
            o.force_push
        },
        per_action_approval: if o.per_action_approval.is_empty() {
            base.per_action_approval
        } else {
            o.per_action_approval
        },
        branch_pattern: if o.branch_pattern.is_empty() {
            base.branch_pattern
        } else {
            o.branch_pattern
        },
        commit_style: if o.commit_style.is_empty() {
            base.commit_style
        } else {
            o.commit_style
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(p: &Path, body: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn missing_files_resolve_to_default() {
        let dir = tempdir().unwrap();
        let p = Policy::resolve(dir.path(), Some("nope"), None).unwrap();
        assert_eq!(p, Policy::default());
        assert!(p.is_effectively_empty());
    }

    #[test]
    fn default_gates_are_auto_and_allowed() {
        let p = Policy::default();
        assert!(matches!(p.push_gate, PushGateMode::Auto));
        assert!(matches!(p.force_push, ForcePushMode::Allowed));
    }

    #[test]
    fn project_overlays_general() {
        let dir = tempdir().unwrap();
        write(
            &dir.path().join("general-policy.yaml"),
            "forbidden_in_commits:\n  - Claude\n  - GPT\n",
        );
        write(
            &dir.path().join("projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - bot-hq\n  - brian\n",
        );
        let p = Policy::resolve(dir.path(), Some("foo"), None).unwrap();
        // overlay replaces (not merges): only project list wins
        assert_eq!(p.forbidden_in_commits, vec!["bot-hq", "brian"]);
    }

    #[test]
    fn project_tightens_push_gate_over_general() {
        let dir = tempdir().unwrap();
        // general omits push_gate (defaults auto); project tightens to ask.
        write(
            &dir.path().join("projects/foo/policy.yaml"),
            "push_gate: ask\n",
        );
        let p = Policy::resolve(dir.path(), Some("foo"), None).unwrap();
        assert!(matches!(p.push_gate, PushGateMode::Ask));
    }

    #[test]
    fn session_snapshot_wins_verbatim_over_blueprints() {
        // With a session-policy snapshot present, resolve returns it VERBATIM —
        // the general+project blueprints (which DIFFER here) are ignored.
        let dir = tempdir().unwrap();
        write(
            &dir.path().join("general-policy.yaml"),
            "forbidden_in_commits:\n  - Claude\n",
        );
        write(
            &dir.path().join("projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - bot-hq\n",
        );
        let snapshot = session_policy::SessionPolicy {
            policy: Policy {
                forbidden_in_commits: vec!["SNAPSHOT-ONLY".into()],
                push_gate: PushGateMode::Ask,
                ..Policy::default()
            },
            tool_gate: Vec::new(),
        };
        session_policy::write_session_policy(dir.path(), "sess-1", &snapshot).unwrap();

        let p = Policy::resolve(dir.path(), Some("foo"), Some("sess-1")).unwrap();
        assert_eq!(p.forbidden_in_commits, vec!["SNAPSHOT-ONLY"]);
        assert!(matches!(p.push_gate, PushGateMode::Ask));
    }

    #[test]
    fn no_snapshot_falls_back_to_blueprint_merge() {
        // Absent snapshot → the general+project overlay is unchanged, even when
        // a session_id is threaded through.
        let dir = tempdir().unwrap();
        write(
            &dir.path().join("general-policy.yaml"),
            "forbidden_in_commits:\n  - Claude\n",
        );
        write(
            &dir.path().join("projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - bot-hq\n",
        );
        let p = Policy::resolve(dir.path(), Some("foo"), Some("no-snapshot")).unwrap();
        assert_eq!(p.forbidden_in_commits, vec!["bot-hq"]);
    }

    #[test]
    fn general_only_when_no_project_overlay() {
        let dir = tempdir().unwrap();
        write(
            &dir.path().join("general-policy.yaml"),
            "forbidden_in_commits:\n  - Claude\n",
        );
        let p = Policy::resolve(dir.path(), Some("nope"), None).unwrap();
        assert_eq!(p.forbidden_in_commits, vec!["Claude"]);
    }

    #[test]
    fn parse_error_is_loud() {
        let dir = tempdir().unwrap();
        write(
            &dir.path().join("general-policy.yaml"),
            "this: is\n  :: not valid yaml\n  - mixed\n",
        );
        let err = Policy::resolve(dir.path(), None, None).unwrap_err();
        assert!(err.to_string().contains("parsing"));
    }

    #[test]
    fn requires_per_action_approval_prefix_match() {
        let p = Policy {
            per_action_approval: vec!["gh release".into(), "terraform apply".into()],
            ..Policy::default()
        };
        assert!(p.requires_per_action_approval("gh release create v1"));
        assert!(p.requires_per_action_approval("terraform apply -auto-approve"));
        assert!(!p.requires_per_action_approval("gh pr list"));
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn first_forbidden_word_finds_match() {
        let mut p = Policy::default();
        p.forbidden_in_commits = vec!["bot-hq".into(), "Claude".into()];
        assert_eq!(p.first_forbidden_word("Co-authored by Claude"), Some("Claude"));
        assert_eq!(p.first_forbidden_word("clean commit"), None);
    }

    #[test]
    fn render_system_prompt_block_empty_for_default() {
        let p = Policy::default();
        assert_eq!(p.render_system_prompt_block(), "");
    }

    #[test]
    fn render_system_prompt_block_includes_forbidden_words() {
        let p = Policy {
            forbidden_in_commits: vec!["bot-hq".into()],
            ..Policy::default()
        };
        let block = p.render_system_prompt_block();
        assert!(block.contains("check_commit_message"));
        assert!(block.contains("bot-hq"));
    }

    #[test]
    fn render_system_prompt_block_includes_push_gate() {
        let p = Policy {
            push_gate: PushGateMode::Ask,
            ..Policy::default()
        };
        let block = p.render_system_prompt_block();
        assert!(block.contains("Push gate"));
        assert!(block.contains("Session Settings"));
    }

    #[test]
    fn render_system_prompt_block_force_push_blocked() {
        let p = Policy {
            force_push: ForcePushMode::Blocked,
            ..Policy::default()
        };
        let block = p.render_system_prompt_block();
        assert!(block.contains("Force-push"));
        assert!(block.contains("BLOCKED"));
    }
}
