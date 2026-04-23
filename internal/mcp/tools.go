package mcp

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/mark3labs/mcp-go/mcp"
	"github.com/mark3labs/mcp-go/server"
)

// ToolDef pairs an mcp.Tool definition with its handler function.
type ToolDef struct {
	Tool    mcp.Tool
	Handler server.ToolHandlerFunc
}

// BuildTools returns all 10 hub tools wired to the given database.
func BuildTools(db *hub.DB) []ToolDef {
	return []ToolDef{
		hubRegister(db),
		hubUnregister(db),
		hubSend(db),
		hubRead(db),
		hubAgents(db),
		hubSessions(db),
		hubSessionCreate(db),
		hubSessionJoin(db),
		hubStatus(db),
		hubSpawn(db),
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
		name, err := req.RequireString("name")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
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

		if err := db.UpdateAgentStatus(id, as); err != nil {
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

		sessionName := fmt.Sprintf("cc-%s", uuid.New().String()[:8])

		// Build the claude command
		claudeCmd := "claude"
		if prompt != "" {
			claudeCmd = fmt.Sprintf("claude -p %q", prompt)
		}

		// Create a new tmux session running claude in the project directory
		cmd := exec.CommandContext(ctx, "tmux", "new-session", "-d",
			"-s", sessionName,
			"-c", project,
			claudeCmd,
		)

		if err := cmd.Run(); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("spawn failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":       "spawned",
			"tmux_session": sessionName,
			"project":      project,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}
