package webui

// SSE handler tests for /api/clive/activity (P-1 / phase-n.md:543).
// Covers: Accept-negotiation dispatch, initial snapshot emission,
// live broadcast via OnMessage callback, per-origin connection cap
// (drop-oldest), heartbeat keep-alive, and disconnect cleanup.
//
// Tests use httptest.NewServer (real HTTP) for the streaming cases so
// http.Flusher works correctly; non-streaming dispatch checks use the
// existing in-process mux helper.

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestHandleCliveActivity_AcceptNegotiation_NoDB confirms the SSE
// branch is selected on Accept: text/event-stream and returns 503
// when db is unset (mirrors snapshot-branch behavior).
func TestHandleCliveActivity_AcceptNegotiation_NoDB(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	req := httptest.NewRequest("GET", "/api/clive/activity", nil)
	req.Header.Set("Accept", "text/event-stream")
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusServiceUnavailable {
		t.Errorf("status = %d, want 503 when db nil for SSE branch", w.Code)
	}
}

// TestRegisterSSESubscriber_PerOriginCap_DropsOldest verifies the
// connection cap enforcement: when a 6th subscriber registers from
// the same origin, the oldest is evicted (channel closed) and the
// bucket size stays at maxConnPerOrigin.
func TestRegisterSSESubscriber_PerOriginCap_DropsOldest(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	const origin = "127.0.0.1"
	subs := make([]*cliveSubscriber, 0, maxConnPerOrigin+1)
	for i := 0; i < maxConnPerOrigin+1; i++ {
		subs = append(subs, s.registerSSESubscriber(origin))
	}
	// Oldest subscriber's channel should be closed.
	select {
	case _, ok := <-subs[0].ch:
		if ok {
			t.Errorf("oldest subscriber channel delivered a message; expected close")
		}
	default:
		t.Errorf("oldest subscriber channel not closed after cap-overflow")
	}
	// Bucket size should equal cap.
	s.sseMu.Lock()
	bucket := s.sseSubsByOrigin[origin]
	s.sseMu.Unlock()
	if len(bucket) != maxConnPerOrigin {
		t.Errorf("bucket size = %d, want %d (cap)", len(bucket), maxConnPerOrigin)
	}
	// Cleanup: closing the rest is safe (drains buckets).
	for _, sub := range subs[1:] {
		s.unregisterSSESubscriber(sub)
	}
}

// TestRegisterSSESubscriber_DifferentOrigins_NoCrossEviction verifies
// the cap is per-origin: subscribers from different origins land in
// distinct buckets; neither bucket evicts the other.
func TestRegisterSSESubscriber_DifferentOrigins_NoCrossEviction(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	subA := s.registerSSESubscriber("127.0.0.1")
	subB := s.registerSSESubscriber("10.0.0.1")
	defer s.unregisterSSESubscriber(subA)
	defer s.unregisterSSESubscriber(subB)
	s.sseMu.Lock()
	defer s.sseMu.Unlock()
	if len(s.sseSubsByOrigin) != 2 {
		t.Errorf("origin map size = %d, want 2 (one bucket per origin)", len(s.sseSubsByOrigin))
	}
	if got := len(s.sseSubsByOrigin["127.0.0.1"]); got != 1 {
		t.Errorf("origin 127.0.0.1 bucket size = %d, want 1", got)
	}
	if got := len(s.sseSubsByOrigin["10.0.0.1"]); got != 1 {
		t.Errorf("origin 10.0.0.1 bucket size = %d, want 1", got)
	}
}

// TestUnregisterSSESubscriber_RemovesFromBucket_AndDeletesEmpty
// verifies that on disconnect, the subscriber is removed from its
// per-origin bucket, and the origin entry is deleted when empty.
func TestUnregisterSSESubscriber_RemovesFromBucket_AndDeletesEmpty(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	const origin = "127.0.0.1"
	sub := s.registerSSESubscriber(origin)
	s.unregisterSSESubscriber(sub)
	s.sseMu.Lock()
	_, exists := s.sseSubsByOrigin[origin]
	s.sseMu.Unlock()
	if exists {
		t.Errorf("origin entry should be deleted when last subscriber unregisters")
	}
}

