package gemma

import (
	"context"
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
	agentID   = "emma"
	agentName = "Emma"

	pollInterval           = 3 * time.Second
	healthInterval         = 30 * time.Second
	heartbeatInterval      = 30 * time.Second
	defaultMonitorInterval = 5 * time.Minute
	// sentinelPollInterval gates the H-22 cross-process MCP-insert catch-up
	// path. See sentinelPollLoop for the rationale on why this exists.
	sentinelPollInterval = 5 * time.Second
	// wakeDispatchInterval is the slice-3 C1 (#7) wake_schedule fire cadence.
	// 1s gives sub-second round-up against the seconds-precision fire_at
	// granularity locked by arch lean 1, and stays comfortably inside the
	// "fires within tick_interval + 1s" acceptance bound from the design.
	wakeDispatchInterval = 1 * time.Second

	defaultModel     = "gemma4:e4b"
	defaultOllamaURL = "http://localhost:11434"
	defaultMaxConc   = 3

	// Phase D — flag dedupe + rate cap (shared across all monitor conditions).
	flagHysteresisWindow   = 30 * time.Minute
	flagRateCapPerHour     = 3
	monitorPreconditionGap = 1 * time.Hour
)

// TaskType determines how command output is handled.
type TaskType string

const (
	TaskExec    TaskType = "exec"
	TaskAnalyze TaskType = "analyze"
)

// canonicalEmmaBlock is Emma's two-class identity + boundary preamble per
// Phase H slice 2 H-24. Prepended to every TaskAnalyze prompt so the
// gemma4:e4b model sees its scope explicitly, refuses interpretive
// queries, and routes them back to Rain for inline handling.
//
// Structured class = parse / summarize / extract / count. These are
// gemma4:e4b safe.
//
// Interpretive class = assess vs spec / contract / criterion. These need
// a richer model (Rain) and are out-of-scope for Emma. Default-deny on
// straddled queries.
//
// See docs/conventions/emma-analyze-classes.md for the full class table.
const canonicalEmmaBlock = `You are Emma, bot-hq's analyze sentinel (model: gemma4:e4b).

Two-class boundary for analyze queries (per Phase H H-24):
- Structured (parse, summarize, extract, count): ANSWER. Examples: parse git log output, list files in diff, count test results.
- Interpretive (assess vs spec/contract/criterion, judge materiality, render verdicts): REFUSE and reply "interpretive query — routing back to Rain per H-24". Examples: diff-gate verdicts, design-spec-match, observation-materiality.

Default-deny on straddled queries — when in doubt, refuse to Rain.`

// allowedCommands is the hardcoded allowlist for command execution.
var allowedCommands = []string{
	"go test",
	"go vet",
	"go build",
	"df -h",
	"ps aux",
	"uptime",
	"free -m",
	"vm_stat",
	"du -sh",
	"wc -l",
	"ls",
	"git status",
	"git log",
	"git diff",
	"gh issue view",
	"gh issue list",
	"gh pr view",
	"gh pr list",
	"curl -s",
	"curl -sL",
}

// SharedSem is the package-level semaphore that caps total concurrent Gemma
// tasks across both the persistent agent and the hub_spawn_gemma MCP tool.
// Initialized by New(); callers that bypass New() must call InitSharedSem().
var SharedSem chan struct{}

// InitSharedSem sets up the shared semaphore if not already created.
func InitSharedSem(maxConc int) {
	if SharedSem == nil {
		SharedSem = make(chan struct{}, maxConc)
	}
}

// ProjectDir returns the bot-hq project directory for health checks.
func ProjectDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, "Projects", "bot-hq")
}

// Gemma manages the Ollama sidecar and processes tasks via the hub.
type Gemma struct {
	db     *hub.DB
	client *Client

	model     string
	ollamaURL string

	ollamaCmd *exec.Cmd
	lastMsgID int64

	monitorInterval time.Duration

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}

	// sentinelMu guards lastSentinelMsgID. The watermark is read+advanced
	// from sentinelPollLoop and replayThroughSentinel — distinct goroutines.
	sentinelMu         sync.Mutex
	lastSentinelMsgID  int64

	// Phase D flag bookkeeping. flagHistory dedupes by condition key
	// (last-fired timestamp). flagWindow is a sliding 1h record of all
	// fired flags for the shared rate cap.
	flagMu      sync.Mutex
	flagHistory map[string]time.Time
	flagWindow  []time.Time

	// Phase H slice 3 C4 (H-3a Shape γ) stale-detection state. paneBaseline
	// tracks the previous tick's tmux #{pane_last_activity} per agent so the
	// next tick can decide "any pane output since last observation?". Mutex
	// guards both the map and the paneActivity injection point.
	staleMu      sync.Mutex
	paneBaseline map[string]int64
	paneActivity paneActivityFn

	// Phase H slice 3 C5 (H-25 roster hygiene) state. lastPruneAt is read+
	// written only from healthLoop so no mutex needed; zero value means
	// "never pruned this run", which triggers a first-tick prune.
	lastPruneAt time.Time

	// Phase H slice 4 C6 (H-31) context-cap halt-flag source. Tests assign
	// directly; production wires a panestate.Manager-backed closure on
	// Start via initContextCapDefault.
	paneSnapFn paneSnapshotFn
}

