# bot-hq — Architecture

**Last revised:** 2026-05-14
**Status:** Active design for the from-scratch rebuild.

This document is the single source of truth for architectural decisions in the bot-hq rebuild. Read this BEFORE PLAN.md.

---

## Overview

bot-hq is a desktop GUI app for driving AI-assisted coding sessions through a bilateral-duo agent model (Brian = HANDS, Rain = EYES) with an optional helper agent (Emma). The user is the orchestrator; the app is the conductor between user and agents.

**Stack:** Rust + Slint single-binary desktop app.
**Working dir during dev:** `~/Projects/bot-hq-rebuild` (working tree of the `bot-hq` git repo). Product name remains **bot-hq** — the `-rebuild` suffix is just the dev path until current bot-hq is decommissioned.

**Why this rebuild:** The current bot-hq (Go daemon + tmux + MCP hub + 29-tool surface + Emma forwarder) accreted complexity to compensate for not having a UI conductor. A native desktop GUI lets the user *be* the conductor, collapsing the hub-and-forwarder layers. Emma demotes from auto-orchestrator to summonable chat-helper.

---

## Core decisions

### Agents

claude-code CLI as subprocess for ALL agents (Emma/Brian/Rain), regardless of model. Spawned with:

```
claude -p \
  --input-format stream-json \
  --output-format stream-json \
  --append-system-prompt-file <concat-file> \
  --mcp-config <ui-signaling-server-config>
```

Per-agent model swap via env-vars from the `agent_configs` table:
- `ANTHROPIC_BASE_URL`
- `ANTHROPIC_AUTH_TOKEN`
- `ANTHROPIC_MODEL`

One code path serves Anthropic, DeepSeek, and any Anthropic-compatible API.

### Slint UI layout

**Topbar:** `Dashboard | Context Library | Settings` + Emma button (float-right).

**Dashboard:** grid of session tiles. Each tile shows:
- Scope title
- Phase chip (I/P/A/V)
- Last activity timestamp
- `[Need User Input]` badge (when applicable)
- Inline clickable choices (when the duo is awaiting picked-option)

Click tile → opens full session view (replaces dashboard grid in content area; topbar Dashboard tab stays highlighted).

**Session view:**
- Top header: phase subtitle + scope title + `← All sessions` back link + interactive IPAV PhaseSelector (segmented control, current phase highlighted)
- Single chronological chat: all messages (user, brian, rain, phase_change) interleaved by `created_at` in one column. Each bubble visually distinguishes author via color (brian = orange, rain = purple, user = blue, system = muted grey) and alignment (user right-aligned in accent-soft, agents left-aligned in elevated surface, phase_change as centered muted italic system line). Replaces an earlier two-pane Brian-left/Rain-right design that hid the user's own messages.
- `[Need User Input]` / pending-choice banner above the prompt bar when applicable (purple for choice, red for awaiting)
- Bottom: broadcast prompt bar (single bar → both agents receive it)

**Emma overlay:** half-pane on right when opened. Distinct header bar (name + presence dot + status + close ×) + divider line for clear visual separation from the underlying view. Toggle via Emma button.

**Tile suppression rule:** when a session is the active view, its tile suppresses choice buttons and `[Need User Input]` flag — those render inline in the session view instead. Never duplicated.

**Context Library tab:** file tree + editor for `<data_dir>/`. Explicit-save UI (no auto-accumulation from agents).

**Settings tab:** per-agent config (provider/model/base_url/auth_token).

**No webUI** — desktop only.

### Bilateral duo coordination

Slint reads each agent's stream-json output and forwards to the peer's stdin as a peer message. Buffer rules per phase:

- **Investigate / Plan:** 1.5s buffer OR `message_stop`, whichever first. Preserves live adversarial riff between Brian and Rain.
- **Apply / Verify:** pure turn-based — forward only on `message_stop`. Less interleaving, more execution focus.

Tool-use events (`ask_user_choice`, `mark_awaiting_user`) are NOT forwarded to peer — they're UI signaling, not agent-to-agent coordination.

### UI signaling — embedded MCP server

Slint runs a local MCP server. Each agent subprocess spawned with `--mcp-config` pointing to it. Tools (2 to start):

- `ask_user_choice(question: string, options: array<string>) -> string`
  - Blocks the agent's turn until the user picks an option. Agent literally cannot proceed without an answer.
- `mark_awaiting_user(reason: string) -> void`
  - Sets the `[Need User Input]` tag on the session's dashboard tile + session-view banner.

**Why tool-based instead of convention-based markup:**
- Structured: typed args, no regex parsing
- Blocking: claude-code's runtime won't continue without `tool_result`
- Validated: malformed args fail at schema level

