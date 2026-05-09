// Package stdiopipe implements T-6 AROUND-CC stdio-pipe driver per
// phase-t.md v5 — the alternative to the tmux-keystroke message-passing
// layer for agent invocation.
//
// Design (from phase-t.md v5 T-6):
//   - bot-hq daemon pre-composes context per-agent
//   - claude CLI subprocess invoked with --print + stdin-piped prompt
//   - Output captured via stdout-pipe (line-streaming for hooks)
//   - Per-agent customization: model-config + prompt-template + tool-permission
//
// MVP scope (this package): driver primitive + Send + Receive + Close API
// + per-agent config wiring (R51 model-config-load via internal/agentconfig).
//
// Production rollout (deferred to user-resume per push-gate-strictness):
//   - Replace internal/rain/rain.spawnTmux + brian/brian spawn paths
//   - Migration strategy: per-agent rolling deployment
//   - Tmux-driver retained as fallback during rollout

package stdiopipe

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/agentconfig"
	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// Driver is one stdio-pipe driver instance for a single agent invocation.
type Driver struct {
	agentID string
	cfg     *hub.AgentModelConfig
	cmd     *exec.Cmd
	stdin   io.WriteCloser
	stdout  io.ReadCloser
	stderr  bytes.Buffer
	scanner *bufio.Scanner
}

// New constructs a Driver for the given agent. Loads agent_model_config
// from hub.db (via T-1.4 R51 + R52 wiring) + prepares the subprocess
// invocation with appropriate env-var swap.
//
// Caller invokes Start() to spawn the subprocess.
func New(db *hub.DB, agentID string) (*Driver, error) {
	if db == nil || agentID == "" {
		return nil, errors.New("db and agentID are required")
	}
	cfg, err := db.GetAgentModelConfig(agentID)
	if err != nil {
		if errors.Is(err, hub.ErrAgentModelConfigNotFound) {
			// Fall through to default Claude OAuth (no env-var injection)
			cfg = &hub.AgentModelConfig{
				AgentID:       agentID,
				Provider:      "anthropic",
				ModelName:     "claude-default",
				BaseURL:       "",
				AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
				Enabled:       true,
			}
		} else {
			return nil, fmt.Errorf("agent_model_config %s: %w", agentID, err)
		}
	}
	return &Driver{agentID: agentID, cfg: cfg}, nil
}

// Start spawns the claude CLI subprocess + opens stdin/stdout/stderr pipes.
// Per R43 narrowed: env-var swap injected for non-Claude paths.
func (d *Driver) Start(ctx context.Context, mcpConfigPath string, workDir string) error {
	if d.cmd != nil {
		return errors.New("driver already started")
	}

	args := []string{"--print", "--mcp-config", mcpConfigPath, "--dangerously-skip-permissions"}
	d.cmd = exec.CommandContext(ctx, "claude", args...)
	d.cmd.Dir = workDir

	// Inject env-vars per R51 PER-AGENT-MODEL-CONFIG-DISCIPLINE
	envVars, err := agentconfig.BuildSpawnEnv(d.cfg)
	if err != nil {
		return fmt.Errorf("BuildSpawnEnv: %w", err)
	}
	env := os.Environ()
	env = append(env, "BOT_HQ_AGENT_ID="+d.agentID)
	for _, ev := range envVars {
		env = append(env, ev.Format())
	}
	d.cmd.Env = env

	stdin, err := d.cmd.StdinPipe()
	if err != nil {
		return fmt.Errorf("stdin pipe: %w", err)
	}
	d.stdin = stdin

	stdout, err := d.cmd.StdoutPipe()
	if err != nil {
		return fmt.Errorf("stdout pipe: %w", err)
	}
	d.stdout = stdout

	d.cmd.Stderr = &d.stderr
	d.scanner = bufio.NewScanner(d.stdout)
	// 1MB buffer to accommodate long outputs
	d.scanner.Buffer(make([]byte, 4096), 1024*1024)

	if err := d.cmd.Start(); err != nil {
		return fmt.Errorf("start claude: %w", err)
	}
	return nil
}

