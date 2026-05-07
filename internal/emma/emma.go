// Package emma is the Phase S S-1b emma-Claude rule-enforcer agent.
//
// User msg 15734 directive: "I want emma to be the enforcer of the trio.
// She will know the rule and the rules are absolute for her. She will be
// the user's companion in enforcing rules. She will have full access to
// the hub like the BRAIN-duo, but she will not speak unless spoken to,
// or unless you violate a rule."
//
// Architecture: parallel to internal/brian/ + internal/rain/ — Claude
// Code session in tmux pane, polls hub for messages, system-prompt
// loads rulebook + scope-prior + tool-restrictions. EYES-class read-
// only enforcer (no Edit/Write/Bash/commit-push tool access enforced
// via system-prompt directive + per-agent settings.json permissions
// deny block — defense-in-depth per Rain msg 15835 OQ-S1b-1).
//
// Speech-trigger gating (rule-text discipline): silent unless `@emma`
// mention in hub broadcast OR rule-violation observed. Output via
// hub_send broadcast only ([HR]-when-violation-warrants-user /
// untagged-when-peer-coord-violation-call). NO system-reminder
// pane-injection.
//
// Scope-prior (A) guided-discretion default per user msg 15760
// "Proceed and smoke all" implicit-greenflag: rulebook + narrative-
// class scope-prior baseline + discretion-clause "rules-are-absolute,
// flag judgment-warranted outside prior list".
//
// Agent-ID convention (Rain msg 15835 OQ-S1b-5 fork (a)):
//   - "emma" = the rule-enforcer ROLE (this package)
//   - "gemma" = the gemma4:e4b LLM process (internal implementation;
//     re-IDed per S-1b commit)
//   - daemoncron emits keep FromAgent="emma" (cron-mechanism
//     contributing to emma's role-identity)
//   - emma-Claude registers as "emma" in hub.DB
package emma

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
)

const (
	agentID   = "emma"
	agentName = "Emma"
	agentType = protocol.AgentBrian // Claude-class peer (no AgentEmma type defined; stays AgentBrian-class to match brian/rain peer-class)

	pollInterval = 3 * time.Second
)

// Emma manages a Claude Code session that acts as the trio rule-
// enforcer. Spawns in tmux + registers as "emma" agent + polls for
// hub messages. EYES-class read-only — system-prompt directive +
// per-agent settings.json permissions block enforce no-edit
// discipline (defense-in-depth).
type Emma struct {
	db          *hub.DB
	workDir     string
	tmuxSession string
	lastMsgID   int64

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}
}

// New creates an Emma instance. workDir is where the Claude Code
// session runs.
func New(db *hub.DB, workDir string) *Emma {
	if workDir == "" {
		home, _ := os.UserHomeDir()
		workDir = filepath.Join(home, "Projects")
	}
	return &Emma{
		db:      db,
		workDir: workDir,
		stopCh:  make(chan struct{}),
	}
}

// Start spawns the Claude Code session in tmux + registers as the
// "emma" agent + begins polling. Mirrors brian/rain Start lifecycle.
func (e *Emma) Start() error {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.running {
		return nil
	}

	if err := os.MkdirAll(e.workDir, 0700); err != nil {
		return fmt.Errorf("emma work dir: %w", err)
	}

	if err := e.writeMCPConfig(); err != nil {
		return fmt.Errorf("emma mcp config: %w", err)
	}

	if err := e.writeSettingsJSON(); err != nil {
		return fmt.Errorf("emma settings.json: %w", err)
	}

	e.tmuxSession = fmt.Sprintf("bot-hq-emma-%d", time.Now().Unix())

	if err := e.spawnTmux(); err != nil {
		return fmt.Errorf("emma tmux spawn: %w", err)
	}

	metaJSON, _ := json.Marshal(map[string]string{"tmux_target": e.tmuxSession})
	agent := protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   agentType,
		Status: protocol.StatusOnline,
		Meta:   string(metaJSON),
	}
	if err := e.db.RegisterAgent(agent); err != nil {
		return fmt.Errorf("emma register: %w", err)
	}

	e.running = true
	go e.pollLoop()

	return nil
}

// Stop shuts down the emma session.
func (e *Emma) Stop() {
	e.mu.Lock()
	defer e.mu.Unlock()
	if !e.running {
		return
	}
	e.running = false
	close(e.stopCh)
	e.db.UpdateAgentStatus(agentID, protocol.StatusOffline, "")
	exec.Command("tmux", "kill-session", "-t", e.tmuxSession).Run()
}

