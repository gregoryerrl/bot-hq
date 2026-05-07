package webui

// SSE live web feed of Clive tool calls (3-layer-2 visibility) per
// Phase P P-1 / phase-n.md:543 / ratchet-ledger §14 v3c-sub-deferral.
//
// Wire model: hub.DB.OnMessage callback fans every new clive-authored
// hub message out to subscriber channels. Each SSE client gets one
// channel; handler streams initial snapshot then events until client
// disconnect or context cancel. Heartbeat every heartbeatInterval keeps
// proxies/clients from idling out the connection.
//
// Per OQ-P-2 LOCKED (msg 14894 Rain CONCUR): cap maxConnPerOrigin
// active SSE connections per remote-origin; on overflow, drop-oldest.

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// maxConnPerOrigin caps active SSE connections per remote-origin per
	// OQ-P-2 LOCKED. Overflow → drop-oldest to bound memory + goroutine
	// growth from runaway tabs/clients.
	maxConnPerOrigin = 5

	// sseHeartbeatInterval is the cadence of SSE comment-line keep-alives
	// emitted to prevent idle-timeout closures by intermediate proxies
	// or browser EventSource implementations.
	sseHeartbeatInterval = 15 * time.Second

	// sseSubscriberBuffer is the per-subscriber channel buffer; if a
	// slow client falls behind by this many messages, further messages
	// are dropped (non-blocking send) so the broadcaster doesn't stall.
	sseSubscriberBuffer = 16
)

// cliveSubscriber is one SSE client's delivery channel + identity.
type cliveSubscriber struct {
	ch     chan protocol.Message
	origin string
}

// initSSE initializes the Server's SSE subscriber state. Must be
// called once at construction. Idempotent (safe to call from tests).
func (s *Server) initSSE() {
	s.sseMu.Lock()
	defer s.sseMu.Unlock()
	if s.sseSubsByOrigin == nil {
		s.sseSubsByOrigin = make(map[string][]*cliveSubscriber)
	}
}

// wireCliveBroadcast registers an OnMessage hook on the hub.DB so each
// new clive-authored message fans out to all current SSE subscribers.
// Called once from NewServer when db != nil.
func (s *Server) wireCliveBroadcast() {
	if s.db == nil {
		return
	}
	s.db.OnMessage(func(m protocol.Message) {
		if m.FromAgent != "clive" {
			return
		}
		// Hold sseMu across the entire iteration to prevent a register-
		// concurrent-with-broadcast race: drop-oldest eviction calls
		// close(oldest.ch), and a select-with-default send to a closed
		// channel still panics. Serializing register/unregister against
		// broadcast eliminates the race; sends are non-blocking
		// (select+default), so total hold time is bounded by the
		// subscriber count (≤ maxConnPerOrigin × distinct origins).
		s.sseMu.Lock()
		defer s.sseMu.Unlock()
		for _, byOrigin := range s.sseSubsByOrigin {
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

// registerSSESubscriber adds a new subscriber for the given origin and
// enforces the per-origin connection cap (maxConnPerOrigin). When the
// cap would be exceeded, the oldest subscriber's channel is closed
// (signalling its handler to disconnect) and removed before the new
// one is added.
func (s *Server) registerSSESubscriber(origin string) *cliveSubscriber {
	sub := &cliveSubscriber{
		ch:     make(chan protocol.Message, sseSubscriberBuffer),
		origin: origin,
	}
	s.sseMu.Lock()
	defer s.sseMu.Unlock()
	bucket := s.sseSubsByOrigin[origin]
	for len(bucket) >= maxConnPerOrigin {
		oldest := bucket[0]
		bucket = bucket[1:]
		close(oldest.ch)
	}
	bucket = append(bucket, sub)
	s.sseSubsByOrigin[origin] = bucket
	return sub
}

// unregisterSSESubscriber removes a subscriber on disconnect. Safe to
// call even if the subscriber was already evicted by drop-oldest (its
// channel will be closed; this is a no-op in that case).
func (s *Server) unregisterSSESubscriber(sub *cliveSubscriber) {
	s.sseMu.Lock()
	defer s.sseMu.Unlock()
	bucket := s.sseSubsByOrigin[sub.origin]
	for i, e := range bucket {
		if e == sub {
			bucket = append(bucket[:i], bucket[i+1:]...)
			break
		}
	}
	if len(bucket) == 0 {
		delete(s.sseSubsByOrigin, sub.origin)
	} else {
		s.sseSubsByOrigin[sub.origin] = bucket
	}
}

// handleCliveActivitySSE serves the SSE branch of /api/clive/activity
// when the request carries Accept: text/event-stream. Initial snapshot
// (last 50 clive messages) is emitted first, then live messages from
// the OnMessage broadcaster, with sseHeartbeatInterval keep-alive
// comments to prevent idle timeouts.
func (s *Server) handleCliveActivitySSE(w http.ResponseWriter, r *http.Request) {
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
	sub := s.registerSSESubscriber(origin)
	defer s.unregisterSSESubscriber(sub)

	// Initial snapshot: last N clive messages, in chronological order.
	// ReadMessages("clive", ...) returns messages addressed TO clive or
	// broadcast; we filter FromAgent=="clive" to keep behavior identical
	// to the existing JSON-snapshot branch in handlers.go (snapshot/SSE
	// parity is contractual — same wire shape on initial fetch).
	if msgs, err := s.db.ReadMessages("clive", 0, 50); err == nil {
		for _, m := range msgs {
			if m.FromAgent != "clive" {
				continue
			}
			if !writeSSEEvent(w, flusher, m) {
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
				// channel closed by drop-oldest eviction
				return
			}
			if !writeSSEEvent(w, flusher, m) {
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

// writeSSEEvent emits one SSE event for a clive message. Returns false
// if the write fails (client gone), prompting the caller to return.
func writeSSEEvent(w http.ResponseWriter, flusher http.Flusher, m protocol.Message) bool {
	payload := map[string]any{
		"id":         m.ID,
		"to_agent":   m.ToAgent,
		"type":       string(m.Type),
		"content":    m.Content,
		"created":    m.Created.UTC().Format("2006-01-02T15:04:05Z"),
		"session_id": m.SessionID,
	}
	data, err := json.Marshal(payload)
	if err != nil {
		return false
	}
	if _, err := fmt.Fprintf(w, "event: clive\ndata: %s\n\n", data); err != nil {
		return false
	}
	flusher.Flush()
	return true
}

// remoteOrigin extracts a stable origin key for connection-cap
// accounting. Strips port from RemoteAddr so connections from the
// same client (different ephemeral ports) share a cap bucket.
func remoteOrigin(r *http.Request) string {
	addr := r.RemoteAddr
	if i := strings.LastIndex(addr, ":"); i > 0 {
		addr = addr[:i]
	}
	return addr
}

