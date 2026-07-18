# bot-hq Plugins — Author Contract (api_version 1)

bot-hq is a lean core — the duo (Brian/Rain), policy enforcement, the
Context Library, sessions — and plugins extend everything else. A plugin
is a **static frontend bundle in a sandboxed iframe**: no build step
required, no host process, talking to bot-hq over one narrow,
consent-gated RPC channel.

The working example at [`examples/hello-plugin/`](../examples/hello-plugin/)
is the template: a manifest, an entry HTML, and the copy-in SDK
(`bhq-sdk.js`). It lists sessions, reads the Context Library index, and
persists a counter — every mechanism described here, exercised for real
(it also runs as a fixture in the integration tests).

## Quick start

1. Copy `examples/hello-plugin/` somewhere, rename the `id` in
   `manifest.json`.
2. In bot-hq → **Plugins**, paste the directory path → **Install**.
   You'll be shown what the plugin requests before anything lands.
3. Enable it. If the manifest declares a panel, a topbar tab with the
   plugin's name appears — that's your iframe.

Iterating: a normal install copies bundle files to
`<data_dir>/plugins/<id>/` (assets are served `Cache-Control:
no-store`, so a reload picks up edits made to the installed copy;
re-install to pick up manifest changes). For real development, use a
**linked install** (below) — it serves straight from your source
directory, so edit → tab reload is the whole loop and git stays the
single source of truth.

## Manifest

`manifest.json` at the bundle root:

```json
{
  "id": "my-plugin",
  "name": "My Plugin",
  "version": "0.1.0",
  "entry": "index.html",
  "api_version": 1,
  "requested_capabilities": ["list_sessions"],
  "slots": [{ "slot_name": null, "panel_route": "/plugins/my-plugin" }]
}
```

- `id` — lowercase ASCII letters/digits/`-` (it becomes a URL host; no
  leading/trailing `-`). Install refuses a colliding id.
- `entry` — the HTML file the host iframes.
- `api_version` — this contract's version. Omitted = 1. A bot-hq that
  doesn't support the declared version refuses the manifest outright
  (an old host never half-runs a newer plugin).
- `requested_capabilities` — catalog command names (below). Anything
  not in the catalog is an install-time error. The user sees this list
  with plain-language descriptions at install and must confirm.
- `slots` — UI contributions. **v1 renders panel plugins only**: any
  entry with a non-null `panel_route` gives the plugin a full-page
  panel + a topbar tab (the route VALUE is reserved; presence is what
  counts — the host routes tabs to `/plugins/view/<id>`). `slot_name`
  is parsed and reserved for a future inline-slot tier.

## Serving & origins

Bundles are served over the `bhq-plugin://` custom URI scheme,
registered once at app start; installs and enables need no restart.
Only installed **and enabled** plugins are served. Paths are traversal-
guarded (canonicalized, symlink escapes refused) and percent-encoding
is rejected — keep bundle filenames URL-safe ASCII.

The URL form is platform-dependent (the host picks automatically):

| Platform | Entry URL | `postMessage` origin |
|---|---|---|
| macOS / Linux | `bhq-plugin://<id>/index.html` | `bhq-plugin://<id>` (may surface as opaque/`null`) |
| Windows | `https://bhq-plugin.localhost/<id>/index.html` | `https://bhq-plugin.localhost` |

Every asset response carries a default CSP:
`default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self'
'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:;
connect-src *` — same-origin scripts/styles plus inline, and **network
fetch to anywhere** (`connect-src *`) so integrations like a GitHub
panel can call their APIs directly.

### Extra CSP origins (consent-gated)

A plugin that needs assets from a CDN can request extra origins for up
to four directives in its manifest:

```json
"csp_extra_origins": {
  "script-src": ["https://cdn.jsdelivr.net", "https://unpkg.com"],
  "style-src":  ["https://fonts.googleapis.com"],
  "font-src":   ["https://fonts.gstatic.com"],
  "img-src":    ["https://raw.githubusercontent.com"]
}
```

The rules:

- **Additive only.** Granted origins are appended to the default source
  lists — never replacing or narrowing them. `default-src`,
  `connect-src`, and every other directive are untouchable.
