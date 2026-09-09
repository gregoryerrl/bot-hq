# Surface Claude Code configuration in bot-hq — Design

- **Date:** 2026-06-02
- **Status:** Design (brainstormed; not yet planned for implementation)
- **Author:** brainstorm session (operator + Claude)
- **Topic:** A Settings subtab that surfaces and controls the Claude Code
  configuration the bot-hq agents inherit, with a per-agent override layer.

---

## 1. Why this exists

bot-hq's agents (Brian = HANDS, Rain = EYES, Emma = solo) **are
`claude-code` headless subprocesses** (`claude -p --input-format
stream-json …`, built in `src/agents/spawn.rs::build_command`). Because
they are real claude-code processes, **they inherit whatever the user's
`~/.claude` install has** — enabled skills, plugins, hooks, CLAUDE.md +
auto-memory, MCP servers, effort level.

That inheritance is a double-edged sword: a user-enabled skill can
*self-invoke* mid-task and derail a Brian/Rain workflow; a plugin hook
(e.g. superpowers `SessionStart`) can inject context that breaks a
third-party gateway (this already happened — it's why Rain runs through a
normalizing proxy). Today this inherited config is **invisible** inside
bot-hq and can only be changed by hand-editing dotfiles.

**Goal:** one place in bot-hq where a user can *see* the Claude Code
config their agents inherit and *control* it — both globally (edit the
real `~/.claude`) and per-agent (disable a risky skill just for the
agents, keeping it in their own interactive Claude). The decision is
deliberately the user's; bot-hq surfaces and structures, it does not
decide.

### Non-goals

- **`policy.yaml` is out of scope.** It is a bot-hq-internal artifact
  injected into agents (forbidden-words, push gate, etc.), not user
  Claude config. It already has per-project (CL tree) and per-session
  (SessionView gear tab) editors; a global form is tracked separately.
- **Don't rebuild what exists.** The trio model/auth cards
  (`agent_configs` table) and the global Tool-Gate keyword list already
  live in the Settings tab and stay as-is (they become sibling subtabs).
- Not a generic dotfile editor: runtime artifacts (transcripts, tasks,
  shell-snapshots, history.jsonl, caches) are never shown as editable.

---

## 2. Background: how config flows into agents (ground truth)

From `src/agents/spawn.rs` + `src/core/session.rs` +
`src/signaling/server.rs`:

| Surface | Brian (HANDS) | Emma (solo) | Rain (EYES, `--bare`) | Mechanism today |
|---|---|---|---|---|
| Skills (user + plugin) | ✅ inherits — can self-invoke | ✅ | ❌ skipped | no override; claude-code loads them |
| Plugins (`enabledPlugins`) | ✅ | ✅ | ❌ (`--bare` skips plugin sync) | no override |
| Hooks (settings + plugin) | ✅ + bot-hq adds a PreToolUse Bash hook | ✅ | ❌ (`--bare` skips hooks) | bot-hq adds via `--settings`, never removes |
| CLAUDE.md + auto-memory | ✅ autodiscovered | ✅ | ❌ (`--bare` skips it) | bot-hq adds its own via `--append-system-prompt` |
| MCP servers | ✅ forwarded (`load_user_mcp_servers`, minus `bot-hq`, `claude-in-chrome`) | ✅ | ❌ empty map | reads `~/.claude/settings.json` + `~/.claude.json`, later wins, `--strict-mcp-config` |
| Model / auth | ⏹ overridden (`ANTHROPIC_MODEL` from `agent_configs`) | ⏹ | ⏹ | bot-hq env |
| Permissions | ⏹ bypass (`--dangerously-skip-permissions`) | ⏹ bypass | ⏹ `dontAsk` + allow/deny lists | bot-hq flags |

Key existing code we extend rather than replace:

- `src/signaling/server.rs:214 load_user_mcp_servers` and
  `:251 default_user_settings_paths` — already read the user's real
  `~/.claude` config and filter reserved keys. This is the seed of the
  read/resolve layer.
- `src/agents/spawn.rs:553-561` — already injects a `--settings` JSON
  (currently just the PreToolUse hook). The override layer adds keys to
  this same payload.

---

## 3. Feasibility: what the per-agent override layer can actually do

Verified against current docs (code.claude.com) and empirically on this
machine (claude-code v2.1.160), all via bot-hq's existing per-spawn
`--settings` / env / `--mcp-config` injection — **no `--bare` needed**:

