package mcp

import (
	"context"
	"crypto/rand"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/sessions"
	"github.com/mark3labs/mcp-go/mcp"
)

// SessionOpenRequest is the input the daemon-side hook receives when
// hub_session_open MCP tool fires. The hook is responsible for spawning
// the BRAIN-duo (brian + rain) tmux sessions with BOT_HQ_SESSION_ID env
// var set; the MCP tool itself only allocates the session container.
type SessionOpenRequest struct {
	SessionID   string
	Project     string
	Scope       string
	PointerList []string
}

// SessionOpenInfo is the result returned to the MCP caller after the
// daemon hook has finished its spawn work.
type SessionOpenInfo struct {
	SessionID       string   `json:"session_id"`
	Project         string   `json:"project"`
	Scope           string   `json:"scope"`
	DiscordThreadID string   `json:"discord_thread_id,omitempty"`
	Agents          []string `json:"agents"`
}

// SessionFinalizeRequest is the input the daemon-side hook receives when
// hub_session_finalize MCP tool fires its Z-3 extension. The hook is
// responsible for killing the duo's tmux sessions, archiving Discord
// thread (best-effort), and reading per-agent state.json files for the
// closing_state manifest section.
type SessionFinalizeRequest struct {
	SessionID       string
	DiscordThreadID string
	Force           bool
}

// SessionFinalizeResult is the daemon-side outcome of finalize: what got
// killed, what got archived, what closing_state was captured per agent.
type SessionFinalizeResult struct {
	KilledTmux       []string
	DiscordArchived  bool
	ClosingState     map[string]string
}

// Package-level hooks installed by the daemon (cmd/bot-hq/main.go) before
// MCP server start. The mcp package can't import internal/brian or
// internal/rain (those packages already depend on mcp via tool registration
// at tmux config write-time). Hook indirection keeps the layering acyclic.
//
// Hooks are best-effort: if nil, the MCP tool reports a clear error
// rather than failing silently. R51 daemon-hook discipline.
var (
	sessionOpenHookMu     sync.RWMutex
	sessionOpenHook       func(SessionOpenRequest) (*SessionOpenInfo, error)
	sessionFinalizeHookMu sync.RWMutex
	sessionFinalizeHook   func(SessionFinalizeRequest) (*SessionFinalizeResult, error)
)

// SetSessionOpenHook installs the daemon-side spawn hook. Called once at
// daemon startup before MCP server begins serving. Setting to nil clears.
func SetSessionOpenHook(h func(SessionOpenRequest) (*SessionOpenInfo, error)) {
	sessionOpenHookMu.Lock()
	defer sessionOpenHookMu.Unlock()
	sessionOpenHook = h
}

// SetSessionFinalizeHook installs the daemon-side finalize hook (kill duo
// tmux + archive Discord + capture per-agent closing_state).
func SetSessionFinalizeHook(h func(SessionFinalizeRequest) (*SessionFinalizeResult, error)) {
	sessionFinalizeHookMu.Lock()
	defer sessionFinalizeHookMu.Unlock()
	sessionFinalizeHook = h
}

func getSessionOpenHook() func(SessionOpenRequest) (*SessionOpenInfo, error) {
	sessionOpenHookMu.RLock()
	defer sessionOpenHookMu.RUnlock()
	return sessionOpenHook
}

func getSessionFinalizeHook() func(SessionFinalizeRequest) (*SessionFinalizeResult, error) {
	sessionFinalizeHookMu.RLock()
	defer sessionFinalizeHookMu.RUnlock()
	return sessionFinalizeHook
}

// sessionOpenMu serializes hub_session_open invocations to prevent
// concurrent slug-uuid collisions under near-simultaneous calls (fault-
// tree F8 — pre-disposition; now mooted by uuid suffix but cheap to
// keep for the deeper concern of partial-state file-system races).
var sessionOpenMu sync.Mutex

// slugSanitizationRe matches allowed scope-slug characters per Z-3
// Round-2 spec: lowercase alphanumeric + hyphen only.
var (
	slugSanitizationRe        = mustCompileSlug()
	slugMaxLength             = 40
	sessionIDUUIDLength       = 6
	sessionIDUUIDAlphabet     = "23456789abcdefghijklmnpqrstuvwxyz" // no 0/1/o/l per Z-3 spec
	sessionIDUUIDAlphabetSize = byte(len(sessionIDUUIDAlphabet))
)

