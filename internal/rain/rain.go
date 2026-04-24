package rain

import (
	"encoding/json"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	agentID   = "rain"
	agentName = "Rain"
	agentType = protocol.AgentQA

	pollInterval   = 3 * time.Second
	healthInterval = 30 * time.Second
)

// Rain manages a Claude Code session that acts as the adversarial QA reviewer.
// It watches all hub activity, challenges Brian's decisions, reviews agent output,
// and flags the user when attention is needed.
type Rain struct {
	db          *hub.DB
	workDir     string
	tmuxSession string
	lastMsgID   int64

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}
}

// New creates a Rain instance. workDir is where the Claude Code session runs.
func New(db *hub.DB, workDir string) *Rain {
	if workDir == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			home = os.TempDir()
		}
		workDir = filepath.Join(home, "Projects")
	}
	return &Rain{
		db:      db,
		workDir: workDir,
		stopCh:  make(chan struct{}),
	}
}

// Start spawns the Claude Code session in tmux and begins polling for messages.
func (r *Rain) Start() error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.running {
		return nil
	}

	if err := os.MkdirAll(r.workDir, 0700); err != nil {
		return fmt.Errorf("rain work dir: %w", err)
	}

	if err := r.writeMCPConfig(); err != nil {
		return fmt.Errorf("rain mcp config: %w", err)
	}

	r.tmuxSession = fmt.Sprintf("bot-hq-rain-%d", time.Now().Unix())

	if err := r.spawnTmux(); err != nil {
		return fmt.Errorf("rain tmux spawn: %w", err)
	}

	agent := protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   agentType,
		Status: protocol.StatusOnline,
	}
	if err := r.db.RegisterAgent(agent); err != nil {
		return fmt.Errorf("rain register: %w", err)
	}

	msgs, err := r.db.GetRecentMessages(1)
	if err == nil && len(msgs) > 0 {
		r.lastMsgID = msgs[0].ID
	}

	r.running = true

	go r.pollLoop()
	go r.healthLoop()

	r.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Rain QA online. Watching.",
	})

	return nil
}

// Stop shuts down the Rain session.
func (r *Rain) Stop() {
	r.mu.Lock()
	defer r.mu.Unlock()
	if !r.running {
		return
	}
	r.running = false
	close(r.stopCh)

	r.db.UpdateAgentStatus(agentID, protocol.StatusOffline, "")
	exec.Command("tmux", "kill-session", "-t", r.tmuxSession).Run()
}

// IsRunning returns whether Rain is active.
func (r *Rain) IsRunning() bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.running
}

// SendCommand sends text to Rain's Claude Code session via tmux.
func (r *Rain) SendCommand(text string) error {
	r.mu.Lock()
	if !r.running {
		r.mu.Unlock()
		return fmt.Errorf("rain is not running")
	}
	session := r.tmuxSession
	r.mu.Unlock()

	cmd := exec.Command("tmux", "send-keys", "-t", session, "-l", text)
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("tmux send: %w", err)
	}
	time.Sleep(500 * time.Millisecond)
	return exec.Command("tmux", "send-keys", "-t", session, "Enter").Run()
}

func (r *Rain) writeMCPConfig() error {
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

	configPath := filepath.Join(r.workDir, ".bot-hq-rain-mcp.json")
	return os.WriteFile(configPath, data, 0600)
}

func (r *Rain) spawnTmux() error {
	createCmd := exec.Command("tmux", "new-session", "-d", "-s", r.tmuxSession,
		"-c", r.workDir, "-x", "200", "-y", "50")
	if err := createCmd.Run(); err != nil {
		return fmt.Errorf("tmux new-session: %w", err)
	}

	configPath := filepath.Join(r.workDir, ".bot-hq-rain-mcp.json")
	claudeCmd := fmt.Sprintf("claude --mcp-config %s --dangerously-skip-permissions", configPath)

	sendCmd := exec.Command("tmux", "send-keys", "-t", r.tmuxSession, "-l", claudeCmd)
	if err := sendCmd.Run(); err != nil {
		return fmt.Errorf("tmux send claude cmd: %w", err)
	}
	if err := exec.Command("tmux", "send-keys", "-t", r.tmuxSession, "Enter").Run(); err != nil {
		return fmt.Errorf("tmux send enter: %w", err)
	}

	time.Sleep(3 * time.Second)

	prompt := r.initialPrompt()
	sendPrompt := exec.Command("tmux", "send-keys", "-t", r.tmuxSession, "-l", prompt)
	if err := sendPrompt.Run(); err != nil {
		return fmt.Errorf("tmux send prompt: %w", err)
	}
	time.Sleep(500 * time.Millisecond)
	return exec.Command("tmux", "send-keys", "-t", r.tmuxSession, "Enter").Run()
}

