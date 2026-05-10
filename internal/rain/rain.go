package rain

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

	// sink wraps tmux pane delivery with isReady-check + retry-queue
	// semantics shared with hub.dispatchToTmux. Constructed in Start()
	// after the tmux session spawns; rebuilt in restart() with the new
	// session name. Phase I W2 Layer-2 (c).
	sink *tmuxsink.Sink

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

	// Register rain agent in the hub. Meta carries tmux_target so panestate's
	// pane-tier observer (extractTmuxTarget) can capture this pane — the launcher
	// is the source of truth for tmux_target since it owns the session-name lifetime.
	metaJSON, _ := json.Marshal(map[string]string{"tmux_target": r.tmuxSession})
	agent := protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   agentType,
		Status: protocol.StatusOnline,
		Meta:   string(metaJSON),
	}
	if err := r.db.RegisterAgent(agent); err != nil {
		return fmt.Errorf("rain register: %w", err)
	}

	// Construct tmuxsink.Sink wrapping rain's pane. Same Sink instance the
	// hub uses for targeted dispatch to rain (lazy-init in hub.getSink),
	// keyed by agentID — but rain owns its own here for self-paste from
	// the poll loop. Per-target sync.Mutex inside Sink serializes against
	// the hub-side instance via Sink's internal lock (both paths route
	// through Sink.Deliver which holds the same mu). Phase I W2 Layer-2 (c).
	r.sink = tmuxsink.New(hub.NewTmuxSinkStore(r.db), agentID, r.tmuxSession)

	// lastMsgID stays at zero. The first poll-tick uses ReadMessages's tail
	// semantics (sinceID<=0 → latest N) to replay recent backlog through the
	// nudge filter chain.
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
//
// Phase I W2 Layer-2 (c): routes through tmuxsink.Sink.Deliver. If the
// pane is busy at send-time (mid-tool-call, modal up), Sink enqueues the
// paste for retry by hub.processMessageQueue's drain ticker — eliminates
// the prior naked tmux send-keys + 500ms sleep race that silently dropped
// keystrokes (dispatch-fail #16 Layer-2). Self-paste calls use msgID=0
// sentinel; queue rows reflect that until a schema-cleanup ratchet lands.
func (r *Rain) SendCommand(text string) error {
	r.mu.Lock()
	if !r.running {
		r.mu.Unlock()
		return fmt.Errorf("rain is not running")
	}
	sink := r.sink
	r.mu.Unlock()

	if sink == nil {
		return fmt.Errorf("rain sink not initialized")
	}
	dec := sink.Deliver(0, text)
	return dec.Err
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

// newSessionArgs returns the `tmux new-session` argument list for spawning
// rain's pane. Extracted as a private method so rain_test.go can assert the
// list contains the BOT_HQ_AGENT_ID env-injection flag without exec'ing tmux.
//
// BOT_HQ_AGENT_ID consumed by internal/outboundhook/hook.go:88 for Stop-hook
// agent attribution. Same pattern as internal/mcp/tools.go:774-778 (hub_spawn).
//
// Phase T T-1.4 (R51 + R52): also loads agent_model_configs row for rain and
// injects ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN + ANTHROPIC_MODEL env-vars
// for non-Claude paths (e.g. DeepSeek-V4-Pro). Empty additions for Claude
// OAuth path (subprocess inherits CLAUDE_CODE_OAUTH_TOKEN from env).
func (r *Rain) newSessionArgs() []string {
	args := []string{
		"new-session", "-d", "-s", r.tmuxSession,
		"-c", r.workDir, "-x", "200", "-y", "50",
		"-e", "BOT_HQ_AGENT_ID=" + agentID,
	}
	// T-1.4: append per-agent model-config env-vars (R51 + R52)
	args = append(args, r.modelConfigEnvArgs()...)
	return args
}

// modelConfigEnvArgs loads rain's agent_model_configs row and returns the
// `-e KEY=VALUE` arg pairs for `tmux new-session`. Returns empty slice when
// no row exists OR Claude OAuth path (no env-var swap needed). Logs errors
// at WARN level + falls through to empty (preserves v4 Claude-only behavior
// per R51 fallthrough discipline).
func (r *Rain) modelConfigEnvArgs() []string {
	// Defensive: db nil in unit tests that exercise newSessionArgs() without
	// a real hub.DB. Fall through to default Claude path.
	if r.db == nil {
		return nil
	}
	cfg, err := r.db.GetAgentModelConfig(agentID)
	if err != nil {
		if errors.Is(err, hub.ErrAgentModelConfigNotFound) {
			// No config row → fall through to default Claude path (no env-var injection)
			return nil
		}
		log.Printf("rain: agent_model_config load failed for %s: %v (falling through to default)", agentID, err)
		return nil
	}

	envs, err := agentconfig.BuildSpawnEnv(cfg)
	if err != nil {
		log.Printf("rain: BuildSpawnEnv failed for %s: %v (config provider=%s ref=%s); falling through to default", agentID, err, cfg.Provider, cfg.AuthSecretRef)
		return nil
	}

	// Log the env-var-list (REDACTED) for diagnostic visibility per R52
	if len(envs) > 0 {
		names := make([]string, 0, len(envs))
		for _, e := range envs {
			names = append(names, e.String())
		}
		log.Printf("rain: agent_model_config injecting env-vars (R51 + R52): provider=%s model=%s vars=[%s]", cfg.Provider, cfg.ModelName, strings.Join(names, ", "))
	}

	return agentconfig.FormatTmuxEnvArgs(envs)
}

func (r *Rain) spawnTmux() error {
	createCmd := exec.Command("tmux", r.newSessionArgs()...)
	if err := createCmd.Run(); err != nil {
		return fmt.Errorf("tmux new-session: %w", err)
	}

	configPath := filepath.Join(r.workDir, ".bot-hq-rain-mcp.json")
	claudeCmd := fmt.Sprintf("claude --mcp-config %s --dangerously-skip-permissions", configPath)

	// Use tmux.SendKeys for both the claude invocation + the prompt paste:
	// it auto-routes large payloads (Phase I const expansion took initialPrompt
	// past tmux's inline command-length limit, exit 1 "command too long").
	if err := tmuxpkg.SendKeys(r.tmuxSession, claudeCmd, true); err != nil {
		return fmt.Errorf("tmux send claude cmd: %w", err)
	}

	time.Sleep(3 * time.Second)

	prompt := r.initialPrompt()
	if err := tmuxpkg.SendKeys(r.tmuxSession, prompt, true); err != nil {
		return fmt.Errorf("tmux send prompt: %w", err)
	}
	return nil
}

// InitialPromptForTest exposes initialPrompt() for cross-package integration
// tests (Phase J T1.4 / B5 promptrule_test.go). Not for production use.
func (r *Rain) InitialPromptForTest() string { return r.initialPrompt() }

func (r *Rain) initialPrompt() string {
	return `You are Rain (agent ID "rain"), bot-hq's adversarial QA agent. Sharp, skeptical, terse. Agents: Brian (orchestrator, ID "brian"), Clive (voice, ID "clive").

STARTUP: hub_register id="rain", name="Rain", type="qa". On first scope-affecting turn for a project (default: bot-hq), call mcp__bot-hq__bot_hq_context_load with project=<key> to load Layer-2 context (merged rules + project library overview); re-call when pivoting to another project. Then watch the hub. Messages arrive automatically; do NOT poll hub_read.

REPLAY-CUTOFF: hub_register returns current_max_msg_id. Treat it as a replay-cutoff watermark — silently discard any incoming hub message with msg.ID <= current_max_msg_id (post-rebuild boot-replay; not fresh traffic). Apply the filter for the duration of this session.

RULES:
` + protocol.DiscV2OutboundRule + `
` + protocol.PhaseIv1ProtocolHardening + `
- FLAG ownership: Rain owns hub_flag elevation. Brian uses @rain mention on flag-worthy events (Phase S S-4: PM removed); Rain decides whether to elevate, peer-coordinate, or wait. Per 2026-04-27 user delegation, Rain may greenflag joint defaults without flagging when user is not in the loop on the specific decision. Self-flag carve-out: see DISC v2.
- ROUTE responses to sender's channel: discord→discord, brian→brian. User routing handled by OUTBOUND.
- Review coder output with claude_read. Look for bugs, missing tests, incomplete work.
- When disagreeing with Brian: "Brian wants X. I think Y because Z. User decision needed." + hub_flag.
- Approve cleanly: "Looks clean." Flag precisely: what's wrong, why it matters.

` + protocol.DiscV2RoleAndPolicyShared + `
` + protocol.DiscV2RoleAndPolicyRainAddendum + `

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
// Phase-S-followup-2 F2-4 (M1+M1-bis): [PM:*] + [HUB-OBS:*] runtime-
// render branches PURGED. All messages render as [HUB:*] regardless
// of ToAgent value. See brian.go:formatNudge for full rationale.
// Post-purge tags:
//
//	[HUB:<sender>]                    — regular message
//	[HUB:FLAG:<sender>]               — MsgFlag class
//	[HUB] <content>                   — [HR]-prefixed (sender-stripped)
//	[HUB:FLAG] <content>              — MsgFlag (sender-stripped)
//
// Phase R R5 (R42 AUTO-BOUNDARY-DISCIPLINE): when sessionPrefix is non-
// empty, prepend it to the formatted nudge. Format: `[SESSION:<8>] `.
// Empty sessionPrefix → unchanged (zero-open → no prefix per Refine-A).
//
// Phase R R2 (authorless [HR]/FLAG): MsgFlag class + content with
// `[HR]` prefix display-strip the sender attribution per Rain msg
// 15510 + 15545 + 15561 BRAIN-final. DB preserves FromAgent for
// forensics; render-layer hides it from the user-facing nudge.
func formatRainNudge(msg protocol.Message, sessionPrefix string) string {
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

// activeSessionPrefix returns the `[SESSION:<8>] ` prefix for Rain's
// nudges when an active session exists, or "" otherwise. Per Phase R
// R5 (R42 AUTO-BOUNDARY-DISCIPLINE) Refine-A: source-of-truth is
// db.ListSessions("active") ordered by updated DESC; first row =
// current session; multiple OPEN sessions → most-recently-updated
// wins; zero open → no prefix.
func (r *Rain) activeSessionPrefix() string {
	if r.db == nil {
		return ""
	}
	sessions, err := r.db.ListSessions(string(protocol.SessionActive))
	if err != nil || len(sessions) == 0 {
		return ""
	}
	id := sessions[0].ID
	if len(id) >= 8 {
		id = id[:8]
	}
	return fmt.Sprintf("[SESSION:%s] ", id)
}

// shouldForwardToRain decides whether a message polled from the hub should
// be nudged into Rain's tmux pane. Extracted as a pure function for testing.
//
// Rain sees: broadcasts (ToAgent==""), historical PMs to="rain" (pre-
// Phase S S-4 messages — DB column preserved for forensics), any
// user/discord traffic regardless of target, results/errors/commands/
// flags from any peer (QA coverage), any broadcast from Brian
// regardless of Type, and broadcasts whose content mentions
// hub_flag/hub_spawn (catches non-Brian agents like Emma calling out
// elevation events). Post-S-4 PM is removed; Rain self-filters via
// @rain mention-detection in content (LLM-side rule-text guidance,
// not Go-side helper) per Phase S S-4 mention-based targeting.
// Rain skips: own messages, inter-agent chatter between coders, non-Brian
// MsgUpdate broadcasts without flag/spawn substrings (handshakes, acks,
// "standing by" — coder-broadcast-flood protection).
//
// Phase I W2 I-7 fix: prior implementation gated FromAgent=="brian"
// broadcasts to Type==MsgResponse only, dropping all of Brian's MsgUpdate
// broadcasts (commit notices, scope-dumps, halt-acks). Today's session
// produced 14 specific dropped msg-IDs (4358, 4386, 4393, 4401, 4408,
// 4420, 4435, 4480, 4491, 4522, 4547, 4563, 4639, 4646), all type=update,
// to_agent='', no flag/spawn substring. Widen the from-brian-broadcast
// branch to forward regardless of Type. The hub_flag/hub_spawn substring
// check is preserved as the secondary catch-all for non-Brian agents
// emitting visibility-worthy broadcasts (existing test contract:
// "hub_flag mention forwards" — emma update content "calling hub_flag"
// must reach Rain).
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
		// Peer-visibility: all Brian broadcasts reach Rain regardless of Type.
		// FromAgent=="brian" gate preserves coder-broadcast-flood protection.
		if msg.FromAgent == "brian" {
			return true
		}
		// Catch-all for non-Brian agents emitting visibility-worthy
		// broadcasts via content keywords (e.g., emma elevation prose).
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

	sessionPrefix := r.activeSessionPrefix()
	for _, msg := range msgs {
		if msg.ID > r.lastMsgID {
			r.lastMsgID = msg.ID
		}
		if !shouldForwardToRain(msg) {
			// Phase I W2 I-7 observability: structured filter-drop log so
			// future filter regressions surface via log analysis without
			// grep heuristics. Quiet-by-default (single line per drop).
			log.Printf("rain: filter-drop msg %d type=%s from=%s to=%s", msg.ID, msg.Type, msg.FromAgent, msg.ToAgent)
			continue
		}
		nudge := formatRainNudge(msg, sessionPrefix)
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

	// Rebuild sink with the new session target. Old Sink (with old target)
	// is garbage; no shared state with the new one besides the DB.
	r.sink = tmuxsink.New(hub.NewTmuxSinkStore(r.db), agentID, r.tmuxSession)

	r.db.UpdateAgentStatus(agentID, protocol.StatusOnline, "")
	r.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   "Rain QA restarted.",
	})
	return nil
}