func mustCompileSlug() *slugRE { return &slugRE{} }

type slugRE struct{}

func (s *slugRE) Sanitize(in string) (string, error) {
	in = strings.ToLower(strings.TrimSpace(in))
	if in == "" {
		return "", fmt.Errorf("slug must not be empty")
	}
	var b strings.Builder
	for _, r := range in {
		switch {
		case r >= 'a' && r <= 'z':
			b.WriteRune(r)
		case r >= '0' && r <= '9':
			b.WriteRune(r)
		case r == '-':
			b.WriteRune(r)
		default:
			// Other chars stripped; whitespace becomes hyphen
			if r == ' ' || r == '_' {
				b.WriteRune('-')
			}
		}
	}
	out := strings.Trim(b.String(), "-")
	if out == "" {
		return "", fmt.Errorf("slug %q sanitizes to empty (all-hyphens or all-symbols)", in)
	}
	if len(out) > slugMaxLength {
		out = out[:slugMaxLength-len("-tr0")] + "-tr0"
	}
	return out, nil
}

// generateSessionUUID returns a 6-char base32-lowercase id from a 32-char
// alphabet (no ambiguous 0/1/o/l). Uses crypto/rand for collision
// resistance. Returns ("", err) on rand-read failure.
func generateSessionUUID() (string, error) {
	buf := make([]byte, sessionIDUUIDLength)
	if _, err := rand.Read(buf); err != nil {
		return "", err
	}
	out := make([]byte, sessionIDUUIDLength)
	for i, b := range buf {
		out[i] = sessionIDUUIDAlphabet[b%sessionIDUUIDAlphabetSize]
	}
	return string(out), nil
}

// AllocateSessionID composes a Z-3 session-id from a scope-slug + 6-char
// uuid suffix. Sanitizes the slug, generates the uuid, joins with "-".
// Exposed for daemon-side callers + tests; the MCP tool wraps this.
func AllocateSessionID(scope string) (string, error) {
	slug, err := slugSanitizationRe.Sanitize(scope)
	if err != nil {
		return "", err
	}
	uuid, err := generateSessionUUID()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("%s-%s", slug, uuid), nil
}

