# Phase E Specification (v3)

Status: drafting | Owner: Brian (HANDS) | Reviewer: Rain (EYES) | Greenlight: saltegge @ msg 2231

## 1. Goal

Surface agent activity in the TUI for **first-order user check** so the user can glance and decide whether to escalate to **second-order** tmux pane inspection.

Three deliverables:
1. New `panestate` package — centralized agent-activity state.
2. Hub-strip above HubTab input bar — per-agent dots showing recency.
3. Agents-tab Status column source switch — reads from `panestate`, not direct DB poll.

Vocabulary:
- **Tier** = user diagnostic order (T1 strip glance, T2 tmux pane). **NOT** agent health classification.
- **Activity recency** = `now - agent.last_seen`. Derived in `panestate`, displayed in TUI.

## 2. Signal Architecture — (G) MCP Middleware

### Why (G), not (E)/(F)
- (E) message-flow inference: misses silent coders that don't broadcast.
- (F) tmux `pane_activity`: empty in tmux 3.6a per Probe 4. `session_activity` only updates on attach/detach. Non-functional today.
- (G) MCP middleware: every MCP tool handler in `internal/mcp/tools.go` receives `from`/`agent_id` as required param. One-touch update of `last_seen` on dispatch. Real-time, sub-second, agent-independent.

### Implementation shape
- Middleware wrapper in `internal/mcp/tools.go` applied at registration time. Pseudocode:
  ```go
  // commonAgentIDKeys locks the priority order for cross-tool agent extraction.
  // First non-empty match wins. Update with comment if a future tool uses a new name.
  var commonAgentIDKeys = []string{"from", "agent_id", "id"}

  func extractAgentID(params map[string]any) string {
      for _, k := range commonAgentIDKeys {
          if v, ok := params[k].(string); ok && v != "" {
              return v
          }
      }
      return ""
  }

  func withLastSeen(handler ToolHandler) ToolHandler {
      return func(ctx, params) (result, error) {
          if id := extractAgentID(params); id != "" {
              db.UpdateAgentLastSeen(id)  // throttled — see below
          }
          return handler(ctx, params)
      }
  }
  ```
- New `db.UpdateAgentLastSeen(id)` method (`internal/hub/db.go`) that touches only `last_seen`, NOT `status` (separate from existing `UpdateAgentStatus`). One row write per call.
- **Per-agent write throttle**: skip the DB write if `now - lastWrite[id] < 1s`. Implemented as a `sync.Map[string]time.Time` in the middleware closure. ~5 LOC. Reduces DB churn from ~50 writes/min to ~6/min per active agent during peer coord. Sub-second granularity not needed (5s threshold tolerates 1s lag).
- **Tools without an agent-id param** (e.g. `claude_*` tools that operate on `session_id` not agent-bound) pass through with no write. Their session_id may be linked to an agent in `agents.meta.tmux_target`, but resolving that adds complexity for marginal gain — defer to Phase F if needed.

### Threshold mapping (locked: 5s / 60s)
```
recency = now - agent.last_seen
recency <  5s   → working  ●
recency < 60s   → online   ◐
recency >= 60s  → stale (hidden from strip; visible greyed in Agents tab)
offline (status=offline) → hidden from strip; visible greyed in Agents tab
```

## 3. `panestate` Package Shape

Location: `internal/panestate/`

### Types
```go
type AgentActivity int
const (
    ActivityWorking AgentActivity = iota  // <5s
    ActivityOnline                          // <60s
    ActivityStale                           // >=60s, not offline
    ActivityOffline                         // status=offline
)

type AgentSnapshot struct {
    ID        string
    Name      string
    Type      protocol.AgentType
    Status    protocol.AgentStatus
    LastSeen  time.Time
    Activity  AgentActivity   // derived from now - LastSeen + Status

    // Phase F forward deps (locked schema, F populates):
    LastClassification string         // markers.toml category from capture-pane scan
    RecentErrors       []ClassifierHit // ring buffer of last 8 hits
}

type ClassifierHit struct {
    Category  string
    Excerpt   string
    Timestamp time.Time
}

type Manager struct {
    mu       sync.RWMutex
    snapshot []AgentSnapshot   // single source of truth, all tabs read this
}
```

### Threading
- Single `Manager` owned by `App`. `App.Init` constructs it and starts polling.
- Polling: 1s tick (existing `App` tick) calls `Manager.Refresh(db)` which queries `db.ListAgents("")` and computes `Activity` per row.
- Tabs hold `*panestate.Manager` reference; `View()` reads via `Manager.Snapshot()` (RLock).
- No channels. RWMutex'd shared snapshot. Replaces my v0's polling assumption with explicit shape.

