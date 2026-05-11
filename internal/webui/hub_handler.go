// Z-5b hub chat surface for the webui: read recent messages, post new
// ones, stream live updates via SSE. Closes the user's mental-model
// gap from the 2026-05-11 EOD retro: a main hub aggregator showing
// session-tagged + system + user↔emma traffic in one feed, with a
// filter dropdown to narrow to a specific session_id.

package webui

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// hubSubscriber is one Hub SSE client's delivery channel + identity.
// Parallel to cliveSubscriber but unfiltered (every hub message fans
// out; client-side filter chooses what to render).
type hubSubscriber struct {
	ch     chan protocol.Message
	origin string
}

// initHubSSE prepares the per-origin subscriber map. Idempotent.
func (s *Server) initHubSSE() {
	s.hubSseMu.Lock()
	defer s.hubSseMu.Unlock()
	if s.hubSseSubsByOrigin == nil {
		s.hubSseSubsByOrigin = make(map[string][]*hubSubscriber)
	}
}

// wireHubBroadcast registers an OnMessage hook on the hub DB that
// fans every new message out to live Hub SSE subscribers. Unlike
// wireCliveBroadcast (clive-only), this hook does not pre-filter —
// the frontend chooses what to render based on the user's filter
// dropdown.
func (s *Server) wireHubBroadcast() {
	if s.db == nil {
		return
	}
	s.db.OnMessage(func(m protocol.Message) {
		s.hubSseMu.Lock()
		defer s.hubSseMu.Unlock()
		for _, byOrigin := range s.hubSseSubsByOrigin {
			for _, sub := range byOrigin {
				select {
				case sub.ch <- m:
				default:
					// slow consumer; drop rather than block broadcaster
				}
			}
		}
	})
}

func (s *Server) registerHubSubscriber(origin string) *hubSubscriber {
	sub := &hubSubscriber{
		ch:     make(chan protocol.Message, sseSubscriberBuffer),
		origin: origin,
	}
	s.hubSseMu.Lock()
	defer s.hubSseMu.Unlock()
	bucket := s.hubSseSubsByOrigin[origin]
	for len(bucket) >= maxConnPerOrigin {
		oldest := bucket[0]
		bucket = bucket[1:]
		close(oldest.ch)
	}
	bucket = append(bucket, sub)
	s.hubSseSubsByOrigin[origin] = bucket
	return sub
}

func (s *Server) unregisterHubSubscriber(sub *hubSubscriber) {
	s.hubSseMu.Lock()
	defer s.hubSseMu.Unlock()
	bucket := s.hubSseSubsByOrigin[sub.origin]
	for i, e := range bucket {
		if e == sub {
			s.hubSseSubsByOrigin[sub.origin] = append(bucket[:i], bucket[i+1:]...)
			return
		}
	}
}

// handleHubMessages serves /api/hub/messages.
//
// GET — read the recent timeline.
//
//	?since_id=N  return only messages with id > N (incremental fetch)
//	?session_id=X filter to one session (empty/omitted = no filter)
//	?limit=N     cap result (default 100, max 500)
//
// POST — post a user message into the hub.
//
//	body: {"content": "...", "session_id": "...", "to_agent": "..."?}
//	emits InsertMessage as from_agent="user", type="command"
//	returns {"id": <int64>}
func (s *Server) handleHubMessages(w http.ResponseWriter, r *http.Request) {
	if s.db == nil {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{"error": "hub.DB not configured"})
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.handleHubMessagesGet(w, r)
	case http.MethodPost:
		s.handleHubMessagesPost(w, r)
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleHubMessagesGet(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	sinceID, _ := strconv.ParseInt(q.Get("since_id"), 10, 64)
	limit, _ := strconv.Atoi(q.Get("limit"))
	if limit <= 0 {
		limit = 100
	}
	if limit > 500 {
		limit = 500
	}
	sessionID := q.Get("session_id")
	hasSessionFilter := q.Has("session_id")

	// ReadMessages with agentID="" returns everything since sinceID
	// up to limit (out-of-session caller semantics per Z-5d). We then
	// session-filter in Go.
	msgs, err := s.db.ReadMessages("", sinceID, limit)
	if err != nil {
		http.Error(w, fmt.Sprintf("read hub messages: %v", err), http.StatusInternalServerError)
		return
	}
	out := make([]map[string]any, 0, len(msgs))
	for _, m := range msgs {
		if hasSessionFilter && m.SessionID != sessionID {
			continue
		}
		out = append(out, hubMessageView(m))
	}
	writeJSON(w, http.StatusOK, map[string]any{"messages": out})
}

func (s *Server) handleHubMessagesPost(w http.ResponseWriter, r *http.Request) {
	var body struct {
		Content   string `json:"content"`
		SessionID string `json:"session_id"`
		ToAgent   string `json:"to_agent"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "invalid JSON body", http.StatusBadRequest)
		return
	}
	if body.Content == "" {
		http.Error(w, "content required", http.StatusBadRequest)
		return
	}
	id, err := s.db.InsertMessage(protocol.Message{
		FromAgent: "user",
		ToAgent:   body.ToAgent,
		Type:      "command",
		Content:   body.Content,
		SessionID: body.SessionID,
	})
	if err != nil {
		http.Error(w, fmt.Sprintf("insert: %v", err), http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"id": id})
}

// handleHubStream serves GET /api/hub/stream as SSE. Initial snapshot
// is the last 100 messages, optionally filtered by ?session_id=. Live
// events flow from the wireHubBroadcast hook until disconnect.
func (s *Server) handleHubStream(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if s.db == nil {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{"error": "hub.DB not configured"})
		return
	}
	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming unsupported", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.Header().Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)

	origin := remoteOrigin(r)
	sub := s.registerHubSubscriber(origin)
	defer s.unregisterHubSubscriber(sub)

	// Initial snapshot — last 100 messages, then live stream.
	if msgs, err := s.db.ReadMessages("", 0, 100); err == nil {
		for _, m := range msgs {
			if !writeHubSSEEvent(w, flusher, m) {
				return
			}
		}
	}

	heartbeat := time.NewTicker(sseHeartbeatInterval)
	defer heartbeat.Stop()
	ctx := r.Context()
	for {
		select {
		case <-ctx.Done():
			return
		case m, ok := <-sub.ch:
			if !ok {
				return
			}
			if !writeHubSSEEvent(w, flusher, m) {
				return
			}
		case <-heartbeat.C:
			if _, err := fmt.Fprint(w, ": heartbeat\n\n"); err != nil {
				return
			}
			flusher.Flush()
		}
	}
}

func writeHubSSEEvent(w http.ResponseWriter, flusher http.Flusher, m protocol.Message) bool {
	data, err := json.Marshal(hubMessageView(m))
	if err != nil {
		return false
	}
	if _, err := fmt.Fprintf(w, "event: hub\ndata: %s\n\n", data); err != nil {
		return false
	}
	flusher.Flush()
	return true
}

// hubMessageView is the canonical wire shape for a hub message in
// the Hub timeline + SSE stream. Keep schema parity with the snapshot
// branch so the frontend can use one rendering function.
func hubMessageView(m protocol.Message) map[string]any {
	return map[string]any{
		"id":         m.ID,
		"from_agent": m.FromAgent,
		"to_agent":   m.ToAgent,
		"type":       string(m.Type),
		"content":    m.Content,
		"created":    m.Created.UTC().Format("2006-01-02T15:04:05Z"),
		"session_id": m.SessionID,
	}
}
