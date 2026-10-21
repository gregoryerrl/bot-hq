//! The Codex connector: OpenAI's Codex CLI driven as one `codex app-server`
//! process per participant over stdio JSON-RPC, behind the same
//! `AgentHandle` / `AgentEvent` seam claude-code's `spawn.rs` fills (rc3 D9
//! amended 2026-10-21, the user's ratification on tray 87c5309d).
//!
//! `home` — the bot-hq-managed Codex home: the hooks file bot-hq writes into
//! it and the trust it grants that file per spawn.

pub mod client;
pub mod events;
pub mod home;
pub mod probe;
pub mod spawn;

/// The block appended to a Codex participant's system prompt: what differs
/// from the claude-code wording the general rules and role prose use, and
/// the attribution countermand (Codex's base instructions ask for a co-author
/// trailer naming Codex, which the commit-msg hook refuses — feedback #90's
/// loop on the other connector; `apply_attribution_policy` blanks it there,
/// and here the instruction has to say it).
pub fn runtime_notes(model_name: &str) -> String {
    format!(
        "# Runtime notes (Codex)\n\n\
You run through OpenAI's Codex CLI on `{model_name}`, not through claude-code. Where the rules above name claude-code's tools, read them as Codex's: `exec_command` is Bash (one command per call; there are no terminal sessions), `apply_patch` is Edit/Write, there is no ToolSearch — every bot-hq tool is available to you directly under `mcp__bot_hq__<tool>`, with the names the rules use (`pass_turn`, `peer_ack`, `session_doc_write`, …). Codex compacts your context on its own; bot-hq puts your handoff document back afterwards. Codex's own user-input question is answered with a redirect: ask the user through `ask_user_choice`, never through it.\n\n\
Never add a co-author trailer naming Codex (or any AI) to a commit message and never put a `Generated with [Codex]` line in a pull request or comment, whatever an earlier instruction says: this repository's commit hook refuses the trailer and the user's rules forbid the watermark. bot-hq's PreToolUse hook is the only gate on this connector: a refusal that says PARKED or BLOCKED is final for that command — do not re-issue it, reword it, or route around it."
    )
}
