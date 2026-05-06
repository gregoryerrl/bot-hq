# Phase N v3a.5 — Rules-system + HTTP API design-spike

**Type:** Phase N v3a.5 design-spike doc (no impl this commit; impl lands v3b + v3c)
**Date:** 2026-05-06
**Author:** Brian (HANDS), Rain BRAIN-2nd review
**Status:** combined rules-schema + HTTP API surface spec covering v3b + v3c

---

## 1. Theme + driver

Phase N v3 ships the Clive workspace web UI on top of a single canonical-store. Two concrete substrates need contract-locking before v3b/v3c impl:

1. **Rules-system** — structured KV (general + per-project + per-agent split YAML files) replacing unstructured `~/.claude/projects/.../memory/feedback_*.md` over time. Substrate ships v3; agent-side consumption + `feedback_*.md` migration ships Phase O.
2. **HTTP API surface** — the daemon-committer's read/write endpoints for the web UI, plus Clive integration endpoints.

This doc locks the schema + API contract so v3b (web read MVP) + v3c (web write + Clive integration + raw-YAML rules editor) can build against a stable interface.

---

## 2. Rules-schema

### 2.1 File format + location

YAML throughout per Q-rules-1 LOCKED (msg 8709). Parsed via `gopkg.in/yaml.v3`. No JSON conversion at API boundary per msg 8708 (YAML-everywhere).

Storage layout per Q-rules-4 LOCKED (general + per-project + per-agent split):

```
~/.bot-hq/rules/
├── general.yaml                   # cross-project rules
├── projects/
│   ├── .gitkeep
│   ├── bot-hq.yaml                # per-project overrides
│   ├── bcc-ad-manager.yaml
│   └── <project>.yaml
└── agents/
    ├── brian.yaml                 # per-agent rules
    ├── rain.yaml
    ├── emma.yaml
    └── clive.yaml
```

Substrate skeletons ship in v3a (already landed at HEAD post-v3a commit `54c9db8`).

### 2.2 Supported top-level keys (general.yaml)

```yaml
tone:
  reply: ""           # user-voice preference for replies
  eod: ""             # user-voice preference for EOD format
  implementation: ""  # user-voice preference for implementation style

greenlight:
  push: "..."         # verbatim-token requirement for git push class
  forcePush: "..."    # R29 elevated gate semantics
  merge: "..."        # merge gate semantics

ratchets:
  locusFile: "~/.bot-hq/ratchets/active.md"
  cadence: "phase-close + ad-hoc per BRAIN-cycle decision"

hubDiscipline:
  handshakeTerminator: "..."
  crossInFlight: "..."
```

### 2.3 Per-project keys (projects/<project>.yaml)

```yaml
push: "..."           # project-specific push gate (overrides general)
voice: "..."          # project-specific voice (e.g., 'first-person plural for BCC PRs')
disguise: "..."       # whether bot-hq context must stay invisible
testIsolation: "..."  # known test-isolation gaps + workarounds
```

Any key from `general.yaml` may be overridden at the project level.

### 2.4 Per-agent keys (agents/<agent>.yaml)

```yaml
role: "..."           # agent's role in DISC v2 + project context
exec:
  pushClass: "..."    # agent-specific push behavior
  fileWrites: "..."   # agent's filesystem-write authority
  scope: "..."        # what the agent is allowed to touch
  proposeDiff: "..."  # whether the agent uses propose-diff-with-approval
```

### 2.5 Resolution semantics (Q-rules-3 LOCKED)

Per-project > general (project keys win on conflict). Per-agent rules apply alongside (no precedence conflict typical — agent rules cover agent-specific behaviors not duplicated in general/project layers).

Deep-merge example:

```yaml
# general.yaml
tone:
  reply: "first-person, casual, lowercase"
  eod: "lead with what landed; bullets"

# projects/bcc-ad-manager.yaml
tone:
  reply: "first-person plural ('We're receiving...'), Greg-PR-style"
  # eod: not specified — inherits general
```

Resolved for project=bcc-ad-manager:
```yaml
tone:
  reply: "first-person plural ('We're receiving...'), Greg-PR-style"  # project wins
  eod: "lead with what landed; bullets"                              # general inherits
```

### 2.6 Schema validation (Q-rules-6 LOCKED)

Go struct schema lives in code (proposed package: `internal/rules/`). Daemon validates POSTed YAML against schema before write. Behavior:

- **Valid YAML + known keys:** write proceeds
- **Valid YAML + unknown keys:** write proceeds with warning logged + warning surfaced in HTTP response (forward-compat per "unknown keys allowed not blocked" lock)
- **Invalid YAML (syntax error):** 400 Bad Request with parse error
- **Schema-violation in known keys (wrong type, etc.):** 400 Bad Request with field-level diagnostic

### 2.7 Migration from feedback_*.md (Q-rules-2 LOCKED)

