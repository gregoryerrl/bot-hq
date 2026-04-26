# Phase G v1 Design (β scope)

Status: drafting | Owner: Brian (HANDS) | Reviewer: Rain (EYES, greenflag-final until rebuild) | Greenlight: saltegge @ msg 2923

## 1. Goal

Land two slices of bot-hq UX + persistence improvements. Slice 1 ships visible UX wins immediately; Slice 2 lays the persistence layer that compounds across future arcs.

**Slice 1 (Code):** A — jump-to-present + agent-pane modal. #20 — rebuild fingerprint counter.
**Slice 2 (Code):** B — SNAP-typed schema + SNAP table + arc table + arc.md narrative.
**Convention v1 (zero-code, effective msg 2922):** greenflag scope stamping, restate-before-execute, user-surface check gate. Already adopted; will be formally locked in DISC/CLAUDE.md as a v2 follow-up.

**Out of scope for v1:** Sessions→Arc tab UI consumer (deferred; Slice 2 ships only the data layer), ack flag, pre-spec adversarial dispatch codification, hub_flag rebuild-procedure variant, Discord pinned flags, Emma expansions, hub_read per-type budget.

**Sequencing:** Slice 1 ships first → flag rebuild → saltegge eyeballs → post-rebuild, Slice 2 starts. β = 2 rebuilds total.

## 2. Slice 1 — A + #20

### 2.1 Jump-to-present (HubTab)

**Problem:** On hub restart the viewport renders mid-conversation; user must scroll to the bottom by hand. There's no shortcut to snap to present.

**Mechanism:** Add a "follow-bottom" sticky flag in `HubTab` and one keybinding.

- New field: `HubTab.followBottom bool` (default `true`).
- On `MessageReceived`:
  - `viewport.SetContent(...)` (already present).
  - Only call `viewport.GotoBottom()` if `followBottom` is true. Currently it's unconditional (`hub_tab.go:98`); the unconditional call fights any user scroll-up. Switch to conditional.
- On viewport scroll (mouse wheel, PgUp/PgDn, j/k while hub viewport has focus):
  - After the viewport update, recompute `followBottom = h.viewport.AtBottom()`. Lets the user scroll up to read history without being snapped back, and re-engages auto-follow when they scroll to bottom.
- Keybinding (when input is **not** focused): `G` and `end` → `viewport.GotoBottom()` + set `followBottom = true`. Vim convention + standard end-key.
- On first `SetSize` after hub init (initial render path): unconditionally `GotoBottom()` once. This fixes the "post-restart renders in the middle" complaint.
- **SetSize gate (Rain C2):** `internal/ui/hub_tab.go:173` currently does unconditional `viewport.GotoBottom()` inside `SetSize` (via `resize` → `SetContent` → snap). Replace with: snap to bottom **only if `followBottom == true`**. The default-true `followBottom` preserves the initial-render snap (which fixes the post-restart-mid-conversation bug); a user mid-scroll-up does NOT get snapped on a terminal resize. Same gate also applies in `SetSessionFilter` (currently unconditional `GotoBottom` at line 173).

**File:** `internal/ui/hub_tab.go` only. ~20 LOC delta.

**Test surface:** extend `internal/ui/hub_tab_test.go`:
- `TestHubTabFollowBottomDefault`: new `HubTab` has `followBottom == true`.
- `TestHubTabAutoFollowOnNewMessage`: simulate AtBottom + MessageReceived → `GotoBottom` invoked (asserted via post-state YOffset / `AtBottom()`).
- `TestHubTabUserScrollUpDisablesFollow`: simulate PgUp → `followBottom == false` and a subsequent MessageReceived does NOT snap to bottom.
- `TestHubTabJumpToPresentKey`: send `G` keypress → `followBottom == true` and `AtBottom() == true`.
- `TestHubTabResizePreservesScrollPosition` **(Rain A1)**: scroll up to disengage `followBottom`, then `SetSize(new_dims)` → assert YOffset preserved (no GotoBottom fired). Counterpart: `followBottom=true` + SetSize → snap-to-bottom still fires.

### 2.2 Agent-pane modal (AgentsTab)

**Problem:** To watch an agent's output the user has to `tmux attach -t <target>` in another terminal. Unergonomic; defeats the TUI's value.

**Mechanism:** Add a cursor + Enter handler to `AgentsTab` that opens a read-only modal viewport showing `tmux capture-pane -t <target> -p` output. Esc closes. `r` refreshes manually. Optional `f` toggles 1Hz auto-follow.

