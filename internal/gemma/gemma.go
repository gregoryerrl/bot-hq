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

	// Phase D flag bookkeeping. flagHistory dedupes by condition key
	// (last-fired timestamp). flagWindow is a sliding 1h record of all
	// fired flags for the shared rate cap.
	flagMu      sync.Mutex
	flagHistory map[string]time.Time
	flagWindow  []time.Time
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

	go g.pollLoop()
	go g.healthLoop()
	go g.heartbeatLoop()
	go g.monitorLoop()

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
		prompt := fmt.Sprintf("Summarize this output concisely. Flag any errors or anomalies:\n\n```\n%s\n```", result)
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
		}
	}
}

// heartbeatLoop refreshes Emma's last_seen on a fast cadence so she stays
// in panestate.ActivityOnline (and thus visible in the hub strip) during
// quiet observation periods. Claude-pane agents get this refresh for free
// via the MCP middleware on every tool call (internal/mcp/tools.go);
// Emma is a Go-internal monitor with no MCP entry point, so the refresh
// must be explicit.
//
// Interval is well within panestate.OnlineWindow (60s) — 30s gives 2x
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
		online, offline := 0, 0
		for _, a := range agents {
			if a.Status == protocol.StatusOnline || a.Status == protocol.StatusWorking {
				online++
			} else {
				offline++
			}
		}
		if offline > online && len(agents) > 1 {
			anomalies = append(anomalies, anomaly{
				key: "agent-imbalance",
				msg: fmt.Sprintf("Agent anomaly: %d online, %d offline", online, offline),
			})
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
