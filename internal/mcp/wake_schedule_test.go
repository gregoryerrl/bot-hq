package mcp

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/mark3labs/mcp-go/mcp"
)

// requireHandler reuses findHandler from server_test.go and fails the test if
// the named tool is missing.
func requireHandler(t *testing.T, db *hub.DB, name string) func(context.Context, mcp.CallToolRequest) (*mcp.CallToolResult, error) {
	t.Helper()
	h := findHandler(BuildTools(db), name)
	if h == nil {
		t.Fatalf("tool %q not registered", name)
	}
	return h
}

// resultText extracts the first text-content payload from a CallToolResult.
// Wake tools always return a single text block (toJSON-encoded map).
func resultText(t *testing.T, res *mcp.CallToolResult) string {
	t.Helper()
	if res == nil || len(res.Content) == 0 {
		t.Fatalf("empty result")
	}
	tc, ok := res.Content[0].(mcp.TextContent)
	if !ok {
		t.Fatalf("first content is not TextContent: %#v", res.Content[0])
	}
	return tc.Text
}

func TestHubScheduleWake_HappyPath(t *testing.T) {
	db := setupTestDB(t)
	h := requireHandler(t, db, "hub_schedule_wake")

	fireAt := time.Now().Add(2 * time.Hour).UTC().Truncate(time.Second)
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":         "rain",
		"target_agent": "brian",
		"fire_at":      fireAt.Format(time.RFC3339),
		"payload":      "wake-up: re-test 3",
	}
	res, err := h(context.Background(), req)
	if err != nil {
		t.Fatalf("handler error: %v", err)
	}
	if res.IsError {
		t.Fatalf("unexpected IsError: %s", resultText(t, res))
	}
	var got map[string]any
	if err := json.Unmarshal([]byte(resultText(t, res)), &got); err != nil {
		t.Fatalf("decode result: %v", err)
	}
	if got["status"] != "scheduled" {
		t.Errorf("status got %v want scheduled", got["status"])
	}
	if _, ok := got["wake_id"]; !ok {
		t.Error("wake_id missing from result")
	}
	if got["scheduled_for"] != fireAt.Format(time.RFC3339) {
		t.Errorf("scheduled_for echo drift: got %v want %v", got["scheduled_for"], fireAt.Format(time.RFC3339))
	}

	// DB row was actually written with the right shape.
	id := int64(got["wake_id"].(float64))
	row, err := db.GetWakeSchedule(id)
	if err != nil {
		t.Fatalf("GetWakeSchedule: %v", err)
	}
	if row.TargetAgent != "brian" || row.CreatedBy != "rain" || row.Payload != "wake-up: re-test 3" {
		t.Errorf("row drifted from inputs: %+v", row)
	}
	if !row.FireAt.Equal(fireAt) {
		t.Errorf("fire_at round-trip drift: got %v want %v", row.FireAt, fireAt)
	}
	if row.FireStatus != hub.WakeStatusPending {
		t.Errorf("status got %q want pending", row.FireStatus)
	}
}

func TestHubScheduleWake_RejectsNonISO8601(t *testing.T) {
	db := setupTestDB(t)
	h := requireHandler(t, db, "hub_schedule_wake")
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":         "rain",
		"target_agent": "brian",
		"fire_at":      "in 5 minutes", // O5: relative time deferred to v2
		"payload":      "p",
	}
	res, err := h(context.Background(), req)
	if err != nil {
		t.Fatalf("handler error: %v", err)
	}
	if !res.IsError {
		t.Fatalf("expected IsError on non-ISO input, got: %s", resultText(t, res))
	}
	if !strings.Contains(resultText(t, res), "ISO 8601") {
		t.Errorf("error message should mention ISO 8601: got %q", resultText(t, res))
	}
}

func TestHubScheduleWake_OpenACL(t *testing.T) {
	// Per arch lean 5: any registered agent can schedule for any target. No
	// per-target check at the tool layer beyond non-empty.
	db := setupTestDB(t)
	h := requireHandler(t, db, "hub_schedule_wake")
	fireAt := time.Now().Add(time.Hour).UTC().Truncate(time.Second).Format(time.RFC3339)

	for _, pair := range []struct{ from, target string }{
		{"brian", "rain"},
		{"rain", "brian"},
		{"emma", "brian"},
		{"coder-deadbeef", "rain"},
		{"rain", "rain"}, // self-wake
	} {
		req := mcp.CallToolRequest{}
		req.Params.Arguments = map[string]any{
			"from":         pair.from,
			"target_agent": pair.target,
			"fire_at":      fireAt,
		}
		res, err := h(context.Background(), req)
		if err != nil || res.IsError {
			t.Errorf("from=%s target=%s should succeed (open ACL): err=%v body=%s", pair.from, pair.target, err, resultText(t, res))
		}
	}
}

func TestHubCancelWake_TransitionsAndIdempotent(t *testing.T) {
	db := setupTestDB(t)
	cancelH := requireHandler(t, db, "hub_cancel_wake")

	id, err := db.InsertWakeSchedule("brian", "rain", "p", time.Now().Add(time.Hour))
	if err != nil {
		t.Fatal(err)
	}

	// First cancel: pending → cancelled
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"from": "rain", "wake_id": float64(id)}
	res, err := cancelH(context.Background(), req)
	if err != nil || res.IsError {
		t.Fatalf("first cancel: err=%v body=%s", err, resultText(t, res))
	}
	var got map[string]any
	json.Unmarshal([]byte(resultText(t, res)), &got)
	if got["status"] != "cancelled" {
		t.Errorf("first cancel status: got %v want cancelled", got["status"])
	}
	if got["fire_status"] != "cancelled" {
		t.Errorf("first cancel fire_status: got %v want cancelled", got["fire_status"])
	}

	// Second cancel: idempotent — reports already_terminal, not error
	res, err = cancelH(context.Background(), req)
	if err != nil || res.IsError {
		t.Fatalf("second cancel: err=%v body=%s", err, resultText(t, res))
	}
	json.Unmarshal([]byte(resultText(t, res)), &got)
	if got["status"] != "already_terminal" {
		t.Errorf("second cancel status: got %v want already_terminal", got["status"])
	}
}

func TestHubCancelWake_AlreadyFiredIdempotent(t *testing.T) {
	db := setupTestDB(t)
	cancelH := requireHandler(t, db, "hub_cancel_wake")

	id, _ := db.InsertWakeSchedule("brian", "rain", "p", time.Now().Add(-time.Second))
	if _, err := db.MarkWakeFired(id); err != nil {
		t.Fatal(err)
	}

	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"from": "rain", "wake_id": float64(id)}
	res, err := cancelH(context.Background(), req)
	if err != nil || res.IsError {
		t.Fatalf("cancel-after-fire: err=%v body=%s", err, resultText(t, res))
	}
	var got map[string]any
	json.Unmarshal([]byte(resultText(t, res)), &got)
	if got["status"] != "already_terminal" {
		t.Errorf("cancel-after-fire status: got %v want already_terminal", got["status"])
	}
	if got["fire_status"] != "fired" {
		t.Errorf("fire_status echo: got %v want fired", got["fire_status"])
	}
}

func TestHubCancelWake_MissingIDError(t *testing.T) {
	db := setupTestDB(t)
	cancelH := requireHandler(t, db, "hub_cancel_wake")
	req := mcp.CallToolRequest{}
	req.Params.Arguments = map[string]any{"from": "rain", "wake_id": float64(987654)}
	res, err := cancelH(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if !res.IsError {
		t.Errorf("expected IsError for missing id, got %s", resultText(t, res))
	}
}
