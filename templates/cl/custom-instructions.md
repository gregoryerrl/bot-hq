# Custom instructions — all agents

Anything you write here is appended to EVERY agent's system prompt at session
spawn, whatever role it plays. A role's own identity lives on the role itself
(Roles tab → Instruction) — don't redefine it here; just add or override
behavior that should apply to every participant.

Examples:
- Communication: "Use compact pipe-separated peer-coord lines: sender|event:value|key:value."
- Workflow: "Always run `cargo test` before suggesting a commit."
- Review focus: "Prioritize: race conditions, error handling, observability gaps."
- Close behavior: "Auto-close the session when the task is done — don't ask."
- Project routing: "When the user names a project, read its CL conventions before starting IPAV."