| Surface | Per-agent override | Lever | Confidence |
|---|---|---|---|
| Individual **user skill** | ✅ clean | `skillOverrides:{"name":"off"\|"user-invocable-only"\|"name-only"\|"on"}` in `--settings` (v2.1.129+; **tested working**) | high |
| Individual **plugin** (+ its skills/hooks/MCP/commands) | ✅ clean | `enabledPlugins:{"name@mkt":false}` in `--settings` (CLI tier beats user tier; merges key-by-key) | high |
| **MCP servers** (per-agent) | ✅ clean | already done — per-agent `--mcp-config` + `--strict-mcp-config` | high |
| **Effort / ultracode** | ✅ clean | `--effort` / `CLAUDE_CODE_EFFORT_LEVEL` env / `"ultracode":true` in `--settings` | high |
| **Model** | ✅ clean | `ANTHROPIC_MODEL` env (already done) | high |
| **Auto-memory** | ✅ clean | `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` or `autoMemoryEnabled:false` in `--settings` | high |
| **CLAUDE.md** | ✅ coarse | `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` (all) or `claudeMdExcludes` glob (per-file) | med-high |
| Other env (max tokens, disable workflows/thinking) | ✅ clean | env / `--settings` | high |
| **Individual hook** (keep some, drop others) | ⚠️ limited | no granular lever; `disableAllHooks` also kills bot-hq's own hook; **but** plugin hooks die when the plugin is disabled, and user-file hooks drop via `--setting-sources` minus `user` (coarse) | med |

**Headline use case works cleanly:** "a skill self-invokes and derails
Brian" → inject `skillOverrides:{"that-skill":"user-invocable-only"}` for
the agents; the user's own interactive Claude keeps the skill. Per-agent,
no global nerf.

**Caveats to honor in the implementation:**

- `skillOverrides` does **not** apply to *plugin* skills (manage those by
  disabling the plugin). It is the key `skillOverrides`, **not**
  `disabledSkills` (which was proposed but never shipped).
- In `-p` mode, **malformed `--settings` JSON is silently ignored** — the
  override would no-op without error. The backend MUST validate the JSON
  it injects.
- Managed/enterprise settings beat `--settings`; none present on a normal
  dev machine, but the UI should detect and flag them.
- Granular individual-hook suppression is the one weak spot; treat it as
  "all-or-nothing or via plugin/source coarse levers" and don't promise
  surgical per-hook control.

---

## 4. Architecture — four layers

### Layer 1 — Read / resolve (Rust backend)

A read module (extending `signaling/server.rs` helpers into a dedicated
`src/claude_config/` module) that produces a **resolved, masked,
provenance-annotated** view of the user's Claude Code config:

- **Config-dir resolution:** honor `CLAUDE_CONFIG_DIR` (default
  `~/.claude`) at read time — never hardcode. Mirrors the
  `BOT_HQ_DATA_DIR` pattern.
- **Sources read:** `~/.claude/settings.json`,
  `~/.claude/settings.local.json` (if any), `~/.claude.json` (root-level;
  OAuth/trust/user-MCP), `~/.claude/CLAUDE.md`, the auto-memory dir
  (`~/.claude/projects/<git-root-slug>/memory/`), `skills/*/SKILL.md`,
  `agents/*.md`, `commands/*.md`, `plugins/{config,known_marketplaces,installed_plugins,blocklist}.json`,
  `keybindings.json`. Detect managed-settings presence (macOS
  `/Library/Application Support/ClaudeCode/…` + `com.anthropic.claudecode`
  MDM plist).
- **Effective-value resolution:** compute the winning value per key
  across the precedence chain (managed > CLI > local > project > user),
  with the **array-merge exception** for `permissions`/hooks. Flag the
  known traps: `settings.json` `mcpServers` is ignored by claude-code
  (it loads MCP from `~/.claude.json`); show which actually loads.
- **Secret masking:** never return raw `auth_token`, MCP bearer headers,
  Discord tokens, OAuth blobs to the frontend; mask to `••••last4` and
  round-trip without re-displaying.

Returns typed structs surfaced via new Tauri commands
(`claude_config_read`, `claude_config_write_field`, …) with `tauri-specta`
bindings.

### Layer 2 — Schema registry + typed resolvers

A **single declarative field registry** (shared shape, defined once)
describes every config field:

```ts
type FieldDescriptor = {
  key: string;                 // dotted path, e.g. "effortLevel"
  surface: SurfaceId;          // groups fields into tree nodes
  label: string;
  help: string;
  type: "toggle" | "enum" | "text" | "number" | "list" | "keyvalue"
      | "markdown" | "secret" | "custom:<widgetId>";
  enum?: { value: string; label: string }[];
  target: ResolverId;          // which backend resolver writes it
  validate?: ValidatorId;
  secret?: boolean;
  applyTiming: "hot-reload" | "restart" | "next-agent-spawn";
  scope: "user" | "project" | "agent-override";
  inheritedBy: ("brian" | "rain" | "emma")[]; // drives the lens
};
```

The frontend auto-renders typed controls from the registry. The backend
has a small set of **typed resolvers**, one per file/format:

- `settings_json` (scalars/env/enabledPlugins/skillOverrides/permissions/hooks/statusLine)
- `claude_dot_json` (durable subset: mcpServers, project trust/MCP flags — never the volatile cache keys)
- `skill_frontmatter` (edit a `SKILL.md`'s YAML front-matter in place)
- `claude_md` (markdown file content: user + project + memory topic files)
- `agent_md` / `command_md` (markdown)
- `plugins_registry` (enabledPlugins toggles, extraKnownMarketplaces)
- `keybindings_json`
- `agent_overrides` (the bot-hq override store — Layer 3)

"Rich" surfaces get bespoke widgets layered on the registry: the **MCP
server list**, the **memory topic-file tree**, the **hooks
event-builder**, and the **CLAUDE.md / memory markdown editor**.

### Layer 3 — Per-agent override store

A new bot-hq file `<data_dir>/claude-overrides.json` (0600), shape:

```jsonc
{
  "_all":  { /* applied to every agent unless overridden per-agent */ },
  "brian": { "skills": {"some-skill": "user-invocable-only"},
             "plugins": {"warp@claude-code-warp": false},
             "mcp": {"discord": false},
             "effort": "high", "ultracode": false,
             "autoMemory": false, "disableClaudeMd": false,
             "env": {"CLAUDE_CODE_MAX_OUTPUT_TOKENS": "32000"} },
  "rain":  { /* mostly moot: --bare already skips skills/plugins/hooks/CLAUDE.md */ },
  "emma":  { /* … */ }
}
```

`build_command` (and the per-agent mcp-config builder in `session.rs`)
**merge these overrides** into the payloads it already constructs:

- skills/plugins/autoMemory/ultracode/effort/claudeMdExcludes → folded
  into the existing `--settings` JSON (alongside the PreToolUse hook).
- `CLAUDE_CODE_DISABLE_AUTO_MEMORY` / `CLAUDE_CODE_DISABLE_CLAUDE_MDS` /
  `CLAUDE_CODE_EFFORT_LEVEL` → env.
- MCP enable/disable → filter the per-agent map already produced by
  `user_mcp_servers_for_agent`.
- The merged `--settings` JSON is **validated** before injection (the
  silent-ignore trap).

Rain note: under `--bare` most overrides are no-ops; the UI greys them
out for Rain and shows why.

### Layer 4 — UI (Settings subtab, reusing the CL shell)

- The **Settings tab becomes tabbed**: `Agents` (existing cards) ·
  `Tool Gate` (existing) · **`Claude Config`** (new).
- The new subtab embeds the **Context Library 2-pane shell**
  (`WorkspaceSidebar` + `EditorArea` patterns: collapsible tree, search,
  collapse-state persistence, multi-tab, context menu) — right pane swaps
  raw editor for **structured forms** from the registry.
- **Surface-first tree** (navigate by *which setting*, not *whose*):
  Core knobs · MCP servers · Skills · Plugins & marketplaces · Hooks ·
  Memory & instructions · Permissions · Keybindings · Status line ·
  Commands · Subagents.
- **Inheritance lens** on every surface: per-agent badges (Brian/Emma
  inherit · Rain skips · overridden-by-bot-hq · forwarded), plus the
  per-agent override toggles inline. A header banner flags managed-policy
  presence and apply-timing ("applies to new agent sessions").

```
Settings:  [ Agents ] [ Tool Gate ] [ Claude Config ]
┌─────────────────────┬───────────────────────────────────────────────┐
│ 🔎 filter config…   │  Skills                          ⓘ applies →    │
│ ─────────────────── │  ───────────────────────────────────────────── │
│ Core knobs          │  superpowers:brainstorming                      │
│ MCP servers         │   global: on                                    │
│ ▸ Skills        ←   │   agents:  ☑Brian ☑Emma  ⊘Rain(--bare)          │
│ Plugins & markets   │           [ on | name-only | manual-only | off ]│
│ Hooks               │  note (user skill)                              │
│ Memory & instr.     │   global: on   agents: manual-only  ⚠ self-inv. │
│ Permissions         │   ...                                           │
│ Keybindings         │                                                 │
│ Status line         │                                                 │
└─────────────────────┴───────────────────────────────────────────────┘
```

---

## 5. Surfaces & coverage (the four bundles)

All four bundles ship (full coverage); built in the phase order below.

- **Core knobs + MCP:** effortLevel, model (display; agent model is the
  `agent_configs` card), editorMode, alwaysThinking, voice,
  max-output-tokens; MCP server list (read `~/.claude.json` +
  `settings.json`, write the right file, secrets masked, per-agent
  forward toggles).
- **Instructions & memory:** `~/.claude/CLAUDE.md` + project CLAUDE.md +
  CLAUDE.local.md; auto-memory MEMORY.md (200-line/25KB cap shown) +
  topic files (tolerate both front-matter shapes); per-agent auto-memory
  / CLAUDE.md suppression.
- **Extensions:** plugins + marketplaces (enable toggles, add
  marketplace); skills enable/disable (global = SKILL.md front-matter;
  per-agent = `skillOverrides`); slash-commands browser; subagents
  browser.
- **Advanced:** permissions (allow/ask/deny/defaultMode/additionalDirectories,
  with array-merge awareness); hooks (event→command builder + read-only
  plugin-hook view; honest about granular-suppression limits);
  keybindings; statusLine.

---

## 6. Apply-timing & security

- **Apply timing:** user-scope edits follow claude-code's own rules
  (permissions/hooks/env hot-reload; model/alwaysThinking need restart).
  Agent overrides take effect on the **next agent spawn** (each `-p` call
  is a fresh process) — surfaced as "applies to new agent sessions; live
  sessions keep current config until reopened" (same caveat the system
  prompt has today).