// Send writes a prompt to the subprocess stdin + closes stdin to signal
// end-of-input (claude --print mode reads stdin until EOF).
func (d *Driver) Send(prompt string) error {
	if d.stdin == nil {
		return errors.New("driver not started or already sent")
	}
	if _, err := io.WriteString(d.stdin, prompt); err != nil {
		return fmt.Errorf("write prompt: %w", err)
	}
	if err := d.stdin.Close(); err != nil {
		return fmt.Errorf("close stdin: %w", err)
	}
	d.stdin = nil
	return nil
}

// Receive reads the next stdout line from the subprocess. Returns io.EOF
// when subprocess closes stdout. Use ReceiveAll() to drain to completion.
func (d *Driver) Receive() (string, error) {
	if d.scanner == nil {
		return "", errors.New("driver not started")
	}
	if d.scanner.Scan() {
		return d.scanner.Text(), nil
	}
	if err := d.scanner.Err(); err != nil {
		return "", fmt.Errorf("scan: %w", err)
	}
	return "", io.EOF
}

// ReceiveAll drains stdout until EOF, returning the joined output. Convenience
// wrapper for non-streaming use-cases.
func (d *Driver) ReceiveAll() (string, error) {
	if d.scanner == nil {
		return "", errors.New("driver not started")
	}
	var sb strings.Builder
	for d.scanner.Scan() {
		sb.WriteString(d.scanner.Text())
		sb.WriteByte('\n')
	}
	if err := d.scanner.Err(); err != nil {
		return "", fmt.Errorf("scan: %w", err)
	}
	return sb.String(), nil
}

// Wait blocks until the subprocess exits. Returns exit-code + any error.
func (d *Driver) Wait() (int, error) {
	if d.cmd == nil {
		return -1, errors.New("driver not started")
	}
	err := d.cmd.Wait()
	if exitErr, ok := err.(*exec.ExitError); ok {
		return exitErr.ExitCode(), nil
	}
	if err != nil {
		return -1, fmt.Errorf("wait: %w", err)
	}
	return d.cmd.ProcessState.ExitCode(), nil
}

// Stderr returns captured stderr output from the subprocess.
func (d *Driver) Stderr() string { return d.stderr.String() }

// AgentID returns the configured agent-id.
func (d *Driver) AgentID() string { return d.agentID }

// Config returns the agent model-config used to spawn this driver.
func (d *Driver) Config() *hub.AgentModelConfig { return d.cfg }

// ====== Pre-injection context control ======

// PreInjectionContext is the per-agent context bundle prepended to the
// agent's prompt at spawn-time. Replaces the implicit tmux-pane scrollback
// with explicit programmer-controlled context-composition.
type PreInjectionContext struct {
	AgentID       string
	SessionAnchor string   // recent session-anchor (last R20 AgentState write summary)
	RecentMsgIDs  []int64  // last-N peer-coord msg-ids
	ActivePhase   string   // active phase-doc reference (e.g. "phase-t.md v5")
	CustomBlocks  []string // per-agent extension hooks (T-6 customization)
}

// Compose renders the pre-injection context into a single prompt-prefix string.
func (p *PreInjectionContext) Compose() string {
	var sb strings.Builder
	fmt.Fprintf(&sb, "<!-- BOT-HQ PRE-INJECTION CONTEXT (T-6 stdio-driver) agent=%s composed_at=%s -->\n", p.AgentID, time.Now().UTC().Format(time.RFC3339))
	if p.ActivePhase != "" {
		fmt.Fprintf(&sb, "## Active phase\n%s\n\n", p.ActivePhase)
	}
	if p.SessionAnchor != "" {
		fmt.Fprintf(&sb, "## Last session anchor\n%s\n\n", p.SessionAnchor)
	}
	if len(p.RecentMsgIDs) > 0 {
		fmt.Fprintf(&sb, "## Recent peer-coord msg-ids\n")
		for _, id := range p.RecentMsgIDs {
			fmt.Fprintf(&sb, "  - %d\n", id)
		}
		sb.WriteByte('\n')
	}
	for _, block := range p.CustomBlocks {
		sb.WriteString(block)
		sb.WriteByte('\n')
	}
	sb.WriteString("<!-- END PRE-INJECTION CONTEXT -->\n\n")
	return sb.String()
}