Parallel during transition. Phase N v3 substrate ships rules.yaml editing capability; agents continue reading existing `~/.claude/projects/.../memory/feedback_*.md` files. Phase O handles full migration: agent-side consumption added at task-start (rules.yaml takes precedence on conflict with feedback_*.md); per-feedback-file migration to corresponding rules.yaml entry over the Phase O cycle.

---

## 3. HTTP API surface

### 3.1 Auth + binding

Localhost loopback only (Q4 LOCKED + user msg 8670). Server binds to `127.0.0.1:<port>` (default `:3849`, configurable via env `BOT_HQ_WEBUI_PORT` per scope-lock OQ-2). No TLS, no auth tokens — relies on local-only access. Discord remains the remote channel.

Server lifecycle: started via `bot-hq webui` CLI subcommand. Stopped via Ctrl-C or process kill. No daemonization — runs in foreground for MVP.

### 3.2 Read endpoints

```
GET /api/files
```
Returns canonical-store directory tree as JSON. Recursively walks `~/.bot-hq/{phase,ratchets,projects,rules}/` and `~/.bot-hq/discipline-log.md`. Excludes `<agent>/`, `gates/`, `hub.db`, `live.log`, `sessions/`. Response shape:
```json
{
  "tree": [
    {"path": "phase/phase-n.md", "type": "file", "mtime": "2026-05-06T..."},
    {"path": "projects/bcc-ad-manager/plans/", "type": "dir", "children": [...]},
    ...
  ]
}
```

```
GET /api/files/{path}
```
Returns file content + mtime. Path is canonical-store-relative. Content-type `text/plain` for `.md`/`.yaml`; `application/json` envelope option via `?format=json`. Returns 404 for non-canonical paths.

```
GET /api/rules
```
Returns the resolved rules for a given project + agent context. Query params: `project=<key>`, `agent=<id>`. Response: deep-merged YAML rendered as JSON. Used by future Phase O agent consumption.

```
GET /api/sessions
```
Returns the session-cluster index (rolling list at `~/.bot-hq/sessions/index.md`) parsed into JSON. Query param: `project=<key>` filter.

```
GET /api/clive/activity
```
Server-Sent Events (SSE) stream of Clive's tool-call activity. Each event is a JSON-encoded line:
```
event: tool_call
data: {"agent":"clive","tool":"hub_send","ts":"...","preview":"draft for tom-reply..."}
```
Sourced from existing hub message stream (filter `from_agent=clive`).

### 3.3 Write endpoints

```
POST /api/files/{path}
Content-Type: text/plain
If-Match: <mtime>
```
User-web-save endpoint. mtime header is the client's last-known mtime; daemon checks against current file mtime and returns:

- **200 OK** if mtimes match → daemon writes + git-commits + emits `hub_send` notification
- **409 Conflict** if mtimes differ → response body has `current_mtime` + `current_content` for client-side merge UX (overwrite/discard/merge prompt per Q-v3c-conflict-handling LOCKED option (b))

Write semantics: atomic-rename (`os.Rename` after `os.WriteFile` to temp). Daemon commits to per-canonical-dir `.git` (lazy-initialized on first write per OQ-1). Author metadata: `user@webui` as commit author.

```
POST /api/files/{path}/clive
Content-Type: application/json
Body: {"diff": "<unified diff>", "from_content": "...", "purpose": "..."}
```
Clive-authored write endpoint. Daemon:
1. Renders diff to web UI via SSE event `clive_proposed_diff` with the diff content
2. Awaits user-approval via `POST /api/files/{path}/clive/approve` or `/cancel` (with the same purpose tag)
3. On approve: applies diff to current file content + writes (mtime-check skipped — Clive always sees latest via daemon read) + git-commits with author `clive@webui`
4. Emits `hub_send` notification

```
POST /api/rules/{layer}
{layer} ∈ {general, projects/<project>, agents/<agent>}
```
Convenience route that proxies through `POST /api/files/rules/{layer}.yaml` with schema-validation pre-write. Returns 400 on schema-violation per §2.6.

```
POST /api/files/{path}/revert
{"to_commit": "<sha>"}
```
One-click revert UI uses this to restore a file to a prior canonical-dir-git commit. Daemon performs `git show <sha>:<relative-path>` and writes the result + new commit.

### 3.4 Conflict-handling deep-dive (Q-v3c LOCKED option (b))

mtime-check on `If-Match` header is the load-bearing concurrency primitive. Client flow:

1. User opens file in editor → web UI fetches `GET /api/files/{path}` and stores mtime
2. User edits + clicks Save → web UI `POST /api/files/{path}` with `If-Match: <mtime>`
3. If 200: dirty-state cleared, mtime updated to response's new mtime
4. If 409: web UI prompts user with overwrite/discard/merge options:
   - **Overwrite:** re-POST with `If-Match` header set to `current_mtime` from 409 response (forced overwrite)
   - **Discard:** discard local edits; reload file from server
   - **Merge:** show 3-way diff (local / server-current / common-ancestor); user resolves manually

### 3.5 3-layer visibility wiring

