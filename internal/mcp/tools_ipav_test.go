package mcp

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/sessions"
	"github.com/mark3labs/mcp-go/mcp"
)

// invokeTool builds a CallToolRequest with the given args + invokes
// handler. Returns the parsed JSON response or an error message string.
//
// findHandler lives in server_test.go and takes (tools, name).
func invokeTool(t *testing.T, handler func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error), args map[string]any) (map[string]any, *mcp.CallToolResult) {
	t.Helper()
	req := mcp.CallToolRequest{}
	req.Params.Arguments = args
	res, err := handler(context.Background(), req)
	if err != nil {
		t.Fatalf("handler error: %v", err)
	}
	if res == nil {
		t.Fatal("handler returned nil result")
	}
	if len(res.Content) == 0 {
		t.Fatal("handler returned empty content")
	}
	tc, ok := res.Content[0].(mcp.TextContent)
	if !ok {
		t.Fatalf("expected TextContent, got %T", res.Content[0])
	}
	if res.IsError {
		return nil, res
	}
	var parsed map[string]any
	if err := json.Unmarshal([]byte(tc.Text), &parsed); err != nil {
		t.Fatalf("invalid JSON %q: %v", tc.Text, err)
	}
	return parsed, res
}

func TestIPAVOpen_createsTaskAndIndexEntry(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "medium",
	})
	if parsed["status"] != "opened" {
		t.Errorf("status = %v, want opened", parsed["status"])
	}
	taskID, _ := parsed["task_id"].(string)
	if taskID == "" {
		t.Error("task_id empty")
	}
	if parsed["current_phase"] != "I" {
		t.Errorf("current_phase = %v, want I", parsed["current_phase"])
	}
	if parsed["decision_class"] != "medium" {
		t.Errorf("decision_class = %v, want medium", parsed["decision_class"])
	}
}

func TestIPAVOpen_invalidDecisionClass(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")

	_, res := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "bogus",
	})
	if res == nil || !res.IsError {
		t.Fatalf("expected error result, got %+v", res)
	}
}

func TestIPAVTransition_validProgression(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	transition := findHandler(tools, "bot_hq_ipav_transition")

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "medium",
	})
	taskID := parsed["task_id"].(string)

	parsed, _ = invokeTool(t, transition, map[string]any{
		"project":  "test-project",
		"task_id":  taskID,
		"to_phase": "plan",
	})
	if parsed["current_phase"] != "P" {
		t.Errorf("after I→P transition, current_phase = %v", parsed["current_phase"])
	}
	if parsed["phase_mode"] != "plan-bilateral" {
		t.Errorf("medium decision_class should auto-set bilateral plan mode; got %v", parsed["phase_mode"])
	}
}

func TestIPAVTransition_invalidJump(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	transition := findHandler(tools, "bot_hq_ipav_transition")

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "low",
	})
	taskID := parsed["task_id"].(string)

	// I → V is not a valid IPAV transition
	_, res := invokeTool(t, transition, map[string]any{
		"project":  "test-project",
		"task_id":  taskID,
		"to_phase": "verify",
	})
	if res == nil || !res.IsError {
		t.Fatalf("expected error for invalid I→V jump, got %+v", res)
	}
}

func TestIPAVSetArtifact_attachesPath(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	setArt := findHandler(tools, "bot_hq_ipav_set_artifact")

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "low",
	})
	taskID := parsed["task_id"].(string)

	parsed, _ = invokeTool(t, setArt, map[string]any{
		"project": "test-project",
		"task_id": taskID,
		"key":     "investigation_doc",
		"path":    "tasks/" + taskID + "/investigation.md",
	})
	if parsed["status"] != "attached" {
		t.Errorf("status = %v, want attached", parsed["status"])
	}
	if parsed["key"] != "investigation_doc" {
		t.Errorf("key = %v", parsed["key"])
	}
}

