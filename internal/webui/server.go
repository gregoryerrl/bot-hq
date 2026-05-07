// Package webui implements the Phase N v3 Clive workspace web UI per
// docs/plans/2026-05-06-phase-n-v3-rules-and-api-design-spike.md.
//
// Scope (v3b read MVP):
//   - HTTP server on 127.0.0.1:<port> (localhost loopback only; no auth)
//   - GET /api/files (canonical-store tree)
//   - GET /api/files/{path} (file content + mtime)
//   - GET /api/rules (resolved per project + agent context)
//   - GET /api/sessions (parsed sessions index)
//   - GET /api/clive/activity (SSE stream from hub)
//   - Static frontend at / (single-file htmx app)
//
// Scope (v3c — separate commit):
//   - POST endpoints for user-web-save + Clive draft-author
//   - Per-canonical-dir .git lazy init + commit + revert
//   - Raw-YAML rules editor with schema validation on save
//   - 3-layer visibility wiring (hub_send notification + git audit)
package webui

import (
	"context"
	"embed"
	"errors"
	"fmt"
	"io/fs"
	"log"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// DefaultPort is the loopback bind port when BOT_HQ_WEBUI_PORT is unset.
// Single canonical webui port post Phase P P-10 unification (voice
// surface migrated INTO this server; no separate :3847 process).
const DefaultPort = 3849

// portEnvVar overrides DefaultPort. Per scope-lock OQ-2 LOCKED.
const portEnvVar = "BOT_HQ_WEBUI_PORT"

//go:embed static/*
var staticFS embed.FS

// Server is the Phase N v3b/v3c Clive workspace HTTP server. Constructed
// via NewServer; lifecycle managed by Start + Shutdown. Handlers in
// handlers.go (read) + write_handlers.go (write).
type Server struct {
	httpServer *http.Server
	db         *hub.DB

	canonicalRoot string // ~/.bot-hq/ (configurable via WithRoot for tests)
	port          int

	proposals *proposalStore // Clive draft-author proposals awaiting user approval (v3c)

	// SSE subscriber state for /api/clive/activity live feed (P-1).
	sseMu           sync.Mutex
	sseSubsByOrigin map[string][]*cliveSubscriber

	// Ambient webui-focus state. Frontend POSTs to /api/webui-context on
	// file-open / project-pick / tab-switch; voice handler reads pre-
	// Gemini.Connect to inject "[USER VIEWING: ...]" into systemInstruction
	// so Clive sees what the user is looking at without the user having
	// to spell out filenames. In-memory only — single-user local-only
	// webui; same-process Clive-after-P-10 needs no IPC. Per user msg
	// 15117 "i want clive to see what i am looking at on the web ui".
	ctxMu    sync.RWMutex
	webuiCtx WebuiContext
	// ctxSubs receive WebuiContext updates whenever SetWebuiContext
	// observes a real change to project/path/viewMode (not on every
	// POST). Voice handlers subscribe so they can SendText a context-
	// update into already-connected Gemini sessions — connect-time
	// systemInstruction injection alone misses focus changes that
	// happen after the voice WS opens (page-load → connect with empty
	// focus → user opens file → Gemini never sees it). Per user follow-
	// up 2026-05-07 "clive can't see the file that i'm looking at".
	ctxSubsMu sync.Mutex
	ctxSubs   map[chan WebuiContext]struct{}
}

// WebuiContext describes what the user is currently looking at in the
// webui. Read by the voice handler before connecting to Gemini so Clive
// has ambient awareness of the focus file/project. Written via POST
// /api/webui-context from the frontend on every focus change.
type WebuiContext struct {
	Project     string    `json:"project"`
	CurrentPath string    `json:"currentPath"`
	ViewMode    string    `json:"viewMode"` // "rendered" | "raw" | "split"
	UpdatedAt   time.Time `json:"updatedAt"`
}

// Option mutates Server config at construction. Pattern mirrors
// internal/snap.NewServer for consistency.
type Option func(*Server)

// WithRoot overrides the canonical-store root (default ~/.bot-hq/).
// Used by tests for R39 TEST-ISOLATION compliance.
func WithRoot(root string) Option {
	return func(s *Server) { s.canonicalRoot = root }
}

// WithPort overrides the bind port (default DefaultPort or env var).
func WithPort(port int) Option {
	return func(s *Server) { s.port = port }
}

// NewServer constructs a Server with the given hub.DB (used for Clive
// activity SSE source) and optional overrides. Defaults: canonical root
// from $HOME/.bot-hq/, port from BOT_HQ_WEBUI_PORT env or DefaultPort.
func NewServer(db *hub.DB, opts ...Option) (*Server, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, fmt.Errorf("home dir: %w", err)
	}
	s := &Server{
		db:            db,
		canonicalRoot: home + "/.bot-hq",
		port:          envPort(),
		proposals:     newProposalStore(),
		ctxSubs:       make(map[chan WebuiContext]struct{}),
	}
	for _, opt := range opts {
		opt(s)
	}
	s.initSSE()
	s.wireCliveBroadcast()
	s.wireUserPendingActions()
	mux := http.NewServeMux()
	s.registerRoutes(mux)
	s.httpServer = &http.Server{
		Addr:    fmt.Sprintf("127.0.0.1:%d", s.port),
		Handler: mux,
	}
	return s, nil
}

// envPort reads BOT_HQ_WEBUI_PORT or returns DefaultPort.
func envPort() int {
	if raw := os.Getenv(portEnvVar); raw != "" {
		if p, err := strconv.Atoi(raw); err == nil && p > 0 && p < 65536 {
			return p
		}
	}
	return DefaultPort
}

