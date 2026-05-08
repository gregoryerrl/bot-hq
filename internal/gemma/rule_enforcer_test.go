package gemma

import (
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// setupRuleEnforcerTestDB creates an isolated hub.DB at t.TempDir
// per R39 TEST-ISOLATION.
func setupRuleEnforcerTestDB(t *testing.T) *hub.DB {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatalf("open hub db: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })
	return db
}

func TestRuleEnforcer_NewNotRunning(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)
	if re.IsRunning() {
		t.Error("RuleEnforcer should not be running pre-Start")
	}
}

func TestRuleEnforcer_StartStop(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)
	re.SetInterval(50 * time.Millisecond)

	if err := re.Start(); err != nil {
		t.Fatalf("Start: %v", err)
	}
	if !re.IsRunning() {
		t.Error("RuleEnforcer should be running post-Start")
	}
	re.Stop()
	if re.IsRunning() {
		t.Error("RuleEnforcer should not be running post-Stop")
	}
}

func TestRuleEnforcer_DotParkingDetectsThreshold(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)
	// Use real-time so m.Created (db-assigned) falls within now-window.

	// Insert 3 bare-`.` from same agent within window
	for i := 0; i < dotParkingThreshold; i++ {
		_, err := db.InsertMessage(protocol.Message{FromAgent: "brian", Type: protocol.MsgUpdate, Content: "."})
		if err != nil {
			t.Fatalf("InsertMessage: %v", err)
		}
	}

	msgs, _ := db.GetRecentMessages(10)
	re.DetectDotParking(msgs)

	// Find emma violation emit
	all, _ := db.GetRecentMessages(20)
	found := false
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID && strings.Contains(m.Content, "violation:R36-DOT-PARKING") {
			found = true
			break
		}
	}
	if !found {
		t.Error("expected R36-DOT-PARKING violation emit; not found")
	}
}

func TestRuleEnforcer_DotParkingBelowThresholdNoEmit(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	// Insert 1 bare-`.` (below threshold)
	_, _ = db.InsertMessage(protocol.Message{FromAgent: "brian", Type: protocol.MsgUpdate, Content: "."})

	msgs, _ := db.GetRecentMessages(10)
	re.DetectDotParking(msgs)

	all, _ := db.GetRecentMessages(20)
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID {
			t.Errorf("below-threshold should not emit violation; got %q", m.Content)
		}
	}
}

func TestRuleEnforcer_ShapeDisclosureSkippedDetected(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	// Insert estimate-emit WITHOUT shape tag
	_, _ = db.InsertMessage(protocol.Message{
		FromAgent: "brian",
		Type:      protocol.MsgUpdate,
		Content:   "F1-X estimate ~150-300 LOC envelope per scope",
	})

	msgs, _ := db.GetRecentMessages(10)
	re.DetectShapeDisclosureSkipped(msgs)

	all, _ := db.GetRecentMessages(20)
	found := false
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID && strings.Contains(m.Content, "violation:R37-SHAPE-DISCLOSURE-SKIPPED") {
			found = true
			break
		}
	}
	if !found {
		t.Error("expected R37-SHAPE-DISCLOSURE-SKIPPED violation; not found")
	}
}

func TestRuleEnforcer_ShapeDisclosureWithTagNoEmit(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	// Insert estimate-emit WITH shape tag (sparse)
	_, _ = db.InsertMessage(protocol.Message{
		FromAgent: "brian",
		Type:      protocol.MsgUpdate,
		Content:   "F1-X estimate ~150-300 LOC sparse core-impl shape per scope",
	})

	msgs, _ := db.GetRecentMessages(10)
	re.DetectShapeDisclosureSkipped(msgs)

	all, _ := db.GetRecentMessages(20)
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID {
			t.Errorf("estimate-with-shape-tag should not emit violation; got %q", m.Content)
		}
	}
}

func TestRuleEnforcer_SnapGatingDetectsRoutine(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	// Insert SNAP-block in routine peer-coord (no phase-transition keyword)
	_, _ = db.InsertMessage(protocol.Message{
		FromAgent: "brian",
		Type:      protocol.MsgUpdate,
		Content:   "brian|peer-ack|Branches: main@abc Agents: brian/rain Pending: nothing Next: continue",
	})

	msgs, _ := db.GetRecentMessages(10)
	re.DetectSnapGatingViolation(msgs)

	all, _ := db.GetRecentMessages(20)
	found := false
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID && strings.Contains(m.Content, "violation:R5-SNAP-GATING") {
			found = true
			break
		}
	}
	if !found {
		t.Error("expected R5-SNAP-GATING violation in routine context; not found")
	}
}