func TestIPAVSetArtifact_invalidKey(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	setArt := findHandler(tools, "bot_hq_ipav_set_artifact")

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "low",
	})
	taskID := parsed["task_id"].(string)

	_, res := invokeTool(t, setArt, map[string]any{
		"project": "test-project",
		"task_id": taskID,
		"key":     "nonsense_key",
		"path":    "x.md",
	})
	if res == nil || !res.IsError {
		t.Fatalf("expected error for invalid key, got %+v", res)
	}
}

func TestIPAVComplete_passSetsClosedAt(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	complete := findHandler(tools, "bot_hq_ipav_complete")

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "low",
	})
	taskID := parsed["task_id"].(string)

	parsed, _ = invokeTool(t, complete, map[string]any{
		"project": "test-project",
		"task_id": taskID,
		"result":  "pass",
	})
	if parsed["status"] != "completed" {
		t.Errorf("status = %v", parsed["status"])
	}
	if parsed["verify_result"] != "pass" {
		t.Errorf("verify_result = %v", parsed["verify_result"])
	}
	closedAt, _ := parsed["closed_at"].(string)
	if closedAt == "" {
		t.Error("closed_at should be set on terminal result")
	}
}

// session-lifecycle-cleanup: result=pass + task bound to a session
// auto-fires the full finalizeSession flow (hook fires, manifest gets
// written, sessions-table status flips to done). Test pre-writes a
// minimal manifest so finalizeSession's sessions.ReadManifest succeeds.
func TestIPAVComplete_passAutoFinalizesBoundSession(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	complete := findHandler(tools, "bot_hq_ipav_complete")

	sessionID := "test-session-xyz"
	if err := sessions.WriteManifest(sessions.Manifest{
		ID:      sessionID,
		Project: "test-project",
		StartTS: time.Now().UTC(),
		Status:  "active",
		Agents:  []string{"brian", "rain"},
	}); err != nil {
		t.Fatalf("seed manifest: %v", err)
	}

	// Stub the finalize hook to capture the request (in-daemon path).
	var captured SessionFinalizeRequest
	hookFired := false
	SetSessionFinalizeHook(func(req SessionFinalizeRequest) (*SessionFinalizeResult, error) {
		hookFired = true
		captured = req
		return &SessionFinalizeResult{KilledTmux: []string{"brian", "rain"}}, nil
	})
	t.Cleanup(func() { SetSessionFinalizeHook(nil) })

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "low",
		"session_id":     sessionID,
	})
	taskID := parsed["task_id"].(string)

	parsed, _ = invokeTool(t, complete, map[string]any{
		"project": "test-project",
		"task_id": taskID,
		"result":  "pass",
		"outcome": "tests pass; closing session",
	})

	if !hookFired {
		t.Fatal("verify-pass on session-bound task must fire SessionFinalizeHook (session-lifecycle-cleanup invariant)")
	}
	if captured.SessionID != sessionID {
		t.Errorf("hook received SessionID=%q, want %q", captured.SessionID, sessionID)
	}
	autoFinalize, _ := parsed["auto_finalize"].(map[string]any)
	if autoFinalize == nil {
		t.Errorf("response should include auto_finalize block when hook fires; got %v", parsed)
	} else {
		if autoFinalize["session_id"] != sessionID {
			t.Errorf("auto_finalize.session_id = %v, want %q", autoFinalize["session_id"], sessionID)
		}
		fp, _ := autoFinalize["finalize_payload"].(map[string]any)
		if fp == nil {
			t.Errorf("auto_finalize should embed finalize_payload from sessions.Finalize; got %v", autoFinalize)
		} else if fp["status"] != "finalized" {
			t.Errorf("finalize_payload.status = %v, want finalized", fp["status"])
		}
	}

	// Verify the manifest was updated to closed status with end_ts.
	m, err := sessions.ReadManifest(sessionID)
	if err != nil {
		t.Fatalf("read manifest after finalize: %v", err)
	}
	if m.EndTS.IsZero() {
		t.Error("manifest EndTS not set — sessions.Finalize did not run")
	}
}

