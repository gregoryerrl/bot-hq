package mcp

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/mark3labs/mcp-go/mcp"
	"github.com/mark3labs/mcp-go/server"
)

// commonAgentIDKeys locks the priority order for agent extraction across all
// MCP tools. First non-empty match wins. Update with a comment if a future
// tool adds a different parameter name.
//
// Spec: docs/plans/phase-e.md §2 (signal architecture).
//
// Design intent (locked by Rain's review on commit 2): anonymous read tools
// (hub_agents, hub_sessions, hub_issue_list, claude_list) intentionally do
// NOT include a caller-context key. An agent that only polls hub_agents and
// never writes/sends will appear stale on the strip. This is correct under
// the "first-order check = is something happening" framing — pure observation
// is not substantive activity. Future contributors: do not add caller_id to
// read-only tools just to "make them update last_seen." That breaks the
// signal-vs-noise contract that makes the strip useful.
var commonAgentIDKeys = []string{"from", "agent_id", "id"}

// lastSeenThrottle is the per-agent minimum interval between UpdateAgentLastSeen
// writes. Sub-second granularity is unnecessary because Phase E thresholds are
// 5s/60s. Reduces DB churn on rapid coord cycles from ~50/min to ~6/min.
const lastSeenThrottle = 1 * time.Second

// lastSeenWriteCache memoizes the wall-clock of the last UpdateAgentLastSeen
// write per agent. Throttle scope is process-wide.
var lastSeenWriteCache sync.Map // map[string]time.Time

// extractAgentID searches request params for an agent identifier, in priority
// order defined by commonAgentIDKeys. Returns empty if the tool's params do
// not include any agent-bound key (e.g. claude_send is one-shot, no agent).
func extractAgentID(req mcp.CallToolRequest) string {
	for _, k := range commonAgentIDKeys {
		if v := req.GetString(k, ""); v != "" {
			return v
		}
	}
	return ""
}

// withLastSeen wraps a tool handler so each invocation refreshes the calling
// agent's last_seen timestamp before the underlying handler runs. Per-agent
// throttle suppresses writes within lastSeenThrottle of the previous write.
//
// Tools without an agent-id param pass through untouched (no DB write).
// DB write errors are best-effort — we never fail a tool call because of a
// last_seen update failure.
func withLastSeen(db *hub.DB, handler server.ToolHandlerFunc) server.ToolHandlerFunc {
	return func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		if id := extractAgentID(req); id != "" {
			now := time.Now()
			shouldWrite := true
			if last, ok := lastSeenWriteCache.Load(id); ok {
				if now.Sub(last.(time.Time)) < lastSeenThrottle {
					shouldWrite = false
				}
			}
			if shouldWrite {
				lastSeenWriteCache.Store(id, now)
				_ = db.UpdateAgentLastSeen(id)
			}
		}
		return handler(ctx, req)
	}
}

// ToolDef pairs an mcp.Tool definition with its handler function.
type ToolDef struct {
	Tool    mcp.Tool
	Handler server.ToolHandlerFunc
}

// BuildTools returns all hub + claude tools wired to the given database.
// Each tool's handler is wrapped with withLastSeen middleware so any tool
// invocation auto-refreshes the calling agent's last_seen timestamp.
func BuildTools(db *hub.DB) []ToolDef {
	raw := []ToolDef{
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
		hubSessionLoad(),
		hubSessionCheckpoint(),
		hubSessionArchive(),
		hubBroadcast(db),
		hubSetCurrentTask(db),
		hubStatus(db),
		hubSpawn(db),
		hubCheckpoint(db),
		hubRestore(db),
		hubIssueCreate(db),
		hubIssueList(db),
		hubIssueUpdate(db),
		hubScheduleWake(db),
		hubCancelWake(db),
		hubSessionClose(db),
		hubSessionFinalize(db),
		hubSessionLookback(),
		hubSessionSummary(),
		hubClearHalt(db),
		hubContextLoad(),
		hubSessionOpen(db),
		hubProjectQuery(db),
		hubIPAVOpen(),
		hubIPAVTransition(),
		hubIPAVSetArtifact(),
		hubIPAVComplete(db),
		hubIPAVList(),
		claudeList(db),
		claudeRead(db),
		claudeMessage(db),
		claudeSend(),
		claudeResume(),
		claudeStop(db),
	}
	wrapped := make([]ToolDef, len(raw))
	for i, t := range raw {
		wrapped[i] = ToolDef{Tool: t.Tool, Handler: withLastSeen(db, t.Handler)}
	}
	return wrapped
}

// --- helpers ---

func toJSON(v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		return fmt.Sprintf(`{"error":%q}`, err.Error())
	}
	return string(b)
}
