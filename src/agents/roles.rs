//! What an agent is FOR — the single place a name becomes capabilities.
//!
//! ## Why this exists
//!
//! "Is this agent EYES?" had four independent answers, and two of them were
//! wrong in ways that only showed up together:
//!
//! - `resolve_agent_kind` asked `agent_name != "brian"` — a deny-list, so an
//!   unrecognised name could reach the native loop;
//! - `ToolPolicy::for_agent` matched `"rain"` and `_` to the same value, so it
//!   was a filter in shape only;
//! - `CommandPolicy::for_agent` said an unrecognised agent gets NO shell and
//!   had tests asserting it — but nothing called it. Production derived the
//!   command policy from the tool policy instead, which handed that same agent
//!   a read-only shell. **The suite certified a guarantee the code did not
//!   provide.**
//! - and the list of agent names was hardcoded separately again for cleanup.
//!
//! One enum answers all four, so adding a third agent is one edit rather than
//! four, and the safe default is stated once instead of re-derived.
//!
//! ## Scope
//!
//! This covers CAPABILITY questions. It deliberately does not absorb the other
//! name-keyed lookups in the tree — `signaling/jsonrpc.rs`'s HANDS_ONLY /
//! EYES_ONLY gates, `claude_config`'s per-agent overrides,
//! `storage/sessions.rs`'s column mapping, `prompts::role_for`, or
//! `spawn.rs`'s DeepSeek-gateway workaround. Those are different concerns or
//! plain name→field lookups; folding them in here would be churn in a policy
//! layer with its own tests and its own reasons.

use super::native::command::CommandPolicy;
use super::native::tools::ToolPolicy;

/// The two roles bot-hq ships. Brian is HANDS, Rain is EYES; together they are
/// BRAIN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    /// Executes: edits, commits, tests, file ops.
    Hands,
    /// Reviews. Read-only by construction, not by prompt.
    Eyes,
}

impl AgentRole {
    /// Every agent name bot-hq knows. Use this instead of re-listing them —
    /// the session-close sweep and any future per-agent iteration should read
    /// from one place.
    pub const NAMES: [&'static str; 2] = ["brian", "rain"];

    /// The role for an agent name, or `None` when the name is unrecognised.
    ///
    /// Deliberately NOT total. A `_ =>` arm here would pick one default for
    /// every caller; making it `None` forces each to state its own safe
    /// answer, which is how the deny-list in `resolve_agent_kind` ended up
    /// letting an unknown name through.
    pub fn for_agent(name: &str) -> Option<Self> {
        match name {
            "brian" => Some(Self::Hands),
            "rain" => Some(Self::Eyes),
            _ => None,
        }
    }

    /// May this role run on bot-hq's native loop rather than `claude-code`?
    ///
    /// HANDS cannot: the subscription OAuth Brian runs on is bound server-side
    /// to the CLI, so a native loop would have no valid credential — and he
    /// depends on the CLI's skills, plugins and full built-in tool surface,
    /// none of which the native path implements.
    pub fn may_run_native(self) -> bool {
        match self {
            Self::Hands => false,
            Self::Eyes => true,
        }
    }

    /// Which built-in tools this role may use.
    ///
    /// EYES is read-only because EYES reviews — a role decision, not a limit of
    /// the loop. HANDS has no native loop to get tools from
    /// ([`may_run_native`](Self::may_run_native)), so the conservative answer
    /// costs nothing and is the right direction to be wrong in.
    pub fn tool_policy(self) -> ToolPolicy {
        match self {
            Self::Hands | Self::Eyes => ToolPolicy::ReadOnly,
        }
    }

    /// What this role may execute via `run_command`.
    ///
    /// HANDS gets `None` rather than a read-only shell: no native HANDS exists,
    /// and silently granting a shell to a role that shouldn't be here at all is
    /// how an enforcement gap starts.
    pub fn command_policy(self) -> CommandPolicy {
        match self {
            Self::Hands => CommandPolicy::None,
            Self::Eyes => CommandPolicy::ReadOnly,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_shipped_agents_map_to_their_roles() {
        assert_eq!(AgentRole::for_agent("brian"), Some(AgentRole::Hands));
        assert_eq!(AgentRole::for_agent("rain"), Some(AgentRole::Eyes));
    }

    #[test]
    fn an_unrecognised_name_has_no_role() {
        // Not a defaulted role — callers must state their own safe answer.
        // The deny-list this replaces (`!= "brian"`) let an unknown name onto
        // the native loop.
        for name in ["emma", "", "Rain", "brian2", "root"] {
            assert_eq!(AgentRole::for_agent(name), None, "{name} got a role");
        }
    }

    #[test]
    fn hands_cannot_run_native() {
        assert!(!AgentRole::Hands.may_run_native());
        assert!(AgentRole::Eyes.may_run_native());
    }

    #[test]
    fn only_eyes_may_run_commands() {
        // The contradiction this type exists to remove: `CommandPolicy::for_agent`
        // promised exactly this and production never asked it, deriving a
        // read-only shell from the tool policy instead.
        assert_eq!(AgentRole::Eyes.command_policy(), CommandPolicy::ReadOnly);
        assert_eq!(AgentRole::Hands.command_policy(), CommandPolicy::None);
    }

    #[test]
    fn no_role_gets_write_tools() {
        for role in [AgentRole::Hands, AgentRole::Eyes] {
            assert_eq!(role.tool_policy(), ToolPolicy::ReadOnly);
        }
    }

    #[test]
    fn every_name_in_names_resolves_to_a_role() {
        // NAMES and `for_agent` must not drift — the sweep in `core/state.rs`
        // iterates the former while the spawn path asks the latter.
        for name in AgentRole::NAMES {
            assert!(
                AgentRole::for_agent(name).is_some(),
                "{name} is in NAMES but has no role"
            );
        }
    }
}