// New creates a Gemma instance from config.
func New(db *hub.DB, cfg hub.GemmaConfig) *Gemma {
	model := cfg.Model
	if model == "" {
		model = defaultModel
	}
	ollamaURL := cfg.OllamaURL
	if ollamaURL == "" {
		ollamaURL = defaultOllamaURL
	}
	maxConc := cfg.MaxConcurrent
	if maxConc <= 0 {
		maxConc = defaultMaxConc
	}

	InitSharedSem(maxConc)

	return &Gemma{
		db:              db,
		model:           model,
		ollamaURL:       ollamaURL,
		monitorInterval: defaultMonitorInterval,
		stopCh:          make(chan struct{}),
		flagHistory:     make(map[string]time.Time),
	}
}

// Start launches the Ollama sidecar, waits for health, and registers on the hub.
func (g *Gemma) Start() error {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.running {
		return nil
	}

	// Start Ollama serve process
	g.ollamaCmd = exec.Command("ollama", "serve")
	g.ollamaCmd.Env = append(os.Environ(),
		"OLLAMA_FLASH_ATTENTION=1",
		"OLLAMA_KV_CACHE_TYPE=q8_0",
	)
	// Discard stdout/stderr — Ollama logs to its own file
	g.ollamaCmd.Stdout = nil
	g.ollamaCmd.Stderr = nil
	if err := g.ollamaCmd.Start(); err != nil {
		return fmt.Errorf("ollama serve: %w", err)
	}

	// Create client
	g.client = NewClient(g.ollamaURL, g.model)

	// Wait for Ollama to become healthy (up to 30s)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if !g.waitForHealth(ctx) {
		g.ollamaCmd.Process.Kill()
		return fmt.Errorf("ollama did not become healthy within 30s")
	}

	// Register on the hub
	agent := protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   protocol.AgentGemma,
		Status: protocol.StatusOnline,
	}
	if err := g.db.RegisterAgent(agent); err != nil {
		return fmt.Errorf("emma register: %w", err)
	}

	// Get current last message ID so we only process new messages
	msgs, err := g.db.GetRecentMessages(1)
	if err == nil && len(msgs) > 0 {
		g.lastMsgID = msgs[0].ID
	}

	g.running = true

	// Phase H slice 4 C6 (H-31): wire the panestate snapshot source if a
	// test hasn't already injected one. Default uses a hub-DB-backed
	// panestate.Manager + real tmux.CapturePane.
	g.initContextCapDefault()

	go g.pollLoop()
	go g.healthLoop()
	go g.heartbeatLoop()
	go g.monitorLoop()
	go g.sentinelPollLoop()
	go g.wakeDispatchLoop()

	// Phase H slice 3 C7: schedule first _internal:docdrift wake if no
	// pending one exists from a prior boot. Re-arms on every fire via
	// runDocDriftScanAndReArm.
	g.bootstrapInternalDocDrift()

	// Boot-time replay: run the most recent N hub messages through the
	// sentinel so any active failures from the pre-restart window are
	// caught. Live in-process messages from this point onward arrive via
	// the OnMessage subscriber wired in cmd/bot-hq/main.go; cross-process
	// MCP-routed inserts arrive via sentinelPollLoop's tick.
	g.replayThroughSentinel()

	g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   fmt.Sprintf("Emma online. Model: %s", g.model),
	})

	return nil
}

// Stop shuts down the Gemma agent and Ollama process.
func (g *Gemma) Stop() {
	g.mu.Lock()
	defer g.mu.Unlock()
	if !g.running {
		return
	}
	g.running = false
	close(g.stopCh)

	g.db.UpdateAgentStatus(agentID, protocol.StatusOffline, "")

	if g.ollamaCmd != nil && g.ollamaCmd.Process != nil {
		g.ollamaCmd.Process.Kill()
		g.ollamaCmd.Wait()
	}
}

// waitForHealth polls until Ollama responds or context expires.
func (g *Gemma) waitForHealth(ctx context.Context) bool {
	ticker := time.NewTicker(500 * time.Millisecond)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return false
		case <-ticker.C:
			if g.client.IsHealthy(ctx) {
				return true
			}
		}
	}
}

// IsCommandAllowed checks if a command matches the allowlist (prefix match).
func IsCommandAllowed(command string) bool {
	trimmed := strings.TrimSpace(command)
	for _, prefix := range allowedCommands {
		if strings.HasPrefix(trimmed, prefix) {
			return true
		}
	}
	return false
}

