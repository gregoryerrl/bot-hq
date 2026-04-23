package mcp

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/mark3labs/mcp-go/mcp"
	"github.com/mark3labs/mcp-go/server"
)

// ToolDef pairs an mcp.Tool definition with its handler function.
type ToolDef struct {
	Tool    mcp.Tool
	Handler server.ToolHandlerFunc
}

// BuildTools returns all hub + claude tools wired to the given database.
func BuildTools(db *hub.DB) []ToolDef {
	return []ToolDef{
		hubRegister(db),
		hubUnregister(db),
		hubDeleteAgent(db),
		hubFlag(db),
		hubSend(db),
		hubRead(db),
		hubAgents(db),
		hubSessions(db),
		hubSessionCreate(db),
		hubSessionJoin(db),
		hubStatus(db),
		hubSpawn(db),
		hubCheckpoint(db),
		hubRestore(db),
		hubIssueCreate(db),
		hubIssueList(db),
		hubIssueUpdate(db),
		claudeList(db),
		claudeRead(db),
		claudeMessage(db),
		claudeSend(),
		claudeResume(),
		claudeStop(db),
	}
}

// --- helpers ---

func toJSON(v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		return fmt.Sprintf(`{"error":%q}`, err.Error())
	}
	return string(b)
}

func splitComma(s string) []string {
	if s == "" {
		return nil
	}
	parts := strings.Split(s, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p != "" {
			out = append(out, p)
		}
	}
	return out
}

// --- tool definitions ---

