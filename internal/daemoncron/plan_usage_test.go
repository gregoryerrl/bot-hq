package daemoncron

import (
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestBuildPreHaltSnapContent_Format(t *testing.T) {
	got := BuildPreHaltSnapContent(92)
	if !strings.HasPrefix(got, "[PRE-HALT-SNAP]") {
		t.Errorf("missing [PRE-HALT-SNAP] prefix; got %q", got)
	}
	if !strings.Contains(got, "92%") {
		t.Errorf("content should embed 92%%; got %q", got)
	}
	if !strings.Contains(got, "checkpoint AgentState (R20)") {
		t.Errorf("content should reference R20 AgentState; got %q", got)
	}
}

func TestBuildPlanCapResumeContent_Format(t *testing.T) {
	got := BuildPlanCapResumeContent(45)
	if !strings.HasPrefix(got, "[RESUME]") {
		t.Errorf("missing [RESUME] prefix; got %q", got)
	}
	if !strings.Contains(got, "plan usage reset to 45%") {
		t.Errorf("content should embed reset percentage; got %q", got)
	}
	if !strings.Contains(got, "R16 cross-restart-resume protocol") {
		t.Errorf("content should reference R16 resume protocol; got %q", got)
	}
}

func TestBuildPlanCapCriticalContent_Format(t *testing.T) {
	got := BuildPlanCapCriticalContent(96)
	if !strings.HasPrefix(got, "[CRITICAL]") {
		t.Errorf("missing [CRITICAL] prefix; got %q", got)
	}
	if !strings.Contains(got, "plan usage at 96%") {
		t.Errorf("content should embed 96%% halt threshold; got %q", got)
	}
	if !strings.Contains(got, "halt") {
		t.Errorf("content should reference halt; got %q", got)
	}
}

func TestEmitPreHaltSnap_FiresFirstCall(t *testing.T) {
	ResetPlanUsageStateForTest()
	db := setupTestDB(t)
	now := time.Now()

	if !EmitPreHaltSnap(db, now, 92) {
		t.Error("first call should fire (cooldown not active)")
	}

	msgs, _ := db.GetRecentMessages(50)
	count := 0
	for _, m := range msgs {
		if m.FromAgent == planUsageAgentID && strings.HasPrefix(m.Content, "[PRE-HALT-SNAP]") {
			count++
		}
	}
	if count != 2 {
		t.Errorf("expected 2 emits (brian + rain); got %d", count)
	}
}

func TestEmitPreHaltSnap_CooldownSuppression(t *testing.T) {
	ResetPlanUsageStateForTest()
	db := setupTestDB(t)
	now := time.Now()

	_ = EmitPreHaltSnap(db, now, 92)
	// Second call within 5min cooldown should suppress.
	if EmitPreHaltSnap(db, now.Add(2*time.Minute), 93) {
		t.Error("second call within cooldown should suppress")
	}
}

func TestEmitPreHaltSnap_CooldownExpires(t *testing.T) {
	ResetPlanUsageStateForTest()
	db := setupTestDB(t)
	now := time.Now()

	_ = EmitPreHaltSnap(db, now, 92)
	// After cooldown expires, fire again.
	if !EmitPreHaltSnap(db, now.Add(6*time.Minute), 93) {
		t.Error("call past cooldown should fire")
	}
}

func TestEmitPreHaltSnap_RecipientsBoth(t *testing.T) {
	ResetPlanUsageStateForTest()
	db := setupTestDB(t)
	_ = EmitPreHaltSnap(db, time.Now(), 92)

	msgs, _ := db.GetRecentMessages(50)
	recipients := map[string]bool{}
	for _, m := range msgs {
		if m.FromAgent == planUsageAgentID && strings.HasPrefix(m.Content, "[PRE-HALT-SNAP]") {
			recipients[m.ToAgent] = true
		}
	}
	if !recipients["brian"] || !recipients["rain"] {
		t.Errorf("expected both brian + rain recipients; got %v", recipients)
	}
}

func TestEmitPlanCapResume_Emits(t *testing.T) {
	ResetPlanUsageStateForTest()
	db := setupTestDB(t)

	EmitPlanCapResume(db, time.Now(), 45)

	msgs, _ := db.GetRecentMessages(50)
	count := 0
	for _, m := range msgs {
		if m.FromAgent == planUsageAgentID && strings.Contains(m.Content, "[RESUME]") {
			count++
			if m.Type != protocol.MsgCommand {
				t.Errorf("RESUME emit should be MsgCommand; got %s", m.Type)
			}
		}
	}
	if count != 2 {
		t.Errorf("expected 2 RESUME emits (brian + rain); got %d", count)
	}
}

func TestEmitPlanCapCritical_FiresOnTransition(t *testing.T) {
	ResetPlanUsageStateForTest()
	db := setupTestDB(t)

	if !EmitPlanCapCritical(db, time.Now(), 96) {
		t.Error("first call should fire (halt-active transition false→true)")
	}

	msgs, _ := db.GetRecentMessages(50)
	criticalCount := 0
	for _, m := range msgs {
		if m.FromAgent == planUsageAgentID && strings.HasPrefix(m.Content, "[CRITICAL]") {
			criticalCount++
			if m.ToAgent != "user" {
				t.Errorf("CRITICAL should target user; got %q", m.ToAgent)
			}
			if m.Type != protocol.MsgFlag {
				t.Errorf("CRITICAL should be MsgFlag; got %s", m.Type)
			}
		}
	}
	if criticalCount != 1 {
		t.Errorf("expected 1 CRITICAL flag; got %d", criticalCount)
	}
}

func TestEmitPlanCapCritical_SuppressesDuplicates(t *testing.T) {
	ResetPlanUsageStateForTest()
	db := setupTestDB(t)

	_ = EmitPlanCapCritical(db, time.Now(), 96)
	// Already-active → second call suppresses.
	if EmitPlanCapCritical(db, time.Now(), 97) {
		t.Error("duplicate call should suppress (halt already active)")
	}
}

func TestEmitPlanCapCritical_RefiresAfterClear(t *testing.T) {
	ResetPlanUsageStateForTest()
	db := setupTestDB(t)

	_ = EmitPlanCapCritical(db, time.Now(), 96)
	ClearPlanCapHaltActive()
	// After clear, re-fire should work (next halt cycle).
	if !EmitPlanCapCritical(db, time.Now(), 96) {
		t.Error("after ClearPlanCapHaltActive, next call should fire")
	}
}
