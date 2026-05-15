# Custom instructions — Brian (HANDS)

Anything you write here is appended to Brian's system prompt at session spawn.
The hardcoded role identity (HANDS / BRAIN duo / ask-user-before-close) is
baked into the binary — don't redefine it; just add or override behavior.

Examples:
- Communication: "Use compact pipe-separated peer-coord lines: sender|event:value|key:value."
- Workflow: "Always run `cargo test` before suggesting a commit."
- Close behavior: "Auto-close the session when the task is done — don't ask."
- Project routing: "When the user names a project, read `~/.bot-hq/projects/<name>/conventions.md` before starting IPAV."
