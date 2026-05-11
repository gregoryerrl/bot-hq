package webui

// Agents endpoint (Phase-R-followup-2 (d)). Surfaces hub.DB
// ListAgents results — including agents.current_task per Phase-R-
// followup commit aeecf80 — as a queryable HTTP/JSON resource.
//
// Closes the data-axis half of the carry-forward "agents.current_task
// webui-surface (DB-only currently)" residue: current_task transitions
// from DB-only to webui-queryable. The UI-render axis (frontend tile)
// is deferred to next-phase deliberate-scope when a UX driver emerges
// (e.g., emma-stale dashboard, agent-coordination view).

import (
	"fmt"
	"net/http"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// handleAgents serves GET /api/agents → list of registered agents
// with full struct fields including current_task.
//
// Optional `?session_id=<id>` (Z-5c) narrows to agents that have
// posted in that session within the last hour. session_id is "" (or
// omitted) returns the global agents-table snapshot. Per-session view
// derives from messages.from_agent rather than agents.session_id
// (last-write-wins per registration, lies under concurrent sessions).
func (s *Server) handleAgents(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if s.db == nil {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{"error": "hub.DB not configured"})
		return
	}
	var (
		agents []protocol.Agent
		err    error
	)
	if sid, ok := r.URL.Query()["session_id"]; ok {
		agents, err = s.db.AgentsActiveInSession(sid[0], time.Hour)
	} else {
		agents, err = s.db.ListAgents("")
	}
	if err != nil {
		http.Error(w, fmt.Sprintf("list agents: %v", err), http.StatusInternalServerError)
		return
	}
	if agents == nil {
		agents = []protocol.Agent{}
	}
	writeJSON(w, http.StatusOK, map[string]any{"agents": agents})
}