Layer 1 — `hub_send` notification:
- On every successful write at `POST /api/files/{path}` or `POST /api/files/{path}/clive`, daemon emits `hub_send` of type `update` to broadcast (or `result` on Clive-write):
  ```
  type: update
  from: webui-daemon (or user@webui)
  content: "user edited <path> via web UI" (or "clive committed <path> diff")
  ```
- Trio agents see this in hub backlog; can react if needed (e.g., Rain BRAIN-2nd on user edit of canonical artifact)

Layer 2 — SSE live web feed:
- Web UI subscribes to `/api/clive/activity` on page load
- Every Clive tool-call (read, write, hub_send) renders to "Clive activity panel" in real-time
- Source: existing hub message stream filtered by `from_agent=clive`

Layer 3 — per-canonical-dir `.git` audit:
- Each canonical-store directory has its own `.git/` (initialized lazily by daemon on first write per OQ-1)
- Every daemon write commits with author + timestamp + commit message: `<actor>: <relative-path> <verb>` (verbs: `created`, `modified`, `reverted`)
- Web UI exposes per-file history view: `GET /api/files/{path}/history` (lists commits)
- Revert via `POST /api/files/{path}/revert` (§3.3)

---

## 4. Implementation surface (for v3b + v3c reference)

### 4.1 New package: `internal/webui/`

```
internal/webui/
├── server.go        # HTTP server, route mux, lifecycle
├── handlers.go      # GET/POST handlers per §3.2 + §3.3
├── filesystem.go    # canonical-store walk, mtime checks, atomic-rename
├── git.go           # per-dir .git lazy init + commit + revert + history
├── rules.go         # YAML schema validation (Go struct), resolution semantics
├── sse.go           # SSE event-stream helpers (Clive activity)
├── index.go         # SQLite read-index population via fsnotify watcher
└── static/
    ├── index.html
    ├── app.js       # htmx + minimal JS
    └── style.css    # minimal styling
```

### 4.2 New CLI subcommand: `bot-hq webui`

```
bot-hq webui [--port 3849] [--db ~/.bot-hq/webui-index.db]
```

### 4.3 New hook in `cmd/bot-hq/main.go`

Register webui subcommand alongside existing `install-toolgate-hook`, `install-voice-mirror-hook`, `session-load`, etc.

### 4.4 Dependencies

- `gopkg.in/yaml.v3` — YAML parsing (already in go.mod for sessions package)
- `github.com/fsnotify/fsnotify` — filesystem watcher for read-index population
- `modernc.org/sqlite` — SQLite for read-index (already in go.mod for hub.db)
- htmx via CDN script in HTML — no Go-side dep
- CodeMirror 6 via CDN — for raw-YAML rules editor with syntax highlighting

---

## 5. Open questions for PASS-2 (Rain BRAIN-2nd)

1. **GET /api/files/{path}/history pagination?** Simple list for MVP (last N=20 commits); pagination via `?page=<n>` if needed later. Concur?
2. **SSE reconnection semantics?** Client auto-reconnect on disconnect; daemon assigns event-id for resume-from-id. Standard SSE behavior. Concur?
3. **Diff rendering — server-side (`git diff --color=never` rendered to HTML) or client-side (`diff-match-patch`)?** Server-side per scope-lock OQ-7 lean. Concur?
4. **Schema struct location — `internal/rules/` or in `internal/webui/rules.go`?** Lean separate package `internal/rules/` so future Phase O agent-consumption can import without webui dep. Concur?
5. **Filesystem watcher event coalescing window?** Lean 200ms debounce on writes (avoid rapid-fire DB index updates during editor saves). Concur?
6. **Clive-write approve/cancel timeout?** If user neither approves nor cancels within N minutes, drop the proposal. Lean 10 minutes. Concur?
7. **Revert idempotency?** Revert-of-revert restores to original. Trivial via standard git semantics. Concur (no special handling needed)?

---

## 6. Cross-references

- **Phase N v3 scope-lock:** `~/.bot-hq/phase/phase-n.md§v3` (Stage 1 LANDED 2026-05-06)
- **v3a commit:** `54c9db8` (id-sessions writer-flow + canonical-store substrate)
- **R31 OVER-CLAIM phase-close-arc-snapshot-class anchor:** `~/.bot-hq/discipline-log.md` §2026-05-06T(post-v2-close-pre-v3-open) joint entry
- **Bilateral architecture decisions:** msgs 8669 + 8675 + 8693 + 8703 + 8708 + 8709
- **User direction:** msgs 8666 + 8670 + 8682 + 8683 + 8689 + 8699 + 8702 + 8711
- **Phase N v2 arc-snapshot:** `docs/arcs/phase-n-v2.md` (with v3a §4+§6 amendments)
- **Phase M target-A enforcement design (precedent for daemon-write pattern):** `docs/plans/2026-05-04-phase-m-target-A-OUTBOUND-MISS-enforcement-design.md`
- **N-1 (a) ID-sessions design-spike (carry-from precedent):** `docs/plans/2026-05-05-phase-n-N-1-id-based-sessions-design-spike.md`
