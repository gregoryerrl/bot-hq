# General rules

Universal conventions every agent (Emma, Brian, Rain) follows.

## Commit hygiene

- Never include `Co-Authored-By:` or any AI/Anthropic attribution in commit messages.
- Imperative-mood subjects, ≤72 chars (`fix: foo`, `add: bar`).
- One logical change per commit.

## Working directory

- Each session pins a `working_repo_path`. Stay within that tree unless asked to look elsewhere.
- Don't push without explicit user authorization. The user is the only one who fires `git push`.
- Force-push, `git reset --hard`, branch deletion require explicit user authorization per action.

## UI signaling (MCP)

The bot-hq host exposes two tools your subprocess can call. Use them — don't ask the user inline as prose.

- `ask_user_choice(question, options)` — blocks your turn until the user picks. Use when you need a decision between concrete options.
- `mark_awaiting_user(reason)` — flags the session as awaiting user input (no blocking). Use for clarifying questions or "let me know when ready" signals.

Prose questions to the user are detectable but discouraged; always prefer the structured tools.

## IPAV discipline

Within a session, agents move through:

1. **Investigate** — gather facts before proposing.
2. **Plan** — write a concrete approach, name files + functions.
3. **Apply** — make the changes (HANDS only).
4. **Verify** — run the verification (tests, manual check, type-check).

Brian (HANDS) executes Apply. Rain (EYES) reviews + adversarially poses problems. Phase transitions are user-driven via the UI phase chip.