// hubSessionOpen is the Z-3 user/emma-initiated session-open MCP tool.
// Spawns a new session container with BRAIN-duo bound to it via
// BOT_HQ_SESSION_ID env.
//
// Args:
//   - project (required): project key (matches projects/<key>.yaml)
//   - scope_name (required): human-readable scope (sanitized to slug;
//     joined with 6-char uuid suffix to form session-id)
//   - pointer_list (optional): emma's curated CL paths as starting points
//   - initial_message (optional): context-message to seed the session's
//     hub stream (logged as MsgUpdate from "system")
//
// Returns: {session_id, project, scope, discord_thread_id, agents:[brian,rain]}.
//
// Daemon-side: requires SetSessionOpenHook installed by main.go. Without
// the hook, the tool returns a clear error so callers know the daemon is
// in a degraded state.
func hubSessionOpen(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_session_open",
		mcp.WithDescription("Open a new bot-hq session-cluster container with BRAIN-duo bound to it via BOT_HQ_SESSION_ID env. Daemon spawns brian + rain tmux sessions; manifest is written; Discord thread is created in the project channel. Sessions are user-opened (or emma-orchestrated); daemon idle = emma-only ambient per Z-3 sessions-as-containers architecture."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key (matches projects/<key>.yaml in CL)")),
		mcp.WithString("scope_name", mcp.Required(), mcp.Description("Human-readable scope name (e.g., 'Z-3 sessions as containers'). Sanitized to slug; combined with 6-char uuid to form session-id.")),
		mcp.WithArray("pointer_list", mcp.Description("Optional list of CL paths emma curates as starting-point reads for BRAIN-duo (paths only, not content). BRAIN-duo may expand on these.")),
		mcp.WithString("initial_message", mcp.Description("Optional context message to seed the session's hub stream.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		scope, err := req.RequireString("scope_name")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		pointerList := []string{}
		if raw := req.GetArguments()["pointer_list"]; raw != nil {
			if list, ok := raw.([]any); ok {
				for _, item := range list {
					if s, ok := item.(string); ok && s != "" {
						pointerList = append(pointerList, s)
					}
				}
			}
		}
		initialMessage := req.GetString("initial_message", "")

		// Validate project exists in CL.
		home, err := os.UserHomeDir()
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("resolve home: %v", err)), nil
		}
		canonRoot := filepath.Join(home, ".bot-hq")
		if env := os.Getenv("BOT_HQ_HOME"); env != "" {
			canonRoot = env
		}
		projectFile := filepath.Join(canonRoot, "projects", project+".yaml")
		if _, statErr := os.Stat(projectFile); statErr != nil {
			return mcp.NewToolResultError(fmt.Sprintf("project %q not registered (missing %s)", project, projectFile)), nil
		}

		// Allocate session-id under mutex to serialize the file-system
		// race against concurrent open calls (per fault-tree F8/§Round-2).
		sessionOpenMu.Lock()
		defer sessionOpenMu.Unlock()

		sessionID, err := AllocateSessionID(scope)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("allocate session id: %v", err)), nil
		}
		// Resolve sessions base via the sessions package so the skeleton
		// and the manifest land at the same path (BOT_HQ_SESSIONS_DIR is
		// the source-of-truth in tests; defaults to ~/.bot-hq/sessions/
		// otherwise).
		sessionDir := filepath.Join(sessions.SessionsDir(), sessionID)
		// Collision check (theoretically negligible with 6-char base32 +
		// crypto/rand; defensive).
		if _, statErr := os.Stat(sessionDir); statErr == nil {
			return mcp.NewToolResultError(fmt.Sprintf("session-id %q already exists (uuid collision; retry)", sessionID)), nil
		}

		// Create session skeleton: sessions/<id>/{brian/, rain/, tasks/}
		for _, sub := range []string{"brian", "rain", "tasks"} {
			if mkErr := os.MkdirAll(filepath.Join(sessionDir, sub), 0o755); mkErr != nil {
				return mcp.NewToolResultError(fmt.Sprintf("create %s/%s: %v", sessionID, sub, mkErr)), nil
			}
		}

		// Write minimal manifest.
		manifest := sessions.Manifest{
			ID:          sessionID,
			Project:     project,
			Scope:       scope,
			PointerList: pointerList,
			StartTS:     time.Now().UTC(),
			Agents:      []string{"brian", "rain"},
			Status:      "active",
		}
		if mErr := sessions.WriteManifest(manifest); mErr != nil {
			return mcp.NewToolResultError(fmt.Sprintf("write manifest at %s (SessionsDir=%s): %v", sessions.ManifestPath(sessionID), sessions.SessionsDir(), mErr)), nil
		}

		// Dispatch to daemon-side hook for spawn + Discord thread.
		hook := getSessionOpenHook()
		var info SessionOpenInfo
		if hook != nil {
			r, hErr := hook(SessionOpenRequest{
				SessionID:   sessionID,
				Project:     project,
				Scope:       scope,
				PointerList: pointerList,
			})
			if hErr != nil {
				return mcp.NewToolResultError(fmt.Sprintf("daemon spawn hook: %v", hErr)), nil
			}
			if r != nil {
				info = *r
			}
		} else {
			// Hook not installed: session container is allocated but no
			// spawn happened. Caller decides what to do (degraded mode).
			info = SessionOpenInfo{
				SessionID: sessionID,
				Project:   project,
				Scope:     scope,
				Agents:    []string{},
			}
		}

		// Seed initial_message into hub stream.
		if initialMessage != "" {
			_, _ = db.InsertMessage(systemMsg(sessionID, "session-open: "+initialMessage))
		}

		// If the daemon hook returned a discord thread, persist it back
		// to the manifest (round-trip).
		if info.DiscordThreadID != "" {
			manifest.DiscordThreadID = info.DiscordThreadID
			_ = sessions.WriteManifest(manifest)
		}

		return mcp.NewToolResultText(toJSON(info)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// systemMsg constructs a system-originated session-bound message for the
// hub stream. session_id is the Z-3 session-binding; from_agent="system"
// matches the existing convention for autostart/lifecycle emits.
func systemMsg(sessionID, content string) protocol.Message {
	return protocol.Message{
		FromAgent: "system",
		Type:      protocol.MsgUpdate,
		Content:   content,
		SessionID: sessionID,
	}
}
