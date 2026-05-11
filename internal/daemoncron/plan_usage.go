package daemoncron

// Plan-usage 3-sub-emit surface — Phase S S-1a-3.
//
// Migrates the 3 plan-usage emit-templates from gemma to daemoncron:
//   1. PRE-HALT-SNAP — emit at 90% threshold; advise agents to
//      checkpoint AgentState + emit SNAP if mid-substantive-work
//   2. PLAN-CAP-RESUME — emit when plan-usage drops below threshold
//      (auto-clear path); recipients resume work
//   3. PLAN-CAP-CRITICAL — emit at 95% halt threshold; user-facing
//      MsgFlag with halt directive
//
// Scope-deferral note: this commit migrates EMIT TEMPLATES + helper
// fns only. The plan-usage POLLING + threshold-detection cadence
// stays gemma-side (anthropic.UsageClient + monitor loop). Cadence
// migration to daemoncron is event-driven-not-cadence-driven so does
// not fit the per-surface goroutine pattern; deferred to S-close
// carry-forward as Phase-S-followup-2-plan-usage-polling-migration
// class. Per Rain msg 15799 sub-commit-plan scope (~150-200 LOC).
//
// Phase-S-followup: gemma's emitPreHaltSnap / emitPlanCapResume /
// 95%-halt emit-paths delegate unconditionally to the helper functions
// defined here. State (cooldown, dedupe) tracked in daemoncron
// package-scoped vars.

import (
	"fmt"
	"log"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// planCapPreHaltFmt — locked content substring for [PRE-HALT-SNAP]
	// per gemma pre-migration; consumer-side R22 prompt-rule recognition
	// gate matches "[PRE-HALT-SNAP]" prefix.
	planCapPreHaltFmt = "[PRE-HALT-SNAP] plan usage at %d%%, approaching halt threshold — checkpoint AgentState (R20) + emit SNAP if mid-substantive-work to preserve resume anchors. headroom remaining before halt-fire."

	// planCapResumeFmt — locked content substring for plan-cap RESUME
	// emit. Brian/Rain STARTUP prompts match against "plan usage reset"
	// substring; consumer-side recognition gate.
	planCapResumeFmt = "plan usage reset to %d%%, resume work via R16 cross-restart-resume protocol bootstrap (a) git status (b) ~/.bot-hq/projects/bot-hq/phase/<active-phase>.md (c) ~/.bot-hq/projects/bot-hq/ratchets/active.md (d) hub_read backlog since halt-fire"

	// planCapReasonFmt — literal substring brian/rain STARTUP prompts
	// match against ("plan usage at <N>%, halt"). Any reformat MUST be
	// kept consumer-recognition-compatible.
	planCapReasonFmt = "plan usage at %d%%, halt + idle in pane"

	// planCapPreHaltCooldown caps EmitPreHaltSnap to once per window.
	planCapPreHaltCooldown = 5 * time.Minute

	// planUsageAgentID is the FromAgent for plan-cap [PRE-HALT-SNAP] /
	// [RESUME] / [CRITICAL] emits. Z-9d flipped from "emma" to "system"
	// to match the SystemMonitor convention — plan-cap signaling is
	// daemon-cadence work, not Emma the orchestrator emitting prose.
	planUsageAgentID = "system"
)

// planUsageState tracks cooldown + halt-active dedupe for the 3 emit
// surfaces. Package-scoped + mu-guarded since gemma calls these
// helpers sequentially from its poll loop (single-caller-context).
var (
	planUsageStateMu  sync.Mutex
	lastPreHaltSnapAt time.Time
	planCapHaltActive bool
)

// BuildPreHaltSnapContent formats the [PRE-HALT-SNAP] content
// for the given plan-usage pct. Pure function — no side effects;
// callers (gemma delegate path or daemoncron emit) can read the
// canonical format. Consumer-side R22 recognition gate matches the
// "[PRE-HALT-SNAP]" prefix.
func BuildPreHaltSnapContent(pct int) string {
	return fmt.Sprintf(planCapPreHaltFmt, pct)
}

// BuildPlanCapResumeContent formats the [RESUME] auto-clear content.
// Same pure-function pattern as BuildPreHaltSnapContent.
func BuildPlanCapResumeContent(pct int) string {
	return fmt.Sprintf("[RESUME] %s", fmt.Sprintf(planCapResumeFmt, pct))
}

