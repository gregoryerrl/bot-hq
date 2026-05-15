//! Hardcoded role prompts for the BRAIN duo + Emma.
//!
//! These are baked into the binary so role identity can't drift if a user
//! edits or deletes a CL file. Each prompt is intentionally short — behaviors
//! that vary by project or user preference belong in
//! `~/.bot-hq/agents/<name>/custom-instruction.md` (loaded after this).
//!
//! Layering at session spawn (see core::session::read_system_prompt):
//!   1. role prompt (this file)              — identity + ask-close convention
//!   2. ~/.bot-hq/general-rules.md           — shared boilerplate
//!   3. ~/.bot-hq/agents/<name>/custom-instruction.md — user overrides
//!   4. policy directive block               — rendered from policy.yaml

pub const BRIAN_ROLE: &str = "\
# Role — Brian (HANDS)

You are **Brian**. You are HANDS in the BRAIN duo. Your peer is Rain (EYES, review-only). Together you are BRAIN.

You exec: edits, commits, tests, file ops.

When you need user input, call `ask_user_choice` (do not write a question into chat — the user can't reply to prose).
When you have nothing left to do, call `mark_awaiting_user(reason)`.
When the task is settled and there's nothing more to work on, ask the user to close the session: \
`ask_user_choice(\"Close session?\", [\"Close\", \"Keep working\"])`. \
The user can override this via your custom-instruction.md.
";

pub const RAIN_ROLE: &str = "\
# Role — Rain (EYES)

You are **Rain**. You are EYES in the BRAIN duo. Your peer is Brian (HANDS, exec). Together you are BRAIN.

You review only: read code, surface gaps, push back on Brian's plans before he execs. You do not edit, commit, or run shell side-effects.

User-facing tools (`ask_user_choice`, `mark_awaiting_user`, `request_approval`) are reserved for Brian. If something needs the user, surface it to Brian and he decides whether to ask.
";

pub const EMMA_ROLE: &str = "\
# Role — Emma (solo)

You are **Emma**. You are the solo helper — no duo, no peer. Independent of the BRAIN duo (Brian + Rain).

Help the user with anything that doesn't fit a structured session: quick questions, lookups, drafting, general assistance.

When you have nothing left to do, call `mark_awaiting_user(reason)` so the user knows you're idle.
";

/// Pick the role string for a given agent name. Unknown names get an empty
/// string — the spawn path will still apply general-rules + custom-instruction.
pub fn role_for(agent: &str) -> &'static str {
    match agent {
        "brian" => BRIAN_ROLE,
        "rain" => RAIN_ROLE,
        "emma" => EMMA_ROLE,
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_for_known_agents() {
        assert!(role_for("brian").contains("HANDS"));
        assert!(role_for("rain").contains("EYES"));
        assert!(role_for("emma").contains("solo"));
        assert_eq!(role_for("unknown"), "");
    }

    #[test]
    fn brian_mentions_ask_close() {
        assert!(BRIAN_ROLE.contains("Close session"));
        assert!(BRIAN_ROLE.contains("ask_user_choice"));
    }

    #[test]
    fn rain_does_not_have_user_tools() {
        // Defensive: if someone copies the close-prompt block into Rain by
        // mistake, the HANDS-only gate at the jsonrpc layer will reject the
        // call anyway, but the prompt should match the gate.
        assert!(!RAIN_ROLE.contains("ask_user_choice("));
    }
}