**Component shape:**

- New `AgentsTab` fields: `cursor int`, `paneModal *PaneModal`. When `paneModal != nil` the modal owns input until closed.
- New file `internal/ui/agents_pane_modal.go` with:
  - `type PaneModal struct { target, content string; viewport viewport.Model; autoFollow bool; lastErr error }`
  - `func NewPaneModal(target string) *PaneModal`
  - `func (m *PaneModal) Refresh() error` — runs `tmux capture-pane -t <target> -p -S -500` **(Rain C3+C4: drop `-e` flag, plain text only; add `-S -500` for 500 lines of scrollback so modal shows recent history not just visible viewport)**, captures stdout, runs through `stripANSI` defensively, sets viewport content, scrolls to bottom. 2-second `context.WithTimeout` guards against tmux daemon hangs.
  - `func (m *PaneModal) Update(msg tea.Msg) (PaneModal, tea.Cmd)` — handles `r` (refresh), `esc` (close — emit `PaneModalClosed{}`), `f` (toggle autoFollow), arrow keys (viewport scroll). When `autoFollow` is on, an internal 1Hz `tea.Tick` triggers `Refresh()`.
  - `func (m *PaneModal) View() string` — renders modal frame: title bar `tmux:<target>  [r] refresh  [f] follow:<on|off>  [esc] close`, content viewport, error footer if `lastErr != nil`.

- AgentsTab Update changes:
  - On `tea.KeyMsg` while `paneModal == nil`:
    - `j/down` → `cursor++` (clamp).
    - `k/up` → `cursor--` (clamp).
    - `enter` → if `cursor` row has a `tmux_target` in meta, instantiate `PaneModal`, call `Refresh()`. If meta lacks a target, no-op.
  - On `tea.KeyMsg` while `paneModal != nil`: forward to `paneModal.Update(msg)`. If it returns `PaneModalClosed`, set `paneModal = nil`.
- AgentsTab View: when `paneModal != nil`, render the modal centered over the agents list (lipgloss.Place). Otherwise render the existing list with a `▸` cursor on the selected row.

