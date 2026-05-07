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

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// handleAgents serves GET /api/agents → list of registered agents
// with full struct fields including current_task. No filters in v1;
// frontend / API consumers can filter client-side over the small N.
func (s *Server) handleAgents(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if s.db == nil {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{"error": "hub.DB not configured"})
		return
	}
	agents, err := s.db.ListAgents("")
	if err != nil {
		http.Error(w, fmt.Sprintf("list agents: %v", err), http.StatusInternalServerError)
		return
	}
	if agents == nil {
		agents = []protocol.Agent{}
	}
	writeJSON(w, http.StatusOK, map[string]any{"agents": agents})
}