// ExecuteTask runs a command and optionally passes output through Ollama.
func (g *Gemma) ExecuteTask(ctx context.Context, command string, taskType TaskType, workDir string) (string, error) {
	if !IsCommandAllowed(command) {
		return "", fmt.Errorf("command not allowed: %s", command)
	}

	// Acquire shared semaphore
	select {
	case SharedSem <- struct{}{}:
		defer func() { <-SharedSem }()
	case <-ctx.Done():
		return "", ctx.Err()
	}

	// Run the command
	parts := strings.Fields(command)
	cmd := exec.CommandContext(ctx, parts[0], parts[1:]...)
	if workDir != "" {
		cmd.Dir = workDir
	}

	output, err := cmd.CombinedOutput()
	exitCode := 0
	if err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			exitCode = exitErr.ExitCode()
		} else {
			return "", fmt.Errorf("exec: %w", err)
		}
	}

	result := string(output)

	switch taskType {
	case TaskExec:
		return fmt.Sprintf("exit_code: %d\n%s", exitCode, result), nil
	case TaskAnalyze:
		prompt := fmt.Sprintf("%s\n\nQuery: Summarize this output concisely. Flag any errors or anomalies:\n\n```\n%s\n```", canonicalEmmaBlock, result)
		analysis, err := g.client.Generate(ctx, prompt)
		if err != nil {
			return fmt.Sprintf("exit_code: %d\n%s\n\n[ollama analysis failed: %v]", exitCode, result, err), nil
		}
		return analysis, nil
	default:
		return "", fmt.Errorf("unknown task type: %s", taskType)
	}
}

// pollLoop checks for new messages directed at the gemma agent.
func (g *Gemma) pollLoop() {
	ticker := time.NewTicker(pollInterval)
	defer ticker.Stop()

	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			g.processNewMessages()
		}
	}
}

// processNewMessages reads hub messages directed at emma and handles them.
func (g *Gemma) processNewMessages() {
	msgs, err := g.db.ReadMessages(agentID, g.lastMsgID, 50)
	if err != nil {
		return
	}

	for _, msg := range msgs {
		if msg.ID > g.lastMsgID {
			g.lastMsgID = msg.ID
		}

		if msg.FromAgent == agentID {
			continue
		}

		// Only process messages directed at us
		if msg.ToAgent != agentID {
			continue
		}

		// Parse task from message content
		go g.handleMessage(msg)
	}
}

// handleMessage processes a single hub message as a task request.
// Expected format: "exec: <command>" or "analyze: <command>"
func (g *Gemma) handleMessage(msg protocol.Message) {
	content := strings.TrimSpace(msg.Content)

	var taskType TaskType
	var command string

	if strings.HasPrefix(content, "exec:") {
		taskType = TaskExec
		command = strings.TrimSpace(strings.TrimPrefix(content, "exec:"))
	} else if strings.HasPrefix(content, "analyze:") {
		taskType = TaskAnalyze
		command = strings.TrimSpace(strings.TrimPrefix(content, "analyze:"))
	} else {
		// No recognized prefix — drop. Emma is a tool agent; unprefixed
		// content (greetings, acks, malformed dispatches) must not be
		// shell-executed. Log with sender + truncated content so triage
		// has a grep target when an expected task never runs.
		truncated := content
		if len(truncated) > 80 {
			truncated = truncated[:80]
		}
		log.Printf("emma: dropped non-prefixed message from %s: %s", msg.FromAgent, truncated)
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()

	result, err := g.ExecuteTask(ctx, command, taskType, "")
	if err != nil {
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   msg.FromAgent,
			Type:      protocol.MsgError,
			Content:   fmt.Sprintf("Task failed: %v", err),
		})
		return
	}

	// Truncate if very long
	if len(result) > 10000 {
		result = result[:10000] + "\n...[truncated]"
	}

	g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		ToAgent:   msg.FromAgent,
		Type:      protocol.MsgResult,
		Content:   result,
	})
}

// healthLoop periodically checks if Ollama is still running.
func (g *Gemma) healthLoop() {
	ticker := time.NewTicker(healthInterval)
	defer ticker.Stop()

	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			healthy := g.client.IsHealthy(ctx)
			cancel()
			if !healthy {
				g.db.InsertMessage(protocol.Message{
					FromAgent: agentID,
					Type:      protocol.MsgError,
					Content:   "Ollama health check failed. Attempting restart...",
				})
				g.restartOllama()
			}
			// Phase H slice 3 C4: piggyback stale-coder detection on the
			// healthLoop tick (30s) — no new ticker, cadence well below the
			// 5min staleThreshold so the two-tick baseline + flag pattern
			// detects a frozen pane within ~5min30s of the threshold trip.
			g.checkStaleAgents()
			// Phase H slice 3 C5: piggyback roster prune at hourly cadence.
			g.runRosterPrune()
			// Phase H slice 4 C6 (H-31): scan panestate for context-cap
			// squeeze; fire halt-flag + set halt_state when any non-emma
			// agent is at or above 95% usage.
			g.checkContextCap(time.Now())
		}
	}
}

// pruneInterval is how often runRosterPrune fires (only one prune per
// pruneInterval window even though healthLoop ticks every 30s).
const pruneInterval = 1 * time.Hour

// pruneThreshold is the last_seen age beyond which an offline agent row is
// eligible for deletion. 24h ensures intermittent agents that go offline
// briefly are not pruned; only long-dead rows reclaim space.
const pruneThreshold = 24 * time.Hour

