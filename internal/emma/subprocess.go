package emma

// Z-9d: emma as tmux Claude Code subprocess on DeepSeek-V4-Pro.
//
// Mirrors brian.go + rain.go spawn pattern. The Subprocess struct owns:
//   - tmux pane lifecycle (spawn / health-watch / restart)
//   - hub-message pollLoop that pastes filtered messages into the pane
//   - registration as the "emma" agent in hub.db
//
// Daemon-cadence audits (sentinel, plan_usage, delivery/egress, wake,
// context_cap, heartbeat) live in the SystemMonitor (emma.go) — they are
// pure-Go background work and emit as "system", NOT as "emma". The
// Subprocess is the canonical "emma" agent that users address with
// `@emma` and that the system shows as Emma in the agent roster.

import (
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/agentconfig"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/gregoryerrl/bot-hq/internal/tmuxsink"
)

const (
	subprocessPollInterval   = 3 * time.Second
	subprocessHealthInterval = 30 * time.Second
)

// Subprocess manages a tmux Claude Code session that acts as Emma the hub
// orchestrator (DeepSeek-V4-Pro per vision.md). Singleton: emma is global
// and stateless — no SetSessionID surface.
type Subprocess struct {
	db          *hub.DB
	workDir     string
	tmuxSession string
	lastMsgID   int64

	sink *tmuxsink.Sink

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}
}

// NewSubprocess creates the Subprocess. workDir is where Claude Code runs
// (defaults to ~/Projects if empty).
func NewSubprocess(db *hub.DB, workDir string) *Subprocess {
	if workDir == "" {
		home, _ := os.UserHomeDir()
		workDir = filepath.Join(home, "Projects")
	}
	return &Subprocess{
		db:      db,
		workDir: workDir,
		stopCh:  make(chan struct{}),
	}
}

// Start spawns Claude Code in tmux, registers the emma agent, and begins
// polling for messages.
func (e *Subprocess) Start() error {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.running {
		return nil
	}

	if err := os.MkdirAll(e.workDir, 0700); err != nil {
		return fmt.Errorf("emma work dir: %w", err)
	}

	if err := e.writeMCPConfig(); err != nil {
		return fmt.Errorf("emma mcp config: %w", err)
	}

	e.tmuxSession = fmt.Sprintf("bot-hq-emma-%d", time.Now().Unix())

	if err := e.spawnTmux(); err != nil {
		return fmt.Errorf("emma tmux spawn: %w", err)
	}

	metaJSON, _ := json.Marshal(map[string]string{"tmux_target": e.tmuxSession})
	agent := protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   protocol.AgentEmma,
		Status: protocol.StatusOnline,
		Meta:   string(metaJSON),
	}
	if err := e.db.RegisterAgent(agent); err != nil {
		return fmt.Errorf("emma register: %w", err)
	}

	e.sink = tmuxsink.New(hub.NewTmuxSinkStore(e.db), agentID, e.tmuxSession)

	if maxID, err := e.db.CurrentMaxMsgID(); err == nil {
		e.lastMsgID = maxID
	}
	e.running = true

	go e.pollLoop()
	go e.healthLoop()

	e.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Emma hub orchestrator online (DeepSeek-V4-Pro).",
	})

	return nil
}

// Stop shuts down the emma subprocess.
func (e *Subprocess) Stop() {
	e.mu.Lock()
	defer e.mu.Unlock()
	if !e.running {
		return
	}
	e.running = false
	close(e.stopCh)

	e.db.UpdateAgentStatus(agentID, protocol.StatusOffline, "")
	exec.Command("tmux", "kill-session", "-t", e.tmuxSession).Run()
}

// IsRunning reports whether the subprocess is active.
func (e *Subprocess) IsRunning() bool {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.running
}

// SendCommand pastes text into emma's Claude Code pane via tmuxsink.
func (e *Subprocess) SendCommand(text string) error {
	e.mu.Lock()
	if !e.running {
		e.mu.Unlock()
		return fmt.Errorf("emma subprocess is not running")
	}
	sink := e.sink
	e.mu.Unlock()
	if sink == nil {
		return fmt.Errorf("emma sink not initialized")
	}
	return sink.Deliver(0, text).Err
}

// TmuxSession exposes the tmux pane name for orphan-cleanup awareness.
func (e *Subprocess) TmuxSession() string {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.tmuxSession
}

func (e *Subprocess) writeMCPConfig() error {
	botHQPath, err := os.Executable()
	if err != nil {
		botHQPath, err = exec.LookPath("bot-hq")
		if err != nil {
			return fmt.Errorf("cannot find bot-hq binary: %w", err)
		}
	}
	config := map[string]any{
		"mcpServers": map[string]any{
			"bot-hq": map[string]any{
				"command": botHQPath,
				"args":    []string{"mcp"},
			},
		},
	}
	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return err
	}
	configPath := filepath.Join(e.workDir, ".bot-hq-emma-mcp.json")
	return os.WriteFile(configPath, data, 0600)
}

