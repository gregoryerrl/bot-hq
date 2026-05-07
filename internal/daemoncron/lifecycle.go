package daemoncron

// Lifecycle-hooks surface — Phase S S-1a-4 (programmatic-move from
// internal/gemma/gemma.go scattered emit-call-sites).
//
// Migrates 4 distinct agent-lifecycle event emit-templates from
// gemma to daemoncron with extract-with-delegate architecture. Events
// are NOT cadence-driven (no own goroutine); gemma's existing
// trigger paths fire them on event-occurrence (Start / Ollama-
// restart-success / Ollama-restart-failure / RosterPrune-fire).
//
// Lifecycle events covered:
//   1. Online — Emma startup announcement (gemma.go:372)
//   2. OllamaRestartSuccess — gemma.go:1433
//   3. OllamaRestartFail — gemma.go:1421 (start) + gemma.go:583
//      (health-fail-pre-restart)
//   4. RosterPrune — gemma.go:709 stale-offline agent cleanup notice
//
// Dual-emit prevention (interpretation (ii)): gemma's emit-call-sites
// check g.isDaemoncronOnline() — when true, delegate to daemoncron's
// helpers; when false, fall through to existing gemma-side emit
// (back-compat / pre-S-1a behavior).

import (
	"fmt"
	"log"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// lifecycleAgentID preserved from gemma pre-migration so consumer-
	// side recognition of "from emma" lifecycle events stays unchanged.
	lifecycleAgentID = "emma"
)

// BuildOnlineContent formats the Emma startup announcement content.
// Pre-S-1a-4: "Emma online. Model: <model>".
func BuildOnlineContent(model string) string {
	return fmt.Sprintf("Emma online. Model: %s", model)
}

// BuildOllamaRestartSuccessContent — pre-S-1a-4 verbatim.
func BuildOllamaRestartSuccessContent() string {
	return "Ollama restarted successfully."
}

// BuildOllamaRestartFailContent formats the start-failure variant.
func BuildOllamaRestartFailContent(err error) string {
	return fmt.Sprintf("Ollama restart failed: %v", err)
}

// BuildOllamaHealthCheckFailContent — pre-restart health-fail emit.
func BuildOllamaHealthCheckFailContent() string {
	return "Ollama health check failed. Attempting restart..."
}

// BuildOllamaRestartTimeoutContent — post-restart-attempt timeout.
func BuildOllamaRestartTimeoutContent() string {
	return "Ollama restart: health check timed out."
}

// BuildRosterPruneContent formats the [ROSTER-PRUNE] notice with the
// pruned agent-id list. count is the slice length; ids are joined
// comma+space for human readability.
func BuildRosterPruneContent(count int, ids []string) string {
	return fmt.Sprintf("[ROSTER-PRUNE] Removed %d stale-offline agent rows (last_seen >24h): %s", count, strings.Join(ids, ", "))
}

// EmitOnline writes the Emma startup announcement as a broadcast
// MsgUpdate. Caller (gemma Start path) provides the model string.
func EmitOnline(db dbInserter, model string) {
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: lifecycleAgentID,
		Type:      protocol.MsgUpdate,
		Content:   BuildOnlineContent(model),
		Created:   time.Now(),
	}); err != nil {
		log.Printf("[daemoncron lifecycle online] insert failed: %v", err)
	}
}

// EmitOllamaHealthCheckFail writes the pre-restart health-failure
// notice as a broadcast MsgError. Paired with subsequent restart
// attempt + EmitOllamaRestartSuccess / EmitOllamaRestartFail.
func EmitOllamaHealthCheckFail(db dbInserter) {
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: lifecycleAgentID,
		Type:      protocol.MsgError,
		Content:   BuildOllamaHealthCheckFailContent(),
		Created:   time.Now(),
	}); err != nil {
		log.Printf("[daemoncron lifecycle ollama-health-fail] insert failed: %v", err)
	}
}

// EmitOllamaRestartSuccess writes the success notice post-restart.
func EmitOllamaRestartSuccess(db dbInserter) {
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: lifecycleAgentID,
		Type:      protocol.MsgUpdate,
		Content:   BuildOllamaRestartSuccessContent(),
		Created:   time.Now(),
	}); err != nil {
		log.Printf("[daemoncron lifecycle ollama-restart-success] insert failed: %v", err)
	}
}

// EmitOllamaRestartFail writes the start-failure notice as MsgError.
func EmitOllamaRestartFail(db dbInserter, restartErr error) {
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: lifecycleAgentID,
		Type:      protocol.MsgError,
		Content:   BuildOllamaRestartFailContent(restartErr),
		Created:   time.Now(),
	}); err != nil {
		log.Printf("[daemoncron lifecycle ollama-restart-fail] insert failed: %v", err)
	}
}

// EmitOllamaRestartTimeout writes the post-restart-attempt timeout
// notice as MsgError.
func EmitOllamaRestartTimeout(db dbInserter) {
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: lifecycleAgentID,
		Type:      protocol.MsgError,
		Content:   BuildOllamaRestartTimeoutContent(),
		Created:   time.Now(),
	}); err != nil {
		log.Printf("[daemoncron lifecycle ollama-restart-timeout] insert failed: %v", err)
	}
}

// EmitRosterPrune writes the [ROSTER-PRUNE] notice to rain (paired
// with the actual hub.DB.PruneStaleOfflineAgents call performed by
// the caller — daemoncron emits the user-visible result, gemma owns
// the prune-execution machinery).
func EmitRosterPrune(db dbInserter, ids []string) {
	if len(ids) == 0 {
		return
	}
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: lifecycleAgentID,
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   BuildRosterPruneContent(len(ids), ids),
		Created:   time.Now(),
	}); err != nil {
		log.Printf("[daemoncron lifecycle roster-prune] insert failed: %v", err)
	}
}
