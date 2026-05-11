package mcp

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/outboundhook"
	"github.com/gregoryerrl/bot-hq/internal/projects"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/mark3labs/mcp-go/mcp"
)

func hubSpawn(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_spawn",
		mcp.WithDescription("Spawn a new Claude Code session in a tmux pane"),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project directory path")),
		mcp.WithString("prompt", mcp.Description("Initial prompt to send to Claude Code")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		prompt := req.GetString("prompt", "")

		// Resolve to absolute path to prevent relative path tricks
		absProject, err := filepath.Abs(project)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("invalid path: %v", err)), nil
		}

		// Block system/dangerous directories
		if isBlockedPath(absProject) {
			return mcp.NewToolResultError(fmt.Sprintf("project path is in a restricted system directory: %s", absProject)), nil
		}

		// Validate project path exists and is a directory
		info, err := os.Stat(absProject)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("project path invalid: %v", err)), nil
		}
		if !info.IsDir() {
			return mcp.NewToolResultError(fmt.Sprintf("project path is not a directory: %s", absProject)), nil
		}
		project = absProject

		// H-14 pre-flight: hub_spawn requires per-project rules to be loaded
		// from ~/.bot-hq/projects/<name>.yaml before any dispatch. Missing
		// rules = bootstrap required. Closes the bcc-ad-manager incident class
		// (wrong-named branches, force-pushes, destructive ops) structurally.
		// hub_spawn_gemma is unaffected (gemma allowlist is the gate there).
		rules, rulesErr := preflightProjectRules(project)
		if rulesErr != nil {
			return mcp.NewToolResultError(rulesErr.Error()), nil
		}

		sessionID := uuid.New().String()[:8]
		sessionName := fmt.Sprintf("cc-%s", sessionID)

		// If the project is the same repo the bot-hq binary lives in,
		// create a git worktree so the coder doesn't modify files the
		// running server depends on.
		worktreePath := ""
		worktreeBranch := ""
		selfPath, _ := os.Executable()
		if selfPath != "" {
			selfDir := filepath.Dir(selfPath)
			// Check if our binary lives inside the target project
			if strings.HasPrefix(selfDir, project+"/") || selfDir == project {
				branchName := fmt.Sprintf("coder-%s", sessionID)
				wtPath := filepath.Join(project, ".worktrees", branchName)
				// Create worktree with a new branch off HEAD
				mkErr := os.MkdirAll(filepath.Dir(wtPath), 0700)
				if mkErr == nil {
					wtCmd := exec.CommandContext(ctx, "git", "-C", project, "worktree", "add", "-b", branchName, wtPath, "HEAD")
					if wtErr := wtCmd.Run(); wtErr == nil {
						worktreePath = wtPath
						worktreeBranch = branchName
						project = wtPath // coder works in the worktree
						// H-3b: install pre-commit freshness gate so the coder
						// can't merge a stale-base commit if main advances
						// during their session. Soft-failure: log on error but
						// don't abort spawn — the hook is a safety rail, not a
						// hard prerequisite. Logging matters: a silent install
						// regression would leave the gate disabled invisibly,
						// defeating its purpose.
						if err := installWorktreeHooks(ctx, wtPath); err != nil {
							log.Printf("[hub-spawn] worktree hooks install failed for %s: %v (spawn continuing without gates)", wtPath, err)
						}
					}
				}
			}
		}

		// Write MCP config so the coder agent can reach bot-hq hub tools
		mcpConfigPath := filepath.Join(project, fmt.Sprintf(".bot-hq-coder-%s-mcp.json", sessionID))
		botHQPath, mcpErr := os.Executable()
		if mcpErr != nil {
			botHQPath, mcpErr = exec.LookPath("bot-hq")
		}
		if mcpErr == nil {
			mcpCfg := map[string]any{
				"mcpServers": map[string]any{
					"bot-hq": map[string]any{
						"command": botHQPath,
						"args":    []string{"mcp"},
					},
				},
			}
			if data, err := json.MarshalIndent(mcpCfg, "", "  "); err == nil {
				os.WriteFile(mcpConfigPath, data, 0600)
			}
		}

		// Spawn-contract bake (slice-5 H-22-bis item 3): install the
		// OUTBOUND-MISS Stop hook into the project-scoped settings.json
		// and set BOT_HQ_AGENT_ID in the tmux session env so the hook
		// knows which agent it fires for. Best-effort — soft-failure
		// logged but spawn continues. Mirrors the worktree-hooks pattern.
		if mcpErr == nil {
			projectSettingsPath := filepath.Join(project, ".claude", "settings.json")
			if err := outboundhook.InstallDuoHook(projectSettingsPath, botHQPath); err != nil {
				log.Printf("[hub-spawn] outbound-miss hook install failed for %s: %v (spawn continuing without hook)", projectSettingsPath, err)
			}
		}

		// Create a new tmux session in the project directory. Set
		// BOT_HQ_AGENT_ID via -e so the spawned claude process inherits
		// it and the Stop hook subcommand can identify the agent.
		cmd := exec.CommandContext(ctx, "tmux", "new-session", "-d",
			"-s", sessionName,
			"-c", project,
			"-e", fmt.Sprintf("BOT_HQ_AGENT_ID=%s", sessionID),
		)
		if err := cmd.Run(); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("spawn failed: %v", err)), nil
		}

		// Build claude command — include MCP config if we wrote one
		claudeCmd := "claude --dangerously-skip-permissions"
		if mcpErr == nil {
			claudeCmd = fmt.Sprintf("claude --mcp-config %s --dangerously-skip-permissions", mcpConfigPath)
		}

		// Send claude command via send-keys (-l for literal to prevent key name injection)
		sendArgs := []string{"send-keys", "-t", sessionName, "-l", claudeCmd}
		if err := exec.CommandContext(ctx, "tmux", sendArgs...).Run(); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("failed to start claude: %v", err)), nil
		}
		// Send Enter separately (cannot use -l for Enter)
		if err := exec.CommandContext(ctx, "tmux", "send-keys", "-t", sessionName, "Enter").Run(); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("failed to send Enter: %v", err)), nil
		}

		// Track in DB
		db.InsertClaudeSession(hub.ClaudeSession{
			ID:         sessionID,
			Project:    project,
			TmuxTarget: sessionName,
			Mode:       "managed",
			Status:     "running",
			Started:    time.Now(),
		})

		// Register as an agent so it shows up in the hub
		metaJSON, _ := json.Marshal(map[string]string{"tmux_target": sessionName})
		db.RegisterAgent(protocol.Agent{
			ID:      sessionID,
			Name:    fmt.Sprintf("Coder %s", sessionID),
			Type:    protocol.AgentCoder,
			Status:  protocol.StatusOnline,
			Project: project,
			Meta:    string(metaJSON),
		})

		// Send initial prompt with hub communication instructions.
		// Bug #2 fix: replace brittle time.Sleep(3s) gate with a state-gated
		// WaitForPrompt. The 3s sleep failed when Claude's boot was slower
		// than expected (cold cache, --mcp-config loading) — the prompt got
		// sent into a pre-prompt buffer and was eaten. Now we poll until
		// the input prompt is visible. BOT_HQ_CC_BOOT_TIMEOUT env var
		// overrides the default 30s for slow-CI / cold-cache contexts.
		bootTimeout := 30 * time.Second
		if v := os.Getenv("BOT_HQ_CC_BOOT_TIMEOUT"); v != "" {
			if d, err := time.ParseDuration(v); err == nil && d > 0 {
				bootTimeout = d
			}
		}
		if at, _, err := tmuxpkg.WaitForPrompt(sessionName, bootTimeout); err != nil || !at {
			return mcp.NewToolResultError(fmt.Sprintf("claude session did not reach prompt within %v (bug #2 boot wait)", bootTimeout)), nil
		}
		worktreeNote := ""
		if worktreePath != "" {
			worktreeNote = fmt.Sprintf(`
NOTE: You are working in a git worktree at %s (branch: %s).
This is an isolated copy — the main repo is running a live server. Commit your changes to this branch.
When done, Brian or the user will merge your branch into main.
Your worktree has a pre-commit hook that blocks commits if origin/main has advanced past your base.
If a commit fails with "stale base", run `+"`git fetch origin && git rebase origin/main`"+` then retry.
`, worktreePath, worktreeBranch)
		}
		hubPreamble := buildCoderPreamble(sessionID, worktreeNote, rules)
		fullPrompt := hubPreamble + prompt
		if prompt == "" {
			fullPrompt = hubPreamble + "Awaiting instructions. Register yourself and stand by."
		}
		// Use -l (literal) to prevent tmux key name injection in user prompts
		exec.CommandContext(ctx, "tmux", "send-keys", "-t", sessionName, "-l", fullPrompt).Run()
		// Claude Code's bracketed paste needs time to process before Enter
		time.Sleep(500 * time.Millisecond)
		exec.CommandContext(ctx, "tmux", "send-keys", "-t", sessionName, "Enter").Run()

		result := map[string]string{
			"status":     "spawned",
			"session_id": sessionID,
			"tmux":       sessionName,
			"project":    project,
		}
		if worktreePath != "" {
			result["worktree"] = worktreePath
			result["branch"] = worktreeBranch
		}
		return mcp.NewToolResultText(toJSON(result)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// Z-3 Group I: hub_spawn_gemma MCP tool DELETED. emma (formerly the
// gemma-package agent) is now a Claude Code instance configured for
// DeepSeek-V4-Pro via R51+R52 env-var injection (same proxy pattern as
// rain) — no longer talks to Ollama. The "gemma" naming was legacy from
// when emma ran the Ollama gemma:e4b model. Removed per architecture/
// sessions-as-containers.md "emma minimum-viable capability" + plan §I3.

// blockedPrefixes are system directories that hub_spawn should never use.
var blockedPrefixes = func() []string {
	common := []string{
		"/etc", "/bin", "/sbin", "/usr", "/lib", "/lib64",
		"/boot", "/dev", "/proc", "/sys", "/run",
		"/var/run", "/var/log",
	}
	if runtime.GOOS == "darwin" {
		common = append(common, "/System", "/Library", "/private/var", "/private/etc")
	}
	return common
}()

// isBlockedPath returns true if the path is inside a system/dangerous directory.
func isBlockedPath(absPath string) bool {
	// Block filesystem root
	if absPath == "/" {
		return true
	}
	for _, prefix := range blockedPrefixes {
		if absPath == prefix || strings.HasPrefix(absPath, prefix+"/") {
			return true
		}
	}
	return false
}

// preflightProjectRules enforces H-14: hub_spawn requires a per-project rules
// file at ~/.bot-hq/projects/<name>.yaml for the project's git remote. Missing
// rules surface a structured bootstrap message; mismatch and load errors are
// passed through with friendly framing.
//
// Rules object is returned for the caller to use in subsequent steps (C3
// will pass it into the coder preamble for H-3c push policy + H-16 tool
// allowlist). C2 only enforces the gate.
func preflightProjectRules(project string) (*projects.Rules, error) {
	rules, err := projects.LoadForProject(project)
	if err == nil {
		return rules, nil
	}

	if errors.Is(err, projects.ErrNoRulesFound) {
		// LoadForProject's error message includes the canonical project name
		// (its derivation from the actual git remote URL). Extract it from
		// the message rather than re-running `exec.Command("git", "remote",
		// "get-url")` — saves one I/O round-trip per failed dispatch.
		name := projectNameFromNoRulesErr(err)
		msg := fmt.Sprintf("hub_spawn blocked: no project rules for %q.\n\n"+
			"Bootstrap required before dispatch:\n"+
			"  1. Inspect existing branch convention:  git -C %s branch -r | head -20\n"+
			"  2. Copy template:  cp docs/examples/projects/_default.yaml ~/.bot-hq/projects/%s.yaml\n"+
			"  3. Edit ~/.bot-hq/projects/%s.yaml — set remote_url, project_name, branch_pattern\n"+
			"  4. Retry hub_spawn\n\n"+
			"This gate exists to prevent bot-hq from leaking AI infrastructure or\n"+
			"taking destructive actions in projects without explicit per-project rules.\n"+
			"See docs/arcs/phase-h.md (H-4 / H-14).",
			name, project, name, name)
		return nil, &preflightError{msg: msg, underlying: projects.ErrNoRulesFound}
	}

	if errors.Is(err, projects.ErrRemoteMismatch) {
		msg := fmt.Sprintf("hub_spawn blocked: project rules file remote_url does not match the project's actual origin.\n%v\n\n"+
			"Either edit the rules file's remote_url to match, or remove it and re-bootstrap.", err)
		return nil, &preflightError{msg: msg, underlying: projects.ErrRemoteMismatch}
	}

	return nil, fmt.Errorf("hub_spawn blocked: project rules error: %w", err)
}

// preflightError wraps a user-facing message with an underlying sentinel so
// callers can `errors.Is(err, projects.ErrNoRulesFound)` etc. without the
// sentinel's text appearing twice in the visible message.
type preflightError struct {
	msg        string
	underlying error
}

func (e *preflightError) Error() string { return e.msg }
func (e *preflightError) Unwrap() error { return e.underlying }

// projectNameFromNoRulesErr extracts the canonical project name from the
// LoadForProject no-rules error message (`for project "<name>"`). Returns
// "<project>" as a defensive fallback if parsing fails (would only happen
// if LoadForProject's message format changes — covered by tests).
func projectNameFromNoRulesErr(err error) string {
	msg := err.Error()
	const marker = `for project "`
	i := strings.LastIndex(msg, marker)
	if i < 0 {
		return "<project>"
	}
	rest := msg[i+len(marker):]
	end := strings.IndexByte(rest, '"')
	if end < 0 {
		return "<project>"
	}
	return rest[:end]
}

// buildCoderPreamble constructs the initial-prompt prefix sent to a spawned
// coder. Always includes the baseline hub-comm instructions; conditionally
// includes a worktree note (when the coder is working in a git worktree)
// and per-project policy sections derived from rules:
//
//   - PUSH POLICY (H-3c) — when rules.PushRequiresApproval
//   - TOOL ALLOWLIST (H-16) — when rules.CoderToolsBlocked is non-empty
//   - BRANCH NAMING — when rules.BranchPattern is non-empty
//
// rules may be nil (defensive); a nil rules object emits no policy sections.
func buildCoderPreamble(sessionID, worktreeNote string, rules *projects.Rules) string {
	var policy strings.Builder
	if rules != nil {
		if rules.PushRequiresApproval {
			policy.WriteString(`
PUSH POLICY: This project requires explicit user approval before any git push.
- Do NOT run ` + "`git push`" + ` or ` + "`git push --set-upstream`" + ` without approval.
- When push is needed, hub_send broadcast with @brian mention: "@brian ready to push branch <name>, awaiting approval".
- Wait for explicit approval before pushing.
`)
		}
		if rules.ForcePushBlocked {
			policy.WriteString(`
FORCE-PUSH POLICY: Force-pushes are HARD-BLOCKED in this project. This includes ` + "`--force`" + ` AND ` + "`--force-with-lease`" + ` variants.
- If a force-push is unavoidable, hub_send broadcast with @brian mention: "@brian request_force_push: <branch>@<sha>".
- WAIT for brian to relay an approved greenlight back to you. Do NOT push until approval arrives.
- Brian will only relay approval after the user types the exact verbatim token. No partial matches accepted.
- Do NOT attempt to construct or guess the token yourself. The user must type it.
`)
		}
		if len(rules.CoderToolsBlocked) > 0 {
			var blocked strings.Builder
			for _, item := range rules.CoderToolsBlocked {
				blocked.WriteString("  - " + item + "\n")
			}
			// Header reads as a list of blocked commands (literal). The H-16
			// feature name is "coder tool allowlist" framed as the policy
			// concept, but the runtime artifact a coder reads is a blocklist —
			// avoid the cognitive mismatch by labeling the section after what
			// it actually is. Per Rain msg 3294 obs #1.
			policy.WriteString(`
BLOCKED COMMANDS: The following commands are BLOCKED in this project. Do not run them.
If asked to run one of these, refuse and PM brian explaining the block.
` + strings.TrimRight(blocked.String(), "\n") + "\n")
		}
		if rules.BranchPattern != "" {
			policy.WriteString("\nBRANCH NAMING: Branches in this project must match pattern: " + rules.BranchPattern + "\n")
			if rules.BranchPatternHelp != "" {
				policy.WriteString("Hint: " + rules.BranchPatternHelp + "\n")
			}
			if len(rules.BranchExamples) > 0 {
				policy.WriteString("Examples: " + strings.Join(rules.BranchExamples, ", ") + "\n")
			}
		}
	}

	return fmt.Sprintf(`You are a coder agent (ID: %s) in the bot-hq system. You have bot-hq MCP tools available.

IMPORTANT: Communicate your progress on the hub so other agents can see what you're doing.
Phase S S-4: PM 'to:' removed — use @<agent> mention in content for targeting.
- When you START work: hub_send(from="%s", type="update", content="@brian Starting: <brief description>")
- When you FINISH or hit a blocker: hub_send(from="%s", type="result", content="@brian <what you did or what's blocking>")
- Keep hub messages short — one or two sentences max.
%s%s
Your task:
`, sessionID, sessionID, sessionID, worktreeNote, policy.String())
}
