package main

import (
	"encoding/json"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/brian"
	"github.com/gregoryerrl/bot-hq/internal/discord"
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
//
// Z-3d session-lifecycle queue bridge: also launches the queue-drain
// goroutine that processes hub_session_open / hub_session_finalize
// requests enqueued by per-agent stdio MCP subprocesses (which can't
// reach the in-daemon hook variable across process boundary).
//
// Z-7: discordBot (nullable) is used to spawn a per-session thread
// under the hub channel on open and archive it on finalize. nil bot =
// Discord disabled; lifecycle proceeds with empty thread-id.
func installSessionLifecycleHooks(h *hub.Hub, brianWorkDir, rainWorkDir string, discordBot *discord.Bot) *sessionDuoRegistry {
	reg := newSessionDuoRegistry()

	openFn := func(req mcp.SessionOpenRequest) (*mcp.SessionOpenInfo, error) {
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

		threadID := ""
		if discordBot != nil {
			// Thread name = full session-id so two sessions with the
			// same scope-slug (different uuid suffix) get distinct,
			// greppable threads.
			if tid, terr := discordBot.CreateSessionThread(req.SessionID); terr == nil {
				threadID = tid
			} else {
				log.Printf("[session-open] discord thread create for %s: %v", req.SessionID, terr)
			}
		}
		log.Printf("[session-open] spawned brian+rain for session=%s scope=%s project=%s discord_thread=%q", req.SessionID, req.Scope, req.Project, threadID)
		return &mcp.SessionOpenInfo{
			SessionID:       req.SessionID,
			Project:         req.Project,
			Scope:           req.Scope,
			DiscordThreadID: threadID,
			Agents:          []string{"brian", "rain"},
		}, nil
	}

	finalizeFn := func(req mcp.SessionFinalizeRequest) (*mcp.SessionFinalizeResult, error) {
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
	}

	mcp.SetSessionOpenHook(openFn)
	mcp.SetSessionFinalizeHook(finalizeFn)

	// Z-3d: drain queue for subprocess MCP requests.
	go runSessionLifecycleQueueLoop(h, openFn, finalizeFn)

	return reg
}

// runSessionLifecycleQueueLoop is the daemon-side drain goroutine for
// the Z-3d session_lifecycle_queue table. Subprocess MCP servers (per-
// agent stdio bot-hq mcp processes) enqueue requests when they need
// daemon-side spawn machinery; this loop picks them up + calls the
// installed hooks + writes results back so subprocesses can return them
// to the MCP caller. Mirrors the existing hub message_queue drain
// pattern.
//
// Tick interval 500ms — matches subprocess poll interval. Lower would
// add SQLite read pressure; higher would slow round-trip.
func runSessionLifecycleQueueLoop(h *hub.Hub,
	openFn func(mcp.SessionOpenRequest) (*mcp.SessionOpenInfo, error),
	finalizeFn func(mcp.SessionFinalizeRequest) (*mcp.SessionFinalizeResult, error),
) {
	ticker := time.NewTicker(500 * time.Millisecond)
	defer ticker.Stop()
	for range ticker.C {
		ops, err := h.DB.ClaimPendingSessionOps(10)
		if err != nil {
			log.Printf("[slq] poll error: %v", err)
			continue
		}
		for _, op := range ops {
			processSessionLifecycleOp(h, op, openFn, finalizeFn)
		}
	}
}

func processSessionLifecycleOp(h *hub.Hub, op hub.SessionLifecycleOp,
	openFn func(mcp.SessionOpenRequest) (*mcp.SessionOpenInfo, error),
	finalizeFn func(mcp.SessionFinalizeRequest) (*mcp.SessionFinalizeResult, error),
) {
	switch op.Kind {
	case "open":
		var pointerList []string
		if op.PointerListJSON != "" && op.PointerListJSON != "[]" {
			_ = json.Unmarshal([]byte(op.PointerListJSON), &pointerList)
		}
		info, err := openFn(mcp.SessionOpenRequest{
			SessionID:   op.SessionID,
			Project:     op.Project,
			Scope:       op.Scope,
			PointerList: pointerList,
		})
		if err != nil {
			_ = h.DB.MarkSessionOpFired(op.ID, "failed", err.Error())
			log.Printf("[slq] open id=%d failed: %v", op.ID, err)
			return
		}
		body, _ := json.Marshal(info)
		if err := h.DB.MarkSessionOpFired(op.ID, "fired", string(body)); err != nil {
			log.Printf("[slq] mark-fired id=%d err: %v", op.ID, err)
			return
		}
		log.Printf("[slq] open id=%d fired session=%s", op.ID, op.SessionID)
	case "finalize":
		result, err := finalizeFn(mcp.SessionFinalizeRequest{
			SessionID:       op.SessionID,
			DiscordThreadID: op.DiscordThreadID,
			Force:           op.Force,
		})
		if err != nil {
			_ = h.DB.MarkSessionOpFired(op.ID, "failed", err.Error())
			log.Printf("[slq] finalize id=%d failed: %v", op.ID, err)
			return
		}
		body, _ := json.Marshal(result)
		if err := h.DB.MarkSessionOpFired(op.ID, "fired", string(body)); err != nil {
			log.Printf("[slq] mark-fired id=%d err: %v", op.ID, err)
			return
		}
		log.Printf("[slq] finalize id=%d fired session=%s", op.ID, op.SessionID)
	default:
		_ = h.DB.MarkSessionOpFired(op.ID, "failed", fmt.Sprintf("unknown kind %q", op.Kind))
		log.Printf("[slq] unknown kind %q for id=%d", op.Kind, op.ID)
	}
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