// runRosterPrune deletes offline agent rows older than pruneThreshold and
// PMs Rain with the pruned IDs for audit. Idempotent (no-op when nothing
// to prune). Live agents (online/working) are protected by the status
// filter inside PruneStaleOfflineAgents — never pruned regardless of age.
func (g *Gemma) runRosterPrune() {
	now := time.Now()
	if !g.lastPruneAt.IsZero() && now.Sub(g.lastPruneAt) < pruneInterval {
		return
	}
	g.lastPruneAt = now
	ids, err := g.db.PruneStaleOfflineAgents(pruneThreshold)
	if err != nil {
		log.Printf("[gemma] roster prune failed: %v", err)
		return
	}
	if len(ids) == 0 {
		return
	}
	g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   fmt.Sprintf("[ROSTER-PRUNE] Removed %d stale-offline agent rows (last_seen >24h): %s", len(ids), strings.Join(ids, ", ")),
	})
}

// replayBacklog is the number of recent hub messages Emma re-classifies
// at boot to catch active failures from the pre-restart window.
const replayBacklog = 50

// OnHubMessage is the OnMessage subscriber Emma registers with the hub.
// Pure dispatcher: skips Emma's own messages, runs SentinelMatch, and
// hands matched messages to dispatchSentinelHit. Default-ignore for
// any non-match.
//
// Wired post-Start in cmd/bot-hq/main.go via h.DB.OnMessage(...).
func (g *Gemma) OnHubMessage(msg protocol.Message) {
	if msg.FromAgent == agentID {
		return // skip self to avoid feedback loops
	}
	d := SentinelMatch(msg)
	if !d.Match {
		return // default-ignore
	}
	g.dispatchSentinelHit(msg, d)
}

// dispatchSentinelHit emits the appropriate hub message for a sentinel
// match. Always-flag matches go out as Type=MsgFlag (Discord-bound);
// pre-filter-only matches go out as Type=MsgUpdate observations to
// Rain. Both paths reuse Gemma.shouldFlag so a noisy storm doesn't
// blow past the existing rate cap and hysteresis window.
func (g *Gemma) dispatchSentinelHit(msg protocol.Message, d SentinelDecision) {
	now := time.Now()
	if d.AlwaysFlag {
		if !g.shouldFlag("sentinel:"+d.Pattern, now) {
			return
		}
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			Type:      protocol.MsgFlag,
			Content:   fmt.Sprintf("Sentinel always-flag hit in msg #%d (from %s, pattern %s): %s", msg.ID, msg.FromAgent, d.Pattern, summarizeOutput(msg.Content, 200)),
		})
		return
	}
	if !g.shouldFlag("sentinel-obs:"+d.Pattern, now) {
		return
	}
	// H-22 dry-run gate: patterns in the tuning-gate period write to a
	// ledger file instead of pinging Rain via hub. Rain reviews the
	// ledger out-of-band and flips the pattern to live after ≤5%
	// false-positive rate confirmed.
	if name, isDryRun := IsDryRunPattern(d.Pattern); isDryRun {
		AppendToDryRunLedger(name, fmt.Sprintf("msg #%d from %s | pattern %s | %s", msg.ID, msg.FromAgent, d.Pattern, summarizeOutput(msg.Content, 200)))
		return
	}
	g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   fmt.Sprintf("Sentinel match in msg #%d (from %s, pattern %s): %s", msg.ID, msg.FromAgent, d.Pattern, summarizeOutput(msg.Content, 200)),
	})
}

// OnHubMessageReplay is the silent-mode variant of OnHubMessage used by
// replayThroughSentinel at boot. It runs SentinelMatch and writes to the
// dry-run ledger for matched dry-run patterns (correctness — preserves
// cross-bounce dedup), but skips shouldFlag entirely so boot-replay
// never arms the hysteresis window for live triggers, and skips
// always-flag/Rain-emit paths so replay never spams Discord/Rain.
//
// Phase H slice 3 #4 (replay silent-mode): caught live during slice-2
// closure cycle (msgs 3463/3467) — boot-replay over the last 50 msgs
// could arm flagHistory["sentinel-obs:queueFailPattern"] for the full
// 30-min hysteresis window, blocking subsequent live triggers. The
// slipping pathology becomes worse under repeated rebuilds within a
// 50-msg traffic burst (sliding-window arming).
//
// Splits replay-path from dispatch-path so hydration side-effects
// (ledger write) are decoupled from notification side-effects
// (hysteresis arm + Rain/Discord emit).
func (g *Gemma) OnHubMessageReplay(msg protocol.Message) {
	if msg.FromAgent == agentID {
		return
	}
	d := SentinelMatch(msg)
	if !d.Match {
		return
	}
	if d.AlwaysFlag {
		return
	}
	if name, isDryRun := IsDryRunPattern(d.Pattern); isDryRun {
		AppendToDryRunLedger(name, fmt.Sprintf("msg #%d from %s | pattern %s | %s", msg.ID, msg.FromAgent, d.Pattern, summarizeOutput(msg.Content, 200)))
		return
	}
}

