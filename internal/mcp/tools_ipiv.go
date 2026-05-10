// Phase Y-2: IPIV MCP tool surface. Wraps internal/cl + internal/mvt
// runtime API into the trio-callable tool surface so brian/rain/emma
// can open + transition + complete IPIV tasks during real work.
//
// Tools landed here:
//   bot_hq_ipiv_open          — open new task with decision_class
//   bot_hq_ipiv_transition    — advance phase (I→P→Implement→V or loop-back)
//   bot_hq_ipiv_set_artifact  — attach artifact path (investigation_doc / fault_tree / plan_doc / verify_report / etc.)
//   bot_hq_ipiv_complete      — close with verify result (pass/fail/escalated)
//   bot_hq_ipiv_list          — enumerate active + recently-closed tasks
//
// Each open + complete auto-regenerates the project INDEX.md so the
// substrate reflects current state for the next Investigator.

package mcp

import (
	"context"
	"fmt"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/cl"
	"github.com/gregoryerrl/bot-hq/internal/mvt"
	"github.com/mark3labs/mcp-go/mcp"
)

// validArtifactKeys mirrors IPIVRuntime.SetPhaseArtifact's key dispatch.
// Documented + locked here so the trio gets a clean error on typos.
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

func runtimeFor(project string) (*mvt.TaskState, *cl.IPIVRuntime, error) {
	c, err := newCLForTool()
	if err != nil {
		return nil, nil, fmt.Errorf("cl: %w", err)
	}
	r, err := cl.NewIPIVRuntime(c, project)
	if err != nil {
		return nil, nil, fmt.Errorf("ipiv runtime: %w", err)
	}
	return nil, r, nil
}

func hubIPIVOpen() ToolDef {
	tool := mcp.NewTool("bot_hq_ipiv_open",
		mcp.WithDescription("Open a new IPIV task for the given project. Required when the task is medium/high decision-class per IPIV-DISCIPLINE. Returns task_id (UUID) + initial state (CurrentPhase=Investigate). Auto-regenerates the project INDEX.md so the new task surfaces in the substrate immediately."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key (e.g., bot-hq, bcc-ad-manager)")),
		mcp.WithString("decision_class", mcp.Required(), mcp.Description("Decision class: low | medium | high. Medium/high triggers bilateral mode auto-set in Investigate + Plan transitions per R44 expanded.")),
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

		_, r, err := runtimeFor(project)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		taskID, ts, err := r.OpenTask(dc)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("ipiv_open: %v", err)), nil
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
			"state_path":     fmt.Sprintf("~/.bot-hq/projects/%s/tasks/%s/ipiv-state.yaml", project, taskID),
		})), nil
	}
	return ToolDef{Tool: tool, Handler: handler}
}

func hubIPIVTransition() ToolDef {
	tool := mcp.NewTool("bot_hq_ipiv_transition",
		mcp.WithDescription("Advance an IPIV task to the next phase. Permitted transitions: Investigate→Plan; Plan→Implement | Plan→Investigate (loop-back); Implement→Verify | Implement→Plan (loop-back); Verify→Implement | Verify→Plan | Verify→Investigate (Verify-fail loop-back). PhaseMode is auto-derived from decision_class. Use bot_hq_ipiv_complete to close instead of transitioning past Verify."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key")),
		mcp.WithString("task_id", mcp.Required(), mcp.Description("Task UUID returned by bot_hq_ipiv_open")),
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

func hubIPIVSetArtifact() ToolDef {
	tool := mcp.NewTool("bot_hq_ipiv_set_artifact",
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

func hubIPIVComplete() ToolDef {
	tool := mcp.NewTool("bot_hq_ipiv_complete",
		mcp.WithDescription("Close an IPIV task with a Verify result. Terminal results (pass / escalated) set ClosedAt; fail leaves the task open for the V→P loop-back and increments verify_loop_count. Per R-T-4: 3+ verify_loop_count escalates to user. Auto-regenerates the project INDEX so the closed task surfaces in 'recently closed'."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key")),
		mcp.WithString("task_id", mcp.Required(), mcp.Description("Task UUID")),
		mcp.WithString("result", mcp.Required(), mcp.Description("Verify result: pass | fail | escalated")),
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

		closed := ""
		if !ts.ClosedAt.IsZero() {
			closed = ts.ClosedAt.Format("2006-01-02T15:04:05Z07:00")
		}
		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":            "completed",
			"task_id":           taskID,
			"verify_result":     string(ts.VerifyResult),
			"verify_loop_count": ts.VerifyLoopCount,
			"closed_at":         closed,
		})), nil
	}
	return ToolDef{Tool: tool, Handler: handler}
}

func hubIPIVList() ToolDef {
	tool := mcp.NewTool("bot_hq_ipiv_list",
		mcp.WithDescription("List active + recently-closed IPIV tasks for a project (or all projects if --all is set via project=__all__). Returns flat JSON list with task_id + current_phase + decision_class + opened_at + closed_at."),
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
			r, err := cl.NewIPIVRuntime(c, p)
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
