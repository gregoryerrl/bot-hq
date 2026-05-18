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

## Context Library (CL) — lookup before reading

Bot-hq keeps a searchable **index** of every CL file. Use it BEFORE you reach for `Read` on a CL path. The index returns lightweight `{file_path, description, tags, updated_at}` rows — descriptions are written by the user (or auto-extracted from H1 on initial backfill) so you can decide what's worth opening without burning context on irrelevant files.

- `cl_index_search(project, query?)` — list relevant CL files. Pass your session's working project name (e.g. `"bcc-ad-manager"`) for project-scoped notes. Pass `"_globals"` for system rules + cross-project files. Omit `project` to search everything. Optional `query` does a case-insensitive substring filter across file_path/description/tags.
- `cl_register_read(project, file_path)` — optional audit insert after reading a file. Powers a future "what context did this agent have?" view. Fire-and-forget.
- `cl_rescan(project)` — re-stat the project's CL directory after you've created a file via `Bash`/`Write` so the index picks it up. Cheap, idempotent.

**`_globals` is not a real working project** — it's a bucket for system-level CL (general-rules.md, agent custom instructions). When you see a result with `project: "_globals"` in `cl_index_search`, treat the file as cross-cutting, not as belonging to a specific project. Don't `working_repo_path`-style reasoning about it.

Workflow at session start: call `cl_index_search(project=<your project>)` once. Read descriptions. Open only the files that look relevant. Skip the rest.

## IPAV discipline

Within a session, agents move through:

1. **Investigate** — gather facts before proposing.
2. **Plan** — write a concrete approach, name files + functions.
3. **Apply** — make the changes (HANDS only).
4. **Verify** — run the verification (tests, manual check, type-check).

Brian (HANDS) executes Apply. Rain (EYES) reviews + adversarially poses problems. Phase transitions are user-driven via the UI phase chip.
