//! Per-project policy: machine-readable subset of CL rules that the enforcement
//! layer (signaling bridge + UI dialogs) reads to decide which agent actions
//! need user approval, which words to grep out of commits, etc.
//!
//! Layout under `<data_dir>/`:
//!
//! ```text
//! general-policy.yaml                       (defaults — overlay base)
//! projects/<project>/policy.yaml            (per-project overrides)
//! ```
//!
//! Missing files are not errors. A project with no `policy.yaml` resolves to
//! [`Policy::default()`] (no forbidden words, no push gate, no blocklist).
//!
//! Resolution: project overlays general. Lists are *replaced* not merged
//! (explicit per-project lists win), so projects can both add and remove.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod audit;
pub mod hooks;
pub mod session_permissions;
pub mod violations;

pub use audit::{audit_policy_files, MutationOutcome};
pub use hooks::{install_hooks, HookInstallReport};
pub use session_permissions::{GrantScope, PermissionAction, SessionPermissions};
pub use violations::{ViolationKind, ViolationOutcome, ViolationsLog};

/// Resolved policy for a (general + per-project) overlay.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    /// Words/phrases that must not appear in commit messages or staged diffs.
    /// Pre-commit grep blocks the commit if any match.
    #[serde(default)]
    pub forbidden_in_commits: Vec<String>,

    /// `git push` approval mode.
    #[serde(default)]
    pub push_gate: PushGate,

    /// Force-push behavior.
    #[serde(default)]
    pub force_push: ForcePush,

    /// Bash commands that require approval before running.
    /// Matched as a prefix on the full command string.
    #[serde(default)]
    pub tool_blocklist: Vec<String>,

    /// Bash commands that always ask, no remembered approval.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PushGate {
    /// `auto` | `per_branch_approval` | `always_ask`.
    pub mode: PushGateMode,
    /// Branch names auto-approved after first ask (persisted by the app).
    pub remembered_approvals: Vec<String>,
}

