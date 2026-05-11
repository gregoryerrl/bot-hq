package main

import (
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"sync"

	"github.com/gregoryerrl/bot-hq/internal/brian"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/mcp"
	"github.com/gregoryerrl/bot-hq/internal/rain"
)

// sessionDuoRegistry maps session_id → (brian, rain) pair so the
// SessionFinalizeHook can kill the correct tmux sessions on close.
// Z-3 sessions-as-containers: per-session BRAIN-duo lifecycle.
type sessionDuoRegistry struct {
	mu  sync.RWMutex
	pairs map[string]*duoPair
}

type duoPair struct {
	Brian *brian.Brian
	Rain  *rain.Rain
}

func newSessionDuoRegistry() *sessionDuoRegistry {
	return &sessionDuoRegistry{pairs: map[string]*duoPair{}}
}

func (r *sessionDuoRegistry) Add(sid string, p *duoPair) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.pairs[sid] = p
}

func (r *sessionDuoRegistry) Get(sid string) (*duoPair, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	p, ok := r.pairs[sid]
	return p, ok
}

func (r *sessionDuoRegistry) Remove(sid string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.pairs, sid)
}

// installSessionLifecycleHooks wires the MCP session_open + finalize
// hooks to the daemon's brian/rain spawn machinery. Called once during
// runHub setup after hub + autostart options are resolved.
//
// Z-3 sessions-as-containers: on hub_session_open, spawn a new
// (brian, rain) pair bound to the session_id via BOT_HQ_SESSION_ID env.
// On hub_session_finalize, kill that pair's tmux sessions, capture
// per-agent state.json for closing_state, optionally archive Discord
// thread.
func installSessionLifecycleHooks(h *hub.Hub, brianWorkDir, rainWorkDir string) *sessionDuoRegistry {
	reg := newSessionDuoRegistry()

	mcp.SetSessionOpenHook(func(req mcp.SessionOpenRequest) (*mcp.SessionOpenInfo, error) {
		b := brian.New(h.DB, brianWorkDir)
		b.SetSessionID(req.SessionID)
		if err := b.Start(); err != nil {
			return nil, fmt.Errorf("brian spawn for session %s: %w", req.SessionID, err)
		}
		r := rain.New(h.DB, rainWorkDir)
		r.SetSessionID(req.SessionID)
		if err := r.Start(); err != nil {
			// Best-effort cleanup of brian.
			b.Stop()
			return nil, fmt.Errorf("rain spawn for session %s: %w", req.SessionID, err)
		}
		reg.Add(req.SessionID, &duoPair{Brian: b, Rain: r})
		log.Printf("[session-open] spawned brian+rain for session=%s scope=%s project=%s", req.SessionID, req.Scope, req.Project)
		return &mcp.SessionOpenInfo{
			SessionID: req.SessionID,
			Project:   req.Project,
			Scope:     req.Scope,
			Agents:    []string{"brian", "rain"},
			// DiscordThreadID populated by Discord lifecycle hook (Group F)
		}, nil
	})

	mcp.SetSessionFinalizeHook(func(req mcp.SessionFinalizeRequest) (*mcp.SessionFinalizeResult, error) {
		result := &mcp.SessionFinalizeResult{
			KilledTmux:   []string{},
			ClosingState: map[string]string{},
		}
		pair, ok := reg.Get(req.SessionID)
		if !ok {
			// Session might have been opened in a previous daemon run.
			// Best-effort: capture state.json files from sessions/<id>/
			// even without an in-process duo pair.
			capturePerAgentState(req.SessionID, result.ClosingState)
			return result, nil
		}
		// Capture per-agent state.json BEFORE killing the duo (state may
		// have been written by hub_session_close's SNAP-storage upstream).
		capturePerAgentState(req.SessionID, result.ClosingState)

		if pair.Brian != nil {
			pair.Brian.Stop()
			result.KilledTmux = append(result.KilledTmux, "brian")
		}
		if pair.Rain != nil {
			pair.Rain.Stop()
			result.KilledTmux = append(result.KilledTmux, "rain")
		}
		reg.Remove(req.SessionID)
		log.Printf("[session-finalize] killed brian+rain for session=%s (force=%v)", req.SessionID, req.Force)
		return result, nil
	})

	return reg
}

// capturePerAgentState reads sessions/<sid>/<agent>/state.json for both
// duo members and populates the closing_state map. Missing files are
// silent — agents may not have written state if no checkpoint event
// fired in the session's lifetime.
func capturePerAgentState(sessionID string, out map[string]string) {
	home, err := os.UserHomeDir()
	if err != nil {
		return
	}
	canonRoot := filepath.Join(home, ".bot-hq")
	if env := os.Getenv("BOT_HQ_HOME"); env != "" {
		canonRoot = env
	}
	if envSD := os.Getenv("BOT_HQ_SESSIONS_DIR"); envSD != "" {
		// SessionsDir is honored separately from canonRoot for test isolation
		for _, agent := range []string{"brian", "rain"} {
			path := filepath.Join(envSD, sessionID, agent, "state.json")
			if data, err := os.ReadFile(path); err == nil {
				out[agent] = string(data)
			}
		}
		return
	}
	for _, agent := range []string{"brian", "rain"} {
		path := filepath.Join(canonRoot, "sessions", sessionID, agent, "state.json")
		if data, err := os.ReadFile(path); err == nil {
			out[agent] = string(data)
		}
	}
}

// killTmuxSession is a small helper for cmd-only callers that need to
// kill a session without a Brian/Rain handle. Best-effort.
func killTmuxSession(sessionName string) error {
	cmd := exec.Command("tmux", "kill-session", "-t", sessionName)
	return cmd.Run()
}