// session-lifecycle-cleanup: result=fail does NOT auto-finalize, even
// when task is session-bound. Preserves V→P / V→I loop-back paths.
func TestIPAVComplete_failDoesNotAutoFinalize(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	complete := findHandler(tools, "bot_hq_ipav_complete")

	hookFired := false
	SetSessionFinalizeHook(func(req SessionFinalizeRequest) (*SessionFinalizeResult, error) {
		hookFired = true
		return &SessionFinalizeResult{}, nil
	})
	t.Cleanup(func() { SetSessionFinalizeHook(nil) })

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "low",
		"session_id":     "test-session-fail",
	})
	taskID := parsed["task_id"].(string)

	_, _ = invokeTool(t, complete, map[string]any{
		"project": "test-project",
		"task_id": taskID,
		"result":  "fail",
	})
	if hookFired {
		t.Error("verify=fail must NOT auto-finalize the session (loop-back path preserved)")
	}
}

// session-lifecycle-cleanup: result=pass on an unbound task (SessionID
// empty — no session at open time) silently no-ops the auto-finalize.
func TestIPAVComplete_passNoAutoFinalizeWithoutSessionBinding(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	complete := findHandler(tools, "bot_hq_ipav_complete")

	hookFired := false
	SetSessionFinalizeHook(func(req SessionFinalizeRequest) (*SessionFinalizeResult, error) {
		hookFired = true
		return &SessionFinalizeResult{}, nil
	})
	t.Cleanup(func() { SetSessionFinalizeHook(nil) })

	// Open WITHOUT session_id (no active session → SessionID="" on task).
	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project-unbound",
		"decision_class": "low",
	})
	taskID := parsed["task_id"].(string)

	_, _ = invokeTool(t, complete, map[string]any{
		"project": "test-project-unbound",
		"task_id": taskID,
		"result":  "pass",
	})
	if hookFired {
		t.Error("verify=pass on unbound task must NOT auto-finalize (nothing to close)")
	}
}

func TestIPAVComplete_failKeepsTaskOpen(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	complete := findHandler(tools, "bot_hq_ipav_complete")

	parsed, _ := invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "low",
	})
	taskID := parsed["task_id"].(string)

	parsed, _ = invokeTool(t, complete, map[string]any{
		"project": "test-project",
		"task_id": taskID,
		"result":  "fail",
	})
	if parsed["closed_at"] != nil && parsed["closed_at"] != "" {
		t.Errorf("fail result should not close task; got closed_at=%v", parsed["closed_at"])
	}
	loopCount, _ := parsed["verify_loop_count"].(float64)
	if loopCount != 1 {
		t.Errorf("verify_loop_count = %v, want 1", loopCount)
	}
}

func TestIPAVList_returnsAllTasks(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	list := findHandler(tools, "bot_hq_ipav_list")

	for i := 0; i < 3; i++ {
		invokeTool(t, open, map[string]any{
			"project":        "test-project",
			"decision_class": "low",
		})
	}

	parsed, _ := invokeTool(t, list, map[string]any{
		"project": "test-project",
	})
	count, _ := parsed["count"].(float64)
	if count != 3 {
		t.Errorf("count = %v, want 3", count)
	}
}

func TestIPAVList_allProjects(t *testing.T) {
	db := setupTestDB(t)
	tools := BuildTools(db)
	open := findHandler(tools, "bot_hq_ipav_open")
	list := findHandler(tools, "bot_hq_ipav_list")

	invokeTool(t, open, map[string]any{
		"project":        "test-project",
		"decision_class": "low",
	})

	parsed, _ := invokeTool(t, list, map[string]any{
		"project": "__all__",
	})
	count, _ := parsed["count"].(float64)
	if count < 1 {
		t.Errorf("__all__ list count = %v, want >= 1", count)
	}
	tasks, _ := parsed["tasks"].([]any)
	for _, raw := range tasks {
		entry := raw.(map[string]any)
		if !strings.HasPrefix(entry["project"].(string), "test-project") && entry["project"] != "test-project" {
			// __all__ might also include real CL projects if isolation
			// fails; this assertion is a leak detector.
			t.Errorf("__all__ surfaced unexpected project %q — CL isolation may be broken", entry["project"])
		}
	}
}
