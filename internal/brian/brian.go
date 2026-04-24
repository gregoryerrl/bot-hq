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
- OUTBOUND: every reply is a hub_send tool call. Freeform tmux text = invisible. If you answered in pane without hub_send, you did not answer. Backfill immediately.
- ALWAYS FLAG. When in doubt, flag. Errors, blocked tasks, completions, rate limits, Rain disagreements, need for user input — hub_flag immediately. Never go idle without flagging.
- DISPATCH via hub_spawn only (never Agent tool). Send handshake + hub_session_create after spawning.
- ROUTE responses to the sender's channel: discord→discord, clive→clive, user→user.
- Messages arrive automatically. Don't poll hub_read in a loop.
- Questions: hub_send response. Tasks: hub_spawn a coder. Routing: hub_send to target agent.

DISC v2 2026-04-24:
- HANDS (brian): exec. Owns git/edits, hub_spawn real coders, merges, action/result user replies.
- EYES (rain): info. Owns read/investigate, hub_spawn_gemma analyze: queries. EYES is read-only: Rain cannot edit code — propose edits to Brian, do not execute. Cannot expand Emma's allowlist — only Brian may propose allowlist changes. Info/verify/status user replies.
- BRAIN (both): both agents plan, critique, redirect on scope/edges/security regardless of execution role. Rain challenges Brian's drafts and plans. Brian challenges Rain's findings, investigations, and proposals. Neither rubber-stamps; silence = implicit approval.
- OUTPUT: user replies split by class (see HANDS/EYES). Joint planning → one speaks (whoever owns the next exec step). Speaker credits proposer inline where material. Exception: when user asks both for input ("what do you think", "weigh in", "push back"), both respond with DRAFT-alone discipline — drafter first, other waits, then critique. Class-split suspended.
- DRAFT: drafter alone. Asker waits.
- FLAG: 1 concern=1 flag. No re-flag unless disagree/correct.
- PIVOT: user w/o executor → brian flags, rain holds 60s.
- TRUST: verify via claude_read before "dispatched" claim. Prefer one-shot spawn.
- SNAP (multi-artifact dispatch/verify):
    Branches: repo:branch@sha(state),...
    Agents:   brian(s), rain(s), emma(s), coder id(s),...
    Pending:  <blocker>
    Next:     <action>
- NUDGE: msgs prefixed [HUB:<sender>] or [HUB:FLAG:<sender>]. After current task: process in order. FLAG=elevated priority. Irrelevant broadcasts skipped silently. Never ignore.

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

// formatNudge builds the compact tag that Brian's session reads.
// Contract is declared in Brian's initial prompt DISCIPLINE block.
//
//	[HUB:<sender>]            — directed to Brian or broadcast.
//	[HUB:FLAG:<sender>]       — MsgFlag-typed; elevated priority.
func formatNudge(msg protocol.Message) string {
	if msg.Type == protocol.MsgFlag {
		return fmt.Sprintf("[HUB:FLAG:%s] %s", msg.FromAgent, msg.Content)
	}
	return fmt.Sprintf("[HUB:%s] %s", msg.FromAgent, msg.Content)
}

// shouldForwardToBrian decides whether a message polled from the hub should
// be nudged into Brian's tmux pane. Extracted as a pure function for testing.
//
// Brian sees: to="brian", to="" (broadcasts), and any user/discord traffic
// regardless of target — so Brian observes Rain's to="user" replies and the
// mirror case for Rain (see rain.go:319-325).
// Brian skips: own messages, inter-agent chatter not involving user/discord.
func shouldForwardToBrian(msg protocol.Message) bool {
	if msg.FromAgent == agentID {
		return false
	}
	if msg.FromAgent == "user" || msg.ToAgent == "user" ||
		msg.FromAgent == "discord" || msg.ToAgent == "discord" {
		return true
	}
	if msg.ToAgent != "" && msg.ToAgent != agentID {
		return false
	}
	return true
}

// processNewMessages checks for new messages and nudges Brian via tmux.
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
		if shouldForwardToBrian(msg) {
			pending = append(pending, formatNudge(msg))
		}
	}

	if len(pending) == 0 {
		return
	}

	// Batch all messages into a single nudge. Each line carries its own
	// [HUB:<sender>] tag; the NUDGE contract in the initial prompt covers
	// "process in order after current task", so no trailing IMPORTANT block.
	nudge := strings.Join(pending, "\n")
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
