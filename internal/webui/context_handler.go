package webui

import (
	"encoding/json"
	"net/http"
	"time"
)

// GetWebuiContext returns a snapshot of the current ambient focus state.
// Safe for concurrent use.
func (s *Server) GetWebuiContext() WebuiContext {
	s.ctxMu.RLock()
	defer s.ctxMu.RUnlock()
	return s.webuiCtx
}

// SetWebuiContext overwrites the ambient focus state. UpdatedAt is set
// to time.Now(). Used by tests + the POST handler.
func (s *Server) SetWebuiContext(ctx WebuiContext) {
	s.ctxMu.Lock()
	defer s.ctxMu.Unlock()
	ctx.UpdatedAt = time.Now()
	s.webuiCtx = ctx
}

// handleWebuiContext serves GET (read current ambient focus) + POST
// (overwrite from frontend on every nav). Body is the WebuiContext JSON
// object minus updatedAt (server-stamped).
func (s *Server) handleWebuiContext(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		writeJSON(w, http.StatusOK, s.GetWebuiContext())
	case http.MethodPost:
		var body WebuiContext
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, "invalid JSON: "+err.Error(), http.StatusBadRequest)
			return
		}
		s.SetWebuiContext(body)
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

// buildVoiceSystemInstruction returns the Gemini systemInstruction for
// a voice session, augmented with current ambient webui-focus context.
// If no focus state is set, returns the default unchanged.
func (s *Server) buildVoiceSystemInstruction() string {
	ctx := s.GetWebuiContext()
	if ctx.CurrentPath == "" && ctx.Project == "" {
		return defaultSystemInstruction
	}
	suffix := "\n\n[USER VIEWING IN WEBUI"
	if ctx.Project != "" {
		suffix += " · project=" + ctx.Project
	}
	if ctx.CurrentPath != "" {
		suffix += " · file=" + ctx.CurrentPath
	}
	if ctx.ViewMode != "" {
		suffix += " · view=" + ctx.ViewMode
	}
	suffix += "]\n\nWhen the user asks about \"this file\", \"this rule\", \"what I'm looking at\", " +
		"or refers to a document without naming it, assume they mean the file above. " +
		"You don't need them to repeat the path."
	return defaultSystemInstruction + suffix
}