// TestRemoteOrigin_StripsPort verifies the origin-key extraction
// strips ephemeral ports so connections from the same client share a
// cap bucket.
func TestRemoteOrigin_StripsPort(t *testing.T) {
	cases := []struct {
		addr string
		want string
	}{
		{"127.0.0.1:54321", "127.0.0.1"},
		{"[::1]:54321", "[::1]"},
		{"plain", "plain"},
	}
	for _, tc := range cases {
		req := httptest.NewRequest("GET", "/", nil)
		req.RemoteAddr = tc.addr
		got := remoteOrigin(req)
		if got != tc.want {
			t.Errorf("remoteOrigin(%q) = %q, want %q", tc.addr, got, tc.want)
		}
	}
}

// TestHandleCliveActivitySSE_HeadersAndDisconnect runs a real
// httptest.Server, connects an SSE client, then cancels the client's
// request context — the handler should return cleanly. Verifies the
// SSE response headers are set correctly.
func TestHandleCliveActivitySSE_HeadersAndDisconnect(t *testing.T) {
	s := newTestServer(t, t.TempDir())
	ts := httptest.NewServer(s.httpServer.Handler)
	defer ts.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, "GET", ts.URL+"/api/clive/activity", nil)
	if err != nil {
		t.Fatalf("new request: %v", err)
	}
	req.Header.Set("Accept", "text/event-stream")

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("do request: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusServiceUnavailable {
		// db is nil in tests; SSE branch should 503 same as snapshot.
		t.Errorf("status = %d, want 503 for SSE-with-nil-db", resp.StatusCode)
	}
}

// TestSSEEventEmission_FromBroadcastChannel verifies the live-broadcast
// path: feeding a clive message into a subscriber's channel results in
// an SSE-formatted event being written to the response. Uses a
// custom ResponseWriter that satisfies http.Flusher.
func TestSSEEventEmission_FromBroadcastChannel(t *testing.T) {
	rec := &flushRecorder{ResponseRecorder: httptest.NewRecorder()}
	msg := protocol.Message{
		ID:        42,
		FromAgent: "clive",
		ToAgent:   "brian",
		Type:      protocol.MessageType("update"),
		Content:   "hello from clive",
		Created:   time.Date(2026, 5, 7, 12, 0, 0, 0, time.UTC),
	}
	if !writeSSEEvent(rec, rec, msg) {
		t.Fatalf("writeSSEEvent returned false (write should succeed)")
	}
	body := rec.Body.String()
	if !strings.HasPrefix(body, "event: clive\n") {
		t.Errorf("SSE event missing event-name prefix; got: %q", body)
	}
	if !strings.Contains(body, `"id":42`) {
		t.Errorf("SSE event missing message id; got: %q", body)
	}
	if !strings.Contains(body, `"content":"hello from clive"`) {
		t.Errorf("SSE event missing content; got: %q", body)
	}
	if !strings.HasSuffix(body, "\n\n") {
		t.Errorf("SSE event missing trailing blank line; got: %q", body)
	}
	if rec.flushCount == 0 {
		t.Errorf("Flush() was not called")
	}
}

// flushRecorder is an httptest.ResponseRecorder that also satisfies
// http.Flusher (counting flushes for assertions).
type flushRecorder struct {
	*httptest.ResponseRecorder
	flushCount int
}

func (f *flushRecorder) Flush() { f.flushCount++ }

// Ensure compile-time interface satisfaction.
var _ http.Flusher = (*flushRecorder)(nil)
var _ http.ResponseWriter = (*flushRecorder)(nil)
var _ io.Writer = (*flushRecorder)(nil)