**Security notes:**
- `tmux capture-pane` is read-only — no `send-keys` in v1. Capture-only is locked in v1; if we ever add send-keys, that's a separate arc with explicit user grant.
- `target` is read from `agent.Meta` (already trusted, came from agent's own register call). Still pass to `tmux` via `exec.Command` arg slice (not shell), no string concat.

**File:** `internal/ui/agents_tab.go` (modify), `internal/ui/agents_pane_modal.go` (new ~120 LOC), wiring in `internal/ui/app.go` for the `tea.Tick` plumbing.

**Test surface:**
- `internal/ui/agents_tab_test.go`:
  - `TestAgentsTabCursorNav`: j/k clamps to bounds.
  - `TestAgentsTabEnterOpensModal`: enter on agent with tmux_target → modal != nil; enter on agent without → modal == nil.
  - `TestAgentsTabModalEscClosesModal`: modal active + Esc → modal == nil.
- `internal/ui/agents_pane_modal_test.go` (new):
  - `TestPaneModalRefreshShellInjection`: target with shell metachars (`; rm -rf /`) — verify no shell expansion (target is passed as exec arg; tmux will reject the literal).
  - `TestPaneModalAutoFollowToggle`: `f` toggles flag.
  - Refresh-stub helper that injects a fake capture function so tests don't shell out to real tmux.

### 2.3 Rebuild fingerprint (#20)

**Problem:** Pre-rebuild agent registrations leak into post-rebuild hub state ("5 online 11 offline" emma anomaly). Currently we squint at `last_seen` and guess.

**Mechanism:**

- Settings row: `hub_rebuild_gen` (string-encoded int). Initialized to `1` if absent.
- On hub start (`internal/hub/hub.go` constructor / `New()`): increment `hub_rebuild_gen` by 1 atomically (read, +1, write inside a transaction). Cache the value on the `Hub` struct as `RebuildGen int`.
- Agents table: add column `rebuild_gen INTEGER NOT NULL DEFAULT 0`. Migration via `ALTER TABLE agents ADD COLUMN rebuild_gen INTEGER NOT NULL DEFAULT 0` guarded by a migration check (sqlite `PRAGMA table_info(agents)` lookup).
- On `RegisterAgent`: stamp `rebuild_gen = hub.RebuildGen` for the agent row.
- Surface (Rain P3 explicit lock):
  - `protocol.Agent.RebuildGen int` field on the struct.
  - **AgentsTab View**: stale-gen agents stay **visible-but-flagged** with a `(stale-gen)` suffix after the name. Saltegge can spot them and prune manually (`hub_unregister`). NOT invisible.
  - **Strip**: stale-gen agents are **omitted** from the visible set (treated like Offline). The strip is a first-order check; cluttering it with definitely-stale registrations defeats the purpose.

**File:** `internal/hub/db.go` (migration + RegisterAgent), `internal/hub/hub.go` (constructor increment), `internal/protocol/types.go` **(Rain C1: corrected from non-existent `protocol.go`)** (Agent struct field), `internal/ui/agents_tab.go` + `internal/ui/strip.go` (display).

**Test surface:**
- `internal/hub/db_test.go`: `TestRebuildGenMigrationIdempotent` (running migrate twice doesn't double-add column), `TestRegisterAgentStampsRebuildGen`.
- `internal/hub/hub_test.go`: `TestHubStartIncrementsRebuildGen` (open DB twice → gen increments).
- `internal/ui/strip_test.go`: stale-gen agent omitted from strip render.
- `internal/ui/agents_tab_test.go` **(Rain A2)**: `TestAgentsTabStaleGenSuffix` — render an agent with `RebuildGen != hub.RebuildGen` → assert `(stale-gen)` suffix appears after the name in the rendered row.

### 2.4 Slice 1 commit shape

**Reordered per Rain P1**: A1 → #20 → A2. Both A2 (modal) and #20 (stale-gen suffix) touch `agents_tab.go`; landing #20 first means A2 builds on the post-#20 View structure, cleaner diff review.

Three commits on `brian/phase-g-v1-slice-1`:
1. **A1** — jump-to-present (`hub_tab.go` + tests). Also **creates `docs/arcs/phase-g-v1.md` skeleton** per Rain P2 — narrative file dogfooded live, not retroactive in B3.
2. **#20** — rebuild_gen migration + hub-start increment + register stamp + UI hint (db.go + hub.go + types.go + agents_tab.go + strip.go + tests).
3. **A2** — agent-pane modal (agents_tab.go cursor/Enter + agents_pane_modal.go new + tests + app.go wiring).

Each commit independently testable (`go test ./internal/ui/... ./internal/hub/... ./internal/protocol/...` clean per commit).

### 2.5 Slice 1 rebuild boundary

After merge: flag rebuild #7 with procedure:
```bash
cd /Users/gregoryerrl/Projects/bot-hq
git pull
# rebuild + restart bot-hq
```

Saltegge verification (user-surface gate per new convention):
- Open hub, verify scrolled-to-bottom on launch.
- Scroll up, verify staying scrolled (not snapped).
- Resize the terminal while scrolled up — verify position preserved (no snap).
- Press `G` (or `end`), verify jump to bottom.
- Tab to Agents, j/k cursor, Enter on an agent with a tmux target → modal opens, shows pane content with scrollback. Esc closes. `r` refreshes. `f` toggles auto-follow.
- **Stale-gen check (Rain P3 explicit):** stale-gen entries appear in agents tab with `(stale-gen)` suffix after the name, can be manually pruned via `hub_unregister`. Strip omits them entirely.

PASS → Slice 2 begins. FAIL → Brian respins.

## 3. Slice 2 — B (post-Slice-1-rebuild)

### 3.1 SNAP-typed schema

**Problem:** SNAP blocks at end of agent messages are convention-only. To make them load-bearing for resume + arc state, they need a typed shape.

**Schema (locked):**

```go
// internal/protocol/snap.go (new)
type SNAP struct {
    Branches []string  `json:"branches"`  // "repo:branch@sha(state)" strings
    Agents   []string  `json:"agents"`    // "name(state)" strings
    Pending  string    `json:"pending"`   // one-line blocker
    Next     string    `json:"next"`      // one-line action
}

// MarshalText renders the canonical SNAP block:
//   SNAP:
//   Branches: a, b, c
//   Agents:   x(s), y(s)
//   Pending:  ...
//   Next:     ...
func (s SNAP) MarshalText() ([]byte, error) { ... }

// ParseSNAP extracts a SNAP block from the *tail* of a message body.
// Returns ok=false if no recognizable SNAP trailer.
func ParseSNAP(body string) (SNAP, bool) { ... }
```

`hub_send` does NOT add new fields; SNAPs ride in the message `content` tail as today. The parser/marshaller is the contract. Agents emit via `MarshalText`, hub parses on insert and stores the structured form in the SNAP table.

**File targets (Rain C1):** new `internal/protocol/snap.go` (struct + Marshal/Parse), new `internal/protocol/arc.go` (Arc struct), tests in same package. **No edit to non-existent `protocol.go`** — package files are `types.go`, `disc.go`, `constants.go` plus test siblings.

### 3.2 SNAP table

```sql
CREATE TABLE IF NOT EXISTS agent_snaps (
    agent_id    TEXT PRIMARY KEY,
    branches    TEXT NOT NULL DEFAULT '',  -- JSON array
    agents      TEXT NOT NULL DEFAULT '',  -- JSON array
    pending     TEXT NOT NULL DEFAULT '',
    next        TEXT NOT NULL DEFAULT '',
    message_id  INTEGER NOT NULL,           -- FK to messages.id (last carrier)
    updated     INTEGER NOT NULL
);
```

One row per agent (latest SNAP wins). Updated atomically inside `InsertMessage` when `ParseSNAP(content)` succeeds.

DB API:
- `func (db *DB) GetAgentSNAP(agentID string) (protocol.SNAP, bool, error)`
- `func (db *DB) ListAgentSNAPs() ([]struct{AgentID string; SNAP protocol.SNAP}, error)`

**Resume primer use case:** On agent register-time `hub_register`, the response includes the agent's last SNAP if one exists. Saves the agent from re-deriving "where were we" out of `hub_read`.

**Stale-gen SNAPs included (Rain A4 explicit lock):** The resume primer does **not** filter on `rebuild_gen`. By definition, a registering agent's prior-gen SNAP is exactly the context that helps the new-gen agent catch up. Filtering by gen would defeat the primer's purpose. The agent's own job is to verify the SNAP is still load-bearing (per memory-staleness discipline) before acting on it.

### 3.3 Arc table

```sql
CREATE TABLE IF NOT EXISTS arcs (
    id          TEXT PRIMARY KEY,         -- e.g. "phase-g-v1"
    name        TEXT NOT NULL,
    status      TEXT NOT NULL,            -- "open" | "closed" | "deferred"
    branch      TEXT DEFAULT '',
    summary     TEXT DEFAULT '',
    opened      INTEGER NOT NULL,
    closed      INTEGER DEFAULT 0,
    md_path     TEXT DEFAULT ''           -- relative path to arc.md narrative
);
```

DB API:
- `func (db *DB) UpsertArc(arc protocol.Arc) error`
- `func (db *DB) GetArc(id string) (protocol.Arc, error)`
- `func (db *DB) ListArcs(statusFilter string) ([]protocol.Arc, error)`

No FK from messages → arcs in v1 (would require backfill). Arc-message linkage is a v2 concern when the Sessions→Arc tab UI is built.

### 3.4 arc.md narrative

Per-arc markdown at `docs/arcs/<arc-id>.md`. Append-only log of decisions, deferred items, branch refs.

Format (template):

```markdown
# Arc: <name>

Status: <open|closed|deferred>  | Branch: <branch>  | Opened: <date>  | Closed: <date|—>

## Context

<narrative paragraph from initial brainstorm / spec link>

## Decisions

- YYYY-MM-DD HH:MM — <decision>; greenflag by <agent>
- ...

## Deferred

- <item> — reason

## Refs

- design doc: docs/plans/...
- plan doc: docs/plans/...
- commits: <sha1>, <sha2>
```

Arc.md is human-curated (Brian/Rain edit during arcs). The DB row is the index; the MD is the narrative. Two surfaces, one truth via `arc.id`.

For Phase G itself, `docs/arcs/phase-g-v1.md` will be created as part of Slice 2's first commit (dogfood the format).

### 3.5 Slice 2 commit shape

Three commits on `brian/phase-g-v1-slice-2`:
1. **B1** — `protocol/snap.go` + `protocol/arc.go` + tests for SNAP marshal/parse roundtrip and ParseSNAP edge cases (no-trailer, partial trailer, multiple SNAP blocks → last wins).
2. **B2** — DB migration (agent_snaps + arcs tables) + DB API methods + tests.
3. **B3** — Wire SNAP parse into `InsertMessage`, expose SNAP via register response, **finalize `docs/arcs/phase-g-v1.md`** (skeleton was created in Slice 1 commit A1 per Rain P2 — B3 closes status to `closed` and appends final decisions/branch refs), tests for hub-side wiring including `TestRegisterAgentResponseCarriesSNAP` **(Rain A3)** which asserts: register agent that has prior SNAP row → response carries parsed SNAP. Backward-compat sub-test: agents not consuming the field still unmarshal cleanly.

### 3.6 Slice 2 rebuild boundary

After merge: flag rebuild #8 with same shape procedure.

Saltegge verification (user-surface gate):
- Stop hub, restart, observe agents register → spot-check that an agent's SNAP survives the restart (e.g. `sqlite3 ~/.bot-hq/hub.db "SELECT * FROM agent_snaps;"` has rows; the resume-primer in register-response carries them).
- Open `docs/arcs/phase-g-v1.md`, verify it's been populated as the arc closed.
- No regressions on Slice 1 features (jump-to-present, modal still work).

PASS → arc closure declared per user-surface gate. FAIL → Brian respins.

## 4. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| `tmux capture-pane` blocks if tmux daemon hangs | Run with a `context.WithTimeout(2s)` wrapper. Surface error in modal footer, don't deadlock the TUI. |
| `ALTER TABLE` migration fails on older sqlite | Guard with `PRAGMA table_info` lookup; only ALTER if column missing. SQLite 3.25+ supports this; bot-hq already requires modern sqlite. |
| SNAP parse misclassifies a freetext "Branches:" line | Anchor parser to require all four labels (`Branches:`, `Agents:`, `Pending:`, `Next:`) appear contiguously at the message tail prefixed with a `SNAP:` header line. Single regex with multiline anchors. |
| `followBottom` sticky-flag bug on resize | On `SetSize`, recompute `followBottom = h.viewport.AtBottom()` post-resize (resize can shift content; preserve user intent). |
| Agent without `tmux_target` in meta crashes modal | Enter is a no-op (early return) when target empty. Test covers it. |
| rebuild_gen migration on agents currently online: their old rows have `rebuild_gen=0` and would be hidden from strip post-restart | Acceptable: post-restart they re-register with the new gen. Pre-restart rows are by definition stale-gen. |

## 5. Test Plan Summary

- **Slice 1:** `go test ./internal/ui/... ./internal/hub/... ./internal/protocol/...` clean. New tests listed §2.1, §2.2, §2.3.
- **Slice 2:** `go test ./internal/protocol/... ./internal/hub/...` clean. New tests listed §3.5.
- **Vet/build:** `go vet ./... && go build ./...` clean per commit.
- **User-surface gate** (per new convention): saltegge eyeball each rebuild, listed §2.5 and §3.6.

## 6. Open Decisions (for Rain spec gate)

1. **`G` vs only `end` for jump-to-present.** Vim convention is `G` (and `gg` for top). I propose both `G` and `end`; reject `gg` for v1 (rare ask). Rain: concur or simplify?
2. **Modal autoFollow default.** Off by default (manual `r` first), user toggles `f` to follow. Alternative: on by default with `f` to disable. I propose **off by default** — explicit user intent reduces tmux-capture churn for idle modals.
3. **Arc-message FK in v1?** I have it as v2 concern. Confirms with Rain — if Rain wants the messages.arc_id column now (cheap migration), bundle into B2. My read: defer.
4. **SNAP storage redundancy.** Each SNAP lives both in `messages.content` (tail) and `agent_snaps` (parsed). On message replay we re-parse from content, not from the SNAP table; the SNAP table is just an index. Confirms with Rain.

## 7. Sequencing Recap

```
Slice 1 (A + #20)
  → branch brian/phase-g-v1-slice-1
  → 3 commits (A1, A2, #20)
  → push, Rain diff gate
  → merge to main
  → flag rebuild #7
  → saltegge user-surface verification
  → PASS proceeds; FAIL respins

Slice 2 (B)
  → branch brian/phase-g-v1-slice-2
  → 3 commits (B1, B2, B3 — B3 includes docs/arcs/phase-g-v1.md)
  → push, Rain diff gate
  → merge to main
  → flag rebuild #8
  → saltegge user-surface verification
  → PASS = Phase G v1 arc closed
```

## 8. Out-of-scope reminders

Defer to Phase G v2:
- Sessions tab → Arc tab UI consumer
- Explicit ack flag (`ack: required|optional|none`)
- Pre-spec adversarial dispatch codification
- hub_flag rebuild-procedure variant
- Discord pinned open-flags message
- hub_read per-type budget
- Formal DISC/CLAUDE.md updates for Convention v1

Defer to Phase G v3 (gate carefully):
- Emma boot-summarization, message pre-classify, log anomaly tail

Skipped:
- Hidden claude_message channels
- Daily/post-arc digest via emma
- Auto-resolve flags on timeout
