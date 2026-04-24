package gemma

import (
	"context"
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
	agentID   = "emma"
	agentName = "Emma"

	pollInterval           = 3 * time.Second
	healthInterval         = 30 * time.Second
	defaultMonitorInterval = 5 * time.Minute

	defaultModel     = "gemma4:e4b"
	defaultOllamaURL = "http://localhost:11434"
	defaultMaxConc   = 3
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
		// Default to exec
		taskType = TaskExec
		command = content
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

// runHealthChecks executes Tier 1 checks and reports anomalies to Brian.
func (g *Gemma) runHealthChecks() {
	var anomalies []string
	projectDir := ProjectDir()

	// 1. Run go test in bot-hq project directory
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	testCmd := exec.CommandContext(ctx, "go", "test", "./...")
	testCmd.Dir = projectDir
	testOut, testErr := testCmd.CombinedOutput()
	cancel()
	if testErr != nil {
		anomalies = append(anomalies, fmt.Sprintf("go test FAIL: %s", summarizeOutput(string(testOut), 500)))
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
		// Skip pseudo-filesystems that always report ~100% on macOS.
		if mount == "/dev" || strings.HasPrefix(mount, "/System/Volumes/VM") || strings.HasPrefix(mount, "/private/var/vm") {
			continue
		}
		anomalies = append(anomalies, fmt.Sprintf("Disk usage high: %s at %s%%", mount, fields[4]))
	}

	// 3. System health: memory (macOS vm_stat)
	ctx3, cancel3 := context.WithTimeout(context.Background(), 10*time.Second)
	vmCmd := exec.CommandContext(ctx3, "vm_stat")
	vmOut, vmErr := vmCmd.CombinedOutput()
	cancel3()
	if vmErr != nil {
		anomalies = append(anomalies, "vm_stat failed")
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
			anomalies = append(anomalies, fmt.Sprintf("Low available memory: %d pages (free+inactive+speculative+purgeable)", availablePages))
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
			anomalies = append(anomalies, fmt.Sprintf("Agent anomaly: %d online, %d offline", online, offline))
		}
	}

	// 5. Hub message activity
	msgs, err := g.db.GetRecentMessages(10)
	if err == nil {
		if len(msgs) == 0 {
			anomalies = append(anomalies, "No recent hub messages")
		} else {
			lastActivity := msgs[0].Created
			if time.Since(lastActivity) > 30*time.Minute {
				anomalies = append(anomalies, fmt.Sprintf("Hub quiet: last message %s ago", time.Since(lastActivity).Round(time.Minute)))
			}
		}
	}

	// Report anomalies to Brian; stay quiet if all healthy
	if len(anomalies) > 0 {
		report := "Monitor check anomalies:\n- " + strings.Join(anomalies, "\n- ")
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   "brian",
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
