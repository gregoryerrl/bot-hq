// Phase Y-2: IPAV MCP tool surface. Wraps internal/cl + internal/mvt
// runtime API into the duo-callable tool surface so brian/rain/emma
// can open + transition + complete IPAV tasks during real work.
//
// Tools landed here:
//   bot_hq_ipav_open          — open new task with decision_class
//   bot_hq_ipav_transition    — advance phase (I→P→Implement→V or loop-back)
//   bot_hq_ipav_set_artifact  — attach artifact path (investigation_doc / fault_tree / plan_doc / verify_report / etc.)
//   bot_hq_ipav_complete      — close with verify result (pass/fail/escalated)
//   bot_hq_ipav_list          — enumerate active + recently-closed tasks
//
// Each open + complete auto-regenerates the project INDEX.md so the
// substrate reflects current state for the next Investigator.

package mcp

import (
	"context"
	"fmt"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/cl"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/mvt"
	"github.com/gregoryerrl/bot-hq/internal/sessions"
	"github.com/mark3labs/mcp-go/mcp"
)

// validArtifactKeys mirrors IPAVRuntime.SetPhaseArtifact's key dispatch.
// Documented + locked here so the duo gets a clean error on typos.
var validArtifactKeys = []string{
	"investigation_doc",
	"fault_tree",
	"plan_doc",
	"plan_bilateral_a",
	"plan_bilateral_b",
	"plan_merge_log",
	"verify_report",
}

func newCLForTool() (*cl.CL, error) {
	return cl.NewCL("")
}

func runtimeFor(project string) (*mvt.TaskState, *cl.IPAVRuntime, error) {
	c, err := newCLForTool()
	if err != nil {
		return nil, nil, fmt.Errorf("cl: %w", err)
	}
	r, err := cl.NewIPAVRuntime(c, project)
	if err != nil {
		return nil, nil, fmt.Errorf("ipav runtime: %w", err)
	}
	return nil, r, nil
}

func hubIPAVOpen() ToolDef {
	tool := mcp.NewTool("bot_hq_ipav_open",
		mcp.WithDescription("Open a new IPAV task for the given project. Required when the task is medium/high decision-class per IPAV-DISCIPLINE. Returns task_id (UUID) + initial state (CurrentPhase=Investigate). Auto-regenerates the project INDEX.md so the new task surfaces in the substrate immediately. session_id binds the task to its session-cluster so bot_hq_ipav_complete(verify=pass) can auto-finalize the session per session-lifecycle-cleanup; when omitted, falls back to sessions.FindActiveForProject."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key (e.g., bot-hq, bcc-ad-manager)")),
		mcp.WithString("decision_class", mcp.Required(), mcp.Description("Decision class: low | medium | high. Medium/high triggers bilateral mode auto-set in Investigate + Plan transitions per R44 expanded.")),
		mcp.WithString("session_id", mcp.Description("Optional explicit session-cluster id (scope-keyed). When omitted, resolves via sessions.FindActiveForProject(project). Empty result = task opens without session binding; auto-finalize on verify-pass silently no-ops.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		decisionClassStr, err := req.RequireString("decision_class")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		dc, err := parseDecisionClass(decisionClassStr)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		// session-lifecycle-cleanup: resolve session-id from explicit
		// param OR fall back to sessions.FindActiveForProject. Empty
		// result is permitted (degraded; auto-finalize won't wire).
		sessionID := req.GetString("session_id", "")
		if sessionID == "" {
			if found, ferr := sessions.FindActiveForProject(project); ferr == nil && found != "" {
				sessionID = found
			}
		}

		_, r, err := runtimeFor(project)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		taskID, ts, err := r.OpenTask(sessionID, dc)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("ipav_open: %v", err)), nil
		}

		// Best-effort INDEX regen; failure here doesn't fail the open.
		c, _ := newCLForTool()
		if c != nil {
			_, _, _ = c.IndexProject(project)
		}

		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":         "opened",
			"project":        project,
			"task_id":        taskID,
			"current_phase":  string(ts.CurrentPhase),
			"phase_mode":     string(ts.PhaseMode),
			"decision_class": string(ts.DecisionClass),
			"opened_at":      ts.OpenedAt.Format("2006-01-02T15:04:05Z07:00"),
			"state_path":     fmt.Sprintf("~/.bot-hq/projects/%s/tasks/%s/ipav-state.yaml", project, taskID),
		})), nil
	}
	return ToolDef{Tool: tool, Handler: handler}
}