func hubRegister(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_register",
		mcp.WithDescription("Register as an agent in the hub"),
		mcp.WithString("id", mcp.Required(), mcp.Description("Unique agent ID")),
		mcp.WithString("name", mcp.Required(), mcp.Description("Human-readable agent name")),
		mcp.WithString("type", mcp.Required(), mcp.Description("Agent type: coder, voice, brain, discord")),
		mcp.WithString("project", mcp.Description("Project path or name the agent is working on")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		id, err := req.RequireString("id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		if strings.TrimSpace(id) == "" {
			return mcp.NewToolResultError("id must not be empty"), nil
		}
		name, err := req.RequireString("name")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		if strings.TrimSpace(name) == "" {
			return mcp.NewToolResultError("name must not be empty"), nil
		}
		typ, err := req.RequireString("type")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		agentType := protocol.AgentType(typ)
		if !agentType.Valid() {
			return mcp.NewToolResultError(fmt.Sprintf("invalid agent type: %s", typ)), nil
		}

		agent := protocol.Agent{
			ID:         id,
			Name:       name,
			Type:       agentType,
			Status:     protocol.StatusOnline,
			Project:    req.GetString("project", ""),
			Registered: time.Now(),
		}

		if err := db.RegisterAgent(agent); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("register failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":  "registered",
			"agent_id": id,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubUnregister(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_unregister",
		mcp.WithDescription("Unregister (go offline) in the hub"),
		mcp.WithString("id", mcp.Required(), mcp.Description("Agent ID to unregister")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		id, err := req.RequireString("id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		if err := db.UnregisterAgent(id); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("unregister failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":  "offline",
			"agent_id": id,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubDeleteAgent(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_delete_agent",
		mcp.WithDescription("Permanently delete an agent record from the hub database"),
		mcp.WithString("id", mcp.Required(), mcp.Description("Agent ID to delete")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		id, err := req.RequireString("id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		// Protect core agents
		switch id {
		case "brain", "live", "discord", "rain":
			return mcp.NewToolResultError(fmt.Sprintf("cannot delete core agent: %s", id)), nil
		}

		if err := db.DeleteAgent(id); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("delete failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":   "deleted",
			"agent_id": id,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubFlag(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_flag",
		mcp.WithDescription("Flag a message for user attention. Use when: agents disagree and need user input, errors occur, rate limits hit, something stops working, or any situation requiring human decision. This sends a Discord notification."),
		mcp.WithString("from", mcp.Required(), mcp.Description("Agent ID raising the flag")),
		mcp.WithString("reason", mcp.Required(), mcp.Description("Why the user's attention is needed — be specific and concise")),
		mcp.WithString("severity", mcp.Description("Severity: info, warning, critical (default: warning)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		from, err := req.RequireString("from")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		reason, err := req.RequireString("reason")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		severity := "warning"
		if s, sErr := req.RequireString("severity"); sErr == nil && s != "" {
			severity = s
		}

		content := fmt.Sprintf("[%s] %s", strings.ToUpper(severity), reason)

		msg := protocol.Message{
			FromAgent: from,
			ToAgent:   "user",
			Type:      protocol.MsgFlag,
			Content:   content,
			Created:   time.Now(),
		}
		if _, err := db.InsertMessage(msg); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("flag failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":   "flagged",
			"severity": severity,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubSend(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_send",
		mcp.WithDescription("Send a message through the hub"),
		mcp.WithString("from", mcp.Required(), mcp.Description("Sender agent ID")),
		mcp.WithString("to", mcp.Description("Recipient agent ID (empty for broadcast)")),
		mcp.WithString("session_id", mcp.Description("Session ID if part of a session")),
		mcp.WithString("type", mcp.Required(), mcp.Description("Message type: handshake, question, response, command, update, result, error")),
		mcp.WithString("content", mcp.Required(), mcp.Description("Message content")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		from, err := req.RequireString("from")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		if strings.TrimSpace(from) == "" {
			return mcp.NewToolResultError("from must not be empty"), nil
		}
		msgType, err := req.RequireString("type")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		content, err := req.RequireString("content")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		mt := protocol.MessageType(msgType)
		if !mt.Valid() {
			return mcp.NewToolResultError(fmt.Sprintf("invalid message type: %s", msgType)), nil
		}

		msg := protocol.Message{
			FromAgent: from,
			ToAgent:   req.GetString("to", ""),
			SessionID: req.GetString("session_id", ""),
			Type:      mt,
			Content:   content,
			Created:   time.Now(),
		}

		id, err := db.InsertMessage(msg)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("send failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":     "sent",
			"message_id": id,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubRead(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_read",
		mcp.WithDescription("Read messages from the hub"),
		mcp.WithString("agent_id", mcp.Required(), mcp.Description("Agent ID to read messages for")),
		mcp.WithNumber("since_id", mcp.Description("Only return messages after this ID")),
		mcp.WithNumber("limit", mcp.Description("Max messages to return (default 50)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		agentID, err := req.RequireString("agent_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		sinceID := int64(req.GetFloat("since_id", 0))
		limit := req.GetInt("limit", 50)

		msgs, err := db.ReadMessages(agentID, sinceID, limit)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("read failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(msgs)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubAgents(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_agents",
		mcp.WithDescription("List agents registered in the hub"),
		mcp.WithString("status", mcp.Description("Filter by status: online, working, idle, offline")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		status := req.GetString("status", "")
		agents, err := db.ListAgents(status)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("list agents failed: %v", err)), nil
		}
		return mcp.NewToolResultText(toJSON(agents)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubSessions(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_sessions",
		mcp.WithDescription("List sessions in the hub"),
		mcp.WithString("status", mcp.Description("Filter by status: active, paused, done")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		status := req.GetString("status", "")
		sessions, err := db.ListSessions(status)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("list sessions failed: %v", err)), nil
		}
		return mcp.NewToolResultText(toJSON(sessions)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubSessionCreate(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_session_create",
		mcp.WithDescription("Create a new session in the hub"),
		mcp.WithString("mode", mcp.Required(), mcp.Description("Session mode: brainstorm, implement, chat")),
		mcp.WithString("purpose", mcp.Required(), mcp.Description("What the session is for")),
		mcp.WithString("agents", mcp.Description("Comma-separated agent IDs to include")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		mode, err := req.RequireString("mode")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		purpose, err := req.RequireString("purpose")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		sm := protocol.SessionMode(mode)
		if !sm.Valid() {
			return mcp.NewToolResultError(fmt.Sprintf("invalid session mode: %s", mode)), nil
		}

		sess := protocol.Session{
			ID:      uuid.New().String(),
			Mode:    sm,
			Purpose: purpose,
			Agents:  splitComma(req.GetString("agents", "")),
			Status:  protocol.SessionActive,
			Created: time.Now(),
		}

		if err := db.CreateSession(sess); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("create session failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":     "created",
			"session_id": sess.ID,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubSessionJoin(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_session_join",
		mcp.WithDescription("Join an existing session"),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session ID to join")),
		mcp.WithString("agent_id", mcp.Required(), mcp.Description("Agent ID joining the session")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		agentID, err := req.RequireString("agent_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		if err := db.JoinSession(sessionID, agentID); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("join session failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":     "joined",
			"session_id": sessionID,
			"agent_id":   agentID,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubStatus(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_status",
		mcp.WithDescription("Update agent status"),
		mcp.WithString("id", mcp.Required(), mcp.Description("Agent ID")),
		mcp.WithString("status", mcp.Required(), mcp.Description("New status: online, working, idle, offline")),
		mcp.WithString("project", mcp.Description("Optionally update the project")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		id, err := req.RequireString("id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		status, err := req.RequireString("status")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		as := protocol.AgentStatus(status)
		if !as.Valid() {
			return mcp.NewToolResultError(fmt.Sprintf("invalid status: %s", status)), nil
		}
		project := req.GetString("project", "")

		if err := db.UpdateAgentStatus(id, as, project); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("status update failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":   "updated",
			"agent_id": id,
			"new_status": string(as),
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

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

		// Create a new tmux session in the project directory
		cmd := exec.CommandContext(ctx, "tmux", "new-session", "-d",
			"-s", sessionName,
			"-c", project,
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

		// Send initial prompt with hub communication instructions
		time.Sleep(3 * time.Second)
		worktreeNote := ""
		if worktreePath != "" {
			worktreeNote = fmt.Sprintf(`
NOTE: You are working in a git worktree at %s (branch: %s).
This is an isolated copy — the main repo is running a live server. Commit your changes to this branch.
When done, Brian or the user will merge your branch into main.
`, worktreePath, worktreeBranch)
		}
		hubPreamble := fmt.Sprintf(`You are a coder agent (ID: %s) in the bot-hq system. You have bot-hq MCP tools available.

IMPORTANT: Communicate your progress on the hub so other agents can see what you're doing.
- When you START work: hub_send(from="%s", to="brain", type="update", content="Starting: <brief description>")
- When you FINISH or hit a blocker: hub_send(from="%s", to="brain", type="result", content="<what you did or what's blocking>")
- Keep hub messages short — one or two sentences max.
%s
Your task:
`, sessionID, sessionID, sessionID, worktreeNote)
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

// --- Claude Code tools ---

func claudeList(db *hub.DB) ToolDef {
	tool := mcp.NewTool("claude_list",
		mcp.WithDescription("Discover and list all running Claude Code sessions in tmux. Returns session IDs, project paths, tmux targets, and status."),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		// Discover live sessions in tmux
		discovered, err := tmuxpkg.DiscoverClaudeSessions()
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("discovery failed: %v", err)), nil
		}

		// Sync with DB: register new ones, update existing
		var results []hub.ClaudeSession
		for _, d := range discovered {
			existing, err := db.FindClaudeSessionByTarget(d.TmuxTarget)
			if err == nil {
				// Update existing
				db.UpdateClaudeSessionStatus(existing.ID, "running", "")
				existing.Status = "running"
				existing.PID = d.PID
				results = append(results, existing)
			} else {
				// New session found — register as attached
				id := uuid.New().String()[:8]
				sess := hub.ClaudeSession{
					ID:         id,
					Project:    d.CWD,
					TmuxTarget: d.TmuxTarget,
					PID:        d.PID,
					Mode:       "attached",
					Status:     "running",
					Started:    time.Now(),
				}
				db.InsertClaudeSession(sess)
				results = append(results, sess)
			}
		}

		// Also include DB sessions that weren't found (mark as stopped)
		dbSessions, _ := db.ListClaudeSessions("running")
		for _, s := range dbSessions {
			found := false
			for _, d := range discovered {
				if d.TmuxTarget == s.TmuxTarget {
					found = true
					break
				}
			}
			if !found {
				db.StopClaudeSession(s.ID)
				s.Status = "stopped"
			}
			// Check if already in results
			inResults := false
			for _, r := range results {
				if r.ID == s.ID {
					inResults = true
					break
				}
			}
			if !inResults {
				results = append(results, s)
			}
		}

		return mcp.NewToolResultText(toJSON(results)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeRead(db *hub.DB) ToolDef {
	tool := mcp.NewTool("claude_read",
		mcp.WithDescription("Read the latest output from a running Claude Code session. Captures the current tmux pane content."),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session ID to read output from")),
		mcp.WithNumber("lines", mcp.Description("Number of lines to capture (default 50)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		lines := req.GetInt("lines", 50)

		sess, err := db.GetClaudeSession(sessionID)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("session not found: %v", err)), nil
		}

		output, err := tmuxpkg.CapturePane(sess.TmuxTarget, lines)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("capture failed: %v", err)), nil
		}

		// Update last output in DB
		db.UpdateClaudeSessionStatus(sessionID, sess.Status, output)

		return mcp.NewToolResultText(output), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeMessage(db *hub.DB) ToolDef {
	tool := mcp.NewTool("claude_message",
		mcp.WithDescription("Send a message to a running Claude Code session. Detects if Claude is at prompt before sending. Returns captured output."),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session ID to message")),
		mcp.WithString("message", mcp.Required(), mcp.Description("Message to send to Claude Code")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		message, err := req.RequireString("message")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		sess, err := db.GetClaudeSession(sessionID)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("session not found: %v", err)), nil
		}

		// Check if Claude is at prompt
		currentOutput, err := tmuxpkg.CapturePane(sess.TmuxTarget, 10)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("capture failed: %v", err)), nil
		}

		lines := strings.Split(strings.TrimSpace(currentOutput), "\n")
		lastLine := ""
		if len(lines) > 0 {
			lastLine = lines[len(lines)-1]
		}

		// Check for prompt indicators (❯, >, or empty line)
		atPrompt := strings.HasSuffix(strings.TrimSpace(lastLine), "❯") ||
			strings.HasSuffix(strings.TrimSpace(lastLine), ">") ||
			strings.TrimSpace(lastLine) == ""

		if !atPrompt {
			// Claude is busy
			output, _ := tmuxpkg.CapturePane(sess.TmuxTarget, 50)
			db.UpdateClaudeSessionStatus(sessionID, "busy", output)
			return mcp.NewToolResultText(fmt.Sprintf("[Claude is busy — not at prompt]\n%s", output)), nil
		}

		// Send the message
		if err := tmuxpkg.SendKeys(sess.TmuxTarget, message, true); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("send failed: %v", err)), nil
		}

		// Wait for Claude to start processing, then capture
		time.Sleep(2 * time.Second)
		output, _ := tmuxpkg.CapturePane(sess.TmuxTarget, 50)
		db.UpdateClaudeSessionStatus(sessionID, "running", output)

		return mcp.NewToolResultText(output), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeSend() ToolDef {
	tool := mcp.NewTool("claude_send",
		mcp.WithDescription("Send a one-shot task to Claude Code using --print mode. Returns the full output. Good for quick questions that don't need a persistent session."),
		mcp.WithString("prompt", mcp.Required(), mcp.Description("Prompt to send to Claude Code")),
		mcp.WithString("cwd", mcp.Description("Working directory (defaults to home)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		prompt, err := req.RequireString("prompt")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		cwd := req.GetString("cwd", "")
		if cwd == "" {
			cwd, _ = os.UserHomeDir()
		}

		cmd := exec.CommandContext(ctx, "claude", "--print", "-p", prompt)
		cmd.Dir = cwd
		out, err := cmd.Output()
		if err != nil {
			// Try to get partial output
			if exitErr, ok := err.(*exec.ExitError); ok && len(exitErr.Stderr) > 0 {
				return mcp.NewToolResultText(fmt.Sprintf("[partial output]\n%s\n[stderr: %s]",
					strings.TrimSpace(string(out)), strings.TrimSpace(string(exitErr.Stderr)))), nil
			}
			if len(out) > 0 {
				return mcp.NewToolResultText(fmt.Sprintf("[partial output]\n%s", strings.TrimSpace(string(out)))), nil
			}
			return mcp.NewToolResultError(fmt.Sprintf("claude --print failed: %v", err)), nil
		}

		output := strings.TrimSpace(string(out))
		if len(output) > 30000 {
			output = output[:30000] + "\n...[truncated]"
		}

		return mcp.NewToolResultText(output), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeResume() ToolDef {
	tool := mcp.NewTool("claude_resume",
		mcp.WithDescription("Resume the last Claude Code conversation using -c flag. Continues where a previous session left off."),
		mcp.WithString("prompt", mcp.Required(), mcp.Description("Prompt to continue the conversation with")),
		mcp.WithString("cwd", mcp.Description("Working directory (defaults to home)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		prompt, err := req.RequireString("prompt")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		cwd := req.GetString("cwd", "")
		if cwd == "" {
			cwd, _ = os.UserHomeDir()
		}

		cmd := exec.CommandContext(ctx, "claude", "-c", "--print", "-p", prompt)
		cmd.Dir = cwd
		out, err := cmd.Output()
		if err != nil {
			if len(out) > 0 {
				return mcp.NewToolResultText(fmt.Sprintf("[partial output]\n%s", strings.TrimSpace(string(out)))), nil
			}
			return mcp.NewToolResultError(fmt.Sprintf("claude -c --print failed: %v", err)), nil
		}

		output := strings.TrimSpace(string(out))
		if len(output) > 30000 {
			output = output[:30000] + "\n...[truncated]"
		}

		return mcp.NewToolResultText(output), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeStop(db *hub.DB) ToolDef {
	tool := mcp.NewTool("claude_stop",
		mcp.WithDescription("Stop a running Claude Code session by killing its tmux session. This is destructive."),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session ID to stop")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		sess, err := db.GetClaudeSession(sessionID)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("session not found: %v", err)), nil
		}

		// Kill the tmux session
		if err := tmuxpkg.KillSession(sess.TmuxTarget); err != nil {
			// Session might already be dead — mark as stopped anyway
		}

		db.StopClaudeSession(sessionID)

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":     "stopped",
			"session_id": sessionID,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubCheckpoint(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_checkpoint",
		mcp.WithDescription("Save agent state checkpoint for persistence across sessions"),
		mcp.WithString("from", mcp.Required(), mcp.Description("Caller agent ID (must match agent_id)")),
		mcp.WithString("agent_id", mcp.Required(), mcp.Description("Agent ID to save checkpoint for")),
		mcp.WithString("data", mcp.Required(), mcp.Description("JSON object containing agent state to persist")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		from, err := req.RequireString("from")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		agentID, err := req.RequireString("agent_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		data, err := req.RequireString("data")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		// Self-only write: agents can only checkpoint their own state
		if from != agentID {
			return mcp.NewToolResultError("agents can only checkpoint their own state"), nil
		}

		// Size limit: reject checkpoint data exceeding 1MB
		if len(data) > 1_000_000 {
			return mcp.NewToolResultError("checkpoint data exceeds 1MB limit"), nil
		}

		if !json.Valid([]byte(data)) {
			return mcp.NewToolResultError("data must be valid JSON"), nil
		}

		if err := db.SaveCheckpoint(agentID, data); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("checkpoint save failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":   "saved",
			"agent_id": agentID,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubIssueCreate(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_issue_create",
		mcp.WithDescription("Create a new issue in the hub issue tracker"),
		mcp.WithString("reporter", mcp.Required(), mcp.Description("Agent ID reporting the issue")),
		mcp.WithString("severity", mcp.Required(), mcp.Description("Issue severity: low, medium, high, critical")),
		mcp.WithString("title", mcp.Required(), mcp.Description("Short issue title")),
		mcp.WithString("description", mcp.Description("Detailed issue description")),
		mcp.WithString("file_path", mcp.Description("File path related to the issue")),
		mcp.WithNumber("line_number", mcp.Description("Line number in the file")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		reporter, err := req.RequireString("reporter")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		severity, err := req.RequireString("severity")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		title, err := req.RequireString("title")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		description := req.GetString("description", "")
		filePath := req.GetString("file_path", "")
		lineNumberRaw := req.GetInt("line_number", 0)
		var lineNumber *int
		if lineNumberRaw != 0 {
			v := lineNumberRaw
			lineNumber = &v
		}

		id := uuid.New().String()
		issue, err := db.CreateIssue(id, reporter, severity, title, description, filePath, lineNumber)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("create issue failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(issue)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubIssueList(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_issue_list",
		mcp.WithDescription("List issues from the hub issue tracker"),
		mcp.WithString("status", mcp.Description("Filter by status: open, in_progress, fixed, wontfix, duplicate")),
		mcp.WithString("severity", mcp.Description("Filter by severity: low, medium, high, critical")),
		mcp.WithString("reporter", mcp.Description("Filter by reporter agent ID")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		status := req.GetString("status", "")
		severity := req.GetString("severity", "")
		reporter := req.GetString("reporter", "")

		issues, err := db.ListIssues(status, severity, reporter)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("list issues failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(issues)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubIssueUpdate(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_issue_update",
		mcp.WithDescription("Update an existing issue in the hub issue tracker"),
		mcp.WithString("id", mcp.Required(), mcp.Description("Issue ID to update")),
		mcp.WithString("status", mcp.Description("New status: open, in_progress, fixed, wontfix, duplicate")),
		mcp.WithString("assigned_to", mcp.Description("Agent ID to assign the issue to")),
		mcp.WithString("resolution", mcp.Description("Resolution description")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		id, err := req.RequireString("id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		status := req.GetString("status", "")
		assignedTo := req.GetString("assigned_to", "")
		resolution := req.GetString("resolution", "")

		issue, err := db.UpdateIssue(id, status, assignedTo, resolution)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("update issue failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(issue)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubRestore(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_restore",
		mcp.WithDescription("Restore agent state checkpoint from a previous session"),
		mcp.WithString("agent_id", mcp.Required(), mcp.Description("Agent ID to restore checkpoint for")),
		mcp.WithBoolean("validate", mcp.Description("Cross-check checkpoint against live hub state")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		agentID, err := req.RequireString("agent_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		cp, err := db.GetCheckpoint(agentID)
		if err != nil {
			return mcp.NewToolResultText(toJSON(map[string]string{
				"status":   "no_checkpoint",
				"agent_id": agentID,
			})), nil
		}

		result := map[string]any{
			"status":   "restored",
			"agent_id": agentID,
			"data":     cp.Data,
			"version":  cp.Version,
			"updated":  cp.Updated.UTC().Format(time.RFC3339),
		}

		var validate bool
		if args, ok := req.Params.Arguments.(map[string]any); ok {
			validate, _ = args["validate"].(bool)
		}
		if validate {
			agent, agentErr := db.GetAgent(agentID)
			if agentErr != nil {
				result["warning"] = "agent not found in hub"
			} else if agent.Status == protocol.StatusOffline {
				result["warning"] = "agent is currently offline"
			}
		}

		return mcp.NewToolResultText(toJSON(result)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

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