// replayThroughSentinel feeds the last replayBacklog hub messages
// through OnHubMessageReplay at boot. Catches active failures from the
// pre-restart window without requiring a separate silence-poll path
// (deferred to post-Phase-F per locked msg 2442).
//
// Uses OnHubMessageReplay (silent variant) instead of OnHubMessage to
// prevent boot-replay from arming hysteresis for live post-boot
// triggers (Phase H slice 3 #4).
//
// Advances lastSentinelMsgID past the replayed window so the
// sentinelPollLoop tick doesn't re-process the same messages. Cross-
// bounce dedup of ledger entries is handled by AppendToDryRunLedger's
// msg-id parse.
func (g *Gemma) replayThroughSentinel() {
	msgs, err := g.db.GetRecentMessages(replayBacklog)
	if err != nil {
		return
	}
	var maxID int64
	for _, m := range msgs {
		g.OnHubMessageReplay(m)
		if m.ID > maxID {
			maxID = m.ID
		}
	}
	g.sentinelMu.Lock()
	if maxID > g.lastSentinelMsgID {
		g.lastSentinelMsgID = maxID
	}
	g.sentinelMu.Unlock()
}

// sentinelPollLoop is the cross-process catch-up path for sentinel
// detection. Closes the H-22 acceptance gap discovered at slice 2
// runtime test 3.
//
// Why this exists: db.OnMessage callbacks (where Emma's OnHubMessage
// is wired in cmd/bot-hq/main.go) are a process-local Go list. They
// fire only for inserts made by *the same process* that registered
// the callback. Brian, Rain, and coders all emit hub_send via the
// MCP server (separate process), so their inserts never traverse the
// TUI process's onMessages list. Pre-hotfix, Emma's only path to see
// such messages was the boot-time replayThroughSentinel window of
// the last 50 messages.
//
// Distinction vs H-18 (Rain's MCP polling rule dropped): Rain polls
// from a Claude Code session via MCP, where messages-arrive-
// automatically is delivered by MCP push. Emma is the in-process Go
// monitor with direct DB access; she has no MCP push primitive for
// cross-process inserts. Different architectural surface — H-18's
// "don't poll" rule applies to MCP clients, not to in-process agents
// reading their own DB.
func (g *Gemma) sentinelPollLoop() {
	ticker := time.NewTicker(sentinelPollInterval)
	defer ticker.Stop()

	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			g.pollSentinel()
		}
	}
}

// pollSentinel reads all DB messages newer than the watermark and
// feeds each through OnHubMessage. Updates the watermark to the max
// processed ID so the next tick is incremental, not cumulative.
//
// Batch limit (200) bounds work per tick. Under sustained burst
// the loop catches up across consecutive ticks; the ledger dedup
// invariant guards against double-write if a boot-replay window
// overlaps with the polling window.
func (g *Gemma) pollSentinel() {
	g.sentinelMu.Lock()
	since := g.lastSentinelMsgID
	g.sentinelMu.Unlock()

	msgs, err := g.db.ReadMessages("", since, 200)
	if err != nil {
		return
	}

	maxID := since
	for _, m := range msgs {
		g.OnHubMessage(m)
		if m.ID > maxID {
			maxID = m.ID
		}
	}

	g.sentinelMu.Lock()
	if maxID > g.lastSentinelMsgID {
		g.lastSentinelMsgID = maxID
	}
	g.sentinelMu.Unlock()
}

// wakeDispatchLoop is the agentic time-trigger primitive landed in Phase H
// slice 3 C1 (#7). Each tick, it scans wake_schedule for pending rows whose
// fire_at is now in the past and dispatches each via hub_send (from='emma',
// type='command'). Reuses Emma's already-running clock infrastructure so no
// new daemon is needed; this is the third in-process Emma tick loop after
// sentinelPollLoop and monitorLoop.
//
// Cross-process safety: db.OnMessage callbacks fire only for inserts made by
// the registering process (per H-22 distinction), but wake dispatch happens
// inside Emma's own process via db.InsertMessage, so the resulting message
// reaches OnMessage subscribers without crossing process boundaries here.
// The MCP wake-schedule writes (cross-process) are caught by reading the
// shared wake_schedule table directly each tick — same pattern as
// sentinelPollLoop's read of the shared messages table.
func (g *Gemma) wakeDispatchLoop() {
	ticker := time.NewTicker(wakeDispatchInterval)
	defer ticker.Stop()
	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			g.dispatchWakes()
		}
	}
}

