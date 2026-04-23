package brain

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	agentID   = "brain"
	agentName = "Brain"
	agentType = protocol.AgentBrain

	pollInterval   = 3 * time.Second
	healthInterval = 30 * time.Second
)

// Brain manages a Claude Code session that acts as the master orchestrator.
// It spawns in tmux, registers as an agent, and polls for user messages.
type Brain struct {
	db          *hub.DB
	workDir     string
	tmuxSession string
	lastMsgID   int64

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}
}

// New creates a Brain instance. workDir is where the Claude Code session runs.
func New(db *hub.DB, workDir string) *Brain {
	if workDir == "" {
		home, _ := os.UserHomeDir()
		workDir = filepath.Join(home, "Projects")
	}
	return &Brain{
		db:      db,
		workDir: workDir,
		stopCh:  make(chan struct{}),
	}
}

// Start spawns the Claude Code session in tmux and begins polling for messages.
func (b *Brain) Start() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.running {
		return nil
	}

	// Ensure work directory exists
	if err := os.MkdirAll(b.workDir, 0700); err != nil {
		return fmt.Errorf("brain work dir: %w", err)
	}

	// Write MCP config for the brain session
	if err := b.writeMCPConfig(); err != nil {
		return fmt.Errorf("brain mcp config: %w", err)
	}

	// Generate a unique tmux session name
	b.tmuxSession = fmt.Sprintf("bot-hq-brain-%d", time.Now().Unix())

	// Spawn tmux session with Claude Code
	if err := b.spawnTmux(); err != nil {
		return fmt.Errorf("brain tmux spawn: %w", err)
	}

	// Register brain agent in the hub
	agent := protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   agentType,
		Status: protocol.StatusOnline,
	}
	if err := b.db.RegisterAgent(agent); err != nil {
		return fmt.Errorf("brain register: %w", err)
	}

	// Get current last message ID so we only process new messages
	msgs, err := b.db.GetRecentMessages(1)
	if err == nil && len(msgs) > 0 {
		b.lastMsgID = msgs[0].ID
	}

	b.running = true

	// Start background polling
	go b.pollLoop()
	go b.healthLoop()

	// Announce
	b.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Brain orchestrator online. Ready for commands.",
	})

	return nil
}

// Stop shuts down the brain session.
func (b *Brain) Stop() {
	b.mu.Lock()
	defer b.mu.Unlock()
	if !b.running {
		return
	}
	b.running = false
	close(b.stopCh)

	// Update agent status
	b.db.UpdateAgentStatus(agentID, protocol.StatusOffline, "")

	// Kill the tmux session
	exec.Command("tmux", "kill-session", "-t", b.tmuxSession).Run()
}

// IsRunning returns whether the brain is active.
func (b *Brain) IsRunning() bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.running
}

// SendCommand sends a user command to the brain's Claude Code session via tmux.
func (b *Brain) SendCommand(text string) error {
	b.mu.Lock()
	if !b.running {
		b.mu.Unlock()
		return fmt.Errorf("brain is not running")
	}
	session := b.tmuxSession
	b.mu.Unlock()

	// Record in hub messages so TUI can display brain→claude exchanges
	b.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		ToAgent:   "claude-session",
		Type:      protocol.MsgCommand,
		Content:   text,
	})

	// Send the text to the tmux pane
	cmd := exec.Command("tmux", "send-keys", "-t", session, "-l", text)
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("tmux send: %w", err)
	}
	// Press Enter
	return exec.Command("tmux", "send-keys", "-t", session, "Enter").Run()
}

