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
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/gregoryerrl/bot-hq/internal/tmuxsink"
)

const (
	agentID   = "brian"
	agentName = "Brian"
	agentType = protocol.AgentBrian

	pollInterval   = 1 * time.Second
	healthInterval = 30 * time.Second

	// bufferQuietWindow is the debounce window for Phase S S-5 brian-
	// 3s message-buffer hotfix. New non-bypass-class messages append to
	// pendingBatch + (re)set lastArrivalTime; emit fires only after
	// bufferQuietWindow of no new appends. User-msgs and FLAG-class
	// MsgFlag bypass the buffer (urgency-class) and flush any pending
	// batch alongside.
	bufferQuietWindow = 3 * time.Second
)

// Brian manages a Claude Code session that acts as the master orchestrator.
// It spawns in tmux, registers as an agent, and polls for user messages.
type Brian struct {
	db          *hub.DB
	workDir     string
	tmuxSession string
	lastMsgID   int64

	// sink wraps tmux pane delivery with isReady-check + retry-queue
	// semantics shared with hub.dispatchToTmux. Constructed in Start()
	// after the tmux session spawns; rebuilt in restart() with the new
	// session name. Phase I W2 Layer-2 (c).
	sink *tmuxsink.Sink

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}

	// Phase S S-5 brian-3s message-buffer state. pendingBatch holds
	// formatted-nudge strings awaiting debounce-quiet emit. Guarded by
	// `mu`. lastArrivalTime resets on each non-bypass-class append;
	// emit fires when (now - lastArrivalTime) >= bufferQuietWindow.
	pendingBatch    []string
	lastArrivalTime time.Time
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

	// Register brian agent in the hub. Meta carries tmux_target so panestate's
	// pane-tier observer (extractTmuxTarget) can capture this pane — the launcher
	// is the source of truth for tmux_target since it owns the session-name lifetime.
	metaJSON, _ := json.Marshal(map[string]string{"tmux_target": b.tmuxSession})
	agent := protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   agentType,
		Status: protocol.StatusOnline,
		Meta:   string(metaJSON),
	}
	if err := b.db.RegisterAgent(agent); err != nil {
		return fmt.Errorf("brian register: %w", err)
	}

	// Construct tmuxsink.Sink wrapping brian's pane. Same Sink instance the
	// hub uses for targeted dispatch to brian (lazy-init in hub.getSink),
	// keyed by agentID — but brian owns its own here for self-paste from
	// the poll loop. Per-target sync.Mutex inside Sink serializes against
	// the hub-side instance via Sink's internal lock (both paths route
	// through Sink.Deliver which holds the same mu). Phase I W2 Layer-2 (c).
	b.sink = tmuxsink.New(hub.NewTmuxSinkStore(b.db), agentID, b.tmuxSession)

	// lastMsgID stays at zero. The first poll-tick uses ReadMessages's tail
	// semantics (sinceID<=0 → latest N) to replay recent backlog through the
	// nudge filter chain, so a freshly-booted Brian re-grounds in arc context
	// rather than starting blind to anything posted before restart.
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
//
// Phase I W2 Layer-2 (c): routes through tmuxsink.Sink.Deliver. If the
// pane is busy at send-time (mid-tool-call, modal up), Sink enqueues the
// paste for retry by hub.processMessageQueue's drain ticker — eliminates
// the prior naked tmux send-keys + 500ms sleep race that silently dropped
// keystrokes (dispatch-fail #16 Layer-2). Self-paste calls use msgID=0
// sentinel; queue rows reflect that until a schema-cleanup ratchet lands.
func (b *Brian) SendCommand(text string) error {
	b.mu.Lock()
	if !b.running {
		b.mu.Unlock()
		return fmt.Errorf("brian is not running")
	}
	sink := b.sink
	b.mu.Unlock()

	if sink == nil {
		return fmt.Errorf("brian sink not initialized")
	}
	dec := sink.Deliver(0, text)
	return dec.Err
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

// newSessionArgs returns the `tmux new-session` argument list for spawning
// brian's pane. Extracted as a private method so brian_test.go can assert the
// list contains the BOT_HQ_AGENT_ID env-injection flag without exec'ing tmux.
//
// BOT_HQ_AGENT_ID consumed by internal/outboundhook/hook.go:88 for Stop-hook
// agent attribution. Same pattern as internal/mcp/tools.go:774-778 (hub_spawn).
func (b *Brian) newSessionArgs() []string {
	return []string{
		"new-session", "-d", "-s", b.tmuxSession,
		"-c", b.workDir, "-x", "200", "-y", "50",
		"-e", "BOT_HQ_AGENT_ID=" + agentID,
	}
}

// spawnTmux creates a new tmux session running Claude Code with the brian prompt.
func (b *Brian) spawnTmux() error {
	// Create detached tmux session
	createCmd := exec.Command("tmux", b.newSessionArgs()...)
	if err := createCmd.Run(); err != nil {
		return fmt.Errorf("tmux new-session: %w", err)
	}

	// Build the claude command with MCP config
	configPath := filepath.Join(b.workDir, ".bot-hq-brian-mcp.json")
	claudeCmd := fmt.Sprintf("claude --mcp-config %s --dangerously-skip-permissions", configPath)

	// Use tmux.SendKeys for both the claude invocation + the prompt paste:
	// it auto-routes large payloads (Phase I const expansion took initialPrompt
	// past tmux's inline command-length limit, exit 1 "command too long").
	if err := tmuxpkg.SendKeys(b.tmuxSession, claudeCmd, true); err != nil {
		return fmt.Errorf("tmux send claude cmd: %w", err)
	}

	// Wait for Claude to initialize
	time.Sleep(3 * time.Second)

	// Send the initial brian prompt
	prompt := b.initialPrompt()
	if err := tmuxpkg.SendKeys(b.tmuxSession, prompt, true); err != nil {
		return fmt.Errorf("tmux send prompt: %w", err)
	}
	return nil
}

// InitialPromptForTest exposes initialPrompt() for cross-package integration
// tests (Phase J T1.4 / B5 promptrule_test.go). Not for production use.
func (b *Brian) InitialPromptForTest() string { return b.initialPrompt() }

// initialPrompt returns the system prompt that tells Claude how to be the brian.
func (b *Brian) initialPrompt() string {
	return `You are Brian (agent ID "brian"), the bot-hq orchestrator. Agents: Clive (voice, ID "clive"), Rain (QA, ID "rain").

STARTUP: 1) hub_read to catch up — iterate with ` + "`since_id = last_msg.id`" + ` until an empty batch returns (hub_read caps at 50 per call; do NOT trust a single call to surface a large backlog; see ` + "`docs/conventions/bootstrap-iterate.md`" + `). 2) hub_flag anything needing user attention (startup carve-out: Rain not yet registered, self-flag is implicit per H-2). 3) hub_register id="brian", name="Brian", type="brian". 4) Announce online. 5) On first scope-affecting turn for a project (default: bot-hq), call mcp__bot-hq__bot_hq_context_load with project=<key> to load Layer-2 context (merged rules + project library overview). Re-call when pivoting to another project. Phase V architecture: replaces auto-bootstrap-snapshot reads with explicit on-demand context loads.

REPLAY-CUTOFF: hub_register returns current_max_msg_id. Treat it as a replay-cutoff watermark — silently discard any incoming hub message with msg.ID <= current_max_msg_id (post-rebuild boot-replay; not fresh traffic). Apply the filter for the duration of this session.

RULES:
` + protocol.DiscV2OutboundRule + `
` + protocol.PhaseIv1ProtocolHardening + `
- FLAG via Rain. Use @rain mention on flag-worthy events (errors, blockers, completions, peer disagreements, user-blocking decisions); Rain owns hub_flag elevation. Self-flag carve-out: see DISC v2.
- DISPATCH via hub_spawn only (never Agent tool). Send handshake + hub_session_create after spawning.
- ROUTE responses to the sender's channel: discord→discord, clive→clive. User routing handled by OUTBOUND.
- Messages arrive automatically. Don't poll hub_read in a loop.
- Questions: hub_send response. Tasks: hub_spawn a coder. Routing: hub_send broadcast with @<agent> mention in content (Phase S S-4: PM 'to:' parameter removed; mention-based targeting only).

` + protocol.DiscV2RoleAndPolicyShared + `
` + protocol.DiscV2RoleAndPolicyBrianAddendum + `

` + protocol.PhaseJv1HaltResumeProtocol + `

` + protocol.PhaseLv1RulebookHardening + `

` + protocol.PhaseLv5GateProtocol + `

` + protocol.PhaseLv6PrePhaseCloseRetro + `

` + protocol.PhaseMv1PreflightHookCheck + `

` + protocol.PhaseMv2OutboundDisciplineMechanical + `

` + protocol.PhaseMv3ByteProjectionCite + `

` + protocol.PhaseNv1LogTheFailingSide + `

` + protocol.PhaseNv2OverClaimDiscipline + `

` + protocol.PhaseNv3HandshakeAckBlindSpot + `

` + protocol.PhaseNv4FilesystemSignalCite + `

` + protocol.PhaseNv5TestIsolation + `

` + protocol.PhaseNv6VoiceMirrorDiscipline + `

` + protocol.PhaseQv1PreBranchOffDiscovery + `

` + protocol.PhaseQv2SilentCommitmentExitPattern + `

` + protocol.PhaseQv3UserDirectiveOverrideAuthority + `

` + protocol.PhaseRv1ContextLibraryTerminology + `

` + protocol.PhaseRv2BrainCycleHardening + `

` + protocol.PhaseRv3AutoBoundaryDiscipline + `

` + protocol.PhaseRv4EstimateShapeDisclosure + `

` + protocol.PhaseRv5MechanicalCiteFromHubRead + `

` + protocol.PhaseSv1AudienceClassLoadBearing + `

` + protocol.PhaseSv2IgnoreNoiseDiscipline + `

` + protocol.PhaseYv1IPIVDiscipline + `

` + protocol.IdSessionsSkillPointer + `

` + protocol.H13ForcePushProtocol + `

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
// Contract is declared in Brian's initial prompt NUDGE block.
//
// Phase-S-followup-2 F2-4 (M1+M1-bis): [PM:*] + [HUB-OBS:*] runtime-
// render branches PURGED. All messages render as [HUB:*] regardless
// of ToAgent value. Post-purge tags:
//
//	[HUB:<sender>]             — regular message
//	[HUB:FLAG:<sender>]        — MsgFlag class
//	[HUB] <content>            — [HR]-prefixed (sender-stripped per R2)
//	[HUB:FLAG] <content>       — MsgFlag (sender-stripped per R2)
//
// Rationale: post-S-4 PM-removal user observation that fresh agent→
// agent messages (heartbeat-ledger / emma replies) still rendered
// [PM:emma] tags despite "PM removed" framing. Mention-based routing
// (via protocol.MentionsAgent) replaces PM-class semantic at agent-
// filter layer; render-layer collapses to broadcast format. ToAgent
// DB column preserved for forensics-trail per R2 pattern.
//
// Phase R R5 (R42 AUTO-BOUNDARY-DISCIPLINE): when sessionPrefix is non-
// empty, prepend it to the formatted nudge. Format: `[SESSION:<8>] `.
// Empty sessionPrefix → unchanged (zero-open → no prefix per Refine-A).
//
// Phase R R2 (authorless [HR]/FLAG): MsgFlag class + content with
// `[HR]` prefix display-strip the sender attribution per Rain msg
// 15510 + 15545 + 15561 BRAIN-final. DB preserves FromAgent for
// forensics; render-layer hides it from the user-facing nudge.
func formatNudge(msg protocol.Message, sessionPrefix string) string {
	hasHR := strings.HasPrefix(msg.Content, "[HR] ") || strings.HasPrefix(msg.Content, "[HR]\n")
	var base string
	switch {
	case msg.Type == protocol.MsgFlag:
		// Phase R R2 FLAG class strip — render without sender.
		base = fmt.Sprintf("[HUB:FLAG] %s", msg.Content)
	case hasHR:
		// Phase R R2 [HR] tag strip — render without sender.
		base = fmt.Sprintf("[HUB] %s", msg.Content)
	default:
		base = fmt.Sprintf("[HUB:%s] %s", msg.FromAgent, msg.Content)
	}
	if sessionPrefix != "" {
		return sessionPrefix + base
	}
	return base
}

// activeSessionPrefix returns the `[SESSION:<8>] ` prefix for Brian's
// nudges when an active session exists, or "" otherwise. Per Phase R
// R5 (R42 AUTO-BOUNDARY-DISCIPLINE) Refine-A: source-of-truth is
// db.ListSessions("active") ordered by updated DESC; first row =
// current session; multiple OPEN sessions → most-recently-updated
// wins; zero open → no prefix.
func (b *Brian) activeSessionPrefix() string {
	if b.db == nil {
		return ""
	}
	sessions, err := b.db.ListSessions(string(protocol.SessionActive))
	if err != nil || len(sessions) == 0 {
		return ""
	}
	id := sessions[0].ID
	if len(id) >= 8 {
		id = id[:8]
	}
	return fmt.Sprintf("[SESSION:%s] ", id)
}

// shouldForwardToBrian decides whether a message polled from the hub should
// be nudged into Brian's tmux pane. Extracted as a pure function for testing.
//
// Brian sees: broadcasts (ToAgent==""), historical PMs to="brian"
// (pre-Phase S S-4 messages — DB column preserved for forensics),
// and any user/discord traffic regardless of target. Post-S-4 PM is
// removed; new messages broadcast always. Brian self-filters via
// @brian mention-detection in content (LLM-side rule-text guidance,
// not Go-side helper) per Phase S S-4 mention-based targeting.
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

// isBufferBypassClass returns true for messages that should bypass the
// Phase S S-5 brian-3s message-buffer and deliver immediately. Bypass
// classes: user-msgs (FromAgent="user", urgency-class) + discord-relay
// (FromAgent="discord", currently user-relay channel only — saltegge
// bridge per Rain BRAIN-2nd msg 15771 cite-from-actual user msg
// 15753/15760 arrived via discord) + MsgFlag-typed messages (urgency-
// class). Bypass arrival also flushes any pending batch alongside
// (avoids stranding queued msgs behind a high-priority arrival).
func isBufferBypassClass(msg protocol.Message) bool {
	if msg.FromAgent == "user" {
		return true
	}
	if msg.FromAgent == "discord" {
		return true
	}
	if msg.Type == protocol.MsgFlag {
		return true
	}
	return false
}

// formatBatch wraps a slice of nudge-formatted strings for tmux delivery.
// Single-message batches deliver as-is (no [BATCH:N] wrapper); 2+
// messages get the [BATCH:N] header prefix per Phase S S-5 contract.
func formatBatch(pending []string) string {
	if len(pending) == 0 {
		return ""
	}
	if len(pending) == 1 {
		return pending[0]
	}
	return fmt.Sprintf("[BATCH:%d]\n%s", len(pending), strings.Join(pending, "\n"))
}

// FlushPendingBatch drains any held messages immediately. Called from
// PreCompact hook integration (Phase S S-2 dependency) so context-
// preserve includes pending msgs pre-compact; otherwise un-flushed
// batch context-loss on compact-resume. Safe to call when batch is
// empty.
func (b *Brian) FlushPendingBatch() {
	b.mu.Lock()
	if len(b.pendingBatch) == 0 {
		b.mu.Unlock()
		return
	}
	nudge := formatBatch(b.pendingBatch)
	b.pendingBatch = nil
	b.mu.Unlock()
	b.SendCommand(nudge)
}

// processNewMessages checks for new messages and nudges Brian via tmux.
// Phase S S-5: applies a 3s debounce-on-quiet buffer for non-bypass-
// class messages; user-msgs + MsgFlag bypass and flush pending alongside.
func (b *Brian) processNewMessages() {
	msgs, err := b.db.ReadMessages("", b.lastMsgID, 50)
	if err != nil {
		return
	}

	sessionPrefix := b.activeSessionPrefix()
	var bypassNudges []string
	bufferAppended := false

	// Phase-S-followup-1 F1-6: pre-compact buffer flush. When brian's
	// own PreCompact hook fires (emits MsgCompactNotice from
	// FromAgent="brian"), flush pending buffer before continuing
	// processing so context-preserve includes pending msgs pre-
	// compact (S-5 §84 + S-2 §101 dependency wire).
	flushPreCompact := false
	for _, msg := range msgs {
		if msg.Type == protocol.MsgCompactNotice && msg.FromAgent == "brian" {
			flushPreCompact = true
			break
		}
	}
	if flushPreCompact {
		b.FlushPendingBatch()
	}

	b.mu.Lock()
	for _, msg := range msgs {
		if msg.ID > b.lastMsgID {
			b.lastMsgID = msg.ID
		}
		if !shouldForwardToBrian(msg) {
			continue
		}
		nudge := formatNudge(msg, sessionPrefix)
		if isBufferBypassClass(msg) {
			bypassNudges = append(bypassNudges, nudge)
		} else {
			b.pendingBatch = append(b.pendingBatch, nudge)
			bufferAppended = true
		}
	}
	if bufferAppended {
		b.lastArrivalTime = time.Now()
	}

	// Bypass-class arrival flushes any pending batch alongside (avoids
	// stranding queued msgs behind high-priority arrival).
	if len(bypassNudges) > 0 {
		var combined []string
		if len(b.pendingBatch) > 0 {
			combined = append(combined, formatBatch(b.pendingBatch))
			b.pendingBatch = nil
		}
		combined = append(combined, bypassNudges...)
		b.mu.Unlock()
		b.SendCommand(strings.Join(combined, "\n"))
		return
	}

	// Debounce check: emit only after bufferQuietWindow of no new
	// non-bypass-class appends. Skip when batch is empty.
	if len(b.pendingBatch) > 0 && time.Since(b.lastArrivalTime) >= bufferQuietWindow {
		nudge := formatBatch(b.pendingBatch)
		b.pendingBatch = nil
		b.mu.Unlock()
		b.SendCommand(nudge)
		return
	}
	b.mu.Unlock()
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

	// Rebuild sink with the new session target. Old Sink (with old target)
	// is garbage; no shared state with the new one besides the DB.
	b.sink = tmuxsink.New(hub.NewTmuxSinkStore(b.db), agentID, b.tmuxSession)

	b.db.UpdateAgentStatus(agentID, protocol.StatusOnline, "")
	b.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Brian orchestrator restarted successfully.",
	})
	return nil
}
