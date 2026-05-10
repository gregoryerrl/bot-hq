package mcp

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/gregoryerrl/bot-hq/internal/contextload"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/mark3labs/mcp-go/mcp"
)

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

// --- Phase H slice 3 C1 (#7) wake-schedule MCP tools ---

// hubScheduleWake persists a future hub_send the Emma tick loop will dispatch
// at fire_at. ISO 8601 only on input (per O5) — relative-time syntactic sugar
// is deferred to v2 to keep the input-parse error surface minimal. Open ACL:
// any agent may schedule a wake for any target (per arch lean 5).
func hubScheduleWake(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_schedule_wake",
		mcp.WithDescription("Schedule a future hub_send. Emma's tick loop dispatches the payload to target_agent at fire_at. Use for cross-session wakes (any agent → any target). Brian-side ScheduleWakeup is now fallback for self-wake-only."),
		mcp.WithString("from", mcp.Required(), mcp.Description("Scheduler agent ID — recorded as created_by")),
		mcp.WithString("target_agent", mcp.Required(), mcp.Description("Agent ID to wake (or scheduler's own ID for self-wake)")),
		mcp.WithString("fire_at", mcp.Required(), mcp.Description("ISO 8601 timestamp — when Emma should dispatch (e.g. 2026-04-27T15:30:00Z)")),
		mcp.WithString("payload", mcp.Description("Raw message content delivered to target_agent (default empty string)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		from, err := req.RequireString("from")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		if strings.TrimSpace(from) == "" {
			return mcp.NewToolResultError("from must not be empty"), nil
		}
		target, err := req.RequireString("target_agent")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		if strings.TrimSpace(target) == "" {
			return mcp.NewToolResultError("target_agent must not be empty"), nil
		}
		fireAtRaw, err := req.RequireString("fire_at")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		fireAt, err := time.Parse(time.RFC3339, fireAtRaw)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("fire_at must be ISO 8601 (RFC3339, e.g. 2026-04-27T15:30:00Z): %v", err)), nil
		}
		payload := req.GetString("payload", "")

		id, err := db.InsertWakeSchedule(target, from, payload, fireAt)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("schedule failed: %v", err)), nil
		}
		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":        "scheduled",
			"wake_id":       id,
			"scheduled_for": fireAt.UTC().Format(time.RFC3339),
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubCancelWake transitions a pending wake to cancelled. Idempotent on rows
// that already left pending — surfaces status=already_terminal so callers can
// distinguish "you cancelled" from "too late, it already fired/was cancelled"
// without needing a separate read. Missing IDs return an error.
func hubCancelWake(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_cancel_wake",
		mcp.WithDescription("Cancel a pending wake scheduled via hub_schedule_wake. Idempotent: cancelling an already-fired or already-cancelled wake reports status=already_terminal, not an error."),
		mcp.WithString("from", mcp.Description("Caller agent ID (for last_seen middleware)")),
		mcp.WithNumber("wake_id", mcp.Required(), mcp.Description("wake_id returned by hub_schedule_wake")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		idF := req.GetFloat("wake_id", 0)
		if idF <= 0 {
			return mcp.NewToolResultError("wake_id must be a positive integer"), nil
		}
		id := int64(idF)
		cancelled, err := db.CancelWake(id)
		if err != nil {
			if errors.Is(err, sql.ErrNoRows) {
				return mcp.NewToolResultError(fmt.Sprintf("wake_id %d not found", id)), nil
			}
			return mcp.NewToolResultError(fmt.Sprintf("cancel failed: %v", err)), nil
		}
		w, _ := db.GetWakeSchedule(id)
		status := "cancelled"
		if !cancelled {
			status = "already_terminal"
		}
		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":      status,
			"wake_id":     id,
			"fire_status": w.FireStatus,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubContextLoad exposes the per-project context loader as an MCP tool.
// Phase V architecture: replaces auto-bootstrap with explicit on-demand
// load. Agent calls this when pivoting to a project; the returned
// markdown blob is the layered (general → project) rules + project
// library overview + cite-anchor sources.
//
// See internal/contextload for the assembly logic.
func hubContextLoad() ToolDef {
	tool := mcp.NewTool("bot_hq_context_load",
		mcp.WithDescription("Load per-project context: merged rules + library overview + cite-anchor sources. Call when pivoting to work on a project."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key (matches projects/<key>.yaml in canonical-store)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		home, err := os.UserHomeDir()
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("resolve home dir: %v", err)), nil
		}
		canonRoot := filepath.Join(home, ".bot-hq")

		c, err := contextload.Load(canonRoot, project)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("context load failed: %v", err)), nil
		}

		return mcp.NewToolResultText(c.Markdown()), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubAgentBootstrap is the Z-0 CL-first bootstrap tool. Returns the full
// durable-substrate snapshot the agent needs on resume per vision.md
// three-layer model (knowledge + discipline + state):
//   - merged rules (general → project → agent)
//   - project library overview + index
//   - phase/<active>.md (active scope-lock doc)
//   - ratchets/active.md (operational ratchet ledger)
//   - <agent>/last_state.json (R20 AgentState resume anchor)
//   - <agent>/discipline-anchors.md (R24 mutual-halt anchor)
//
// Replaces the prior "STARTUP: iterate hub_read until empty" backlog-scrape
// pattern. Agents bootstrap from CL (durable substrate), not from
// hub-message ephemera. See internal/contextload.LoadBootstrap.
func hubAgentBootstrap() ToolDef {
	tool := mcp.NewTool("bot_hq_agent_bootstrap",
		mcp.WithDescription("Bootstrap an agent from CL durable state per vision.md. Returns merged rules + project library + active phase doc + ratchets/active.md + per-agent last_state.json + discipline-anchors.md. Call once at session start in lieu of hub_read backlog iteration."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key (matches projects/<key>.yaml in canonical-store)")),
		mcp.WithString("agent", mcp.Required(), mcp.Description("Agent identity (brian/rain/emma) — selects per-agent rule layer + state files")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		agent, err := req.RequireString("agent")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		home, err := os.UserHomeDir()
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("resolve home dir: %v", err)), nil
		}
		canonRoot := filepath.Join(home, ".bot-hq")

		bc, err := contextload.LoadBootstrap(canonRoot, project, agent)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("bootstrap load failed: %v", err)), nil
		}

		return mcp.NewToolResultText(bc.Markdown()), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}
