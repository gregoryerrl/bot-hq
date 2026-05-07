package webui

// Pending-actions queue endpoints (P-9 / phase-n.md:818). Surfaces
// the hub.pending_actions table via:
//   GET  /api/pending-actions          → list pending entries (default)
//   GET  /api/pending-actions?all=1    → include ack'd entries
//   GET  /api/pending-actions?count=1  → just the badge count
//   POST /api/pending-actions/{id}/ack → transition pending → ack
//
// Auto-create policy: hub.OnMessage callback fires on every new hub
// message. We create a pending_action when the message is [HR]-tagged
// (must-read class per AUDIENCE-CLASS-DISCRIMINATOR) AND broadcast or
// targeted at the user. Per Brian-lean fork pick (a-i SQLite + d-i
// cross-restart persisted).

import (
	"fmt"
	"net/http"
	"os"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// hrTagPrefix is the must-read marker per AUDIENCE-CLASS-DISCRIMINATOR.
// Detection is prefix-only (early-bytes check) to stay cheap on every
// hub message.
const hrTagPrefix = "[HR]"

// wireUserPendingActions registers an OnMessage hook on hub.DB so
// each new [HR]-tagged user-targeted message auto-creates a pending
// action queue entry. Called once at NewServer construction.
func (s *Server) wireUserPendingActions() {
	if s.db == nil {
		return
	}
	s.db.OnMessage(func(m protocol.Message) {
		if !shouldQueueAsPending(m) {
			return
		}
		summary := pendingSummaryFor(m)
		_, err := s.db.InsertPendingAction(m.FromAgent, kindForMessage(m), summary, m.ID)
		if err != nil {
			// Best-effort; don't fail the broadcaster on queue failure.
			fmt.Fprintf(stderrSink(), "pending-actions: insert failed: %v\n", err)
		}
	})
}

// shouldQueueAsPending applies the auto-create policy: [HR]-tagged
// OR MsgFlag-typed + broadcast-or-user-targeted + not-from-user
// (don't queue user's own messages). Tunable in one place to ease
// policy iteration. Phase-R-followup-2 (a): MsgFlag is implicitly
// elevated by type — queue regardless of [HR] prefix presence so
// FLAG-class never falls through the render-side R2 strip gap.
func shouldQueueAsPending(m protocol.Message) bool {
	if m.FromAgent == "" || m.FromAgent == "user" {
		return false
	}
	if m.ToAgent != "" && m.ToAgent != "user" {
		// PMs to other agents (e.g., brian → rain) are peer-coord, not
		// user-actionable. Skip.
		return false
	}
	if m.Type == protocol.MsgFlag {
		return true
	}
	return strings.HasPrefix(strings.TrimLeft(m.Content, " \t"), hrTagPrefix)
}

// pendingSummaryFor extracts a human-readable snippet from the
// message content for the queue summary. Strips the [HR] prefix +
// trims whitespace; truncation handled by InsertPendingAction.
func pendingSummaryFor(m protocol.Message) string {
	c := strings.TrimLeft(m.Content, " \t")
	c = strings.TrimPrefix(c, hrTagPrefix)
	c = strings.TrimSpace(c)
	// Replace newlines with " · " so the summary stays one-line in
	// frontend rendering.
	c = strings.ReplaceAll(c, "\n", " · ")
	return c
}

// kindForMessage classifies the source message for queue rendering.
// Drives optional frontend grouping/icon dispatch.
func kindForMessage(m protocol.Message) string {
	switch m.Type {
	case protocol.MsgFlag:
		return "flag"
	case protocol.MsgError:
		return "error"
	case protocol.MsgCommand:
		return "command"
	case protocol.MsgResult:
		return "result"
	default:
		return "hr-broadcast"
	}
}

// handlePendingActions serves the GET endpoint.
func (s *Server) handlePendingActions(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if s.db == nil {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{"error": "hub.DB not configured"})
		return
	}
	q := r.URL.Query()
	if q.Get("count") == "1" {
		n, err := s.db.CountPendingActions()
		if err != nil {
			http.Error(w, fmt.Sprintf("count: %v", err), http.StatusInternalServerError)
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"count": n})
		return
	}
	includeAcked := q.Get("all") == "1"
	limit := 50
	if raw := q.Get("limit"); raw != "" {
		var n int
		if _, err := fmt.Sscanf(raw, "%d", &n); err == nil && n > 0 {
			limit = n
		}
	}
	actions, err := s.db.ListPendingActions(limit, includeAcked)
	if err != nil {
		http.Error(w, fmt.Sprintf("list: %v", err), http.StatusInternalServerError)
		return
	}
	if actions == nil {
		actions = []hub.PendingAction{}
	}
	writeJSON(w, http.StatusOK, map[string]any{"actions": actions})
}

// handlePendingActionAck serves POST /api/pending-actions/{id}/ack.
// Idempotent: ack'ing an already-ack'd entry returns 200 + ack=false.
func (s *Server) handlePendingActionAck(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if s.db == nil {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{"error": "hub.DB not configured"})
		return
	}
	// Path: /api/pending-actions/{id}/ack — require explicit /ack
	// suffix per Rain BRAIN-2nd non-blocking #2 (defense-in-depth
	// against accidental URL-shape drift catching POST /api/pending-
	// actions/{id} without the suffix).
	if !strings.HasSuffix(r.URL.Path, "/ack") {
		http.Error(w, "invalid path: missing /ack suffix", http.StatusBadRequest)
		return
	}
	rest := strings.TrimPrefix(r.URL.Path, "/api/pending-actions/")
	rest = strings.TrimSuffix(rest, "/ack")
	if rest == "" {
		http.Error(w, "id required", http.StatusBadRequest)
		return
	}
	var id int64
	if _, err := fmt.Sscanf(rest, "%d", &id); err != nil || id <= 0 {
		http.Error(w, "invalid id", http.StatusBadRequest)
		return
	}
	ok, err := s.db.AckPendingAction(id)
	if err != nil {
		http.Error(w, fmt.Sprintf("ack: %v", err), http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"ack": ok, "id": id})
}

// stderrSink returns the io.Writer used for best-effort error logs
// from hooks. Indirection lets tests redirect via test helpers.
func stderrSink() *os.File {
	return os.Stderr
}
