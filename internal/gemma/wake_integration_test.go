package gemma_test

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/gemma"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/mcp"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	mcpgo "github.com/mark3labs/mcp-go/mcp"
)

// TestWakeScheduleEndToEnd exercises the full slice-3 C1 (#7) chain:
// hub_schedule_wake MCP call → DB insert → Emma wakeDispatchLoop tick →
// hub_send (from=emma, type=command) → message visible to target via
// db.ReadMessages. Acceptance bound: target receives the message within
// the design-locked tick_interval + slack (10s for fire_at=NOW+5s).
//
// Uses the MCP tool layer end-to-end (not direct db.InsertWakeSchedule)
// so the ISO 8601 parser, the open-ACL surface, and the result schema
// are all exercised in a single run — the gold-class acceptance test
// the slice-3 design names for surface #7.
func TestWakeScheduleEndToEnd(t *testing.T) {
	if testing.Short() {
		t.Skip("end-to-end timing test; skipped under -short")
	}

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	// Spin Emma's wake-dispatch loop in a private goroutine. We don't call
	// gemma.New()/Start() because that boots Ollama and the full agent
	// surface; this test only needs the dispatch tick and uses a hand-built
	// stop channel to guarantee teardown even on assertion failure.
	stop := make(chan struct{})
	defer close(stop)
	g := gemma.NewWakeOnlyForTest(db, stop)
	go g.RunWakeDispatchLoopForTest()

	// Schedule a wake via the MCP tool — this is the real cross-process call
	// shape (Rain → MCP → DB) we want to lock.
	scheduleH := findToolHandler(t, db, "hub_schedule_wake")
	fireAt := time.Now().Add(2 * time.Second).UTC().Truncate(time.Second)
	req := mcpgo.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":         "rain",
		"target_agent": "brian",
		"fire_at":      fireAt.Format(time.RFC3339),
		"payload":      "wake-up: re-test 3",
	}
	res, err := scheduleH(context.Background(), req)
	if err != nil || res.IsError {
		t.Fatalf("hub_schedule_wake failed: err=%v body=%s", err, textOf(res))
	}
	var scheduleResult map[string]any
	if err := json.Unmarshal([]byte(textOf(res)), &scheduleResult); err != nil {
		t.Fatal(err)
	}
	wakeID := int64(scheduleResult["wake_id"].(float64))

	// Wait up to 10s for the dispatched message to surface in brian's hub
	// inbox. Acceptance bound from the design doc: fire_at = +2s should
	// arrive comfortably inside 10s with the 1s wakeDispatchInterval.
	deadline := time.Now().Add(10 * time.Second)
	var dispatched *protocol.Message
	for time.Now().Before(deadline) {
		msgs, err := db.ReadMessages("brian", 0, 50)
		if err != nil {
			t.Fatal(err)
		}
		for i := range msgs {
			if msgs[i].FromAgent == "emma" && msgs[i].Content == "wake-up: re-test 3" {
				dispatched = &msgs[i]
				break
			}
		}
		if dispatched != nil {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	if dispatched == nil {
		t.Fatalf("wake never dispatched within 10s of fire_at %s", fireAt.Format(time.RFC3339))
	}
	if dispatched.Type != protocol.MsgCommand {
		t.Errorf("dispatched message type: got %q want command", dispatched.Type)
	}

	// State-machine invariant: the row must end in 'fired' (not pending,
	// not failed). Locks the pending → fired success-edge from O6.
	row, err := db.GetWakeSchedule(wakeID)
	if err != nil {
		t.Fatal(err)
	}
	if row.FireStatus != hub.WakeStatusFired {
		t.Errorf("wake row final state: got %q want fired", row.FireStatus)
	}
	if row.FiredAt.IsZero() {
		t.Error("fired_at not set after dispatch")
	}
}

// TestWakeCancelBeforeFireBlocksDispatch locks the cancel-before-fire path
// end-to-end via MCP: a wake scheduled for the future, cancelled before its
// fire_at elapses, must never produce a hub_send when its fire_at arrives.
func TestWakeCancelBeforeFireBlocksDispatch(t *testing.T) {
	if testing.Short() {
		t.Skip("end-to-end timing test; skipped under -short")
	}
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	stop := make(chan struct{})
	defer close(stop)
	g := gemma.NewWakeOnlyForTest(db, stop)
	go g.RunWakeDispatchLoopForTest()

	scheduleH := findToolHandler(t, db, "hub_schedule_wake")
	cancelH := findToolHandler(t, db, "hub_cancel_wake")

	fireAt := time.Now().Add(2 * time.Second).UTC().Truncate(time.Second)
	req := mcpgo.CallToolRequest{}
	req.Params.Arguments = map[string]any{
		"from":         "rain",
		"target_agent": "brian",
		"fire_at":      fireAt.Format(time.RFC3339),
		"payload":      "should-not-fire",
	}
	res, err := scheduleH(context.Background(), req)
	if err != nil || res.IsError {
		t.Fatalf("schedule failed: %v", err)
	}
	var scheduleResult map[string]any
	json.Unmarshal([]byte(textOf(res)), &scheduleResult)
	wakeID := int64(scheduleResult["wake_id"].(float64))

	// Cancel before fire_at.
	cReq := mcpgo.CallToolRequest{}
	cReq.Params.Arguments = map[string]any{"from": "rain", "wake_id": float64(wakeID)}
	if _, err := cancelH(context.Background(), cReq); err != nil {
		t.Fatalf("cancel: %v", err)
	}

	// Sleep past fire_at + dispatch slack and confirm no message landed.
	time.Sleep(4 * time.Second)
	msgs, _ := db.ReadMessages("brian", 0, 50)
	for _, m := range msgs {
		if m.FromAgent == "emma" && m.Content == "should-not-fire" {
			t.Fatalf("cancelled wake fired anyway: %+v", m)
		}
	}
	row, _ := db.GetWakeSchedule(wakeID)
	if row.FireStatus != hub.WakeStatusCancelled {
		t.Errorf("wake row state: got %q want cancelled", row.FireStatus)
	}
}

func findToolHandler(t *testing.T, db *hub.DB, name string) func(context.Context, mcpgo.CallToolRequest) (*mcpgo.CallToolResult, error) {
	t.Helper()
	for _, td := range mcp.BuildTools(db) {
		if td.Tool.Name == name {
			return td.Handler
		}
	}
	t.Fatalf("tool %q not registered", name)
	return nil
}

func textOf(res *mcpgo.CallToolResult) string {
	if res == nil || len(res.Content) == 0 {
		return ""
	}
	tc, ok := res.Content[0].(mcpgo.TextContent)
	if !ok {
		return ""
	}
	return tc.Text
}