// dispatchWakes fires every pending wake whose fire_at has elapsed. Each row
// transitions to a terminal state (fired on hub_send success, failed on
// hub_send error) — drop-on-fail per arch lean 4 (no retry in v1; failed
// rows surface via the Emma log path). The terminal-flip uses
// MarkWakeFired/MarkWakeFailed which are pending-only UPDATEs, so a concurrent
// hub_cancel_wake racing the dispatch tick will leave the row in whichever
// state won the race without state-machine corruption.
//
// Phase H slice 3 C7 (#6 H-23 periodic invoker): wakes with target_agent
// prefixed `_internal:` route to in-process handlers via dispatchInternalWake
// instead of writing a hub_send. Each handler is responsible for re-arming
// itself.
func (g *Gemma) dispatchWakes() {
	wakes, err := g.db.ListPendingWakes(time.Now())
	if err != nil {
		log.Printf("[wake] list pending: %v", err)
		return
	}
	for _, w := range wakes {
		if strings.HasPrefix(w.TargetAgent, internalTargetPrefix) {
			g.dispatchInternalWake(w)
			if _, err := g.db.MarkWakeFired(w.ID); err != nil {
				log.Printf("[wake] mark-fired errored (id=%d): %v", w.ID, err)
			}
			continue
		}
		msg := protocol.Message{
			FromAgent: agentID,
			ToAgent:   w.TargetAgent,
			Type:      protocol.MsgCommand,
			Content:   w.Payload,
			Created:   time.Now(),
		}
		if _, err := g.db.InsertMessage(msg); err != nil {
			log.Printf("[wake] dispatch failed (id=%d target=%s): %v", w.ID, w.TargetAgent, err)
			if _, mErr := g.db.MarkWakeFailed(w.ID); mErr != nil {
				log.Printf("[wake] mark-failed errored (id=%d): %v", w.ID, mErr)
			}
			continue
		}
		if _, err := g.db.MarkWakeFired(w.ID); err != nil {
			log.Printf("[wake] mark-fired errored (id=%d): %v", w.ID, err)
		}
	}
}

// internalTargetPrefix marks wake_schedule rows whose target is an in-
// process handler instead of a registered agent. Routed via
// dispatchInternalWake.
const internalTargetPrefix = "_internal:"

// internalDocDriftTarget is the target_agent value for the periodic H-23
// doc-drift sentinel scan. Each fire scans bot-hq's docs/arcs and emits
// observations via slice 2's EmitDocDriftObservations, then re-arms.
const internalDocDriftTarget = "_internal:docdrift"

// docDriftInterval is the cadence between H-23 doc-drift scans. 30 minutes
// is well below the typical arc-pointer-drift turnaround (a closed-arc
// pointer that should have been refined would otherwise sit stale until
// next manual catch).
const docDriftInterval = 30 * time.Minute

// dispatchInternalWake routes _internal:* wakes to in-process handlers.
// Each handler is responsible for re-arming itself; this dispatcher is
// pure routing. Unknown internal targets are logged and dropped (the row
// is still marked fired by the caller so it doesn't pile up as pending).
//
// Phase H slice 3 C7 (#6 H-23 periodic invoker).
func (g *Gemma) dispatchInternalWake(w hub.WakeSchedule) {
	switch w.TargetAgent {
	case internalDocDriftTarget:
		g.runDocDriftScanAndReArm()
	default:
		log.Printf("[wake] unknown internal target: %s (id=%d)", w.TargetAgent, w.ID)
	}
}

// runDocDriftScanAndReArm scans bot-hq's docs/arcs for arc-pointer drift
// and emits observations to the slice 2 docdrift ledger via the existing
// EmitDocDriftObservations path. Re-arms the next wake unconditionally so
// the loop continues even if the scan itself errored.
//
// Best-effort: if cwd is not a bot-hq checkout (no docs/arcs/), the scan
// returns 0 observations and the re-arm still fires — the next tick is
// equally a no-op until the binary lands somewhere with arcs to scan.
func (g *Gemma) runDocDriftScanAndReArm() {
	obs, err := ScanArcsForDocDrift("docs/arcs", ".")
	if err != nil {
		log.Printf("[wake] docdrift scan: %v", err)
	} else {
		EmitDocDriftObservations(obs)
	}
	next := time.Now().Add(docDriftInterval)
	if _, err := g.db.InsertWakeSchedule(internalDocDriftTarget, agentID, "", next); err != nil {
		log.Printf("[wake] docdrift re-arm: %v", err)
	}
}

// bootstrapInternalDocDrift schedules the first _internal:docdrift wake at
// Emma start if no pending one exists. Subsequent wakes are scheduled by
// runDocDriftScanAndReArm. Idempotent across rebuilds — skips if a
// pending wake from a prior boot is still in the table.
func (g *Gemma) bootstrapInternalDocDrift() {
	pending, err := g.db.HasPendingWakeForTarget(internalDocDriftTarget)
	if err != nil {
		log.Printf("[wake] docdrift bootstrap pending check: %v", err)
		return
	}
	if pending {
		return
	}
	next := time.Now().Add(docDriftInterval)
	if _, err := g.db.InsertWakeSchedule(internalDocDriftTarget, agentID, "", next); err != nil {
		log.Printf("[wake] docdrift bootstrap: %v", err)
	}
}

// heartbeatLoop refreshes Emma's last_seen on a fast cadence so she stays
// in panestate.ActivityOnline (and thus visible in the hub strip) during
// quiet observation periods. Claude-pane agents get this refresh for free
// via the MCP middleware on every tool call (internal/mcp/tools.go);
// Emma is a Go-internal monitor with no MCP entry point, so the refresh
// must be explicit.
//
// Interval is well within panestate.HeartbeatOnlineWindow (60s) — 30s gives 2x
// margin against GC pauses and schedule jitter.
func (g *Gemma) heartbeatLoop() {
	ticker := time.NewTicker(heartbeatInterval)
	defer ticker.Stop()

	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			_ = g.db.UpdateAgentLastSeen(agentID)
		}
	}
}

