// Package panestate centralizes per-agent activity state for TUI consumption.
//
// Phase E uses last_seen recency (refreshed by MCP middleware in Phase E commit 2)
// as the truthful activity signal. Status field is consulted only for offline.
//
// Phase F prerequisite: heartbeat goroutine (when added) calls
// db.UpdateAgentLastSeen on a timer for agents that don't initiate MCP calls
// (e.g. dormant coders). Capture-pane classifier populates LastClassification
// and RecentErrors. See docs/plans/phase-e.md §6.
package panestate

import (
	"encoding/json"
	"hash/fnv"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// Threshold windows for activity classification. Tunable from observation.
//
// Heartbeat- and pane-tier thresholds are independent per F-core Push 4 lock:
// pane-content delta and heartbeat-cadence are orthogonal signals; sharing a
// single threshold would couple their retune paths. Initial pane values match
// heartbeat values for Phase-E-equivalence at F-core-a (no source wired) but
// are tunable independently once F-core-b activates pane sourcing.
const (
	HeartbeatWorkingWindow = 5 * time.Second
	HeartbeatOnlineWindow  = 60 * time.Second
	PaneWorkingWindow      = 5 * time.Second
	PaneOnlineWindow       = 60 * time.Second
	StaleAgentWindow       = 7 * 24 * time.Hour
)

// paneCaptureLines bounds the capture-pane window for activity hashing.
// Captures the last N lines of scrollback (`tmux capture-pane -p -S -<N>`),
// NOT the current visible viewport. Viewport-based hashing produces an
// always-active false signal as content scrolls past — scrollback-tail
// changes only when new output is emitted, which is the truthful signal.
// 50 lines covers Claude Code multi-line tool-result blocks (typically
// 15–30 lines) with margin; FNV-64 hashing of ~4KB is sub-millisecond.
const paneCaptureLines = 50

// paneState is the per-agent cache entry used by Manager.Refresh to detect
// pane-content deltas across UI ticks. lastHash is the FNV-64 of the most
// recent capture; lastActivity is the timestamp at which the hash last
// changed (zero before the first observed change).
type paneState struct {
	lastHash     uint64
	lastActivity time.Time
}

// AgentActivity is the derived activity tier for first-order user check.
type AgentActivity int

const (
	ActivityWorking AgentActivity = iota
	ActivityOnline
	ActivityStale
	ActivityOffline
)

func (a AgentActivity) String() string {
	switch a {
	case ActivityWorking:
		return "working"
	case ActivityOnline:
		return "online"
	case ActivityStale:
		return "stale"
	case ActivityOffline:
		return "offline"
	}
	return "unknown"
}

// ClassifierHit is a Phase F forward-dependency type.
//
// Phase F prerequisite: capture-pane classifier (markers.toml regex) populates these.
// Phase F's stall-detector consumes them. Do not remove without updating Phase F's spec.
type ClassifierHit struct {
	Category  string
	Excerpt   string
	Timestamp time.Time
}

// AgentSnapshot is the per-agent state surface tabs read from.
type AgentSnapshot struct {
	ID       string
	Name     string
	Type     protocol.AgentType
	Status   protocol.AgentStatus
	LastSeen time.Time
	Activity AgentActivity

	// LastPaneActivity is the most recent timestamp at which a pane-content
	// delta was observed for this agent. F-core-a scaffolds the field through
	// ComputeActivity but leaves it zero (no source wired); F-core-b will
	// populate it from the capture-pane source, activating the OR-combination
	// against LastSeen at runtime.
	LastPaneActivity time.Time

	// Phase F forward dependencies. Populated in Phase F by capture-pane classifier.
	LastClassification string
	RecentErrors       []ClassifierHit
}

// AgentSource is the dependency for Manager.Refresh — anything that lists agents.
// Tests pass a fake; production passes *hub.DB.
type AgentSource interface {
	ListAgents(statusFilter string) ([]protocol.Agent, error)
}

// Manager owns the shared snapshot. Tabs read via Snapshot(), App refreshes via Refresh().
//
// paneCache is mutated only inside Refresh, which is called from a single UI
// ticker (see internal/ui/app.go); no separate lock is needed for the cache.
// snapshot/raw are guarded by mu for concurrent reads from tabs.
type Manager struct {
	src         AgentSource
	capturePane func(target string, lines int) (string, error)
	paneCache   map[string]paneState
	mu          sync.RWMutex
	snapshot    []AgentSnapshot
	raw         []protocol.Agent
}

// NewManager constructs a Manager bound to the given source and pane-capture
// function. Snapshot is empty until Refresh runs.
//
// capturePane is required: production passes tmux.CapturePane; tests pass a
// fake. Permitting nil would create a silent fallback to heartbeat-only mode
// indistinguishable from a wiring bug — panic at construction is fail-fast.
func NewManager(src AgentSource, capturePane func(string, int) (string, error)) *Manager {
	if capturePane == nil {
		panic("panestate.NewManager: capturePane is required (use tmux.CapturePane in production, fake in tests)")
	}
	return &Manager{
		src:         src,
		capturePane: capturePane,
		paneCache:   make(map[string]paneState),
	}
}

// Refresh queries the source and recomputes activity for each agent.
// Call once per UI tick.
func (m *Manager) Refresh() error {
	agents, err := m.src.ListAgents("")
	if err != nil {
		return err
	}
	now := time.Now()
	snap := make([]AgentSnapshot, len(agents))
	for i, a := range agents {
		paneActive := m.observePaneActivity(a, now)
		snap[i] = AgentSnapshot{
			ID:               a.ID,
			Name:             a.Name,
			Type:             a.Type,
			Status:           a.Status,
			LastSeen:         a.LastSeen,
			LastPaneActivity: paneActive,
			Activity:         ComputeActivity(a.Status, a.LastSeen, paneActive, now),
		}
	}
	m.mu.Lock()
	m.snapshot = snap
	m.raw = agents
	m.mu.Unlock()
	return nil
}

// observePaneActivity returns the LastPaneActivity timestamp for an agent by
// hashing its tmux pane scrollback-tail and comparing to the cached prior
// hash. Per F-core-b spec §2:
//
//   - No tmux_target → time.Time{} (heartbeat-only path).
//   - capturePane error → carry forward cached lastActivity (transient tmux
//     glitch should not demote an active row; if tmux is permanently gone,
//     the carry-forward will eventually exceed PaneOnlineWindow naturally).
//   - First tick (no cache entry) → seed cache, return time.Time{} (no prior
//     frame to compare; promoting on first sight inflates working-tier on
//     startup).
//   - Hash differs → paneActive = now, cache updated.
//   - Hash matches → carry forward cached lastActivity.
func (m *Manager) observePaneActivity(a protocol.Agent, now time.Time) time.Time {
	tmuxTarget := extractTmuxTarget(a)
	if tmuxTarget == "" {
		return time.Time{}
	}
	cached, hadCache := m.paneCache[a.ID]
	output, err := m.capturePane(tmuxTarget, paneCaptureLines)
	if err != nil {
		return cached.lastActivity
	}
	h := fnv.New64a()
	h.Write([]byte(output))
	hash := h.Sum64()
	if !hadCache {
		m.paneCache[a.ID] = paneState{lastHash: hash, lastActivity: time.Time{}}
		return time.Time{}
	}
	if hash != cached.lastHash {
		m.paneCache[a.ID] = paneState{lastHash: hash, lastActivity: now}
		return now
	}
	return cached.lastActivity
}

// extractTmuxTarget reads the tmux_target field from an agent's Meta JSON.
// Mirrors the parse pattern in internal/hub/hub.go:257-262 — kept inline to
// avoid coupling panestate to hub package internals (anti-precedent §3).
func extractTmuxTarget(a protocol.Agent) string {
	if a.Meta == "" {
		return ""
	}
	var meta struct {
		TmuxTarget string `json:"tmux_target"`
	}
	if err := json.Unmarshal([]byte(a.Meta), &meta); err != nil {
		return ""
	}
	return meta.TmuxTarget
}

// Snapshot returns a copy of the activity-derived state. Safe for concurrent reads.
func (m *Manager) Snapshot() []AgentSnapshot {
	m.mu.RLock()
	defer m.mu.RUnlock()
	out := make([]AgentSnapshot, len(m.snapshot))
	copy(out, m.snapshot)
	return out
}

// Agents returns a copy of the raw agent list as last seen by Refresh.
// Used by tab renderers that consume protocol.Agent directly during the
// Phase E migration. Phase F may collapse this once tabs read solely from
// AgentSnapshot.
func (m *Manager) Agents() []protocol.Agent {
	m.mu.RLock()
	defer m.mu.RUnlock()
	out := make([]protocol.Agent, len(m.raw))
	copy(out, m.raw)
	return out
}

// ComputeActivity derives the activity tier from status + heartbeat recency
// + optional pane-activity recency.
//
// Logic:
//
//	status == StatusOffline → ActivityOffline (always)
//	heartbeatRecency < HeartbeatWorkingWindow ||
//	  (paneActive set && paneRecency < PaneWorkingWindow) → ActivityWorking
//	heartbeatRecency < HeartbeatOnlineWindow ||
//	  (paneActive set && paneRecency < PaneOnlineWindow) → ActivityOnline
//	otherwise → ActivityStale
//
// Independent-threshold OR-combination per F-core Push 4 lock: pane-content
// delta and heartbeat-cadence are orthogonal signals; either being recent
// counts the agent as working. paneActive zero (no source wired) skips the
// pane-tier comparisons → behavior reduces to heartbeat-only logic identical
// to Phase E. F-core-a (case-i ratchet) preserves Phase-E behavior bit-for-
// bit at runtime; F-core-b activates the source.
//
// Status field beyond offline is ignored — Phase E treats last_seen recency as
// the truthful signal. Existing StatusWorking values in DB are legacy hints.
func ComputeActivity(status protocol.AgentStatus, lastSeen, paneActive, now time.Time) AgentActivity {
	if status == protocol.StatusOffline {
		return ActivityOffline
	}
	heartbeatRecency := now.Sub(lastSeen)
	panePresent := !paneActive.IsZero()
	var paneRecency time.Duration
	if panePresent {
		paneRecency = now.Sub(paneActive)
	}
	switch {
	case heartbeatRecency < HeartbeatWorkingWindow ||
		(panePresent && paneRecency < PaneWorkingWindow):
		return ActivityWorking
	case heartbeatRecency < HeartbeatOnlineWindow ||
		(panePresent && paneRecency < PaneOnlineWindow):
		return ActivityOnline
	default:
		return ActivityStale
	}
}
