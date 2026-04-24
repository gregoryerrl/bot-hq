package brian

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	agentID   = "brian"
	agentName = "Brian"
	agentType = protocol.AgentBrian

	pollInterval   = 3 * time.Second
	healthInterval = 30 * time.Second
)

// Brian manages a Claude Code session that acts as the master orchestrator.
// It spawns in tmux, registers as an agent, and polls for user messages.
type Brian struct {
	db          *hub.DB
	workDir     string
	tmuxSession string
	lastMsgID   int64

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}
}

// New creates a Brian instance. workDir is where the Claude Code session runs.
func New(db *hub.DB, workDir string) *Brian {
	if workDir == "" {
		home, _ := os.UserHomeDir()
		workDir = filepath.Join(home, "Projects")
	}
	return &Brian{
		db:      db,
		workDir: workDir,
		stopCh:  make(chan struct{}),
	}
}

// Start spawns the Claude Code session in tmux and begins polling for messages.
func (b *Brian) Start() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.running {
		return nil
	}

	// Ensure work directory exists
	if err := os.MkdirAll(b.workDir, 0700); err != nil {
		return fmt.Errorf("brian work dir: %w", err)
	}

	// Write MCP config for the brian session
	if err := b.writeMCPConfig(); err != nil {
		return fmt.Errorf("brian mcp config: %w", err)
	}

	// Generate a unique tmux session name
	b.tmuxSession = fmt.Sprintf("bot-hq-brian-%d", time.Now().Unix())

	// Spawn tmux session with Claude Code
	if err := b.spawnTmux(); err != nil {
		return fmt.Errorf("brian tmux spawn: %w", err)
	}

	// Register brian agent in the hub
	agent := protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   agentType,
		Status: protocol.StatusOnline,
	}
	if err := b.db.RegisterAgent(agent); err != nil {
		return fmt.Errorf("brian register: %w", err)
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
		Content:   "Brian orchestrator online. Ready for commands.",
	})

	return nil
}

// Stop shuts down the brian session.
func (b *Brian) Stop() {
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

// IsRunning returns whether the brian is active.
func (b *Brian) IsRunning() bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.running
}

// SendCommand sends a user command to the brian's Claude Code session via tmux.
func (b *Brian) SendCommand(text string) error {
	b.mu.Lock()
	if !b.running {
		b.mu.Unlock()
		return fmt.Errorf("brian is not running")
	}
	session := b.tmuxSession
	b.mu.Unlock()

	// No need to echo the nudge back into the hub — Brian's Claude Code
	// session will respond via hub_send which creates its own message.

	// Send the text to the tmux pane
	cmd := exec.Command("tmux", "send-keys", "-t", session, "-l", text)
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("tmux send: %w", err)
	}
	// Claude Code's bracketed paste needs time to process before Enter
	time.Sleep(500 * time.Millisecond)
	return exec.Command("tmux", "send-keys", "-t", session, "Enter").Run()
}

// writeMCPConfig creates a temporary MCP config file that gives the brian
// access to all bot-hq hub tools.
func (b *Brian) writeMCPConfig() error {
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

	configPath := filepath.Join(b.workDir, ".bot-hq-brian-mcp.json")
	return os.WriteFile(configPath, data, 0600)
}

// spawnTmux creates a new tmux session running Claude Code with the brian prompt.
func (b *Brian) spawnTmux() error {
	// Create detached tmux session
	createCmd := exec.Command("tmux", "new-session", "-d", "-s", b.tmuxSession,
		"-c", b.workDir, "-x", "200", "-y", "50")
	if err := createCmd.Run(); err != nil {
		return fmt.Errorf("tmux new-session: %w", err)
	}

	// Build the claude command with MCP config
	configPath := filepath.Join(b.workDir, ".bot-hq-brian-mcp.json")
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

	// Send the initial brian prompt
	prompt := b.initialPrompt()
	sendPrompt := exec.Command("tmux", "send-keys", "-t", b.tmuxSession, "-l", prompt)
	if err := sendPrompt.Run(); err != nil {
		return fmt.Errorf("tmux send prompt: %w", err)
	}
	// Claude Code's bracketed paste needs time to process before Enter
	time.Sleep(500 * time.Millisecond)
	return exec.Command("tmux", "send-keys", "-t", b.tmuxSession, "Enter").Run()
}

