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
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// Threshold windows for activity classification. Tunable from observation.
const (
	WorkingWindow    = 5 * time.Second
	OnlineWindow     = 60 * time.Second
	StaleAgentWindow = 7 * 24 * time.Hour
)

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
type Manager struct {
	src      AgentSource
	mu       sync.RWMutex
	snapshot []AgentSnapshot
}

// NewManager constructs a Manager bound to the given source. Snapshot is empty
// until Refresh runs.
func NewManager(src AgentSource) *Manager {
	return &Manager{src: src}
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
		snap[i] = AgentSnapshot{
			ID:       a.ID,
			Name:     a.Name,
			Type:     a.Type,
			Status:   a.Status,
			LastSeen: a.LastSeen,
			Activity: ComputeActivity(a.Status, a.LastSeen, now),
		}
	}
	m.mu.Lock()
	m.snapshot = snap
	m.mu.Unlock()
	return nil
}

// Snapshot returns a copy of the current state. Safe for concurrent reads.
func (m *Manager) Snapshot() []AgentSnapshot {
	m.mu.RLock()
	defer m.mu.RUnlock()
	out := make([]AgentSnapshot, len(m.snapshot))
	copy(out, m.snapshot)
	return out
}

// ComputeActivity derives the activity tier from status + recency.
//
// Logic:
//
//	status == StatusOffline → ActivityOffline (always)
//	recency <  WorkingWindow → ActivityWorking
//	recency <  OnlineWindow  → ActivityOnline
//	otherwise               → ActivityStale
//
// Status field beyond offline is ignored — Phase E treats last_seen recency as
// the truthful signal. Existing StatusWorking values in DB are legacy hints.
func ComputeActivity(status protocol.AgentStatus, lastSeen, now time.Time) AgentActivity {
	if status == protocol.StatusOffline {
		return ActivityOffline
	}
	recency := now.Sub(lastSeen)
	switch {
	case recency < WorkingWindow:
		return ActivityWorking
	case recency < OnlineWindow:
		return ActivityOnline
	default:
		return ActivityStale
	}
}
