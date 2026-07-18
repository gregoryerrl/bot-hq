# Custom instructions — all agents

Anything you write here is appended to EVERY agent's system prompt at session
spawn (Brian and Rain alike). The hardcoded role identities (HANDS / EYES /
BRAIN duo / ask-user-before-close) are baked into the binary — don't redefine
them; just add or override behavior.

Examples:
- Communication: "Use compact pipe-separated peer-coord lines: sender|event:value|key:value."
- Workflow: "Always run `cargo test` before suggesting a commit."
- Review focus: "Prioritize: race conditions, error handling, observability gaps."
- Close behavior: "Auto-close the session when the task is done — don't ask."
- Project routing: "When the user names a project, read its CL conventions before starting IPAV."
