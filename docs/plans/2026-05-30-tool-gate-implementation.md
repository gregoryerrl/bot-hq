# Tool Gate — implementation hand-off spec

**Status:** design LOCKED, ready to build. Written for a FRESH session (self-contained — do not assume access to the originating session's scratch docs).
**Origin:** the 2026-05-29 fabricated-comment incident (an agent ran `gh issue comment` under the user's identity with no gate). The incident remediation shipped in commit `2fbab40` (a prompt-level `tool_blocklist` PreToolUse hard-gate). This feature REPLACES that ad-hoc gating with a real, user-configurable, executing **Tool Gate**.

---

## 1. What you're building (LOCKED design — do not relitigate)

A global, user-configured **keyword detector over agent Bash tool calls** that, on a match, either auto-runs the command or surfaces an **Approve/Reject** prompt in the bell — and **on approve, bot-hq itself executes the command** and returns the output. It is an **action request**, not a permission request.

- **Keyword list:** GLOBAL (one list for all sessions/projects), configured in bot-hq **Settings**. Each entry = `{ keyword: string, mode: "gate" | "auto_allow" }`. Stored as global config under the data dir (NOT per-project `policy.yaml`).
- **Matching:** case-insensitive; a Bash call matches if a keyword appears in the **tool name** (`bash` → gates the whole Bash tool) OR the **command string** (`gh`/`git`/`sql`/`curl`/`push` → those commands). Plain `ls`/`cat`/`npm test` with no keyword → not gated.
- **On match:**
  - `mode: gate` → the PreToolUse hook **blocks** the direct Bash call (exit code 2) with a message routing the agent to call the `action_gate(command)` MCP tool. `action_gate` surfaces Approve/Reject in the bell → **on Approve, bot-hq EXECUTES the command** in the session's working repo and returns stdout/stderr/exit to the agent. Reject → not run.
  - `mode: auto_allow` → the hook lets the command run normally (exit 0). (This is how commit/push become frictionless — add `git commit`/`git push` as `auto_allow`.)
- **Scope: Bash only (v1).** `Read`/`Grep`/`Glob`/`Write`/`Edit`/`NotebookEdit`/`WebFetch`/`WebSearch`/`Task`/`TodoWrite` and all MCP tools are EXEMPT. Reason: the risky actions (git/gh/sql/curl/rm) all run through Bash; "execute-on-approve" only maps to Bash (you can't "execute" a file-write). Keep the config model tool-general so non-Bash (approval-only) can be added later, but do not build that now.

### Confirmed decisions (with rationale — do not reopen)
1. **Mechanism = a bot-hq feature ON TOP OF claude-code's PreToolUse hook.** Gating a tool call *before* it executes has exactly ONE tripwire for built-in tools: claude-code's PreToolUse hook. bot-hq otherwise sees tool calls only *after* they run (in the stream-json output). There is no zero-hook way to gate Bash. So: bot-hq owns the config + detect→gate→execute logic; the hook is just the tripwire, injected at spawn via `--settings` (bot-hq already does this — see §3).
2. **KEEP `policy.yaml`.** The gate replaces only its `tool_blocklist` ROLE. `policy.yaml`'s `forbidden_in_commits` (the disguise words `bot-hq`/`Claude`/`Anthropic`/…) is enforced by the **git commit hooks** and is LOAD-BEARING — it's what stops an agent leaking those words into a *client* repo's commit. Also keep `push_gate`/`force_push`/`commit_style`/`branch_pattern`. Do NOT delete `policy.yaml`.
3. **Remove the commit/push GrantPills** (`SessionView.tsx`). Their auto-proceed behavior is replaced by `auto_allow` keyword entries in the global Settings.
4. **Execute-on-approve is Bash-only** (can't execute a Write). Non-Bash exempt in v1.

---

## 2. Architecture: REUSE this existing machinery (do not rebuild)

The "surface Approve/Reject → block until resolved → unblock the caller" flow ALREADY EXISTS as `request_approval`/`ask_user_choice`. `action_gate` is essentially `request_approval` + command classification + server-side execution. Key file:line:

- **Blocking+resolve machinery:** `src/signaling/bridge/questions.rs` — `ask_user_choice_inner` (~:158) parks a `tokio::oneshot` sender in a `pending` map, persists a `session_questions` row, broadcasts `SignalingEvent::PendingChoice`, and **blocks on `rx.await`**. `resolve_choice` (~:280) sends `tx.send(picked)` to unblock, logs a violation, and for approved `push_gate` writes a session grant.
- **Tool definition + dispatch:** `src/signaling/protocol.rs:174-212` (`request_approval` descriptor) and `src/signaling/jsonrpc.rs:204-240` (parse args → `bridge.request_approval(...)`). Add `action_gate` alongside.
- **Question storage:** `src/storage/questions.rs`, `migrations/0003_session_questions.sql` (`session_questions` table).
- **Bridge→UI events:** `src/tauri_events/bridge_subscriber.rs` routes `SignalingEvent::PendingChoice` → Tauri event `session:pending_choice`; constants in `src/tauri_events/types.rs`.
- **Approve/Reject UI:** `frontend/src/components/ChoicePrompt.tsx` (renders options + submits `resolve_choice`). The bell tray: `frontend/src/components/PendingTray.tsx` (already updated this session — see §6).
- **Session grants + file mirror (for push):** `src/signaling/bridge/permissions.rs` writes `<data_dir>/.local/session-permissions/<sid>.json`; the pre-push git hook reads it at `src/policy/hooks.rs` (~:291, `run_pre_push`).
- **The PreToolUse hook today:** `src/policy/hooks.rs` `run_tool_blocklist` (dispatched via `run_cli`; `main.rs:43-48` routes `policy-check`). Injected at spawn in `src/agents/spawn.rs` (the `else`/non-Rain branch, ~:285) via `--settings '<json>'` with a `PreToolUse`/`matcher:"Bash"` hook → `<exe> policy-check tool-blocklist --data-dir <d> --project <p> --session <s>`.
- **Settings page (EXISTS):** `frontend/src/app/Settings.tsx`, route in `frontend/src/Router.tsx:16`. Add the "Gated Bash Keywords" section here.
- **Tauri command pattern:** per-domain modules in `src/tauri_cmd/*.rs` (e.g. `policy.rs`, `agent_configs.rs`), registered in `src/tauri_cmd/mod.rs` + the `tauri-specta` builder (regenerates `frontend/src/lib/bindings.ts` on launch).
- **policy.yaml model + matcher:** `src/policy/mod.rs` (`Policy`, `is_blocked_command` ~:176 — prefix match; reuse the matching style).

---

## 3. ⚠ LOAD-BEARING GOTCHAS (verified this session — heed them)

1. **`{"decision":"deny"}` JSON is SILENTLY IGNORED under `--dangerously-skip-permissions`.** HANDS/Emma run in bypass mode, where a JSON permission-decision from a PreToolUse hook is NOT honored. **Only EXIT CODE 2 blocks** (a "blocking error" that fires before the permission layer; stderr is fed to the agent). The current `tool_blocklist` hook and `approval-gate.js` were both fixed to exit-2 in `2fbab40`. **Your hook MUST block via exit 2, not JSON.** Verify empirically (below) — do not assume.
2. **A hook subprocess CANNOT reach the running bot-hq app.** The signaling server binds an ephemeral, unpersisted 127.0.0.1 port; hooks get no token; and claude-code hooks have a timeout. So **do NOT** try to make the hook itself surface the prompt and wait. The hook only **blocks (exit 2) and routes to `action_gate`**; the wait + execute live in the `action_gate` MCP tool (agent-mediated, using the proven `oneshot`). This is why the design is agent-mediated, not hook-surfaces-and-waits.
3. **`action_gate` EXECUTES the command** (the "action request" model) — bot-hq runs it as a subprocess in the session's `working_repo_path`, with a timeout, capturing combined output, returning it to the agent. It does NOT just return "approved" for the agent to re-run (the agent's Bash is hard-blocked for gated commands anyway).
4. **Disguise safety:** the bcc client projects must never get `bot-hq`/`Claude` strings in their commits. The Tool Gate config is GLOBAL (bot-hq-side, under the data dir) — nothing is written into client repos, so it's disguise-safe. Keep it that way.
5. **`git push` executed by the gate still hits the pre-push git hook** (`push_gate` per_branch_approval). RECONCILE: when `action_gate` executes an approved/auto_allow push, record the session grant first (reuse `permissions.rs`/the push-grant path) so the pre-push hook passes — otherwise the gate-run push is double-gated and blocked. (commit-msg/pre-commit hooks still fire on gate-run `git commit` — that's correct; disguise check stays.)

---

## 4. Implementation sub-chunks (compile/test after each)

1. **Global keyword config store.** New module (e.g. `src/policy/tool_gate.rs` or `src/storage/`): a `GatedKeyword { keyword, mode }` model + load/save to a global file under the data dir (e.g. `<data_dir>/tool-gate.json`). Unit-test load/save + default (empty). Add `Tauri` get/set commands in a `src/tauri_cmd/tool_gate.rs`, register in `tauri_cmd/mod.rs` + the specta builder.
2. **Settings UI section.** In `frontend/src/app/Settings.tsx`, add a "Gated Bash Keywords" section: list of `{keyword, mode}` with add/remove + a Gate/Auto-allow toggle, wired to the new get/set commands (via `useTauriQuery`/`useTauriMutation`, `bindings.ts`). Typecheck: `npm --prefix frontend run lint`.
3. **Matcher + executor (pure/tested).** `fn match_keyword(tool_name, command, &[GatedKeyword]) -> Option<Mode>` (case-insensitive; checks tool name + command). `fn run_in_repo(command, cwd, timeout) -> {stdout, stderr, code}`. Unit tests incl. the `bash`-keyword-gates-all and `gh`-gates-command cases.
4. **`action_gate(command)` MCP tool.** Add descriptor (`protocol.rs`) + dispatch (`jsonrpc.rs`) + bridge method. Logic: read global keywords → `match_keyword`. No match or `auto_allow` → execute immediately, return output. `gate` → reuse `ask_user_choice_inner` to prompt Approve/Reject → on Approve execute + return output, on Reject return not-run. For push: record the session grant before executing (gotcha §3.5). Tests.
5. **PreToolUse hook rework.** Change `run_tool_blocklist` (rename to e.g. `run_tool_gate`) to read the GLOBAL keyword config (not just per-project `policy.yaml`): on a Bash command, `match_keyword`; `gate` → exit 2 with a stderr message instructing the agent to call `action_gate(command)`; `auto_allow`/no-match → exit 0. Keep `--data-dir`/`--session`. Update the `--settings` injection in `spawn.rs` if the subcommand name changes. Tests.
6. **Agent prompt rules + cleanup.** In `src/agents/general_rules.rs` (or `prompts.rs`): instruct agents that gated Bash commands must go through `action_gate(command)`. Retire `approval-gate.js`'s git/gh overlap (now owned by the gate). Remove the commit/push **GrantPills** from `frontend/src/app/SessionView.tsx` (+ the now-unused `grant/revoke_session_permission` UI if nothing else uses it — but KEEP the underlying session-permission mechanism for the pre-push push grant per §3.5).
7. **Verify (see §5).**

---

## 5. Verification plan

- **Unit:** `cargo test` (matcher, config store, action_gate, hook). `npm --prefix frontend run lint` (frontend tsc).
- **End-to-end hook block (CRITICAL — proves exit-2 honored under bypass).** Reuse this session's proven pattern: a throwaway temp data-dir with a test keyword config, build a `--settings` JSON pointing the PreToolUse Bash hook at `target/debug/bot-hq policy-check <gate-subcommand> --data-dir <tmp>`, then run a headless `claude -p --dangerously-skip-permissions --settings '<json>' "...run a gated command and an allowed command..."` and judge by **file existence** (a blocked `touch /tmp/X` must NOT create its file; an allowed one must). Do NOT judge by the agent's prose. (This is exactly how the `2fbab40` gate was verified after the JSON-deny-ignored-under-bypass surprise.)
- **action_gate execute path:** directly invoke / unit-test that an approved command actually runs in the repo and returns output; that a rejected one does not.
- **Manual:** Settings UI add/remove keyword + mode; gh command → bell Approve → executes; `git push` as auto_allow → runs without prompt.

---

## 6. State at hand-off

- **Incident remediation:** committed as `2fbab40` on `main` (UNPUSHED — push was blocked by `2fbab40`'s own gate; the user will resolve once the Tool Gate replaces it, or push manually).
- **Nav bell (chunk 1 of THIS feature): DONE but UNCOMMITTED** in the working tree — `frontend/src/components/PendingTray.tsx` (icon-only bell, floating count badge, "Notifications" labels, surfaces questions + approval-gates). Verified clean (`tsc --noEmit`). The fresh session should commit this with its work (or re-verify).
- Working tree also has a pre-existing `frontend/src/lib/bindings.ts` modification and an untracked `.claude/scheduled_tasks.lock` — NOT part of this feature; leave them.
- **Nothing else built.** Backend (config store, action_gate, hook rework), Settings UI, agent rules, and GrantPill removal are all TODO per §4.

## 7. Commit / disguise hygiene (bot-hq repo)
Commit messages follow the repo's resolved policy — forbidden words are enforced by the `commit-msg` git hook; call `check_commit_message` before committing. Push only with explicit user authorization.