### Constants
```go
const (
    WorkingWindow = 5 * time.Second
    OnlineWindow  = 60 * time.Second
    StaleAgentWindow = 7 * 24 * time.Hour  // for filter queries (Phase F audit)
)
```

## 4. File Touch List

New:
- `internal/panestate/panestate.go` — types, Manager, ComputeActivity
- `internal/panestate/panestate_test.go` — table tests for ComputeActivity, Manager.Refresh

Modified:
- `internal/mcp/tools.go` — middleware wrapper applied to all tool registrations
- `internal/mcp/tools_test.go` — test middleware updates `last_seen` on tool call
- `internal/hub/db.go` — new `UpdateAgentLastSeen(id)` method
- `internal/hub/db_test.go` — test method touches only `last_seen`
- `internal/protocol/types.go` — DELETE `StatusIdle` const (dead code per Probe findings)
- `internal/ui/app.go` — construct `panestate.Manager`, pass to tabs
- `internal/ui/agents_tab.go` — read snapshot from `*panestate.Manager`, render Activity column
- `internal/ui/hub_tab.go` — render hub-strip above input bar in `View()`
- `internal/ui/styles.go` — **rename** existing `StatusOnline`/`StatusOffline` to `ActivityWorking`/`ActivityOnline`/`ActivityStale`/`ActivityOffline` to match the new activity-based model. Activity-derived dots supersede status-based dots throughout the UI; renaming avoids parallel taxonomies.

## 5. Commit Cut (5 commits, ordered, each testable)

### Commit 1 — `panestate` package + `StatusIdle` cleanup
- New `internal/panestate/{panestate.go, panestate_test.go}`
- Types, Manager, `ComputeActivity(snapshot, now)` pure fn, `StaleAgentWindow` const
- DELETE `protocol.StatusIdle` (and its case in `Valid()`, all references)
- Tests:
  - Table test for `ComputeActivity`: <5s → Working, <60s → Online, ≥60s → Stale, status=offline → Offline
  - `Manager.Refresh` populates snapshot from a fake DB list
  - Greppable assertion: no `StatusIdle` references remain

Independently testable: yes (pure package, no consumers yet).