// IsRunning reports whether emma is active.
func (e *Emma) IsRunning() bool {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.running
}

// writeMCPConfig writes the bot-hq MCP config so emma's Claude
// session has hub_read / hub_send / hub_flag / hub_session_load
// access. The settings.json (writeSettingsJSON) deny-list constrains
// the OTHER tool surfaces (Edit / Write / Bash / etc.) at the
// permissions-toolgate layer.
func (e *Emma) writeMCPConfig() error {
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
	configPath := filepath.Join(e.workDir, ".bot-hq-emma-mcp.json")
	return os.WriteFile(configPath, data, 0600)
}

// writeSettingsJSON writes a per-agent ~/.claude settings file that
// declares emma's tool-permission-deny list — EYES-class read-only
// boundary enforcement at the permissions-toolgate layer (defense-
// in-depth alongside the system-prompt directive). Per Rain msg
// 15835 OQ-S1b-1 BOTH-layers disposition.
//
// Note: Claude Code reads ~/.claude/settings.json globally per-user;
// per-agent isolation requires CLAUDE_HOME-style env override OR
// distinct workDir-scoped settings file. This commit writes a
// workDir-local settings file that emma's session can pick up via
// claude-cli `--settings` flag (if supported) OR via documentation
// for user post-rebuild manual install. Pragmatic scope: write the
// canonical deny-list file; user-side install path documented in
// commit-body.
func (e *Emma) writeSettingsJSON() error {
	settings := map[string]any{
		"permissions": map[string]any{
			"deny": []string{
				"Edit",
				"Write",
				"Bash",
				"NotebookEdit",
				"Skill",
				"mcp__bot-hq__hub_spawn",
				"mcp__bot-hq__hub_spawn_gemma",
			},
			"allow": []string{
				"Read",
				"mcp__bot-hq__hub_read",
				"mcp__bot-hq__hub_send",
				"mcp__bot-hq__hub_flag",
				"mcp__bot-hq__hub_register",
				"mcp__bot-hq__hub_session_load",
				"mcp__bot-hq__hub_session_create",
				"mcp__bot-hq__hub_session_close",
				"mcp__bot-hq__hub_status",
				"mcp__bot-hq__hub_agents",
				"mcp__bot-hq__hub_read",
			},
		},
	}
	data, err := json.MarshalIndent(settings, "", "  ")
	if err != nil {
		return err
	}
	settingsPath := filepath.Join(e.workDir, ".bot-hq-emma-settings.json")
	return os.WriteFile(settingsPath, data, 0600)
}

// newSessionArgs returns the tmux new-session arg list for emma's
// pane. Mirrors brian pattern; injects BOT_HQ_AGENT_ID env-var.
func (e *Emma) newSessionArgs() []string {
	return []string{
		"new-session", "-d", "-s", e.tmuxSession,
		"-c", e.workDir, "-x", "200", "-y", "50",
		"-e", "BOT_HQ_AGENT_ID=" + agentID,
	}
}

// spawnTmux creates emma's tmux session running Claude Code with
// the rule-enforcer prompt + per-agent settings.json applied.
func (e *Emma) spawnTmux() error {
	createCmd := exec.Command("tmux", e.newSessionArgs()...)
	if err := createCmd.Run(); err != nil {
		return fmt.Errorf("tmux new-session: %w", err)
	}
	configPath := filepath.Join(e.workDir, ".bot-hq-emma-mcp.json")
	settingsPath := filepath.Join(e.workDir, ".bot-hq-emma-settings.json")
	claudeCmd := fmt.Sprintf("claude --mcp-config %s --settings %s --dangerously-skip-permissions", configPath, settingsPath)
	if err := tmuxpkg.SendKeys(e.tmuxSession, claudeCmd, true); err != nil {
		return fmt.Errorf("tmux send claude cmd: %w", err)
	}
	time.Sleep(3 * time.Second)
	prompt := e.initialPrompt()
	if err := tmuxpkg.SendKeys(e.tmuxSession, prompt, true); err != nil {
		return fmt.Errorf("tmux send prompt: %w", err)
	}
	return nil
}

// InitialPromptForTest exposes initialPrompt() for cross-package tests.
func (e *Emma) InitialPromptForTest() string { return e.initialPrompt() }