// writeMCPConfig creates a temporary MCP config file that gives the brain
// access to all bot-hq hub tools.
func (b *Brain) writeMCPConfig() error {
	botHQPath, err := os.Executable()
	if err != nil {
		// Fall back to looking in PATH
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

	configPath := filepath.Join(b.workDir, ".bot-hq-brain-mcp.json")
	return os.WriteFile(configPath, data, 0600)
}

// spawnTmux creates a new tmux session running Claude Code with the brain prompt.
func (b *Brain) spawnTmux() error {
	// Create detached tmux session
	createCmd := exec.Command("tmux", "new-session", "-d", "-s", b.tmuxSession,
		"-c", b.workDir, "-x", "200", "-y", "50")
	if err := createCmd.Run(); err != nil {
		return fmt.Errorf("tmux new-session: %w", err)
	}

	// Build the claude command with MCP config
	configPath := filepath.Join(b.workDir, ".bot-hq-brain-mcp.json")
	claudeCmd := fmt.Sprintf("claude --mcp-config %s --dangerously-skip-permissions", configPath)

	// Send the claude command to the tmux session
	sendCmd := exec.Command("tmux", "send-keys", "-t", b.tmuxSession, "-l", claudeCmd)
	if err := sendCmd.Run(); err != nil {
		return fmt.Errorf("tmux send claude cmd: %w", err)
	}
	if err := exec.Command("tmux", "send-keys", "-t", b.tmuxSession, "Enter").Run(); err != nil {
		return fmt.Errorf("tmux send enter: %w", err)
	}

	// Wait for Claude to initialize
	time.Sleep(3 * time.Second)

	// Send the initial brain prompt
	prompt := b.initialPrompt()
	sendPrompt := exec.Command("tmux", "send-keys", "-t", b.tmuxSession, "-l", prompt)
	if err := sendPrompt.Run(); err != nil {
		return fmt.Errorf("tmux send prompt: %w", err)
	}
	return exec.Command("tmux", "send-keys", "-t", b.tmuxSession, "Enter").Run()
}

// initialPrompt returns the system prompt that tells Claude how to be the brain.
func (b *Brain) initialPrompt() string {
	return `You are the Brain orchestrator for bot-hq. You have access to bot-hq MCP tools.

Your responsibilities:
1. Register yourself: call hub_register with id="brain", name="Brain", type="brain"
2. Monitor messages: periodically call hub_read with agent_id="brain" to check for new messages
3. When you see messages from "user" (type="command"), respond helpfully:
   - If it's a question, answer it using hub_send (from="brain", to="user", type="response")
   - If it's a task like "spawn project X", use hub_spawn or other tools
   - If it's a message for another agent, route it with hub_send
4. Keep your status updated with hub_status

Start now: register yourself, then enter a loop where you check for messages every 5-10 seconds using hub_read. Always respond to user commands.`
}

// pollLoop checks for new messages directed at the brain and forwards them
// to the Claude session via tmux.
func (b *Brain) pollLoop() {
	ticker := time.NewTicker(pollInterval)
	defer ticker.Stop()

	for {
		select {
		case <-b.stopCh:
			return
		case <-ticker.C:
			b.processNewMessages()
		}
	}
}

// formatNudge creates a nudge message that includes the actual content,
// so Claude doesn't need to call hub_read for every user message.
func formatNudge(from, content string) string {
	return fmt.Sprintf("[Hub message from %s]: %s\n\nRespond to this using hub_send (from=\"brain\", to=\"user\", type=\"response\").", from, content)
}

// processNewMessages checks for user commands that arrived since the last poll
// and sends them to the brain's Claude session.
func (b *Brain) processNewMessages() {
	msgs, err := b.db.ReadMessages("", b.lastMsgID, 50)
	if err != nil {
		return
	}

	for _, msg := range msgs {
		if msg.ID > b.lastMsgID {
			b.lastMsgID = msg.ID
		}

		// Only forward user commands to the brain session
		if msg.FromAgent == "user" && msg.Type == protocol.MsgCommand {
			nudge := formatNudge(msg.FromAgent, msg.Content)
			b.SendCommand(nudge)
		}
	}
}

// healthLoop periodically checks if the tmux session is still alive.
func (b *Brain) healthLoop() {
	ticker := time.NewTicker(healthInterval)
	defer ticker.Stop()

	for {
		select {
		case <-b.stopCh:
			return
		case <-ticker.C:
			if !b.isTmuxAlive() {
				b.db.InsertMessage(protocol.Message{
					FromAgent: agentID,
					Type:      protocol.MsgError,
					Content:   "Brain tmux session died. Attempting restart...",
				})
				// Try to restart
				if err := b.restart(); err != nil {
					b.db.InsertMessage(protocol.Message{
						FromAgent: agentID,
						Type:      protocol.MsgError,
						Content:   fmt.Sprintf("Brain restart failed: %v", err),
					})
				}
			}
		}
	}
}

// isTmuxAlive checks if the brain's tmux session exists.
func (b *Brain) isTmuxAlive() bool {
	return exec.Command("tmux", "has-session", "-t", b.tmuxSession).Run() == nil
}

// restart recreates the tmux session.
func (b *Brain) restart() error {
	b.mu.Lock()
	defer b.mu.Unlock()

	// Kill old session if it exists
	exec.Command("tmux", "kill-session", "-t", b.tmuxSession).Run()

	// Respawn
	b.tmuxSession = fmt.Sprintf("bot-hq-brain-%d", time.Now().Unix())
	if err := b.spawnTmux(); err != nil {
		return err
	}

	b.db.UpdateAgentStatus(agentID, protocol.StatusOnline, "")
	b.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Brain orchestrator restarted successfully.",
	})
	return nil
}
