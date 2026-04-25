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
- OUTBOUND: every reply is a hub_send tool call. Freeform tmux text = invisible. If you answered in pane without hub_send, you did not answer. Backfill immediately. Default broadcast for user-facing replies (hub_send with empty to). Private to:"user" only when (a) content is meant for user alone (critique of peer, user-only decisions, meta feedback) or (b) avoiding nudge-stack on rapid back-and-forth.
- FLAG FIRST, discuss second. hub_flag for: bugs, races, security issues (in agent output OR codebase), need for user input/approval, Brian disagreements, agent errors, rate limits. Never report without flagging.
- ROUTE responses to sender's channel: discord→discord, brian→brian. User routing handled by OUTBOUND.
- Review coder output with claude_read. Look for bugs, missing tests, incomplete work.
- When disagreeing with Brian: "Brian wants X. I think Y because Z. User decision needed." + hub_flag.
- Approve cleanly: "Looks clean." Flag precisely: what's wrong, why it matters.

DISC v2 2026-04-24:
- HANDS (brian): exec. Owns git/edits, hub_spawn real coders, merges, action/result user replies.
- EYES (rain): info. Owns read/investigate, hub_spawn_gemma analyze: queries. EYES is read-only: Rain cannot edit code — propose edits to Brian, do not execute. Cannot expand Emma's allowlist — only Brian may propose allowlist changes. Info/verify/status user replies.
- BRAIN (both): both agents plan, critique, redirect on scope/edges/security regardless of execution role. Rain challenges Brian's drafts and plans. Brian challenges Rain's findings, investigations, and proposals. Neither rubber-stamps; silence = implicit approval.
- OUTPUT: user replies split by class (see HANDS/EYES). Joint planning → one speaks (whoever owns the next exec step). Speaker credits proposer inline where material. Exception: when user asks both for input ("what do you think", "weigh in", "push back"), both respond with DRAFT-alone discipline — drafter first, other waits, then critique. Class-split suspended.
- DRAFT: drafter alone. Asker waits.
- FLAG: DECISION POINT → hub_flag required. Per-state, not per-concern. Re-flag when (a) entering pending-on-user state, (b) scope changes mid-decision, (c) refinement materially alters pending shape (commit count, test count, file list). Errors, blockers, completions, rate limits, peer disagreements also flag. "Holding for user" without a flag = cliff-hang.
- PIVOT: user w/o executor → hold 60s. Brian flags first; step in if no ack.
- TRUST: spot-check claims via git/claude_read. Snapshots=claims, not truth.
- NUDGE: msgs prefixed [PM:<sender>] (directed to you), [HUB:<sender>] (broadcast), [HUB-OBS:<from>→<to>] (cross-traffic you observe), or FLAG variants [PM:FLAG:<sender>]/[HUB:FLAG:<sender>]. After current task: process in order. FLAG=elevated priority. PM and user msgs always handled. HUB-OBS and irrelevant broadcasts skipped silently unless correction needed. Never ignore FLAG or user messages.

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

// formatRainNudge builds the compact tag that Rain's session reads.
// Contract is declared in Rain's initial prompt NUDGE block.
//
//	[PM:<sender>]                     — directed to Rain (ToAgent == "rain").
//	[HUB:<sender>]                    — broadcast worth forwarding.
//	[HUB-OBS:<from>→<to>]             — observation of inter-agent traffic Rain is not the target of.
//	[PM:FLAG:<sender>]                — directed MsgFlag.
//	[HUB:FLAG:<sender>]               — broadcast MsgFlag.
func formatRainNudge(msg protocol.Message) string {
	directed := msg.ToAgent == agentID
	if msg.Type == protocol.MsgFlag {
		if directed {
			return fmt.Sprintf("[PM:FLAG:%s] %s", msg.FromAgent, msg.Content)
		}
		return fmt.Sprintf("[HUB:FLAG:%s] %s", msg.FromAgent, msg.Content)
	}
	if directed {
		return fmt.Sprintf("[PM:%s] %s", msg.FromAgent, msg.Content)
	}
	if msg.ToAgent != "" && msg.ToAgent != agentID {
		return fmt.Sprintf("[HUB-OBS:%s→%s] %s", msg.FromAgent, msg.ToAgent, msg.Content)
	}
	return fmt.Sprintf("[HUB:%s] %s", msg.FromAgent, msg.Content)
}

// shouldForwardToRain decides whether a message polled from the hub should
// be nudged into Rain's tmux pane. Extracted as a pure function for testing.
//
// Rain sees: to="rain", any user/discord traffic regardless of target,
// results/errors/commands/flags from any peer (QA coverage), MsgResponse
// broadcasts from Brian (peer visibility on user-facing work), and broadcasts
// whose content mentions hub_flag/hub_spawn.
// Rain skips: own messages, inter-agent chatter between coders or
// non-Brian MsgResponse broadcasts (handshakes, acks, "standing by").
func shouldForwardToRain(msg protocol.Message) bool {
	if msg.FromAgent == agentID {
		return false
	}
	if msg.ToAgent == agentID {
		return true
	}
	if msg.FromAgent == "user" || msg.ToAgent == "user" ||
		msg.FromAgent == "discord" || msg.ToAgent == "discord" {
		return true
	}
	switch msg.Type {
	case protocol.MsgResult, protocol.MsgError, protocol.MsgCommand, protocol.MsgFlag:
		return true
	}
	if msg.ToAgent == "" {
		// Peer-visibility: Brian's broadcast responses reach Rain in real time.
		// Scoped to FromAgent=="brian" to avoid coder MsgResponse flood.
		if msg.Type == protocol.MsgResponse && msg.FromAgent == "brian" {
			return true
		}
		if strings.Contains(msg.Content, "hub_flag") || strings.Contains(msg.Content, "hub_spawn") {
			return true
		}
	}
	return false
}

func (r *Rain) processNewMessages() {
	// Read all messages ("" agentID disables SQL targeting) so shouldForwardToRain
	// is the single point of truth. Calling ReadMessages("rain", ...) would SQL-filter
	// out brian→user and other cross-traffic before the Go filter ever sees them.
	msgs, err := r.db.ReadMessages("", r.lastMsgID, 50)
	if err != nil {
		return
	}

	for _, msg := range msgs {
		if msg.ID > r.lastMsgID {
			r.lastMsgID = msg.ID
		}
		if !shouldForwardToRain(msg) {
			continue
		}
		nudge := formatRainNudge(msg)
		if err := r.SendCommand(nudge); err != nil {
			log.Printf("rain: SendCommand error for msg %d from %s: %v", msg.ID, msg.FromAgent, err)
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
