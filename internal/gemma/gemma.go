package gemma

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	agentID   = "gemma-agent"
	agentName = "Gemma"

	pollInterval   = 3 * time.Second
	healthInterval = 30 * time.Second

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
	"cat",
	"ls",
	"git status",
	"git log",
	"git diff",
}

// Gemma manages the Ollama sidecar and processes tasks via the hub.
type Gemma struct {
	db     *hub.DB
	client *Client
	sem    chan struct{}

	model     string
	ollamaURL string

	ollamaCmd *exec.Cmd
	lastMsgID int64

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

	return &Gemma{
		db:        db,
		model:     model,
		ollamaURL: ollamaURL,
		sem:       make(chan struct{}, maxConc),
		stopCh:    make(chan struct{}),
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
		return fmt.Errorf("gemma register: %w", err)
	}

	// Get current last message ID so we only process new messages
	msgs, err := g.db.GetRecentMessages(1)
	if err == nil && len(msgs) > 0 {
		g.lastMsgID = msgs[0].ID
	}

	g.running = true

	go g.pollLoop()
	go g.healthLoop()

	g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   fmt.Sprintf("Gemma agent online. Model: %s", g.model),
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

	// Acquire semaphore
	select {
	case g.sem <- struct{}{}:
		defer func() { <-g.sem }()
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

// processNewMessages reads hub messages directed at gemma-agent and handles them.
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