func hubIPAVTransition() ToolDef {
	tool := mcp.NewTool("bot_hq_ipav_transition",
		mcp.WithDescription("Advance an IPAV task to the next phase. Permitted transitions: Investigate→Plan; Plan→Implement | Plan→Investigate (loop-back); Implement→Verify | Implement→Plan (loop-back); Verify→Implement | Verify→Plan | Verify→Investigate (Verify-fail loop-back). PhaseMode is auto-derived from decision_class. Use bot_hq_ipav_complete to close instead of transitioning past Verify."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key")),
		mcp.WithString("task_id", mcp.Required(), mcp.Description("Task UUID returned by bot_hq_ipav_open")),
		mcp.WithString("to_phase", mcp.Required(), mcp.Description("Target phase: investigate | plan | implement | verify")),
		mcp.WithString("agent_models", mcp.Description("Comma-separated agent_id=model_name pairs (e.g., 'brian=claude-default,rain=deepseek-v4-pro') for phase-history attribution")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		taskID, err := req.RequireString("task_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		toPhaseStr, err := req.RequireString("to_phase")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		to, err := parseStage(toPhaseStr)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		agentModels := parseAgentModels(req.GetString("agent_models", ""))

		_, r, err := runtimeFor(project)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		ts, err := r.TransitionPhase(taskID, to, agentModels)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("transition: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":        "transitioned",
			"task_id":       taskID,
			"current_phase": string(ts.CurrentPhase),
			"phase_mode":    string(ts.PhaseMode),
		})), nil
	}
	return ToolDef{Tool: tool, Handler: handler}
}

func hubIPAVSetArtifact() ToolDef {
	tool := mcp.NewTool("bot_hq_ipav_set_artifact",
		mcp.WithDescription("Attach an artifact path to the current phase. Valid keys: investigation_doc, fault_tree, plan_doc, plan_bilateral_a, plan_bilateral_b, plan_merge_log, verify_report. Per-phase artifact paths typically live under ~/.bot-hq/projects/<project>/tasks/<task-id>/."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key")),
		mcp.WithString("task_id", mcp.Required(), mcp.Description("Task UUID")),
		mcp.WithString("key", mcp.Required(), mcp.Description("Artifact key (see description for valid values)")),
		mcp.WithString("path", mcp.Required(), mcp.Description("Filesystem path to the artifact (typically ~/.bot-hq/projects/<project>/tasks/<task-id>/<artifact>.md)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		taskID, err := req.RequireString("task_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		key, err := req.RequireString("key")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		if !validArtifactKey(key) {
			return mcp.NewToolResultError(fmt.Sprintf("invalid artifact key %q; valid: %s", key, strings.Join(validArtifactKeys, ", "))), nil
		}
		path, err := req.RequireString("path")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		_, r, err := runtimeFor(project)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		ts, err := r.SetPhaseArtifact(taskID, key, path)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("set_artifact: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":        "attached",
			"task_id":       taskID,
			"key":           key,
			"path":          path,
			"current_phase": string(ts.CurrentPhase),
		})), nil
	}
	return ToolDef{Tool: tool, Handler: handler}
}

func hubIPAVComplete(db *hub.DB) ToolDef {
	tool := mcp.NewTool("bot_hq_ipav_complete",
		mcp.WithDescription("Close an IPAV task with a Verify result. Terminal results (pass / escalated) set ClosedAt; fail leaves the task open for the V→P loop-back and increments verify_loop_count. Per R-T-4: 3+ verify_loop_count escalates to user. Auto-regenerates the project INDEX so the closed task surfaces in 'recently closed'. session-lifecycle-cleanup: result=pass AND task.SessionID present auto-fires hub_session_finalize for the session — kills the duo cleanly so 'no active session → no BRAIN-duo' holds."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key")),
		mcp.WithString("task_id", mcp.Required(), mcp.Description("Task UUID")),
		mcp.WithString("result", mcp.Required(), mcp.Description("Verify result: pass | fail | escalated")),
		mcp.WithString("outcome", mcp.Description("Agent-authored narrative for the auto-finalize when result=pass. Multi-paragraph fine; serves as the retrospective payload for the closed session. When omitted, a templated outcome is generated from the task id + verify result.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		taskID, err := req.RequireString("task_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		resultStr, err := req.RequireString("result")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		result, err := parseVerifyResult(resultStr)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		_, r, err := runtimeFor(project)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		ts, err := r.CompleteTask(taskID, result)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("complete: %v", err)), nil
		}

		c, _ := newCLForTool()
		if c != nil {
			_, _, _ = c.IndexProject(project)
		}

		// session-lifecycle-cleanup: on verify-pass AND task is bound to
		// a session, auto-fire the full hub_session_finalize flow via
		// finalizeSession() — same path as the explicit MCP tool, so we
		// get the daemon-hook-OR-subprocess-queue dance + manifest write
		// + status update + closing_state capture. The "no active
		// session → no BRAIN-duo" invariant rides on this coupling.
		// Other verify results (fail, escalated) preserve the existing
		// loop-back / user-action paths and do NOT auto-finalize.
		autoFinalize := map[string]any{}
		if result == mvt.VerifyPass && ts.SessionID != "" {
			outcome := req.GetString("outcome", "")
			if outcome == "" {
				outcome = fmt.Sprintf("Auto-finalized on IPAV task %s verify=pass (no explicit outcome supplied).", taskID)
			}
			payload, fErr := finalizeSession(db, project, ts.SessionID, outcome, "closed", "", false)
			if fErr != nil {
				autoFinalize["error"] = fErr.Error()
			} else {
				autoFinalize["session_id"] = ts.SessionID
				autoFinalize["outcome"] = outcome
				autoFinalize["finalize_payload"] = payload
			}
		}

		closed := ""
		if !ts.ClosedAt.IsZero() {
			closed = ts.ClosedAt.Format("2006-01-02T15:04:05Z07:00")
		}
		respPayload := map[string]any{
			"status":            "completed",
			"task_id":           taskID,
			"verify_result":     string(ts.VerifyResult),
			"verify_loop_count": ts.VerifyLoopCount,
			"closed_at":         closed,
		}
		if len(autoFinalize) > 0 {
			respPayload["auto_finalize"] = autoFinalize
		}
		return mcp.NewToolResultText(toJSON(respPayload)), nil
	}
	return ToolDef{Tool: tool, Handler: handler}
}

func hubIPAVList() ToolDef {
	tool := mcp.NewTool("bot_hq_ipav_list",
		mcp.WithDescription("List active + recently-closed IPAV tasks for a project (or all projects if --all is set via project=__all__). Returns flat JSON list with task_id + current_phase + decision_class + opened_at + closed_at."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key, or __all__ to enumerate across all registered projects.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		c, err := newCLForTool()
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		type listEntry struct {
			Project       string `json:"project"`
			TaskID        string `json:"task_id"`
			CurrentPhase  string `json:"current_phase"`
			DecisionClass string `json:"decision_class"`
			OpenedAt      string `json:"opened_at"`
			ClosedAt      string `json:"closed_at,omitempty"`
			VerifyResult  string `json:"verify_result,omitempty"`
		}

		var projects []string
		if project == "__all__" {
			projects, err = c.ListProjects()
			if err != nil {
				return mcp.NewToolResultError(fmt.Sprintf("list projects: %v", err)), nil
			}
		} else {
			projects = []string{project}
		}

		var entries []listEntry
		for _, p := range projects {
			r, err := cl.NewIPAVRuntime(c, p)
			if err != nil {
				return mcp.NewToolResultError(fmt.Sprintf("runtime %s: %v", p, err)), nil
			}
			tasks, err := r.ListTasks()
			if err != nil {
				return mcp.NewToolResultError(fmt.Sprintf("list tasks %s: %v", p, err)), nil
			}
			for _, ts := range tasks {
				e := listEntry{
					Project:       p,
					TaskID:        ts.TaskID,
					CurrentPhase:  string(ts.CurrentPhase),
					DecisionClass: string(ts.DecisionClass),
					OpenedAt:      ts.OpenedAt.Format("2006-01-02T15:04:05Z07:00"),
					VerifyResult:  string(ts.VerifyResult),
				}
				if !ts.ClosedAt.IsZero() {
					e.ClosedAt = ts.ClosedAt.Format("2006-01-02T15:04:05Z07:00")
				}
				entries = append(entries, e)
			}
		}

		return mcp.NewToolResultText(toJSON(map[string]any{
			"project": project,
			"count":   len(entries),
			"tasks":   entries,
		})), nil
	}
	return ToolDef{Tool: tool, Handler: handler}
}

// --- helpers ---

func parseDecisionClass(s string) (mvt.DecisionClass, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "low":
		return mvt.DecisionLow, nil
	case "medium":
		return mvt.DecisionMedium, nil
	case "high":
		return mvt.DecisionHigh, nil
	default:
		return "", fmt.Errorf("invalid decision_class %q; valid: low | medium | high", s)
	}
}

func parseStage(s string) (mvt.Stage, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "investigate", "i":
		return mvt.StageInvestigate, nil
	case "plan", "p":
		return mvt.StagePlan, nil
	case "implement":
		return mvt.StageImplement, nil
	case "verify", "v":
		return mvt.StageVerify, nil
	default:
		return "", fmt.Errorf("invalid to_phase %q; valid: investigate | plan | implement | verify", s)
	}
}

func parseVerifyResult(s string) (mvt.VerifyResult, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "pass":
		return mvt.VerifyPass, nil
	case "fail":
		return mvt.VerifyFail, nil
	case "escalated":
		return mvt.VerifyEscalated, nil
	default:
		return "", fmt.Errorf("invalid result %q; valid: pass | fail | escalated", s)
	}
}

func parseAgentModels(s string) map[string]string {
	if strings.TrimSpace(s) == "" {
		return nil
	}
	out := map[string]string{}
	for _, pair := range strings.Split(s, ",") {
		parts := strings.SplitN(pair, "=", 2)
		if len(parts) != 2 {
			continue
		}
		k := strings.TrimSpace(parts[0])
		v := strings.TrimSpace(parts[1])
		if k != "" && v != "" {
			out[k] = v
		}
	}
	return out
}

func validArtifactKey(k string) bool {
	for _, v := range validArtifactKeys {
		if v == k {
			return true
		}
	}
	return false
}
