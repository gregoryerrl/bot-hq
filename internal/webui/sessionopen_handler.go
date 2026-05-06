package webui

import (
	"fmt"
	"net/http"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/agents/sessionopen"
)

// handleSessionOpen responds to GET /api/session-open?project=X&agent=Y with
// the session-open payload (overview + bootstrap + rules_resolved + tasks +
// stats). Per Phase N v3.x-2 design-spike §2.2.
//
// Query params:
//
//	project  optional; defaults to "bot-hq"
//	agent    optional; if set, agent layer merges into rules_resolved.agent
//
// The endpoint never errors on missing files (overview/bootstrap/tasks all
// return empty); it errors only on yaml-parse failures or filesystem
// permission denials. Token-bound flags surface in payload.stats.
func (s *Server) handleSessionOpen(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	project := strings.TrimSpace(r.URL.Query().Get("project"))
	if project == "" {
		project = "bot-hq"
	}
	agent := strings.TrimSpace(r.URL.Query().Get("agent"))

	payload, err := sessionopen.Build(s.canonicalRoot, project, agent)
	if err != nil {
		http.Error(w, fmt.Sprintf("session-open build: %v", err), http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, payload)
}
