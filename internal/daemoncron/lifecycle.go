package daemoncron

// Lifecycle-hooks surface — Phase S S-1a-4 (programmatic-move from
// internal/emma/emma.go scattered emit-call-sites) + Z-9d Ollama-strip.
//
// Pre-Z-9d this surface covered Emma startup + Ollama-restart events
// emitted under FromAgent="emma". Z-9d retired the Ollama sidecar (emma
// is now the tmux Claude Code Subprocess on DeepSeek-V4-Pro; the daemon-
// cadence audits live in the SystemMonitor and emit under FromAgent=
// "system"), so the Ollama-class emit helpers were removed alongside
// the SystemMonitor rewrite. RosterPrune is the sole surviving emit
// here; it survived because the SystemMonitor still owns roster hygiene.
//
// If you're looking for the deleted helpers:
//   - EmitOnline (Emma startup announcement) — the Subprocess emits its
//     own "Emma hub orchestrator online (DeepSeek-V4-Pro)" on Start.
//   - EmitOllamaHealthCheckFail / EmitOllamaRestartSuccess /
//     EmitOllamaRestartFail / EmitOllamaRestartTimeout — entire Ollama
//     surface gone; subprocess health goes through tmux has-session
//     checks.

import (
	"fmt"
	"log"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// lifecycleAgentID is the FromAgent identifier for emits in this
	// surface. Z-9d: was "emma" pre-Ollama-strip; flipped to "system"
	// to match the SystemMonitor convention (Z-8b extended). Reasoning:
	// roster-prune is daemon-cadence work, not Emma the orchestrator
	// emitting prose.
	lifecycleAgentID = "system"
)

// BuildRosterPruneContent formats the [ROSTER-PRUNE] notice with the
// pruned agent-id list. count is the slice length; ids are joined
// comma+space for human readability.
func BuildRosterPruneContent(count int, ids []string) string {
	return fmt.Sprintf("[ROSTER-PRUNE] Removed %d stale-offline agent rows (last_seen >24h): %s", count, strings.Join(ids, ", "))
}

// EmitRosterPrune writes the [ROSTER-PRUNE] notice to rain (paired
// with the actual hub.DB.PruneStaleOfflineAgents call performed by
// the caller — daemoncron emits the user-visible result, the
// SystemMonitor owns the prune-execution machinery).
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
