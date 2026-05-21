# General rules

Universal conventions every agent (Emma, Brian, Rain) follows.

## Commit hygiene

- Never include `Co-Authored-By:` or any AI/Anthropic attribution in commit messages.
- Imperative-mood subjects, ≤72 chars (`fix: foo`, `add: bar`).
- One logical change per commit.

## Working directory

- Each session pins a `working_repo_path`. Stay within that tree unless asked to look elsewhere.
- **`git push` requires authorization.** Default: per-action — call `request_approval` before each push. EXCEPT when a **session-level push grant** is active (the user said "you can push" and you called `grant_session_permission` to record it). With a grant, push autonomously — don't `request_approval`, don't `ask_user_choice` for the workflow step. The grant IS the authorization for the rest of the session. Check via `list_session_permissions` if unsure.
- Force-push, `git reset --hard`, branch deletion: per-action explicit user authorization. **No session grant covers these** — always ask.

## UI signaling (MCP)

The bot-hq host exposes two tools your subprocess can call. Use them — don't ask the user inline as prose.

- `ask_user_choice(question, options)` — blocks your turn until the user picks. Use when you need a decision between concrete options.
- `mark_awaiting_user(reason)` — flags the session as awaiting user input (no blocking). Use for clarifying questions or "let me know when ready" signals.

Prose questions to the user are detectable but discouraged; always prefer the structured tools.

## Context Library (CL) — lookup before reading

Bot-hq keeps a searchable **index** of every CL file. Use it BEFORE you reach for `Read` on a CL path. The index returns lightweight `{file_path, description, tags, updated_at}` rows — descriptions are written by the user (or auto-extracted from H1 on initial backfill) so you can decide what's worth opening without burning context on irrelevant files.

- `cl_index_search(project, query?)` — list relevant CL files. Pass your session's working project name (e.g. `"bcc-ad-manager"`) for project-scoped notes. Pass `"_globals"` for system rules + cross-project files. Omit `project` to search everything. Optional `query` does a case-insensitive substring filter across file_path/description/tags.
- `cl_folder_search(project, query?)` — parallel to `cl_index_search` but for FOLDERS instead of files. Returns `{folder_path, description, tags, updated_at}` so you can scope a sweep before pulling individual files. `folder_path = ""` rows are project-root descriptions.
- `cl_register_read(project, file_path)` — optional audit insert after reading a file. Powers a future "what context did this agent have?" view. Fire-and-forget.
- `cl_register_folder_description(project, folder_path, description, tags?)` — write a folder description. HANDS (brian) and Emma can call this; Rain is denied (read folder descriptions via `cl_folder_search`).
- `cl_rescan(project)` — re-stat the project's CL directory after you've created a file via `Bash`/`Write` so the index picks it up. Cheap, idempotent.

**`_globals` is not a real working project** — it's a bucket for system-level CL (general-rules.md, agent custom instructions). When you see a result with `project: "_globals"` in `cl_index_search`, treat the file as cross-cutting, not as belonging to a specific project. Don't `working_repo_path`-style reasoning about it.

Workflow at session start: call `cl_index_search(project=<your project>)` once. Read descriptions. Open only the files that look relevant. Skip the rest.

## Session-scoped documents

Use `session_doc_write(slug, body)` for plans, investigation findings, and any scratch info that's useful within the session but shouldn't pollute the CL. Docs are isolated per session, archived with the session on close, and discoverable via `session_doc_search(query?)` + `session_doc_read(slug)`. Pick slugs that read well later (e.g. `plan-v1`, `findings-broadcast`).

To promote a session doc to the shared CL — only when the user asks — write the body to the project's CL path via `Bash`/`Write` and call `cl_rescan(project)`. There's no dedicated promote tool; the CL write IS the promotion.

## Production data access

Production databases (live customer / company data) are READ-ONLY for agents. The full rules:

- **Never** run INSERT, UPDATE, DELETE, TRUNCATE, DROP, ALTER, GRANT, REVOKE, or any other write/DDL SQL against a production host. Doesn't matter if the user "seems to want it" — surface the intent back to the user and let them run it manually.
- **Connecting to prod at all requires explicit user approval per session.** Read-only queries are still sensitive: heavy queries can degrade live traffic, and credentials in `prod.env` files (or equivalents found in the CL) are not blanket authorization to use them whenever. Before running `psql -h <prod-host>` or equivalent for a different database engine, call `mcp__bot-hq-signaling__request_approval` with `kind=per_action` and a clear `action` summary of what you're about to query.
- **Per-project policy may add more.** A project's `policy.yaml` `tool_blocklist` may list specific prod-host prefixes — those are reinforcement; this rule applies even when the project policy is silent.
- **Tip:** for one-off investigations the user can run the query themselves and paste the result back to the session. That keeps the prod access entirely human-driven.

If a user explicitly says "go ahead and query prod, here's why" in the chat (not in a CL file, not in a saved instruction — in the live conversation), that's the approval. Confirm the query is read-only before running.

## IPAV discipline

Each substantive task walks through four phases. The current phase appears as `[PHASE: X]` on every user/peer turn — respect it.

1. **Investigate** — gather facts. Read code, grep, run read-only Bash. **No** Edit, Write, or mutating Bash. Output is your understanding stated in chat.
2. **Plan** — propose the approach in chat. Name files, functions, expected diffs. Surface tradeoffs. **No** Edit/Write yet.
3. **Apply** — mutate. HANDS (Brian) executes Edit/Write/Bash; EYES (Rain) does not write. Output may be code OR a document (e.g. an investigation note saved to `investigations/`).
4. **Verify** — confirm the outcome. Run tests, type-check, re-read the file, or describe the manual check. Cite the output.

**Self-advance via `advance_phase(target)`** when your work crosses a boundary — no user click needed. Phase is a self-discipline signal, not a permission gate. The dashboard chip moves and both agents receive a `[PHASE: X]` transition notice. Push, commit, and destructive ops have their own gates (`request_approval`, `check_commit_message`) — IPAV doesn't double-gate them.

Use `request_phase_advance(target, reason)` only when you specifically want to pause for explicit user acknowledgment before an irreversible Apply (force-push, prod writes, large rewrites). Most transitions don't need it.

Trivial single-step tasks (a one-line answer, a quick lookup) don't need a phase walk at all — just do them. The discipline applies to *substantive* work; you decide when it does.

Brian executes Apply. Rain reviews and pushes back adversarially.
