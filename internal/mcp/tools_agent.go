package mcp

import (
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/sessions"
	"github.com/mark3labs/mcp-go/mcp"
)

func hubRegister(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_register",
		mcp.WithDescription("Register as an agent in the hub"),
		mcp.WithString("id", mcp.Required(), mcp.Description("Unique agent ID")),
		mcp.WithString("name", mcp.Required(), mcp.Description("Human-readable agent name")),
		mcp.WithString("type", mcp.Required(), mcp.Description("Agent type: coder, voice, brian, discord")),
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

		// Meta is owned by the launcher (e.g. internal/brian/brian.go RegisterAgent
		// call site, which writes tmux_target). hub_register may be called from any
		// MCP client (today: Claude STARTUP prompt) after the launcher has populated
		// Meta. Since db.RegisterAgent is INSERT OR REPLACE, we must read-and-preserve
		// the launcher's Meta here or the round-trip silently clobbers tmux_target,
		// which presents as panestate's pane-tier observer never finding the pane
		// (H6 — registration plumbing gap). If a future caller needs to populate
		// Meta via MCP, add an explicit tmux_target param to the tool schema rather
		// than removing this preservation.
		existing, _ := db.GetAgent(id)
		// Z-3: bind agent to its containing session via BOT_HQ_SESSION_ID
		// env (set at tmux spawn-time by the daemon's session_open hook).
		// Empty env = global agent (emma, clive, discord) — leaves
		// session_id empty per the Z-3 substrate restructure.
		sessionID := os.Getenv("BOT_HQ_SESSION_ID")
		agent := protocol.Agent{
			ID:             id,
			Name:           name,
			Type:           agentType,
			Status:         protocol.StatusOnline,
			Project:        req.GetString("project", ""),
			Meta:           existing.Meta,
			Registered:     time.Now(),
			AgentSessionID: sessionID,
		}

		// Phase H slice 3 C3 (#2): atomic register + watermark. Returns the
		// MAX(messages.id) at register-commit so the agent self-filters
		// inbound msg.ID <= current_max_msg_id as boot-replay (post-rebuild
		// context-bootstrap-replay pathology — see design doc §C3).
		watermark, lastSnap, err := db.RegisterAgentWithWatermark(agent)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("register failed: %v", err)), nil
		}

		response := map[string]any{
			"status":             "registered",
			"agent_id":           id,
			"current_max_msg_id": watermark,
			"last_session_snap":  lastSnap,
		}

		// Phase N v2 #6 N-1(b)-C: auto-load most-recent session for
		// project per Q-V (s) RATIFIED auto-load-most-recent + on-
		// demand-load (s). When register supplies a project key,
		// surface the most-recent session-id matching that project so
		// the agent can hub_session_load <id> on the next turn for
		// cross-session context-retention forward-loop. Empty when no
		// matching session exists; absent when project unset.
		if project := agent.Project; project != "" {
			recent, lookupErr := sessions.MostRecentForProject(project)
			if lookupErr == nil && recent != "" {
				response["most_recent_session_id"] = recent
			}

			// Phase N v3 v3a writer-flow wiring: idempotently ensure
			// today's session-cluster manifest exists for this project,
			// adding the registering agent to its agents list. Per Q-III
			// hybrid lean — minimal-create at session-open. WriteManifest
			// is idempotent; safe to call repeatedly.
			//
			// Read-merge-write to preserve prior agents list when the
			// manifest already exists for today (other agents may have
			// registered earlier in the same UTC day).
			clusterID := sessions.MakeSessionID(time.Now(), project)
			existing, _ := sessions.ReadManifest(clusterID) // ignore err — empty on miss
			merged := mergeAgentList(existing.Agents, id)
			startTS := existing.StartTS
			if startTS.IsZero() {
				startTS = time.Now()
			}
			body := existing.Body
			if body == "" {
				body = fmt.Sprintf("Project: %s\nFirst register: agent=%s at %s\n", project, id, startTS.UTC().Format(time.RFC3339))
			}
			manifest := sessions.Manifest{
				ID:      clusterID,
				Project: project,
				StartTS: startTS,
				EndTS:   existing.EndTS,
				Agents:  merged,
				Body:    body,
			}
			if werr := sessions.WriteManifest(manifest); werr == nil {
				response["session_cluster_id"] = clusterID
				_ = sessions.WriteIndex() // best-effort index refresh
			}
		}

		return mcp.NewToolResultText(toJSON(response)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// mergeAgentList appends id to existing if not already present, preserving
// order. Used by hubRegister to maintain per-cluster agent rosters across
// repeated registrations within the same UTC day.
func mergeAgentList(existing []string, id string) []string {
	for _, a := range existing {
		if a == id {
			return existing
		}
	}
	return append(existing, id)
}

// hubClearHalt is the explicit operator-invocable clear path for the
// halt_state machine landed in Phase H slice 4 C6 (H-31). The default
// halt-clear path is causality-only — set on Emma's context-cap flag, cleared
// when the duo re-registers post-rebuild past set_at. This tool provides an
// abort lever for the case where the operator wants to dismiss the halt
// without restarting duo sessions (e.g. false-fire investigation, manual
// recovery). No args; idempotent — calling on an inactive halt returns
// `not_active`.
func hubClearHalt(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_clear_halt",
		mcp.WithDescription("Manually clear an active halt_state without duo re-register. Use when an operator wants to abort the halt-all-work convention (e.g. false-fire, in-place recovery). The default halt-clear is causality-only via duo re-registration; this is the explicit override."),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		halted, err := db.IsHalted()
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("halt status check failed: %v", err)), nil
		}
		if !halted {
			return mcp.NewToolResultText(toJSON(map[string]string{
				"status": "not_active",
			})), nil
		}
		if err := db.ClearHaltManually(); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("clear halt failed: %v", err)), nil
		}
		return mcp.NewToolResultText(toJSON(map[string]string{
			"status": "cleared",
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
			"status":   "offline",
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
		case "brian", "clive", "discord", "rain", "emma":
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

func hubAgents(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_agents",
		mcp.WithDescription("List agents registered in the hub"),
		mcp.WithString("status", mcp.Description("Filter by status: online, working, offline")),
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

// hubSetCurrentTask implements Phase-R-followup (f) emma-stale current_task
// data-model. Agents call this at multi-step work-thread boundaries to
// declare intentional-idle periods (e.g., long brainstorm cycles, batch
// smoke directives, mid-fire compositions). Empty-string `task` clears
// the declaration. emma-stale checker treats non-empty current_task as
// intentional-idle and short-circuits stale-coder PMs in flagStaleAgent
// at policy-tier (before LastSeen-advance + HasRecentMessageFrom checks).
//
// Per phase-r.md (f) carry-forward graduation + Rain msg 15654 BRAIN-2nd.
func hubSetCurrentTask(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_set_current_task",
		mcp.WithDescription("Phase-R-followup (f) — declare or clear an active multi-step work-thread to suppress emma-stale PMs during intentional-idle periods. Pass non-empty `task` at work-start (short summary like 'Phase-R-followup (f) impl'); pass empty string at work-end to clear. emma-stale flagStaleAgent short-circuits when current_task != \"\". Forensic-trail preserved via agents.current_task DB column."),
		mcp.WithString("from", mcp.Required(), mcp.Description("Agent ID whose current_task is being set (self-write).")),
		mcp.WithString("task", mcp.Required(), mcp.Description("Task summary (non-empty = active multi-step work-thread; empty string = clear declaration).")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		from, err := req.RequireString("from")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		task, err := req.RequireString("task")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		if err := db.SetAgentCurrentTask(from, task); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("hub_set_current_task: %v", err)), nil
		}
		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":   "set",
			"agent_id": from,
			"task":     task,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubStatus(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_status",
		mcp.WithDescription("Update agent status"),
		mcp.WithString("id", mcp.Required(), mcp.Description("Agent ID")),
		mcp.WithString("status", mcp.Required(), mcp.Description("New status: online, working, offline")),
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
			"status":     "updated",
			"agent_id":   id,
			"new_status": string(as),
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}