impl Default for PushGate {
    fn default() -> Self {
        Self {
            mode: PushGateMode::Auto,
            remembered_approvals: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PushGateMode {
    /// No prompt — pushes go through.
    Auto,
    /// Prompt once per branch, then remember.
    PerBranchApproval,
    /// Prompt every single push.
    AlwaysAsk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ForcePush {
    /// `blocked` | `token_required` | `allowed`.
    pub mode: ForcePushMode,
    /// Token template (when mode = token_required). Supports `{branch}` and
    /// `{sha}` placeholders. The user must type a string matching this exact
    /// format substituted with the actual branch + SHA being pushed.
    pub token_format: String,
}

impl Default for ForcePush {
    /// Permissive default: no policy file = no enforcement. The user opts in
    /// to blocking by writing `force_push.mode: blocked` (or token_required)
    /// in policy.yaml. We don't enforce rules the user didn't ask for.
    fn default() -> Self {
        Self {
            mode: ForcePushMode::Allowed,
            token_format: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForcePushMode {
    Blocked,
    TokenRequired,
    Allowed,
}

impl Policy {
    /// Load + resolve policy for `project` against `data_dir`.
    /// - Reads `<data_dir>/general-policy.yaml` as the base.
    /// - If `project` is `Some(p)`, overlays `<data_dir>/projects/<p>/policy.yaml`.
    /// - Either missing → contribute nothing (no error).
    /// - Parse errors return Err (loud — the user needs to know their YAML is broken).
    ///
    /// `remembered_approvals` from `policy.yaml` are PERMANENT (user-curated).
    /// `_session_id` is reserved for a future permanent-permissions-per-session
    /// overlay; today it's unused. Session-level grants live in a separate
    /// module ([`crate::policy::session_permissions`]).
    pub fn resolve(
        data_dir: &Path,
        project: Option<&str>,
        _session_id: Option<&str>,
    ) -> Result<Self> {
        let general_path = data_dir.join("general-policy.yaml");
        let base = load_one(&general_path)?.unwrap_or_default();

        let overlay = match project {
            Some(p) => {
                let proj_path = data_dir.join("projects").join(p).join("policy.yaml");
                load_one(&proj_path)?
            }
            None => None,
        };

        Ok(merge(base, overlay))
    }

    /// Returns true if `command` matches any prefix in `tool_blocklist`.
    pub fn is_blocked_command(&self, command: &str) -> bool {
        let cmd = command.trim();
        self.tool_blocklist
            .iter()
            .any(|prefix| cmd.starts_with(prefix.trim()))
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

        if !matches!(self.push_gate.mode, PushGateMode::Auto) {
            out.push_str("### Push gate\n\n");
            out.push_str(&format!(
                "Push mode: **{}**. Before every `git push`, call \
                 `mcp__bot-hq-signaling__request_approval` with kind=\"push_gate\". \
                 The user picks Approve / Deny in the bot-hq UI.\n\n",
                self.push_gate.mode.label()
            ));
        }

        if matches!(self.force_push.mode, ForcePushMode::Blocked) {
            out.push_str("### Force-push\n\n");
            out.push_str(
                "Force-push is BLOCKED. Do not call `git push --force` / \
                 `--force-with-lease` under any circumstances.\n\n",
            );
        } else if matches!(self.force_push.mode, ForcePushMode::TokenRequired) {
            out.push_str("### Force-push\n\n");
            out.push_str(&format!(
                "Force-push requires a verbatim user token. Call \
                 `mcp__bot-hq-signaling__request_approval` with kind=\"force_push\". \
                 Token format: `{}`. No token, no force-push.\n\n",
                self.force_push.token_format
            ));
        }

        if !self.tool_blocklist.is_empty() {
            out.push_str("### Blocked bash commands\n\n");
            out.push_str(
                "Each of the following requires `request_approval` with \
                 kind=\"tool_blocklist\" before running:\n\n",
            );
            for cmd in &self.tool_blocklist {
                out.push_str(&format!("- `{cmd}`\n"));
            }
            out.push('\n');
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
            out.push_str(&format!(
                "### Commit style\n\n{}\n\n",
                self.commit_style
            ));
        }

        out
    }

    fn is_effectively_empty(&self) -> bool {
        self.forbidden_in_commits.is_empty()
            && matches!(self.push_gate.mode, PushGateMode::Auto)
            && matches!(self.force_push.mode, ForcePushMode::Allowed)
            && self.tool_blocklist.is_empty()
            && self.per_action_approval.is_empty()
            && self.branch_pattern.is_empty()
            && self.commit_style.is_empty()
    }
}

impl PushGateMode {
    fn label(&self) -> &'static str {
        match self {
            PushGateMode::Auto => "auto",
            PushGateMode::PerBranchApproval => "per_branch_approval",
            PushGateMode::AlwaysAsk => "always_ask",
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
/// Scalars (push_gate.mode, etc.) are replaced when overlay defines them.
fn merge(base: Policy, overlay: Option<Policy>) -> Policy {
    let Some(o) = overlay else { return base };
    Policy {
        forbidden_in_commits: if o.forbidden_in_commits.is_empty() {
            base.forbidden_in_commits
        } else {
            o.forbidden_in_commits
        },
        push_gate: if matches!(o.push_gate.mode, PushGateMode::Auto)
            && o.push_gate.remembered_approvals.is_empty()
        {
            base.push_gate
        } else {
            o.push_gate
        },
        force_push: if matches!(o.force_push.mode, ForcePushMode::Allowed)
            && o.force_push.token_format.is_empty()
        {
            base.force_push
        } else {
            o.force_push
        },
        tool_blocklist: if o.tool_blocklist.is_empty() {
            base.tool_blocklist
        } else {
            o.tool_blocklist
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
    fn is_blocked_command_prefix_match() {
        let mut p = Policy::default();
        p.tool_blocklist = vec!["git push".into(), "rm -rf".into()];
        assert!(p.is_blocked_command("git push origin main"));
        assert!(p.is_blocked_command("rm -rf /tmp/foo"));
        assert!(!p.is_blocked_command("git status"));
        assert!(!p.is_blocked_command("ls"));
    }

    #[test]
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
        let mut p = Policy::default();
        p.forbidden_in_commits = vec!["bot-hq".into()];
        let block = p.render_system_prompt_block();
        assert!(block.contains("check_commit_message"));
        assert!(block.contains("bot-hq"));
    }

    #[test]
    fn render_system_prompt_block_includes_push_gate() {
        let mut p = Policy::default();
        p.push_gate.mode = PushGateMode::PerBranchApproval;
        let block = p.render_system_prompt_block();
        assert!(block.contains("Push gate"));
        assert!(block.contains("per_branch_approval"));
        assert!(block.contains("request_approval"));
    }

    #[test]
    fn render_system_prompt_block_force_push_token_required() {
        let mut p = Policy::default();
        p.force_push.mode = ForcePushMode::TokenRequired;
        p.force_push.token_format = "force-push-greenlight: {branch}@{sha}".into();
        let block = p.render_system_prompt_block();
        assert!(block.contains("Force-push"));
        assert!(block.contains("force-push-greenlight"));
    }
}
