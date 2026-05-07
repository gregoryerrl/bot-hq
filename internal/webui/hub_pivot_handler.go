package webui

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// handleHubPivot implements POST /api/hub-pivot — Phase O drain item #4
// per design-spike 157ea7f §2.4. Emits a hub_send broadcast announcing
// an agent's project context-switch so peers see the pivot in real time.
//
// Request body (JSON):
//
//	{
//	  "agent":         "brian",            // required: the pivoting agent
//	  "project":       "bcc-ad-manager",   // required: target project
//	  "prev_project":  "bot-hq"            // optional: previous project
//	}
//
// Response (JSON, 200):
//
//	{ "ok": true, "msg_id": <int64> }
//
// 400 on missing/invalid fields. 405 on non-POST. 503 if hub DB unwired.
//
// Best-effort: callers (CLI runContextSwitch) should NOT fail if this
// returns non-200 — the local pivot has already happened; the hub
// notification is informational. Daemon down → CLI degrades silently.
func (s *Server) handleHubPivot(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if s.db == nil {
		http.Error(w, "hub db unwired", http.StatusServiceUnavailable)
		return
	}

	var body struct {
		Agent       string `json:"agent"`
		Project     string `json:"project"`
		PrevProject string `json:"prev_project"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "invalid JSON: "+err.Error(), http.StatusBadRequest)
		return
	}
	body.Agent = strings.TrimSpace(body.Agent)
	body.Project = strings.TrimSpace(body.Project)
	body.PrevProject = strings.TrimSpace(body.PrevProject)
	if body.Agent == "" {
		http.Error(w, "agent required", http.StatusBadRequest)
		return
	}
	if body.Project == "" {
		http.Error(w, "project required", http.StatusBadRequest)
		return
	}

	content := formatPivotContent(body.Agent, body.Project, body.PrevProject)
	msg := protocol.Message{
		FromAgent: body.Agent,
		ToAgent:   "",
		Type:      protocol.MsgUpdate,
		Content:   content,
		Created:   time.Now(),
	}
	id, err := s.db.InsertMessage(msg)
	if err != nil {
		http.Error(w, "insert message: "+err.Error(), http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"ok":     true,
		"msg_id": id,
	})
}

// formatPivotContent builds the hub message body for a context-switch
// announcement. Stable format so peers can grep / filter on the
// `[CONTEXT-SWITCH]` prefix.
func formatPivotContent(agent, project, prevProject string) string {
	if prevProject != "" && prevProject != project {
		return fmt.Sprintf("[CONTEXT-SWITCH] agent %s pivoted: %s -> %s", agent, prevProject, project)
	}
	return fmt.Sprintf("[CONTEXT-SWITCH] agent %s pivoted to project %s", agent, project)
}