### Commit 2 — MCP middleware + `db.UpdateAgentLastSeen`
- New `db.UpdateAgentLastSeen(id string) error` in `internal/hub/db.go`
- Middleware wrapper `withLastSeen` in `internal/mcp/tools.go` applied at registration to all tool handlers
- Tests:
  - `db.UpdateAgentLastSeen` updates only `last_seen`, leaves `status` untouched (regression-locks v2.1's bug-pattern)
  - Middleware test: after calling any wrapped tool with `from=X`, `db.GetAgent(X).LastSeen` is within 1s of `now`
  - Middleware test: tool calls without an agent-id param are pass-through (no DB write)
  - **Per-tool extraction matrix**: parametrized test iterating every existing tool registration (`hub_send`, `hub_read`, `hub_register`, `hub_status`, `hub_agents`, `hub_flag`, `hub_session_create`, `hub_spawn`, etc.) — each verified to extract the agent ID correctly via `commonAgentIDKeys` priority order. Locks against future tools accidentally falling outside the extraction set.
  - **Throttle test**: two rapid wrapped-tool calls within 1s for same agent → only one `UpdateAgentLastSeen` write. Second call within window is suppressed.

Independently testable: yes (DB + MCP only, no UI).

### Commit 3 — App wires `panestate.Manager`
- `App.Init` constructs `Manager`, kicks off refresh loop on existing 1s tick
- Tabs receive `*panestate.Manager` via constructor params (replaces direct DB list calls in `App.Update`)
- Tests:
  - `App` constructs Manager non-nil
  - `App.Update(tickMsg)` calls `Manager.Refresh` and propagates snapshot
  - Tabs receive snapshot on update
  - **Snapshot freshness assertion**: after `Manager.Refresh` runs against a DB list with new `last_seen`, `Manager.Snapshot()` returns the new state. Locks against tabs holding stale `[]AgentSnapshot` copies instead of `*Manager` references.

Behavior change at user-visible level: zero (data flow rerouted, render unchanged).

### Commit 4 — hub-strip render + agents-tab source switch
- `HubTab.View()`: insert strip line between separator and input. Pseudocode: `lipgloss.JoinVertical(viewport, separator, strip, input)`
- Strip rendering: per-agent dots colored by Activity, hide Stale + Offline. **Order by agent type tier**, name as tiebreaker within tier:
  - Tier 1 (peer-coord): `AgentBrian`, `AgentQA`, `AgentVoice`
  - Tier 2 (services): `AgentDiscord`, `AgentGemma`
  - Tier 3 (workers): `AgentCoder`
  - Robust to new agent additions; no hardcoded name list.
- `AgentsTab.View()`: Status column reads `Activity` from `*panestate.Manager`, not raw `ag.Status`. Display: `● working` / `◐ online` / `○ stale` / `· offline`.
- Wrap behavior: cap at 8 agents visible, surplus collapsed to "+N".
- Tests (use **substring assertions** on rendered output, not byte-exact golden — lipgloss escape codes vary across terminal envs):
  - `HubTab.View()` output contains strip line at expected position when Manager has agents (assert dot chars + agent IDs are present)
  - `AgentsTab.View()` output contains expected Activity labels (e.g. "working", "online") next to corresponding agent IDs
  - Wrap test: 12-agent input → 8 distinct agent IDs + literal "+4" at narrow width (40 cols), all 12 present at wide (120 cols)
  - Empty test: 0 alive agents → strip line contains no dot characters (`●` / `◐` absent)

### Commit 5 — Phase F dependency comment
- Inline comment in `panestate.go` near `LastClassification` / `RecentErrors` fields:
  ```go
  // Phase F prerequisite: capture-pane classifier (markers.toml regex) populates these.
  // Phase F's stall-detector consumes them. Do not remove without updating Phase F's spec.
  // See docs/plans/phase-f.md (when drafted).
  ```
- Inline comment in `db.go` near `UpdateAgentLastSeen`:
  ```go
  // Phase F prerequisite: heartbeat goroutine (when added) calls this on a timer
  // for agents that don't initiate MCP calls (e.g. dormant coders awaiting input).
  ```
- No code change beyond comments. Tests: none required (documentation commit).

## 6. Out of Scope

- `(F)` tmux pane_activity / session_activity — Probe 4 falsified, deferred to Phase F
- Capture-pane content hashing / classifier (markers.toml) — Phase F
- Heartbeat MCP tool — Phase F (b) infra
- WS-frame instrumentation — Phase F (Clive-only today, narrow value alone)
- DB pruning of stale-online agents (the `claude_stop` no-offline-flip bug, see §8) — separate bug bundle
- `hub_spawn` prompt delivery fix — separate bug bundle
- `claude_message` false-busy fix — separate bug bundle
- Phase F itself (stall detector, error-string detector)

## 7. Post-Merge Verification Ladder

Saltegge's smoke check after merge requires:

1. Merge `brian/phase-e` → main
2. `go build ./... && go test ./...` clean
3. **Rebuild bot-hq binary** (Phase E adds middleware that runs on hub server)
4. **Restart Brian + Rain sessions** (so MCP middleware applies to their connections)
5. **Per-agent independent transitions** — trigger an MCP call from Brian's session (e.g. `hub_send`). **Brian's** dot transitions to `●`. Rain and Emma should remain at their current states (likely `◐`). Repeat from Rain's session → Rain's dot transitions to `●`, Brian's stays at whatever recency dictates. Each agent's dot updates independently of the others.
6. Wait 10s without further MCP calls from a given agent. That agent's dot fades to `◐` online.
7. Wait 60s without calls. Agent's dot drops from strip (stale, hidden).
8. Re-trigger MCP call from the dropped agent → its dot reappears at `●`.

If any step fails, smoke regression. Don't ship without verifying steps 5–8 manually.

### Vocabulary flip (read before smoke)

Phase E commit 4 changes the Agents tab Status column from raw `protocol.AgentStatus` (`online` / `working` / `offline`) to derived `panestate.AgentActivity` (`working` / `online` / `stale` / `offline`). After rebuild, expect:

- The Status column shows the **activity vocab**, not the legacy status enum.
- Currently-online-but-quiet agents (last_seen older than 60s) display as `stale`.
- The new `stale` label is **expected behavior**, not a regression — it indicates the strip's first-order check correctly identifies an inactive agent that should be escalated to second-order tmux inspection.
- The Agents tab summary changed from `[N online, M offline]` to `[N alive, M offline]` where `alive = working + online`. Stale agents fall into the `M offline` bucket today (cosmetic; precision is a Phase F follow-up).

## 8. Bug List — Separate Post-Phase-E Fix Bundles

These surfaced during the experiment phase but are **out of scope for Phase E**. Logged for later sequencing:

1. **Bridge follow-up bundle** (locked at msg 2147): `bot.go:170` filter widening + DISC v2.x audience-driven discipline + 2 ratchet tests.
2. **`hub_spawn` prompt delivery gap**: spawn pipeline registers coder + boots Claude session but doesn't deliver task prompt to pane. Empty `❯` after spawn.
3. **`claude_message` false-busy on fresh boot**: returns "Claude is busy — not at prompt" when pane visibly shows empty `❯`.
4. **`claude_stop` no-offline-flip**: killing a coder via `claude_stop` does not call `UpdateAgentStatus(offline)`. Source of all 6+ stale-online coder rows in DB. My experiment coder `6093e608` became the seventh.

Recommend tackling #4 first in the next bundle since it actively accrues noise on every spawn.

## 9. Open Questions for Reviewer (Rain)

1. **Middleware vs per-handler one-liner.** Spec proposes wrapper. Per-handler `db.UpdateAgentLastSeen(from)` calls would also work (more verbose, easier to debug). Lean wrapper. Concur?
2. **5s/60s threshold values.** Picked from peer-coord typing pace observation. Tunable later. Concur?
3. **8-agent strip cap.** Picked from your prior suggestion. Concur or want a different cap?
4. **`panestate.Manager` polling cadence.** Reuses App's 1s tick. Acceptable, or should Manager own its own poller?

## 10. Acceptance Criteria

Phase E is done when:
- All 5 commits merged on main
- `go build ./... && go test ./...` clean
- Rebuild + restart smoke ladder (§7) passes manually
- Strip visible on HubTab, dots transition correctly during real coordination
- Agents tab Status column reflects Activity, not raw `ag.Status`
- `panestate.Manager` is the only consumer of `db.ListAgents` for tab rendering

## 11. Post-Merge Addendum

Corrections + observations accumulated during smoke-ladder execution and the
bug-fix bundles (#4, #2/#3, #1) that followed Phase E. Folded back here as a
single addendum rather than amending §1–§10 in place — Phase E shipped as
designed, these are notes for future readers about what we learned after.

### 11.1 Threshold wording correction (§7 step 6)

§7 step 6 reads "wait 10s" but the working-window threshold per §2.54 is 5s.
The 10s figure was a margin past the edge, not a threshold. Read "wait past
the 5s threshold (≥10s gives clean margin for observation)."

### 11.2 Coord vs service silence under live testing

Smoke ladder §7 step 7 requires an agent to cross the 60s stale boundary by
holding silent. Service agents (Discord, Emma) cross naturally because they
don't poll on idle. Coordinating agents (Brian, Rain) bump `last_seen` on
every ack and routine `hub_read` poll, so they cross the boundary only under
a deliberate-hold posture (no acks, no polls) sustained past 60s. Both paths
are valid evidence; service-agent path is lower-friction during live testing.

Empirically validated: Brian crossed 60.7s naturally during smoke ladder
execution after deliberately suppressing acks and polls.

### 11.3 Phase E status post-merge

Phase E delivered the foundation (`panestate.Manager`, strip render, Activity
vocab, MCP-middleware first-order signal, threshold model). The user-visible
value — knowing if an agent is *currently working in its pane* without
checking tmux — is delivered by Phase F (deferred tmux-pane introspection).
Phase E in isolation is largely redundant for the "is the agent working"
question (the strip lights only on hub-traffic, equivalent to reading the
message log). Phase F closes the gap by adding pane-content introspection
into the same Manager / strip / vocab.

### 11.4 Observation discipline for live signal tests

When one agent owns a step's signal during a smoke ladder or live test, peer
agents must silence (no PM-ack, no broadcast) during the observation window.
Otherwise peer acks pollute the scan with their own `last_seen` bumps and the
observation captures a mixed state instead of the intended isolated event.

Mitigation when silence isn't viable: report all bumps observed and treat
scans as samples-of-time, not freeze-frames. The math reconstructs intent
across multiple samples even when timing is noisy.

For user-triggered tests specifically, one agent claims via brief "I'll fire"
PM before sending; the other silences explicitly. Default claimer = the
agent the user addressed, or HANDS if ambiguous. We violated this discipline
inside 5 minutes of writing it down (msg 2305-2307 cycle); explicit claim
protocol prevents the reflexive parallel-fire failure mode.