// monitorLoop proactively runs Tier 1 health checks on a configurable interval.
func (g *Gemma) monitorLoop() {
	ticker := time.NewTicker(g.monitorInterval)
	defer ticker.Stop()

	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			g.runHealthChecks()
		}
	}
}

// anomaly is a single detected condition with a stable key for hysteresis
// dedupe and a human-readable message for the flag body.
type anomaly struct {
	key, msg string
}

// checkAgentImbalance reports an offline-ratio anomaly if non-coder agents
// skew offline. Coders (protocol.AgentCoder) are excluded from the count
// because they are spawn-and-die by design — their offline state is the
// expected steady state, not an anomaly. Returns (anomaly, true) when the
// imbalance trips, otherwise (zero, false).
func checkAgentImbalance(agents []protocol.Agent) (anomaly, bool) {
	online, offline := 0, 0
	considered := 0
	for _, a := range agents {
		if a.Type == protocol.AgentCoder {
			continue
		}
		considered++
		if a.Status == protocol.StatusOnline || a.Status == protocol.StatusWorking {
			online++
		} else {
			offline++
		}
	}
	if offline > online && considered > 1 {
		return anomaly{
			key: "agent-imbalance",
			msg: fmt.Sprintf("Agent anomaly: %d online, %d offline (coders excluded)", online, offline),
		}, true
	}
	return anomaly{}, false
}

// pseudoFilesystemMounts lists mount-point prefixes whose capacity reading
// is a kernel artifact, not real disk pressure. Filter them before flagging.
// macOS: devfs at /dev, VM scratch at /System/Volumes/VM, firmlinks under
// /System/Volumes/{Preboot,Update}, signed system at /System/Volumes/iSCPreboot.
// Linux: /proc, /sys, /run, /dev are pseudo-fs.
var pseudoFilesystemMounts = []string{
	"/dev",
	"/proc",
	"/sys",
	"/run",
	"/System/Volumes/VM",
	"/System/Volumes/Preboot",
	"/System/Volumes/Update",
	"/System/Volumes/iSCPreboot",
	"/System/Volumes/xarts",
	"/System/Volumes/Hardware",
	"/private/var/vm",
}

func isPseudoMount(mount string) bool {
	for _, prefix := range pseudoFilesystemMounts {
		if mount == prefix || strings.HasPrefix(mount, prefix+"/") {
			return true
		}
	}
	return false
}

// shouldSkipMonitor returns true when monitorLoop should no-op entirely.
// Precondition: no non-emma hub traffic in the last hour AND no agents
// currently in `working` status. Avoids alerting about an empty house.
func (g *Gemma) shouldSkipMonitor() bool {
	cutoff := time.Now().Add(-monitorPreconditionGap)
	msgs, err := g.db.GetRecentMessages(50)
	if err == nil {
		for _, m := range msgs {
			if m.FromAgent != agentID && m.Created.After(cutoff) {
				return false
			}
		}
	}

	agents, err := g.db.ListAgents("")
	if err == nil {
		for _, a := range agents {
			if a.Status == protocol.StatusWorking {
				return false
			}
		}
	}
	return true
}

// shouldFlag applies hysteresis (per-condition) and the shared rate cap.
// Returns true if this anomaly should be emitted now and records it.
func (g *Gemma) shouldFlag(condition string, now time.Time) bool {
	g.flagMu.Lock()
	defer g.flagMu.Unlock()

	if last, ok := g.flagHistory[condition]; ok {
		if now.Sub(last) < flagHysteresisWindow {
			return false
		}
	}

	cutoff := now.Add(-time.Hour)
	pruned := g.flagWindow[:0]
	for _, t := range g.flagWindow {
		if t.After(cutoff) {
			pruned = append(pruned, t)
		}
	}
	g.flagWindow = pruned
	if len(g.flagWindow) >= flagRateCapPerHour {
		return false
	}

	g.flagHistory[condition] = now
	g.flagWindow = append(g.flagWindow, now)
	return true
}