// newSessionArgs returns the `tmux new-session` argument list. Includes
// BOT_HQ_AGENT_ID env-injection and agent_model_configs DeepSeek-V4-Pro
// env-swap (ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN + ANTHROPIC_MODEL)
// via BuildSpawnEnv. Extracted for unit tests.
func (e *Subprocess) newSessionArgs() []string {
	args := []string{
		"new-session", "-d", "-s", e.tmuxSession,
		"-c", e.workDir, "-x", "200", "-y", "50",
		"-e", "BOT_HQ_AGENT_ID=" + agentID,
	}
	args = append(args, e.modelConfigEnvArgs()...)
	return args
}

// modelConfigEnvArgs resolves emma's agent_model_configs row + returns the
// `-e KEY=VALUE` arg pairs for tmux. Returns empty when no row exists or
// Claude OAuth path applies. Logs but does not abort on resolver errors —
// emma falls through to default Claude path so the pane still spawns
// (degraded: replies via brian's OAuth instead of DeepSeek, observable
// via the REDACTED-env warn-log).
func (e *Subprocess) modelConfigEnvArgs() []string {
	if e.db == nil {
		return nil
	}
	cfg, err := e.db.GetAgentModelConfig(agentID)
	if err != nil {
		if errors.Is(err, hub.ErrAgentModelConfigNotFound) {
			return nil
		}
		log.Printf("emma: agent_model_config load failed for %s: %v (falling through to default)", agentID, err)
		return nil
	}
	envs, err := agentconfig.BuildSpawnEnv(cfg)
	if err != nil {
		log.Printf("emma: BuildSpawnEnv failed for %s: %v (config provider=%s ref=%s); falling through to default", agentID, err, cfg.Provider, cfg.AuthSecretRef)
		return nil
	}
	if len(envs) > 0 {
		names := make([]string, 0, len(envs))
		for _, ev := range envs {
			names = append(names, ev.String())
		}
		log.Printf("emma: agent_model_config injecting env-vars (Z-9d R51 + R52): provider=%s model=%s vars=[%s]", cfg.Provider, cfg.ModelName, strings.Join(names, ", "))
	}
	return agentconfig.FormatTmuxEnvArgs(envs)
}

func (e *Subprocess) spawnTmux() error {
	createCmd := exec.Command("tmux", e.newSessionArgs()...)
	if err := createCmd.Run(); err != nil {
		return fmt.Errorf("tmux new-session: %w", err)
	}

	configPath := filepath.Join(e.workDir, ".bot-hq-emma-mcp.json")
	claudeCmd := fmt.Sprintf("claude --mcp-config %s --dangerously-skip-permissions", configPath)
	if err := tmuxpkg.SendKeys(e.tmuxSession, claudeCmd, true); err != nil {
		return fmt.Errorf("tmux send claude cmd: %w", err)
	}

	time.Sleep(3 * time.Second)

	if err := tmuxpkg.SendKeys(e.tmuxSession, e.initialPrompt(), true); err != nil {
		return fmt.Errorf("tmux send prompt: %w", err)
	}
	return nil
}

// InitialPromptForTest exposes initialPrompt() to cross-package tests.
func (e *Subprocess) InitialPromptForTest() string { return e.initialPrompt() }

// initialPrompt returns the system prompt that tells Claude Code how to be
// Emma. Persona anchored to vision.md + architecture/emma-role.md: hub
// orchestrator, global, stateless. Does NOT participate in BRAIN-cycle,
// does NOT hold state, does NOT elevate. Looks at CL, points BRAIN-duo,
// manages sessions, brainstorms with user.
func (e *Subprocess) initialPrompt() string {
	return `You are Emma (agent ID "emma"), the hub orchestrator of bot-hq. Global, stateless, on DeepSeek-V4-Pro per vision.md. Agents in scope: Brian (HANDS half of BRAIN-duo, ID "brian"), Rain (EYES half of BRAIN-duo, ID "rain"), Clive (voice, ID "clive").

ROLE (vision.md verbatim): you look at the Context Library (CL at ~/.bot-hq/), point BRAIN-duo at relevant areas, manage session lifecycle (open/close), and brainstorm new project ideation with the user. You DO NOT participate in BRAIN-cycle. You DO NOT hold state. You DO NOT elevate — BRAIN-duo (Brian + Rain) owns [HR]/Flag/Tag elevation. Decline + redirect anything outside this scope.

STARTUP: 1) hub_register id="emma", name="Emma", type="emma". 2) Announce online via hub_send broadcast. Do NOT iterate hub_read for catch-up — the daemon's pollLoop feeds you incoming messages directly into this pane.

REPLAY-CUTOFF: hub_register returns current_max_msg_id. Treat as replay-cutoff watermark — silently discard any incoming hub message with msg.ID <= current_max_msg_id.

WHEN YOU'RE INVOKED:
- ` + "`@emma`" + ` mention or directed message → respond as the hub orchestrator. Look at CL pointers (vision.md, project README/INDEX, phase docs, ratchets/active.md) before answering substantively.
- User brainstorms new project ideation → engage; reflect their thinking back; surface trade-offs; defer to user decision.
- BRAIN-duo asks for cross-session context / CL navigation → consult CL, give them the pointer (file path + section), don't restate verbatim.
- Session lifecycle ask → use hub_session_open / hub_session_finalize via MCP.

WHAT YOU DON'T DO:
- BRAIN-cycle: no IPAV/IPIV participation, no apply, no verify, no plan-merge. That's Brian + Rain only.
- State: no last_state.json mutation, no memory persistence. CL holds memory.
- Elevation: never emit hub_flag or [HR] prefix. Route to Rain ("@rain — flag-worthy: ...") for elevation calls.
- Rule enforcement: dropped surface as of Z-9d. Do not parse "rule:" prefixes, do not append to custom-rules.md.

ROUTING: hub_send broadcast (ToAgent="") by default; @<agent> mention in content to target a specific peer. Replies route to sender's channel via OUTBOUND (discord→discord, clive→clive).

CL POINTERS (canonical entry points):
- ~/.bot-hq/README.md — manifest
- ~/.bot-hq/projects/bot-hq/{README,INDEX}.md — bot-hq project library
- ~/.bot-hq/projects/bot-hq/phase/ — phase scope-lock docs
- ~/.bot-hq/projects/bot-hq/architecture/ — load-bearing architecture decisions
- ~/.bot-hq/vision.md — load-bearing vision

` + protocol.IdSessionsSkillPointer + `

Start now: register, then watch for mentions.`
}

