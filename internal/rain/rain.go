package rain

import (
	"encoding/json"
	"fmt"
	"log"
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
	return `You are Rain (agent ID "rain"), bot-hq's adversarial QA agent. Sharp, skeptical, terse. Agents: Brian (orchestrator, ID "brian"), Clive (voice, ID "clive").

STARTUP: hub_register id="rain", name="Rain", type="qa". Then poll hub_read (no agent filter) every 5-10s.

RULES:
- FLAG FIRST, discuss second. hub_flag for: bugs, races, security issues (in agent output OR codebase), need for user input/approval, Brian disagreements, agent errors, rate limits. Never report without flagging.
- ROUTE responses to sender's channel: discord→discord, brian→brian, user→user.
- You CANNOT spawn agents or modify code. Tell Brian if work is needed.
- Review coder output with claude_read. Look for bugs, missing tests, incomplete work.
- Challenge Brian's decisions on approach, scope, edge cases, security. Stand your ground.
- When disagreeing with Brian: "Brian wants X. I think Y because Z. User decision needed." + hub_flag.
- Approve cleanly: "Looks clean." Flag precisely: what's wrong, why it matters.

DISCIPLINE (persistent, adopted 2026-04-24):
- ROLE: you verify + challenge; Brian executes. Do not draft in parallel or pre-scope his work.
- DRAFTER DRAFTS ALONE: when Brian asks you to draft X, you draft; he stops writing X. When you ask Brian to execute, you stop re-scoping.
- FLAG ONCE: don't re-flag Brian's concerns unless you disagree or need to correct.
- UNOWNED PIVOTS: when user pivots without naming an executor, hold 60s. Brian flags first; if he hasn't acked in 60s, step in.
- VERIFY BEFORE TRUSTING: when Brian reports a coder dispatched/completed, spot-check via git state or claude_read. No claim-based sign-offs.

Start now: register, then watch everything.`
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
	return fmt.Sprintf("[Hub message from %s]: %s\n\nIMPORTANT: After completing your current task, you MUST address the user's message above. Do not ignore it.", from, content)
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

		// Skip own messages
		if msg.FromAgent == agentID {
			continue
		}

		// Messages addressed directly to rain — always forward
		if msg.ToAgent == agentID {
			nudge := formatRainNudge(msg.FromAgent, msg.Content)
			if err := r.SendCommand(nudge); err != nil {
				log.Printf("rain: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
			}
			continue
		}

		// Broadcast observations — filter to only high-value messages
		if msg.ToAgent == "" {
			// Always forward messages from/to user
			if msg.FromAgent == "user" || msg.ToAgent == "user" {
				nudge := formatRainNudge(msg.FromAgent, msg.Content)
				if err := r.SendCommand(nudge); err != nil {
					log.Printf("rain: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
				}
				continue
			}
			// Forward results, errors, commands, and flags
			switch msg.Type {
			case protocol.MsgResult, protocol.MsgError, protocol.MsgCommand, protocol.MsgFlag:
				nudge := formatRainNudge(msg.FromAgent, msg.Content)
				if err := r.SendCommand(nudge); err != nil {
					log.Printf("rain: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
				}
				continue
			}
			// Forward messages mentioning hub_flag or hub_spawn
			if strings.Contains(msg.Content, "hub_flag") || strings.Contains(msg.Content, "hub_spawn") {
				nudge := formatRainNudge(msg.FromAgent, msg.Content)
				if err := r.SendCommand(nudge); err != nil {
					log.Printf("rain: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
				}
			}
			// Skip everything else (acks, handshakes, "Standing by" responses)
			continue
		}

		// Directed inter-agent messages (to != rain, to != "") — filter by type
		// Rain needs to see coder results, errors, flags, and commands for QA
		if msg.FromAgent == "user" || msg.ToAgent == "user" {
			observe := fmt.Sprintf("[Hub traffic %s → %s]: %s", msg.FromAgent, msg.ToAgent, msg.Content)
			if err := r.SendCommand(observe); err != nil {
				log.Printf("rain: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
			}
			continue
		}
		switch msg.Type {
		case protocol.MsgResult, protocol.MsgError, protocol.MsgCommand, protocol.MsgFlag:
			observe := fmt.Sprintf("[Hub traffic %s → %s]: %s", msg.FromAgent, msg.ToAgent, msg.Content)
			if err := r.SendCommand(observe); err != nil {
				log.Printf("rain: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
			}
		}
		// Skip acks, handshakes, and routine responses between agents
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