// BuildPlanCapCriticalContent formats the [CRITICAL] halt-threshold
// content. Used by 95%-halt emit-path; consumer-side STARTUP-prompt
// recognition gate matches "plan usage at <N>%, halt" substring.
func BuildPlanCapCriticalContent(pct int) string {
	return fmt.Sprintf("[CRITICAL] %s", fmt.Sprintf(planCapReasonFmt, pct))
}

// dbInserter is the minimal hub.DB surface needed for plan-usage
// emits. Defined as an interface so gemma can pass its hub.DB
// directly without importing the daemoncron Cron struct (avoids a
// circular-dep-class concern when gemma delegates to daemoncron).
type dbInserter interface {
	InsertMessage(protocol.Message) (int64, error)
}

// EmitPreHaltSnap emits the [PRE-HALT-SNAP] MsgUpdate to brian
// + rain, gated by planCapPreHaltCooldown. Returns true if emit
// fired; false if cooldown suppressed.
//
// Cadence migration deferred — gemma polls anthropic API + calls this
// when threshold crosses. daemoncron tracks the per-process cooldown
// state package-scoped (single-caller-context per runtime).
func EmitPreHaltSnap(db dbInserter, now time.Time, pct int) bool {
	planUsageStateMu.Lock()
	coolingDown := !lastPreHaltSnapAt.IsZero() && now.Sub(lastPreHaltSnapAt) < planCapPreHaltCooldown
	if !coolingDown {
		lastPreHaltSnapAt = now
	}
	planUsageStateMu.Unlock()
	if coolingDown {
		return false
	}

	content := BuildPreHaltSnapContent(pct)
	for _, target := range []string{"brian", "rain"} {
		if _, err := db.InsertMessage(protocol.Message{
			FromAgent: planUsageAgentID,
			ToAgent:   target,
			Type:      protocol.MsgUpdate,
			Content:   content,
			Created:   now,
		}); err != nil {
			log.Printf("[daemoncron plan-cap pre-halt-snap] insert failed for %s: %v", target, err)
		}
	}
	return true
}

// EmitPlanCapResume emits the [RESUME] MsgCommand to brian + rain
// telling them plan-usage has dropped below halt threshold. Auto-
// clear path — gemma calls this when its poll loop observes recovery.
func EmitPlanCapResume(db dbInserter, now time.Time, pct int) {
	content := BuildPlanCapResumeContent(pct)
	for _, target := range []string{"brian", "rain"} {
		if _, err := db.InsertMessage(protocol.Message{
			FromAgent: planUsageAgentID,
			ToAgent:   target,
			Type:      protocol.MsgCommand,
			Content:   content,
			Created:   now,
		}); err != nil {
			log.Printf("[daemoncron plan-cap resume] insert failed for %s: %v", target, err)
		}
	}
}

// EmitPlanCapCritical emits the [CRITICAL] MsgFlag to user at 95%
// halt threshold. Gated by halt-active transition (false→true) +
// shouldFlag rate-cap upstream (caller's responsibility).
//
// Returns true if emit fired (false→true transition); false if
// already-active (suppress dup user-flag).
func EmitPlanCapCritical(db dbInserter, now time.Time, pct int) bool {
	planUsageStateMu.Lock()
	wasActive := planCapHaltActive
	planCapHaltActive = true
	planUsageStateMu.Unlock()
	if wasActive {
		return false
	}

	content := BuildPlanCapCriticalContent(pct)
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: planUsageAgentID,
		ToAgent:   "user",
		Type:      protocol.MsgFlag,
		Content:   content,
		Created:   now,
	}); err != nil {
		log.Printf("[daemoncron plan-cap critical] flag insert failed: %v", err)
	}
	return true
}

// ClearPlanCapHaltActive flips the halt-active in-memory state to
// false. Used when gemma's auto-clear path emits resume — pairs with
// EmitPlanCapCritical's transition gate.
func ClearPlanCapHaltActive() {
	planUsageStateMu.Lock()
	defer planUsageStateMu.Unlock()
	planCapHaltActive = false
}

// ResetPlanUsageStateForTest clears all plan-usage package-scoped
// state. Test-only.
func ResetPlanUsageStateForTest() {
	planUsageStateMu.Lock()
	defer planUsageStateMu.Unlock()
	lastPreHaltSnapAt = time.Time{}
	planCapHaltActive = false
}