// initialPrompt returns the system prompt that tells Claude how to be the brian.
func (b *Brian) initialPrompt() string {
	return `You are Brian (agent ID "brian"), the bot-hq orchestrator. Agents: Clive (voice, ID "clive"), Rain (QA, ID "rain").

STARTUP: 1) hub_read to catch up. 2) hub_flag anything needing user attention. 3) hub_register id="brian", name="Brian", type="brian". 4) Announce online.

RULES:
- ALWAYS FLAG. When in doubt, flag. Errors, blocked tasks, completions, rate limits, Rain disagreements, need for user input — hub_flag immediately. Never go idle without flagging.
- DISPATCH via hub_spawn only (never Agent tool). Send handshake + hub_session_create after spawning.
- ROUTE responses to the sender's channel: discord→discord, clive→clive, user→user.
- Rain challenges your decisions — engage critically. Stand your ground if right, adjust if wrong. Escalate via hub_flag only if unresolved.
- Messages arrive automatically. Don't poll hub_read in a loop.
- Questions: hub_send response. Tasks: hub_spawn a coder. Routing: hub_send to target agent.

Start now: follow STARTUP.`
}

// pollLoop checks for new messages directed at the brian and forwards them
// to the Claude session via tmux.
func (b *Brian) pollLoop() {
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
// The reply target matches the sender so responses route back through the same channel.
func formatNudge(from, content string) string {
	return fmt.Sprintf("[Hub message from %s]: %s\n\nIMPORTANT: After completing your current task, you MUST address the user's message above. Do not ignore it.", from, content)
}

// processNewMessages checks for user commands that arrived since the last poll
// and sends them to the brian's Claude session as a single batched nudge.
// Brian sees: to="brian", to="" (broadcasts).
// Brian skips: own messages, messages to other specific agents (including to="user").
func (b *Brian) processNewMessages() {
	msgs, err := b.db.ReadMessages("", b.lastMsgID, 50)
	if err != nil {
		return
	}

	var pending []string
	for _, msg := range msgs {
		if msg.ID > b.lastMsgID {
			b.lastMsgID = msg.ID
		}

		// Skip own messages
		if msg.FromAgent == agentID {
			continue
		}

		// Skip messages to other specific agents (not brian, not broadcast)
		if msg.ToAgent != "" && msg.ToAgent != agentID {
			continue
		}

		pending = append(pending, fmt.Sprintf("[Hub message from %s]: %s", msg.FromAgent, msg.Content))
	}

	if len(pending) == 0 {
		return
	}

	// Batch all messages into a single nudge
	combined := strings.Join(pending, "\n\n")
	nudge := fmt.Sprintf("%s\n\nIMPORTANT: After completing your current task, you MUST address ALL messages above. Do not ignore any.", combined)
	b.SendCommand(nudge)
}

// healthLoop periodically checks if the tmux session is still alive.
func (b *Brian) healthLoop() {
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
					Content:   "Brian tmux session died. Attempting restart...",
				})
				// Try to restart
				if err := b.restart(); err != nil {
					b.db.InsertMessage(protocol.Message{
						FromAgent: agentID,
						Type:      protocol.MsgError,
						Content:   fmt.Sprintf("Brian restart failed: %v", err),
					})
				}
			}
		}
	}
}

// isTmuxAlive checks if the brian's tmux session exists.
func (b *Brian) isTmuxAlive() bool {
	return exec.Command("tmux", "has-session", "-t", b.tmuxSession).Run() == nil
}

// restart recreates the tmux session.
func (b *Brian) restart() error {
	b.mu.Lock()
	defer b.mu.Unlock()

	// Kill old session if it exists
	exec.Command("tmux", "kill-session", "-t", b.tmuxSession).Run()

	// Respawn
	b.tmuxSession = fmt.Sprintf("bot-hq-brian-%d", time.Now().Unix())
	if err := b.spawnTmux(); err != nil {
		return err
	}

	b.db.UpdateAgentStatus(agentID, protocol.StatusOnline, "")
	b.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Brian orchestrator restarted successfully.",
	})
	return nil
}
