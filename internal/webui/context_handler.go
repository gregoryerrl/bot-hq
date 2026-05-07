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
// to time.Now(). If project/path/viewMode actually changed (vs prior
// state), fan out non-blocking to all subscribers so live voice
// sessions can re-inject the focus context into Gemini.
func (s *Server) SetWebuiContext(ctx WebuiContext) {
	s.ctxMu.Lock()
	prev := s.webuiCtx
	ctx.UpdatedAt = time.Now()
	s.webuiCtx = ctx
	s.ctxMu.Unlock()
	if prev.Project == ctx.Project && prev.CurrentPath == ctx.CurrentPath && prev.ViewMode == ctx.ViewMode {
		return
	}
	s.ctxSubsMu.Lock()
	subs := make([]chan WebuiContext, 0, len(s.ctxSubs))
	for ch := range s.ctxSubs {
		subs = append(subs, ch)
	}
	s.ctxSubsMu.Unlock()
	for _, ch := range subs {
		select {
		case ch <- ctx:
		default:
			// drop on overflow rather than block setter
		}
	}
}

// SubscribeWebuiContext registers a channel that receives WebuiContext
// updates whenever the focus state actually changes. Returned func
// unsubscribes + closes the channel; safe to call once on disconnect.
// Buffered to absorb rapid focus changes without blocking SetWebuiContext.
func (s *Server) SubscribeWebuiContext() (<-chan WebuiContext, func()) {
	ch := make(chan WebuiContext, 4)
	s.ctxSubsMu.Lock()
	if s.ctxSubs == nil {
		s.ctxSubs = make(map[chan WebuiContext]struct{})
	}
	s.ctxSubs[ch] = struct{}{}
	s.ctxSubsMu.Unlock()
	unsub := func() {
		s.ctxSubsMu.Lock()
		if _, ok := s.ctxSubs[ch]; ok {
			delete(s.ctxSubs, ch)
			close(ch)
		}
		s.ctxSubsMu.Unlock()
	}
	return ch, unsub
}

// formatFocusContext renders the "[USER VIEWING IN WEBUI ...]" line
// shared by buildVoiceSystemInstruction (connect-time) and the live
// voice-handler re-injection path (SendText on subsequent changes).
// Returns "" when no focus is set.
func formatFocusContext(ctx WebuiContext) string {
	if ctx.CurrentPath == "" && ctx.Project == "" {
		return ""
	}
	out := "[USER VIEWING IN WEBUI"
	if ctx.Project != "" {
		out += " · project=" + ctx.Project
	}
	if ctx.CurrentPath != "" {
		out += " · file=" + ctx.CurrentPath
	}
	if ctx.ViewMode != "" {
		out += " · view=" + ctx.ViewMode
	}
	out += "]\n\nThe file above is what the user is currently looking at. " +
		"When they say \"this file\", \"this rule\", \"what I'm looking at\", " +
		"or refer to a document without naming it, assume they mean that file. " +
		"You don't need them to repeat the path. Treat this as authoritative " +
		"focus state — do not say you can't see what they're looking at."
	return out
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
	focus := formatFocusContext(s.GetWebuiContext())
	if focus == "" {
		return defaultSystemInstruction
	}
	return defaultSystemInstruction + "\n\n" + focus
}