func TestRuleEnforcer_SnapGatingPhaseTransitionNoEmit(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	// Insert SNAP-block at phase-transition (LANDED keyword present)
	_, _ = db.InsertMessage(protocol.Message{
		FromAgent: "brian",
		Type:      protocol.MsgUpdate,
		Content:   "brian|F1-1-LANDED|Branches: main@abc Agents: brian/rain Pending: F1-2 Next: F1-2",
	})

	msgs, _ := db.GetRecentMessages(10)
	re.DetectSnapGatingViolation(msgs)

	all, _ := db.GetRecentMessages(20)
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID {
			t.Errorf("phase-transition SNAP should not emit violation; got %q", m.Content)
		}
	}
}

func TestRuleEnforcer_SkipsOwnEmits(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	// Insert 3 bare-`.` from emma itself (should be skipped — self-skip)
	for i := 0; i < dotParkingThreshold; i++ {
		_, _ = db.InsertMessage(protocol.Message{FromAgent: ruleEnforcerAgentID, Type: protocol.MsgUpdate, Content: "."})
	}

	msgs, _ := db.GetRecentMessages(10)
	re.DetectDotParking(msgs)

	all, _ := db.GetRecentMessages(20)
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID && strings.Contains(m.Content, "violation:") {
			t.Errorf("emma should not flag own emits; got %q", m.Content)
		}
	}
}

func TestRuleEnforcer_RunCycleNoMessagesNoOp(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)
	// Empty DB — RunEnforcementCycle should no-op cleanly
	re.RunEnforcementCycle()
	msgs, _ := db.GetRecentMessages(20)
	if len(msgs) != 0 {
		t.Errorf("expected empty DB; got %d msgs", len(msgs))
	}
}

func TestRuleEnforcer_ParseLLMNoViolations(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	count := re.parseAndEmitLLMViolations("NO VIOLATIONS")
	if count != 0 {
		t.Errorf("expected 0 emits on NO VIOLATIONS response; got %d", count)
	}
	all, _ := db.GetRecentMessages(20)
	if len(all) != 0 {
		t.Errorf("expected no emits; got %d msgs", len(all))
	}
}

func TestRuleEnforcer_ParseLLMSingleViolation(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	resp := "VIOLATION: R-INT-1 | msg: 12345 | brian stopped after user said 'proceed'"
	count := re.parseAndEmitLLMViolations(resp)
	if count != 1 {
		t.Errorf("expected 1 emit; got %d", count)
	}
	all, _ := db.GetRecentMessages(20)
	found := false
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID && strings.Contains(m.Content, "violation:R-INT-1") && strings.Contains(m.Content, "msg:12345") {
			found = true
			break
		}
	}
	if !found {
		t.Error("expected R-INT-1 violation emit; not found")
	}
}

func TestRuleEnforcer_ParseLLMMultipleViolations(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	resp := `VIOLATION: R-INT-1 | msg: 100 | first
VIOLATION: R-INT-3 | msg: 200 | second
VIOLATION: R-INT-5 | msg: 300 | third`
	count := re.parseAndEmitLLMViolations(resp)
	if count != 3 {
		t.Errorf("expected 3 emits; got %d", count)
	}
}

func TestRuleEnforcer_ParseLLMMalformedSkipped(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	// Lines that don't start with VIOLATION: or have missing pipe-fields are skipped.
	resp := `random preamble text
VIOLATION: R-INT-1 missing-pipe-format
VIOLATION: R-INT-2 | msg: 50 | valid line
some explanation
VIOLATION: malformed`
	count := re.parseAndEmitLLMViolations(resp)
	if count != 1 {
		t.Errorf("expected 1 valid emit (R-INT-2); got %d", count)
	}
}

func TestRuleEnforcer_LLMSkippedWhenClientNil(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)

	// Insert one message; LLM-judgment requires client; nil client should skip silently
	_, _ = db.InsertMessage(protocol.Message{FromAgent: "brian", Type: protocol.MsgUpdate, Content: "test"})

	// detectInterpretiveViolations with nil client = silent no-op
	msgs, _ := db.GetRecentMessages(10)
	re.detectInterpretiveViolations(msgs)

	all, _ := db.GetRecentMessages(20)
	for _, m := range all {
		if m.FromAgent == ruleEnforcerAgentID {
			t.Errorf("nil-client should not emit; got %q", m.Content)
		}
	}
}

func TestRuleEnforcer_LLMDisabledViaSetLLMEnabled(t *testing.T) {
	db := setupRuleEnforcerTestDB(t)
	re := NewRuleEnforcer(db, nil)
	re.SetLLMEnabled(false)

	// Verify mu-protected toggle is respected
	re.mu.Lock()
	enabled := re.llmEnabled
	re.mu.Unlock()
	if enabled {
		t.Error("SetLLMEnabled(false) should disable; still true")
	}
}