// runHealthChecks executes Tier 1 checks and reports anomalies to Rain.
// Refinement A: hub-quiet is no longer a standalone flag — it's only
// useful inside a Tier 3 stall-detector conjunction (Phase F).
// Refinement B: the whole loop no-ops when nobody is working.
func (g *Gemma) runHealthChecks() {
	if g.shouldSkipMonitor() {
		return
	}

	var anomalies []anomaly
	projectDir := ProjectDir()

	// 1. Run go test in bot-hq project directory
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	testCmd := exec.CommandContext(ctx, "go", "test", "./...")
	testCmd.Dir = projectDir
	testOut, testErr := testCmd.CombinedOutput()
	cancel()
	if testErr != nil {
		anomalies = append(anomalies, anomaly{
			key: "go-test-fail",
			msg: fmt.Sprintf("go test FAIL: %s", summarizeOutput(string(testOut), 500)),
		})
	}

	// 2. System health: disk space
	ctx2, cancel2 := context.WithTimeout(context.Background(), 10*time.Second)
	dfCmd := exec.CommandContext(ctx2, "df", "-h")
	dfOut, _ := dfCmd.CombinedOutput()
	cancel2()
	for _, line := range strings.Split(string(dfOut), "\n") {
		fields := strings.Fields(line)
		// macOS df -h: Filesystem Size Used Avail Capacity iused ifree %iused Mounted-on (9 cols)
		// Linux df -h: Filesystem Size Used Avail Use% Mounted-on (6 cols)
		if len(fields) < 6 {
			continue
		}
		pct := strings.TrimSuffix(fields[4], "%")
		var usage int
		if _, err := fmt.Sscanf(pct, "%d", &usage); err != nil || usage < 90 {
			continue
		}
		mount := fields[len(fields)-1]
		if isPseudoMount(mount) {
			continue
		}
		anomalies = append(anomalies, anomaly{
			key: "disk:" + mount,
			msg: fmt.Sprintf("Disk usage high: %s at %s%%", mount, fields[4]),
		})
	}

	// 3. System health: memory (macOS vm_stat)
	ctx3, cancel3 := context.WithTimeout(context.Background(), 10*time.Second)
	vmCmd := exec.CommandContext(ctx3, "vm_stat")
	vmOut, vmErr := vmCmd.CombinedOutput()
	cancel3()
	if vmErr != nil {
		anomalies = append(anomalies, anomaly{key: "vm-stat-failed", msg: "vm_stat failed"})
	} else {
		// Sum "available" pages: free + inactive + speculative + purgeable.
		// macOS reports very few "Pages free" under normal load because the kernel
		// aggressively caches via the other buckets; only the combined total reflects
		// actual memory pressure.
		availablePages := 0
		for _, line := range strings.Split(string(vmOut), "\n") {
			var match string
			switch {
			case strings.HasPrefix(line, "Pages free:"):
				match = "Pages free:"
			case strings.HasPrefix(line, "Pages inactive:"):
				match = "Pages inactive:"
			case strings.HasPrefix(line, "Pages speculative:"):
				match = "Pages speculative:"
			case strings.HasPrefix(line, "Pages purgeable:"):
				match = "Pages purgeable:"
			}
			if match == "" {
				continue
			}
			parts := strings.Fields(line)
			if len(parts) < 3 {
				continue
			}
			pagesStr := strings.TrimSuffix(parts[len(parts)-1], ".")
			var pages int
			fmt.Sscanf(pagesStr, "%d", &pages)
			availablePages += pages
		}
		// Flag only if combined available memory drops below ~512MB (16384 pages on Apple Silicon 16KB pages).
		if availablePages > 0 && availablePages < 32768 {
			anomalies = append(anomalies, anomaly{
				key: "low-memory",
				msg: fmt.Sprintf("Low available memory: %d pages (free+inactive+speculative+purgeable)", availablePages),
			})
		}
	}

	// 4. Hub agent status
	agents, err := g.db.ListAgents("")
	if err == nil {
		if a, ok := checkAgentImbalance(agents); ok {
			anomalies = append(anomalies, a)
		}
	}

	// Apply hysteresis + rate cap, then emit the survivors as one bundled flag.
	now := time.Now()
	var allowed []string
	for _, a := range anomalies {
		if g.shouldFlag(a.key, now) {
			allowed = append(allowed, a.msg)
		}
	}
	if len(allowed) > 0 {
		report := "Monitor check anomalies:\n- " + strings.Join(allowed, "\n- ")
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   "rain",
			Type:      protocol.MsgResult,
			Content:   report,
		})
	}
}

// summarizeOutput truncates output to maxLen, keeping the tail.
func summarizeOutput(s string, maxLen int) string {
	s = strings.TrimSpace(s)
	if len(s) <= maxLen {
		return s
	}
	return "..." + s[len(s)-maxLen:]
}

// restartOllama kills and restarts the Ollama process.
func (g *Gemma) restartOllama() {
	g.mu.Lock()
	defer g.mu.Unlock()

	if g.ollamaCmd != nil && g.ollamaCmd.Process != nil {
		g.ollamaCmd.Process.Kill()
		g.ollamaCmd.Wait()
	}

	g.ollamaCmd = exec.Command("ollama", "serve")
	g.ollamaCmd.Env = append(os.Environ(),
		"OLLAMA_FLASH_ATTENTION=1",
		"OLLAMA_KV_CACHE_TYPE=q8_0",
	)
	g.ollamaCmd.Stdout = nil
	g.ollamaCmd.Stderr = nil
	if err := g.ollamaCmd.Start(); err != nil {
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			Type:      protocol.MsgError,
			Content:   fmt.Sprintf("Ollama restart failed: %v", err),
		})
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if g.waitForHealth(ctx) {
		g.db.UpdateAgentStatus(agentID, protocol.StatusOnline, "")
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			Type:      protocol.MsgUpdate,
			Content:   "Ollama restarted successfully.",
		})
	} else {
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			Type:      protocol.MsgError,
			Content:   "Ollama restart: health check timed out.",
		})
	}
}