// Addr returns the bind address (e.g., "127.0.0.1:3849"). Useful for
// tests that need to know the actual port (e.g., when port=0 is requested
// for ephemeral binding).
func (s *Server) Addr() string {
	return s.httpServer.Addr
}

// CanonicalRoot returns the configured ~/.bot-hq/ root path. Useful for
// tests to verify the server reads from the expected location.
func (s *Server) CanonicalRoot() string {
	return s.canonicalRoot
}

// Start binds the server to the configured port on both IPv4 and IPv6
// loopback (127.0.0.1 + ::1) so browsers resolve "localhost" to either
// stack without NetworkError. Both listeners share the same handler.
// Blocks until ctx is canceled or Shutdown is called. Returns
// http.ErrServerClosed on graceful shutdown; other errors are bind/runtime
// failures.
func (s *Server) Start(ctx context.Context) error {
	// Bind IPv4 first to resolve the port (matters when port=0 ephemeral),
	// then bind IPv6 to the same resolved port.
	ln4, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", s.port))
	if err != nil {
		return fmt.Errorf("listen 127.0.0.1:%d: %w", s.port, err)
	}
	if tcpAddr, ok := ln4.Addr().(*net.TCPAddr); ok {
		s.port = tcpAddr.Port
		s.httpServer.Addr = tcpAddr.String()
	}
	ln6, err := net.Listen("tcp", fmt.Sprintf("[::1]:%d", s.port))
	if err != nil {
		// IPv6 loopback unavailable — log and continue on IPv4 only rather
		// than failing startup. Some constrained envs disable ::1.
		log.Printf("[webui] IPv6 loopback bind failed (continuing IPv4-only): %v", err)
		ln6 = nil
	}

	listeners := []net.Listener{ln4}
	if ln6 != nil {
		listeners = append(listeners, ln6)
		log.Printf("[webui] serving on http://127.0.0.1:%d/ + http://[::1]:%d/ (canonical root: %s)", s.port, s.port, s.canonicalRoot)
	} else {
		log.Printf("[webui] serving on http://127.0.0.1:%d/ (canonical root: %s)", s.port, s.canonicalRoot)
	}

	errCh := make(chan error, len(listeners))
	for _, ln := range listeners {
		ln := ln
		go func() {
			if err := s.httpServer.Serve(ln); err != nil && !errors.Is(err, http.ErrServerClosed) {
				errCh <- err
			}
		}()
	}

	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = s.httpServer.Shutdown(shutdownCtx)
		return ctx.Err()
	case err := <-errCh:
		return err
	}
}

// Shutdown gracefully stops the server. Idempotent.
func (s *Server) Shutdown(ctx context.Context) error {
	return s.httpServer.Shutdown(ctx)
}

// staticHandler serves the embedded frontend at /. Strip-prefixes "static/"
// so /index.html etc. resolve.
func staticHandler() http.Handler {
	sub, err := fs.Sub(staticFS, "static")
	if err != nil {
		// Compile-time embed should make this unreachable; panic surfaces
		// the bug at Server construction time rather than at first request.
		panic(fmt.Sprintf("webui static embed: %v", err))
	}
	return http.FileServer(http.FS(sub))
}

// registerRoutes wires the HTTP mux. Read endpoints (handlers.go) +
// write endpoints (write_handlers.go).
func (s *Server) registerRoutes(mux *http.ServeMux) {
	// Files endpoint: GET → tree (no path); GET → content; POST → save.
	// /api/files/{path}/clive  → Clive propose-or-approve
	// /api/files/{path}/revert → revert to prior commit
	mux.HandleFunc("/api/files", s.handleFilesTree)
	mux.HandleFunc("/api/files/", s.dispatchFilesPath)
	mux.HandleFunc("/api/projects", s.handleProjects)
	mux.HandleFunc("/api/recent-edits", s.handleRecentEdits)
	mux.HandleFunc("/api/search", s.handleSearch)
	mux.HandleFunc("/api/destinations", s.handleDestinations)
	mux.HandleFunc("/api/rules", s.handleRules)
	mux.HandleFunc("/api/sessions", s.handleSessions)
	mux.HandleFunc("/api/clive/activity", s.handleCliveActivity)
	mux.HandleFunc("/api/session-open", s.handleSessionOpen)
	mux.HandleFunc("/api/hub-pivot", s.handleHubPivot)
	mux.HandleFunc("/api/pending-actions", s.handlePendingActions)
	mux.HandleFunc("/api/pending-actions/", s.handlePendingActionAck)
	mux.HandleFunc("/api/agents", s.handleAgents)
	mux.HandleFunc("/api/voice/ws", s.handleVoiceWS)
	mux.HandleFunc("/api/webui-context", s.handleWebuiContext)
	mux.HandleFunc("/api/external-file/", s.handleExternalFile)
	mux.Handle("/", staticHandler())
}

// dispatchFilesPath routes /api/files/{path} variants by URL suffix +
// HTTP method. GET on a file path → handleFileContent; POST on a file
// path (no special suffix) → handleFileWrite; POST .../clive[/approve|
// /cancel] → handleCliveProposeOrApprove; POST .../revert →
// handleFileRevert.
func (s *Server) dispatchFilesPath(w http.ResponseWriter, r *http.Request) {
	path := r.URL.Path
	switch {
	case strings.HasSuffix(path, "/clive"),
		strings.HasSuffix(path, "/clive/approve"),
		strings.HasSuffix(path, "/clive/cancel"):
		s.handleCliveProposeOrApprove(w, r)
	case strings.HasSuffix(path, "/revert"):
		s.handleFileRevert(w, r)
	case strings.HasSuffix(path, "/history"):
		s.handleFileHistory(w, r)
	default:
		if r.Method == http.MethodPost {
			s.handleFileWrite(w, r)
		} else {
			s.handleFileContent(w, r)
		}
	}
}
