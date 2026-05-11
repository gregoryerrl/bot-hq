package mcp

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/citecheck"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/pastedetect"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/mark3labs/mcp-go/mcp"
)

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
		mcp.WithDescription("Send a message through the hub. Phase S S-4: PM `to:` parameter REMOVED — all messages broadcast. To target a specific agent, use @<agent> mention in content (e.g., `@brian please review` / `@rain BRAIN-2nd needed` / `@emma rule-violation observed`). The agent recognizes its own name and self-filters relevance via DB-side audience-class-discriminator (R6) load-bearing post-PM-removal. Historical to_agent column preserved for forensics-trail; new messages always broadcast. Z-3 sessions-as-containers: session_id auto-tags from sender's BOT_HQ_SESSION_ID env via agent registry; to_session override for explicit cross-session routing (rare)."),
		mcp.WithString("from", mcp.Required(), mcp.Description("Sender agent ID")),
		mcp.WithString("session_id", mcp.Description("Session ID override — defaults to sender's bound session_id from agents table. Empty = global emit (visible cross-session).")),
		mcp.WithString("to_session", mcp.Description("Z-3: route this message to a specific session (cross-session addressing — rare; agents in different sessions communicating). Takes precedence over session_id override.")),
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

		// Phase T T-11 cycle-3: paste-detection screen. Halt secrets at
		// the daemon entry point so they never land in hub.db / Discord
		// forwards. Mechanical enforcement-conversion of the "do NOT
		// paste into hub" warning (msg 17454 step-2) that recurred at
		// msg 17460.
		if d := pastedetect.Inspect(content); d.Found() {
			return mcp.NewToolResultError(pastedetect.FormatBlockReason(d)), nil
		}

		// Phase T T-12 cycle-3: cite-check (informational, NOT blocking).
		// Detect msg-id citations in content + verify they resolve in
		// hub.db. Concerns are appended to the success response so the
		// emitting agent gets immediate mechanical feedback on cite drift.
		// R31-sub graduation: enforcement-conversion via daemon-side post-
		// validation; agent self-corrects on next emit.
		citeConcerns := citecheck.Inspect(content, func(id int64) (bool, error) {
			return db.MessageExists(id)
		})

		mt := protocol.MessageType(msgType)
		if !mt.Valid() {
			return mcp.NewToolResultError(fmt.Sprintf("invalid message type: %s", msgType)), nil
		}

		// Z-3 session_id resolution priority:
		//  1. explicit to_session arg (cross-session routing)
		//  2. explicit session_id arg (override)
		//  3. sender's agents.session_id binding (auto-tag at send-time)
		//  4. empty (global emit, visible cross-session)
		sessionID := req.GetString("to_session", "")
		if sessionID == "" {
			sessionID = req.GetString("session_id", "")
		}
		if sessionID == "" {
			if a, gErr := db.GetAgent(from); gErr == nil {
				sessionID = a.AgentSessionID
			}
		}

		msg := protocol.Message{
			FromAgent: from,
			// Phase S S-4: ToAgent always empty; PM removed in favor of
			// @<agent> mention-detection in content. Historical
			// messages with non-empty to_agent preserved at DB layer
			// for forensics-trail per R2 authorless-display pattern.
			ToAgent:   "",
			SessionID: sessionID,
			Type:      mt,
			Content:   content,
			Created:   time.Now(),
		}

		id, err := db.InsertMessage(msg)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("send failed: %v", err)), nil
		}

		response := map[string]any{
			"status":     "sent",
			"message_id": id,
		}
		if notice := citecheck.FormatNotice(citeConcerns); notice != "" {
			response["cite_check"] = notice
		}
		return mcp.NewToolResultText(toJSON(response)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubRead(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_read",
		mcp.WithDescription("Read messages from the hub. Z-3 sessions-as-containers: optional session_id filter narrows to a specific session (defaults to caller's bound session); use \"__all__\" to read cross-session."),
		mcp.WithString("agent_id", mcp.Required(), mcp.Description("Agent ID to read messages for")),
		mcp.WithNumber("since_id", mcp.Description("Only return messages after this ID")),
		mcp.WithNumber("limit", mcp.Description("Max messages to return (default 50)")),
		mcp.WithString("session_id", mcp.Description("Z-3 session filter. Defaults to caller's agents.session_id (per-session view). Pass \"__all__\" to disable filtering for cross-session reads (emma's global view).")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		agentID, err := req.RequireString("agent_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		sinceID := int64(req.GetFloat("since_id", 0))
		limit := req.GetInt("limit", 50)

		// Z-3 session filter resolution.
		sessionFilter := req.GetString("session_id", "")
		// Empty string + agent is known → default to agent's bound session.
		if sessionFilter == "" {
			if a, gErr := db.GetAgent(agentID); gErr == nil {
				sessionFilter = a.AgentSessionID
			}
		}
		applyFilter := sessionFilter != "" && sessionFilter != "__all__"

		msgs, err := db.ReadMessages(agentID, sinceID, limit)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("read failed: %v", err)), nil
		}

		if applyFilter {
			filtered := msgs[:0]
			for _, m := range msgs {
				// Show: session-tagged matches OR untagged (global/system).
				if m.SessionID == "" || m.SessionID == sessionFilter {
					filtered = append(filtered, m)
				}
			}
			msgs = filtered
		}

		// Truncate older message content to save tokens.
		// Keep the 10 most recent at full length; truncate older ones to 200 chars.
		if len(msgs) > 10 {
			for i := 0; i < len(msgs)-10; i++ {
				if len(msgs[i].Content) > 200 {
					msgs[i].Content = msgs[i].Content[:200] + "...[truncated]"
				}
			}
		}

		return mcp.NewToolResultText(toJSON(msgs)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubBroadcast implements Phase R R2 duo-consensus authorless [HR]
// broadcast. Wraps hub_send-equivalent emit with two key behaviors:
//
//   - Auto-prefix `[HR] ` to content (idempotent via HasPrefix guard
//     per Rain msg 15561 Refine-1). Caller may pre-prefix; double-
//     prefix is suppressed.
//   - DB `from_agent` column preserves caller for audit-trail forensics
//     (R31 STAT-CLAIM-CITE downstream; bot-hq-query inspectability per
//     Refine-6). Display-layer (tmux pane formatNudge / Discord
//     bridge / webui) strips sender at render time per R2 authorless
//     semantic.
//
// Rain-gated via PreToolUse toolgate-hook (BOT_HQ_AGENT_ID=rain check
// per Refine-5). Brian invocation hard-blocks at hook-layer; this
// handler executes only on Rain-authorized fire.
//
// Per phase-r.md R2 cluster + Rain msg 15561 BRAIN-2nd 6 refinements.
func hubBroadcast(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_broadcast",
		mcp.WithDescription("Phase R R2 — emit a duo-consensus [HR]-tagged broadcast. Auto-prefixes [HR] to content (idempotent). DB preserves from_agent for forensics; display layer strips sender at render time. Rain-gated via PreToolUse toolgate-hook (BOT_HQ_AGENT_ID=rain check); Brian invocation blocked. Use for BRAIN-cycle final drafts where authorless duo-voice rendering is intended."),
		mcp.WithString("from", mcp.Required(), mcp.Description("Sender agent ID (preserved in DB for forensics; stripped at display)")),
		mcp.WithString("content", mcp.Required(), mcp.Description("Message content. Auto-prefixed with `[HR] ` if not already; double-prefix suppressed.")),
		mcp.WithString("type", mcp.Description("Message type: response | update | result. Default: response. Excludes flag (use hub_flag) and command/error.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		from, err := req.RequireString("from")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		content, err := req.RequireString("content")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		// Refine-1 idempotency: don't double-prefix if caller already
		// included [HR] prefix.
		if !strings.HasPrefix(content, "[HR] ") && !strings.HasPrefix(content, "[HR]\n") {
			content = "[HR] " + content
		}

		typeStr := req.GetString("type", "response")
		mt := protocol.MessageType(typeStr)
		// Restrict to non-elevation, non-error message types. Flag uses
		// existing hub_flag tool; command/error are agent-class restricted.
		switch mt {
		case protocol.MsgResponse, protocol.MsgUpdate, protocol.MsgResult:
			// allowed
		default:
			return mcp.NewToolResultError(fmt.Sprintf("hub_broadcast: type %q not supported (use response | update | result; flag → hub_flag; command/error not for broadcast class)", typeStr)), nil
		}

		msgID, err := db.InsertMessage(protocol.Message{
			FromAgent: from,
			ToAgent:   "", // broadcast
			Type:      mt,
			Content:   content,
			Created:   time.Now(),
		})
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("hub_broadcast insert: %v", err)), nil
		}
		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":     "broadcast",
			"message_id": msgID,
			"from":       from,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}