func (r *Rain) initialPrompt() string {
	return `You are Rain, the adversarial QA agent for bot-hq. You have access to bot-hq MCP tools.

Your name is Rain (agent ID "rain"). You are sharp, skeptical, and direct. You don't sugarcoat. You think like a QA lead who has seen too many production incidents caused by unchecked optimism. Your job is to catch what others miss.

The orchestrator is Brian (agent ID "brain"). The voice interface is Clive (agent ID "live").

WHAT YOU CAN DO:
- hub_register: Register yourself
- hub_read: Read all hub messages (not just yours — you watch everything)
- hub_send: Send messages to agents (challenge Brian, question coders, report to user)
- hub_agents: Check who's online
- hub_sessions: Check active sessions
- hub_flag: Flag the user when their attention is needed (sends Discord notification)
- claude_read: Read the output of any Claude Code tmux session to review their work
- claude_list: List all Claude Code sessions

WHAT YOU CANNOT DO:
- You CANNOT spawn agents (hub_spawn). If work needs to be done, tell Brian.
- You CANNOT directly modify code. You review and challenge.

RESPONSE ROUTING RULE: Always route responses back through the same channel the message arrived from. If a message comes from "discord", reply with to="discord". If from "live" (Clive), reply with to="live". If from "brain", reply with to="brain". If from "user" directly, reply with to="user". This ensures replies reach the user wherever they are.

YOUR RESPONSIBILITIES:
1. Register yourself: hub_register with id="rain", name="Rain", type="qa"
2. Watch everything: poll hub_read frequently (every 5-10 seconds) with NO agent filter to see ALL messages
3. When Brian spawns agents or makes decisions, question them:
   - Is this the right approach? Are there edge cases?
   - Did Brian consider error handling, security, performance?
   - Is the scope appropriate or is Brian overcomplicating/oversimplifying?
4. When coder agents report results, review their actual output:
   - Use claude_read to check what they actually did
   - Look for bugs, missing tests, security issues, incomplete implementations
5. When you and Brian disagree, present both sides to the user:
   - Use hub_flag to notify the user via Discord
   - Be specific: "Brian wants X. I think Y because Z. User decision needed."
6. Flag the user (hub_flag) when:
   - You find ANY bug, race condition, or security issue — in agent output OR codebase
   - You need user input, approval, or a decision to proceed
   - You and Brian disagree on approach
   - An agent errors out or stops responding
   - Claude Code rate limits are hit
   - Anything that needs human judgment
   NEVER report an issue or ask a question without also flagging. If the user needs to know, flag first, discuss second.

PERSONALITY:
- Terse. No filler. Say what needs to be said.
- Skeptical by default. Trust but verify.
- When you approve something, a simple "Looks clean." is enough.
- When you flag an issue, be precise about what's wrong and why it matters.
- You respect Brian but you don't defer to him. You serve the user.

Start now: register yourself, then enter a loop where you check ALL messages every 5-10 seconds using hub_read (no agent_id filter). Watch. Question. Flag.`
}

func (r *Rain) pollLoop() {
	ticker := time.NewTicker(pollInterval)
	defer ticker.Stop()

	for {
		select {
		case <-r.stopCh:
			return
		case <-ticker.C:
			r.processNewMessages()
		}
	}
}

func formatRainNudge(from, content string) string {
	return fmt.Sprintf("[Hub message from %s]: %s\n\nIf this needs your input, respond via hub_send (from=\"rain\", to=\"%s\"). If it needs the user's attention, use hub_flag.\n\nIMPORTANT: After completing your current task, you MUST address the user's message above. Do not ignore it.", from, content, from)
}

func (r *Rain) processNewMessages() {
	msgs, err := r.db.ReadMessages(agentID, r.lastMsgID, 50)
	if err != nil {
		return
	}

	for _, msg := range msgs {
		if msg.ID > r.lastMsgID {
			r.lastMsgID = msg.ID
		}

		// Forward messages addressed to rain (from anyone except rain itself)
		if msg.FromAgent != agentID {
			nudge := formatRainNudge(msg.FromAgent, msg.Content)
			if err := r.SendCommand(nudge); err != nil {
				log.Printf("rain: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
			}
		}
	}
}

func (r *Rain) healthLoop() {
	ticker := time.NewTicker(healthInterval)
	defer ticker.Stop()

	for {
		select {
		case <-r.stopCh:
			return
		case <-ticker.C:
			if !r.isTmuxAlive() {
				r.db.InsertMessage(protocol.Message{
					FromAgent: agentID,
					Type:      protocol.MsgError,
					Content:   "Rain tmux session died. Attempting restart...",
				})
				if err := r.restart(); err != nil {
					r.db.InsertMessage(protocol.Message{
						FromAgent: agentID,
						Type:      protocol.MsgError,
						Content:   fmt.Sprintf("Rain restart failed: %v", err),
					})
				}
			}
		}
	}
}

func (r *Rain) isTmuxAlive() bool {
	return exec.Command("tmux", "has-session", "-t", r.tmuxSession).Run() == nil
}

func (r *Rain) restart() error {
	r.mu.Lock()
	defer r.mu.Unlock()

	exec.Command("tmux", "kill-session", "-t", r.tmuxSession).Run()

	r.tmuxSession = fmt.Sprintf("bot-hq-rain-%d", time.Now().Unix())
	if err := r.spawnTmux(); err != nil {
		return err
	}

	r.db.UpdateAgentStatus(agentID, protocol.StatusOnline, "")
	r.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Rain QA restarted.",
	})
	return nil
}
