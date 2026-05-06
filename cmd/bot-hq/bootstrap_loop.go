package main

import (
	"context"
	"fmt"
	"log"
	"path/filepath"
	"strings"
	"time"

	bootstrappkg "github.com/gregoryerrl/bot-hq/internal/agents/bootstrap"
	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// runBootstrapDefensiveLoop is the Phase N v3.x-2 bootstrap-write defensive
// snapshot loop. Per design-spike §2.3: writes projects/bot-hq/bootstrap.md
// (and any other registered projects) every 10 minutes OR every 25 new
// hub-msgs since the last write, whichever comes first.
//
// On graceful shutdown (ctx cancel), emits one final snapshot with
// write_trigger=graceful so the next session-open sees clean state.
func runBootstrapDefensiveLoop(ctx context.Context, h *hub.Hub, home string) {
	canonRoot := filepath.Join(home, ".bot-hq")
	ticker := time.NewTicker(10 * time.Minute)
	defer ticker.Stop()

	const msgThreshold = 25
	lastMsgID := int64(0)
	if h != nil && h.DB != nil {
		// Best-effort: seed lastMsgID with current max so we don't trigger on stale.
		if msgs, err := h.DB.ReadMessages("", 0, 1); err == nil && len(msgs) > 0 {
			lastMsgID = msgs[0].ID
		}
	}

	writeSnapshot := func(trigger string) {
		project := "bot-hq" // default; expand to multi-project iteration in Phase O
		fm := bootstrappkg.Frontmatter{
			LastSessionCloseAt: time.Now().UTC(),
			PhaseOrMilestone:   "phase-n-v3.x-2",
			KeyState:           summarizeKeyState(h),
			WriteTrigger:       trigger,
		}
		body := fmt.Sprintf("# Bootstrap snapshot (%s)\n\nWritten by orchestrator defensive loop at %s.\n",
			trigger, time.Now().UTC().Format(time.RFC3339))
		if err := bootstrappkg.Write(canonRoot, project, bootstrappkg.Bootstrap{Frontmatter: fm, Body: body}); err != nil {
			log.Printf("[bootstrap-loop] write failed: %v", err)
		}
	}

	checkMsgRate := func() {
		if h == nil || h.DB == nil {
			return
		}
		msgs, err := h.DB.ReadMessages("", lastMsgID, msgThreshold+1)
		if err != nil {
			return
		}
		if len(msgs) >= msgThreshold {
			writeSnapshot("defensive")
			if len(msgs) > 0 {
				lastMsgID = msgs[0].ID
			}
		}
	}

	for {
		select {
		case <-ctx.Done():
			writeSnapshot("graceful")
			return
		case <-ticker.C:
			writeSnapshot("defensive")
		case <-time.After(30 * time.Second):
			// Mid-tick poll for msg-threshold trigger.
			checkMsgRate()
		}
	}
}

// summarizeKeyState produces a one-line state summary for the bootstrap
// frontmatter. Best-effort; intentionally cheap (no DB scan beyond agent count).
func summarizeKeyState(h *hub.Hub) string {
	if h == nil || h.DB == nil {
		return "hub unavailable"
	}
	agents, err := h.DB.ListAgents("")
	if err != nil {
		return "agent list unavailable"
	}
	var names []string
	for _, a := range agents {
		names = append(names, a.ID)
	}
	if len(names) == 0 {
		return "no agents registered"
	}
	return "agents=" + strings.Join(names, ",")
}