NOT a port of the old 29-tool hub surface. Distinct purpose: UI↔agent signaling, not agent-to-agent coordination.

**Residual-risk mitigation (deferred):** `Stop` hook to detect un-tooled prose questions and force re-emit. Add only if it shows up as a real problem.

### bot-hq.db (sqlite)

**Tables:**
- `messages` (id PK, session_id, author, kind, content, created_at)
- `sessions` (id PK, title, working_repo_path, created_at, closed_at, archived)
- `agent_configs` (agent_name PK, provider, model_name, base_url, auth_token)

**Author enum:** `user` / `emma` / `brian` / `rain`. NO `system` author — heartbeat-ledger / stale-coder / scaffold concepts are gone.

**Emma:** singleton session with `id="emma"`, seeded on first migration, never closes.

**Phase-change events:** stored as synthetic `author=user` messages ("phase advanced to PLAN") so chat history reads coherently and agents see them as natural switch prompts.

### IPAV state

In-memory cache only: `HashMap<SessionId, SessionState>` where `SessionState { current_phase, phase_log }`. Not persisted. Subprocesses die with the app; restart = fresh sessions.

### CL layout

Minimal core + per-project extension.

**Always loaded at spawn:**
- `agents/<name>/startup.md`
- `general-rules.md`

**Loaded when working_repo basename matches:**
- `projects/<project>/conventions.md`
- `projects/<project>/notes.md`

**CL writes are EXPLICIT** — user action only via the Context Library tab. Never auto-accumulated from agent activity.

**Dropped from current bot-hq's CL:**
- Phase docs → replaced by IPAV cache
- Ratchets, discipline-log, voice-mirror-log → fold into per-project `notes.md` if needed
- Rulebook, mcp-tool-manifest, agent-onboarding → tight `startup.md` per agent
- Session manifests → replaced by `sessions` table

### Data locations

Defaults (env-overridable via `BOT_HQ_DATA_DIR`):
- **CL root:** `~/.bot-hq/`
- **DB file:** `~/.bot-hq/.local/bot-hq.db`

During development: `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` (avoids colliding with current running bot-hq's `~/.bot-hq/`).

Auth tokens stored plaintext in sqlite for v1. Security caveat: any backup capturing `~/.bot-hq/` captures secrets. v2 upgrade: OS keychain integration (`keyring` crate).

### Plugins

- **1st: Discord** — deferred to a separate plan
- **2nd: Clive** — deferred to a separate plan (port from current bot-hq)

Contract TBD per-plugin. Not blocking core build.

### Kept from current bot-hq

- CL concept (slimmer scope)
- Sessions (scope-keyed containers)
- Bilateral Duo (Brian HANDS / Rain EYES)
- IPAV (Investigate → Plan → Apply → Verify)

### Migration

No runtime/DB migration from current bot-hq. Sessions, hub history, last_state files do NOT carry over.

However, the existing `~/.bot-hq/` CL is **distilled** into the new minimal structure as Phase A of the implementation plan — startup prompts, general rules, per-project conventions/notes are rebuilt from current CL wisdom.

Current bot-hq keeps running until the rebuild reaches feature parity.

---

## Glossary

- **Bilateral duo:** Brian (HANDS — exec, owns edits/commits/push) + Rain (EYES — review, adversarial counterpart). Spawned per session.
- **IPAV:** Investigate → Plan → Apply → Verify. Discipline framework agents follow within a session.
- **CL (Context Library):** filesystem space at `<BOT_HQ_DATA_DIR>` holding startup prompts, rules, and per-project conventions/notes.
- **Session:** a scope-keyed work container (UUID), holding a duo of agent subprocesses + chat history in the `messages` table.
- **Emma:** chat helper agent. Singleton (one per app). User-summonable. No longer the auto-orchestrator she was in current bot-hq.
- **claude-code:** the Anthropic CLI tool that wraps a language model. We spawn one subprocess per agent via `-p` mode.
- **stream-json:** claude-code's `--output-format stream-json` mode. One JSON event per line on stdout: text chunks, tool calls, tool results, message_stop, etc.
- **MCP (Model Context Protocol):** the protocol claude-code uses for external tool servers. We embed a small MCP server in Slint for the 2 UI-signaling tools.
- **Dashboard:** the main view — grid of session tiles, multi-session at-a-glance.
- **Phase chip:** small UI element showing current IPAV phase per session.
- **Tile suppression:** rule that the active session's tile doesn't render choice buttons / Need-Input flag (since they show inline in the session view); other tiles render normally.