- **Exactly these four directives.** Any other key is an install-time
  error.
- **Explicit https origins only** — `https://host[:port]`, lowercase.
  Install rejects wildcards (`*`, `https://*.example.com`), bare
  schemes (`https:`), CSP keyword sources (`'unsafe-eval'`, nonces,
  hashes), `data:`/`blob:`, non-https schemes, and paths/queries. Max
  16 origins per directive.
- **Consent-gated and frozen at install.** The install screen lists the
  exact origins per directive ("Can load and run code from:
  cdn.jsdelivr.net, unpkg.com"). What the user approves is recorded in
  the host's DB at install time, and serving reads ONLY that record —
  editing an installed manifest never changes the served CSP.
  Re-install to request different origins.
- **Older hosts fail closed.** A bot-hq predating this field ignores it
  entirely: the plugin installs, but assets are served under the strict
  default CSP (CDN loads blocked — degrade gracefully). And because the
  grant is recorded at install time, a manifest stored by an older host
  can never activate origins later via a host upgrade — granting always
  goes through a consent screen on (re-)install.

This tier is NOT a browser surface: arbitrary sites still refuse
framing (`X-Frame-Options`), the iframe sandbox is unchanged, and
`connect-src` was already `*`. An agent-drivable Browser tab remains a
child-webview tier (future work, below).

The iframe sandbox is `allow-scripts allow-same-origin` — no top
navigation, no popups, no forms submission out of the frame.

## Linked installs (dev mode)

Check **"Linked — serve from this directory (no copy)"** when
installing a local path and bot-hq serves your bundle straight from the
source directory: nothing is copied, one write location, edits are
visible on the next tab reload. Every serving guard is identical —
same CSP, `no-store`, disabled-plugin refusal, percent-encoding
rejection; traversal and symlink checks treat YOUR directory as the
boundary (a symlink inside the repo pointing outside it is refused).

The consent rule, load-bearing: **assets are live; the manifest is
not.** Capabilities and CSP origins are frozen at install-consent time
in the host's DB — the host never re-reads a linked `manifest.json`
into effect on mount or enable. When the source manifest drifts from
what you approved, the Plugins tab shows "Manifest changed — review
and re-approve"; grants change only after that consent dialog. So
editing a repo manifest (yours, or a collaborator's in a pulled
branch) can never silently widen a linked plugin's capabilities.
Re-approval updates the consented manifest IN PLACE — the plugin's KV
state survives (unlike uninstall → reinstall).

Lifecycle differences: Disable stops serving, as always. **Uninstall
never deletes or modifies a linked source directory** — it's your
repo; only the registry entry and KV rows are removed. Linked installs
are local directories only — no URLs, no zips.

**Switching copy↔linked (or refreshing from a new source) is
"Reinstall…" on the plugin card** — an in-place operation: the same
consent screen as install runs against the source manifest, the
registry row is updated, and **your KV state survives** (uninstall →
re-install still works, but it deletes KV). Converting to copy
materializes the bundle into the managed directory; converting to
linked removes the now-unused managed copy — the dialog states each
consequence before you confirm.

Copy-mode installs record where they came from; **"Update from
source"** on the card re-copies assets in place with no consent screen
while the source manifest is byte-identical to the approved one. Any
manifest change routes through Reinstall's consent. URL installs
re-fetch via Reinstall instead.

## RPC protocol

Plugins never call Tauri. The channel is `window.postMessage` with the
host shell, authenticated by a **per-mount nonce** the host appends to
your entry URL (`?bhq=<nonce>`) — plus source-window and origin checks
host-side. The SDK handles all of it; the raw shapes, if you'd rather
not use the SDK:

```
plugin → host:  { type: "bhq:invoke", id, cmd, args?, nonce }
host → plugin:  { type: "bhq:result", id, ok: true,  data }
                { type: "bhq:result", id, ok: false, error }
host → plugin:  { type: "bhq:event", topic }          (push tier, below)
host → plugin:  { type: "bhq:ping" }                  (every 5s)
plugin → host:  { type: "bhq:pong", nonce }
```

### Push events (`bhq:event`)

Two topics in v1 — hardcoded, no general pub/sub (`BHQ.onEvent(topic, cb)`
in the SDK):

- `plugin_assets_changed` — a file in YOUR served directory changed on
  disk (debounced ~500ms; build/VCS churn like `target/`,
  `node_modules/`, `.git/` is filtered). No grant needed — it's your own
  content. Pairs with linked installs: a shelf-style UI can re-fetch its
  own materials the moment you save, instead of waiting for a manual
  reload.
- `sessions_changed` — a session was created or closed. Delivered ONLY
  if you hold the `list_sessions` grant (a plugin that can't read the
  list has no use for the change signal). Re-invoke `list_sessions` on
  it; the event carries no data.

Events fire while mounted (the channel only exists then) and carry no
payload — they are refresh nudges, not data feeds.

Correlate replies by `id`. Answer pings promptly: three unanswered
pings and the host declares the plugin crashed, tears the iframe down,
and offers the user a Reload (fresh mount, fresh nonce).

Enforcement is **Rust-side**: every invoke re-checks that your plugin
is enabled and that `cmd` is both in the catalog and in your granted
set, then dispatches through an explicit per-command match. The shell's
JS checks are transport hygiene, not the security boundary.

## SDK

Copy `bhq-sdk.js` into your bundle (≈90 lines, no dependencies):

```html
<script type="module">
  import { invoke } from "./bhq-sdk.js";
  const sessions = await invoke("list_sessions");
</script>
```

It auto-answers heartbeat pings and exposes `window.BHQ.invoke` for
non-module scripts. An npm package is a later nicety; the file IS the
SDK.

## Command catalog (api_version 1)

v1 is read-first: the only writes a plugin can request are to its own
namespaced key/value store. Args are a JSON object; results are the
host's JSON views (same shapes the bot-hq UI renders).

| Command | Args | Grants |
|---|---|---|
| `list_sessions` | — | list of active sessions (titles, repos, status) |
| `get_session` | `session_id` | one session's details |
| `list_messages` | `session_id`, `since_id?` | a session's chat history |
| `session_doc_search` | `session_id`, `query?`, `phase?` | a session's I/P/A/V phase documents |
| `cl_index_search` | `project?`, `query?` | Context Library file index — rows: `{ project, file_path, description, tags, updated_at }` (same shape agents see) |
| `cl_folder_search` | `project?`, `query?` | Context Library folder descriptions — rows: `{ project, folder_path, description, tags, updated_at }` |
| `cl_retrieve` | `project`, `query`, `paths?`, `budget_tokens?` | best-matching CL sections (BM25; budget capped at 20k tokens; `stale` is reserved and always `false` in v1) |
| `cl_read_file` | `project`, `file_path` | whole CL files (1 MB cap, truncation flagged) |
| `list_projects` | — | registered projects |
| `compute_apply_diff` | `session_id` | a session's color-classified git diff |
| `spawn_session` | `prompt`, `project?`, `title?` | open a NEW agent session with that prompt (per-spawn confirm dialog — below; returns `{ session_id }`) |
| `plugin_session_create` | `first_message`, `project?`, `title?`, `duo?` | (needs `plugin_sessions`) create a helper session you own; returns `{ session_id }`; single agent unless `duo:true` |
| `plugin_session_send` | `session_id`, `text` | (needs `plugin_sessions`) send a message to a session you created |
| `plugin_session_wait` | `session_id`, `since_id?`, `timeout_ms?` | (needs `plugin_sessions`) long-poll new messages (default 25 s, clamp 100 ms–60 s) → `[AgentMessage]` |
| `plugin_session_messages` | `session_id`, `since_id?` | (needs `plugin_sessions`) read a created session's messages → `[AgentMessage]` |
| `plugin_session_close` | `session_id` | (needs `plugin_sessions`) close + archive a session you created |
| `plugin_kv_get` | `key` | your plugin's own saved state |
| `plugin_kv_set` | `key`, `value` | write your own state (key ≤256 B, value ≤256 KB; namespaced server-side; wiped on uninstall) |

> **BREAKING (2026-07-05):** `cl_index_search` and `cl_folder_search`
> rows were aligned to the agent-side shape — the field is now
> **`project`** (was `project_id`), and the internal `id` / `created_at`
> fields are gone. Update any plugin reading `row.project_id`. Other
> commands' shapes are unchanged; results remain the host's JSON views.

Not grantable, by design: touching sessions your plugin did NOT create
(`broadcast_message`, and the raw send/drive/close of arbitrary
sessions), mutating the Context Library (canon changes are
user/agent-proposal flows), installing plugins, or policy. There are
two deliberate session exceptions, each narrowly fenced — `spawn_session`
(create only, guarded by a per-spawn dialog) and `plugin_sessions`
(create AND drive your OWN sessions, guarded by an ownership fence):

### spawn_session — double consent

`spawn_session` opens a NEW session (agents spawned, your prompt
broadcast as its first message) and returns `{ "session_id": "…" }`.
It is the only session-mutating grant, and it is guarded twice:

1. **Install-time grant** — it appears on the consent screen like any
   capability ("Open new agent sessions with a prompt you will see and
   approve each time").
2. **A per-spawn confirm dialog** — EVERY call raises a host dialog
   showing your plugin's name, the target project, and the full
   prompt; the invoke resolves only on Approve and rejects with
   `spawn_session: rejected by user` on Reject. Not optional in v1
   (no "don't ask again"). The dialog renders the complete prompt
   (scrollable, with a long-prompt signpost) and warns when the last
   non-empty line ends with ":" — a heuristic for an unfilled template
   tail; it can false-positive on legit colon-ended prompts and never
   blocks. Gate degenerate prompts client-side; the dialog is the last
   line of defense, not your validator. (A structured task-summary
   field on spawn_session was considered and deferred — a
   plugin-authored summary could itself mislead while the real risk is
   the prompt tail.)

Why the second layer: plugin content can include user-commissioned
HTML (the Cognotify materials model) rendered same-origin with the
panel. The grant belongs to the PLUGIN origin — the host cannot
distinguish panel code from a material's script. Without the per-spawn
confirm, one malicious material on a granted plugin could silently
spawn agent sessions with attacker-chosen prompts. The dialog puts a
human between any in-origin script and a new session.

Details: `prompt` is required and non-empty (the 64 KB args envelope
bounds it); `project` is optional but must name an existing project;
`title` is optional (default `<plugin-id> session`). Spawns are
mounted-only — the RPC channel exists only while your panel is open;
there is no background spawn. The new session runs under the same
policy gates as any user-created session. Existing sessions stay out
of reach: reading them is separate grants (`list_sessions`,
`get_session`, `list_messages`); controlling them is not grantable at
all. Older hosts: a manifest requesting `spawn_session` is REFUSED at
install by a bot-hq predating it — unknown capability names are
install-time errors, so this capability fails closed by rejection
(stricter than `csp_extra_origins`' ignore-and-stay-strict).

### plugin_sessions — drive your own sessions

`plugin_sessions` is ONE grant that unlocks five commands for a plugin
to run its own helper agent sessions end to end: `plugin_session_create`
→ `plugin_session_send` / `plugin_session_wait` /
`plugin_session_messages` → `plugin_session_close`. It exists so a panel
can hold a conversation with an agent (a tutor, an assistant) without
ever handling a driver token or opening a port — the host owns the
machinery; the plugin never sees a credential.

**Ownership fence — the safety property.** Every command except `create`
requires the target session's `created_by_plugin` to equal YOUR plugin
id (`require_owned_session`). A plugin can send to, read, or close ONLY
the sessions it created — never the user's own sessions, never another
plugin's. An absent session and a foreign-owned session fail identically
(`session "…" was not created by plugin "…"`), so the fence never leaks
which session ids exist. That bounded blast radius — exactly the
sessions that plugin minted — is what makes the capability safe to grant
to third-party plugins.

**Single-agent by default.** `create` runs solo unless you pass
`duo: true` (a Brian+Rain review pair — double the cost). New sessions
appear in the user's dashboard and run under the same policy gates as
any session; `close` archives (recoverable), never hard-deletes.

**Consent model — install grant, no per-call dialog.** Unlike
`spawn_session` (a confirm dialog on every create), `plugin_sessions` is
gated only by the install-time grant. What makes that acceptable: for
the intended chat use the first message is the user's own typed input —
the human already authored the prompt, so a per-message dialog would be
redundant friction — and all driving is ownership-fenced +
dashboard-visible. Residual risk to weigh before granting: same-origin
plugin content (e.g. a user-commissioned Cognotify material) could call
`plugin_session_create` with a prompt the user did NOT type; the fence
still limits that session to the plugin, forced-solo and dashboard-
visible, but there is no per-create human check on the prompt as there
is for `spawn_session`. Grant `plugin_sessions` to plugins whose
content you trust; if you need the per-create human gate, use
`spawn_session` instead.

**Flow.** `create` broadcasts `first_message` as the session's opening
prompt and returns `{ "session_id": "…" }` (ownership is stamped before
the id is returned — no unowned window). `wait` blocks server-side for
messages past `since_id` (default 25 s, clamped 100 ms–60 s), returning
`[]` on timeout — safe over the invoke tier (no bridge timeout).
`messages` reads history without blocking. Message reads return the
host's `AgentMessage` shape (`{ id, session_id, author, kind, content,
created_at }`), the same rows `list_messages` returns. Mounted-only: the
RPC channel exists only while your panel is open.

## Lifecycle

install (consent screen) → enable → mount (user opens your tab) →
heartbeat while mounted → unmount (tab closed; clean, not a crash) or
crash (3 missed pings → fallback card → user reloads) → disable /
uninstall (bundle dir + your KV rows are removed).

Plugins run **while mounted** — there is no background execution in v1.
State you need across mounts goes in `plugin_kv_*`. KV survives
disable, re-approve, and Reinstall (mode switches included); only
uninstall deletes it — if your users may uninstall and return, offer
your own export/import.

The registry (not the disk) decides what's installed: if
`~/.bot-hq/plugins/<id>/` survives or reappears after an uninstall
(out-of-band writes, restores), the next install of that id surfaces a
consented cleanup in the install dialog instead of hard-failing — the
leftovers are only removed on your approval.

## What plugins can't do yet (designed extension points)

- **Agent surface** — plugins can't contribute MCP tools to Brian/Rain
  or inject anything into agent context. (Cognotify's
  "zero agent pollution" is v1's default and only mode.) A
  plugin-declared tool tier is future work.
- **New agents** — a plugin can't add an agent to sessions. Interim
  lever: the **external MCP driver server** (port 7892, bearer-token)
  already lets any process you run create and drive sessions — a
  "backend-style" plugin is an ordinary program using it. (For plain
  session CREATION from a panel, `spawn_session` is the grantable,
  double-consented path.)
- **Real browser surface** — arbitrary sites refuse iframing
  (`X-Frame-Options`); an agent-drivable Browser tab needs a
  child-webview tier.
- **Background execution** — CL cloud sync as a daemon wants host
  scheduling or a sidecar tier; today it runs while its panel is open.
- **Prompt/personality packs** — agent character customization is CL
  territory (`custom-instructions.md` via the proposals flow), not a
  plugin surface yet.
- **Bundle installs from URL** — URL install currently fetches
  `manifest.json` + the entry file ONLY (no other assets). Use a local
  directory for multi-file bundles; zip/signed bundles are future work.
- **Inline slots** (`slot_name`) — reserved.

## Security model, summarized

Per-plugin origin + per-mount nonce authenticate the channel; the Rust
proxy enforces the grant on every call; the catalog caps what is
grantable at all; install shows the user exactly what's requested;
serving refuses disabled plugins, traversal, and oversized KV writes.
A plugin compromise is contained to: its own bundle, its own KV, the
read commands the user approved, whatever `connect-src *` lets it
fetch from the network under its own (non-host) origin, script/
style/font/image loads from the exact extra origins the user granted
at install (none by default), and — if `spawn_session` was granted —
new sessions a human explicitly approved one dialog at a time.