// formatEmmaNudge mirrors brian/rain nudge format for tmux-paste consistency.
// Phase R R2 (authorless [HR]/FLAG strip): MsgFlag + [HR]-prefix content
// render without sender attribution per DB-truth/render-strip split.
func formatEmmaNudge(msg protocol.Message) string {
	hasHR := strings.HasPrefix(msg.Content, "[HR] ") || strings.HasPrefix(msg.Content, "[HR]\n")
	switch {
	case msg.Type == protocol.MsgFlag:
		return fmt.Sprintf("[HUB:FLAG] %s", msg.Content)
	case hasHR:
		return fmt.Sprintf("[HUB] %s", msg.Content)
	default:
		return fmt.Sprintf("[HUB:%s] %s", msg.FromAgent, msg.Content)
	}
}

// shouldForwardToEmma decides whether a polled message lands in emma's pane.
// Emma's scope (matches the SystemMonitor's prior emma.go routing):
//   - drop self-emits
//   - cross-session non-elevated chatter is INVISIBLE (PassesMainHubView)
//   - within main-hub-view scope: forward directed (ToAgent==emma) OR
//     broadcast-mention (@emma in content)
func shouldForwardToEmma(msg protocol.Message) bool {
	if msg.FromAgent == agentID {
		return false
	}
	if !protocol.PassesMainHubView(msg) {
		return false
	}
	if msg.ToAgent == agentID {
		return true
	}
	if msg.ToAgent == "" && protocol.MentionsAgentLenient(msg.Content, agentID, msg.SessionID) {
		return true
	}
	return false
}

func (e *Subprocess) pollLoop() {
	ticker := time.NewTicker(subprocessPollInterval)
	defer ticker.Stop()
	for {
		select {
		case <-e.stopCh:
			return
		case <-ticker.C:
			e.processNewMessages()
		}
	}
}

func (e *Subprocess) processNewMessages() {
	msgs, err := e.db.ReadMessages("", e.lastMsgID, 50)
	if err != nil {
		return
	}
	for _, msg := range msgs {
		if msg.ID > e.lastMsgID {
			e.lastMsgID = msg.ID
		}
		if !shouldForwardToEmma(msg) {
			continue
		}
		nudge := formatEmmaNudge(msg)
		if err := e.SendCommand(nudge); err != nil {
			log.Printf("emma subprocess: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
		}
	}
}

func (e *Subprocess) healthLoop() {
	ticker := time.NewTicker(subprocessHealthInterval)
	defer ticker.Stop()
	for {
		select {
		case <-e.stopCh:
			return
		case <-ticker.C:
			if !e.isTmuxAlive() {
				e.db.InsertMessage(protocol.Message{
					FromAgent: agentID,
					Type:      protocol.MsgError,
					Content:   "Emma tmux session died. Attempting restart...",
				})
				if err := e.restart(); err != nil {
					e.db.InsertMessage(protocol.Message{
						FromAgent: agentID,
						Type:      protocol.MsgError,
						Content:   fmt.Sprintf("Emma restart failed: %v", err),
					})
				}
			}
		}
	}
}

func (e *Subprocess) isTmuxAlive() bool {
	return exec.Command("tmux", "has-session", "-t", e.tmuxSession).Run() == nil
}

func (e *Subprocess) restart() error {
	e.mu.Lock()
	defer e.mu.Unlock()

	exec.Command("tmux", "kill-session", "-t", e.tmuxSession).Run()

	e.tmuxSession = fmt.Sprintf("bot-hq-emma-%d", time.Now().Unix())
	if err := e.spawnTmux(); err != nil {
		return err
	}
	e.sink = tmuxsink.New(hub.NewTmuxSinkStore(e.db), agentID, e.tmuxSession)

	e.db.UpdateAgentStatus(agentID, protocol.StatusOnline, "")
	e.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Emma hub orchestrator restarted.",
	})
	return nil
}