- **Security:** mask all secrets; never log tokens; validate every
  injected `--settings` JSON (silent-ignore trap); `claude-overrides.json`
  is 0600; reuse the CL path-traversal guards for any file write; honor
  the bot-hq self-containment rule — bot-hq still installs **nothing** of
  its own into `~/.claude` (it only edits user config the user asked it
  to, and keeps its enforcement in injected `--settings`).

---

## 7. Suggested phasing (testable increments)

1. **Read/resolve + bindings** — backend reader, config-dir resolution,
   effective-value + masking; a read-only "Claude Config" subtab showing
   the inheritance lens (no writes). Ships value immediately (visibility).
2. **Override store + spawn integration** — `claude-overrides.json`,
   merge into `build_command` + per-agent mcp-config, JSON validation,
   tests. Wire the per-agent toggles for the cleanly-overridable surfaces
   (skills, plugins, MCP, effort/ultracode, auto-memory).
3. **Global structured editors** — schema registry + resolvers; Core
   knobs first, then Extensions, then Instructions & memory, then
   Advanced.
4. **Rich widgets** — MCP list editor, memory tree, hooks builder,
   markdown editors.

---

## 8. Open questions / risks

- **`--setting-sources` interaction** with bot-hq's existing flags (the
  coarse user-hook/CLAUDE.md suppression lever) — needs a small spike;
  only matters if we expose user-file-hook suppression.
- **`disableAllHooks` self-disable** — confirm empirically whether it
  kills bot-hq's own injected hook (worst-case assumption: yes; so we
  won't use it).
- **`~/.claude.json` editing surface** — limit to the durable subset;
  decide whether to write it at all in v1 or treat user-MCP as
  read-with-forward-toggles only.
- **Effective-value engine depth** — full precedence engine vs simple
  "user value + override badge"; phase 1 can do the simpler version.

---

## 9. Decisions locked in this brainstorm

1. Scope: **both** — one config rooted at `~/.claude` + inheritance lens
   + per-agent override layer.
2. Interaction: **structured schema-aware forms** (no raw hand-editing;
   the UI serializes to the correct file/format).
3. Surfaces: **all four bundles** (full coverage).
4. Architecture: **schema-driven registry + custom widgets + typed
   resolvers + override store**.
5. Edit scope: **global `~/.claude` edits + agent-scoped overrides**.
6. Placement: **a subtab inside the Settings tab**, reusing the Context
   Library 2-pane shell; existing Agents cards + Tool Gate become sibling
   subtabs.
7. Out of scope: `policy.yaml` (bot-hq-internal).
