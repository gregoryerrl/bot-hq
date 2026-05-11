// Package emma houses two distinct surfaces post-Z-9d:
//
//  1. Subprocess (subprocess.go) — the tmux Claude Code pane that
//     IS Emma the hub orchestrator (DeepSeek-V4-Pro per vision.md).
//     Registers as the "emma" agent in hub.db.agents.
//
//  2. SystemMonitor (this file + delivery_audit / egress_audit /
//     plan_usage / sentinel / wake / context_cap) — the pure-Go
//     daemon-cadence audit + observation engine. Signs emits as
//     "system" (Z-8b convention); does NOT register as an agent.
//
// Historical drift: pre-Z-9d this package ran Ollama (gemma4:e4b) and
// did BOTH roles via a single Emma struct. Z-9d split them per vision.md.
// The Ollama path + canonicalEmmaBlock + custom_rules + rule_enforcer +
// mention_directive surfaces were retired with the same slice.
package emma

import (
	"context"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/daemoncron"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// agentID is the shared agent identifier for emma the hub orchestrator
	// (the Subprocess registers under this ID). SystemMonitor uses it for
	// self-filtering (skip emma-subprocess's own emits from sentinel) and
	// for context-cap exclusion (emma's pane is stateless; never halt on it).
	agentID   = "emma"
	agentName = "Emma"

	// systemFromAgent is the synthetic from_agent label that SystemMonitor
	// emits under (Z-8b convention extended Z-9d). The label is NOT a
	// registered agent — there is no agents.id="system" row. It exists
	// only as a discriminator on messages so the user can distinguish
	// daemon-cadence audits from agent-authored prose.
	systemFromAgent = "system"

	healthInterval         = 30 * time.Second
	defaultMonitorInterval = 5 * time.Minute
	// sentinelPollInterval gates the H-22 cross-process MCP-insert catch-up
	// path. See sentinelPollLoop for the rationale on why this exists.
	sentinelPollInterval = 5 * time.Second
	// wakeDispatchInterval is the slice-3 C1 (#7) wake_schedule fire cadence.
	// 1s gives sub-second round-up against the seconds-precision fire_at
	// granularity locked by arch lean 1, and stays comfortably inside the
	// "fires within tick_interval + 1s" acceptance bound from the design.
	wakeDispatchInterval = 1 * time.Second

	// Phase D — flag dedupe + rate cap (shared across all monitor conditions).
	flagHysteresisWindow   = 30 * time.Minute
	flagRateCapPerHour     = 3
	monitorPreconditionGap = 1 * time.Hour
)

// SystemMonitor runs the daemon-cadence audits + observation work that
// used to live inside the Ollama-driven Emma. Pure Go, no LLM, signs
// emits as "system". The conversational "emma" agent is the Subprocess
// in subprocess.go.
type SystemMonitor struct {
	db        *hub.DB
	lastMsgID int64

	monitorInterval time.Duration

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}

	// sentinelMu guards lastSentinelMsgID. The watermark is read+advanced
	// from sentinelPollLoop and replayThroughSentinel — distinct goroutines.
	sentinelMu        sync.Mutex
	lastSentinelMsgID int64

	// Phase D flag bookkeeping. flagHistory dedupes by condition key
	// (last-fired timestamp). flagWindow is a sliding 1h record of all
	// fired flags for the shared rate cap.
	flagMu      sync.Mutex
	flagHistory map[string]time.Time
	flagWindow  []time.Time

	// Phase H slice 3 C5 (H-25 roster hygiene) state. lastPruneAt is read+
	// written only from healthLoop so no mutex needed; zero value means
	// "never pruned this run", which triggers a first-tick prune.
	lastPruneAt time.Time

	// Phase H slice 4 C6 (H-31) context-cap halt-flag source. Tests assign
	// directly; production wires a panestate.Manager-backed closure on
	// Start via initContextCapDefault.
	paneSnapFn paneSnapshotFn

	// Phase H slice 5 C1 (H-32+H-33) plan-usage state. Same field shape as
	// pre-Z-9d. planUsageFetch is the fetcher abstraction (default wired to
	// a real anthropic.UsageClient on Start; tests inject a fake).
	// hubPublisher is the panestate.Manager.SetHubSnapshot sink — production
	// wires it in cmd/bot-hq/main.go after the TUI's Manager exists; tests
	// inject a recorder.
	planUsageFetch    PlanUsageFetcher
	hubPublisher      func(panestate.HubSnapshot)
	planUsageMu       sync.Mutex
	lastPlanPoll      time.Time
	planBackoffUntil  time.Time
	planUsageWarnedOS bool

	// Phase J tail (RESUME-spam fix) + tail-2 (K-1) in-memory transition
	// tracking for plan-cap halt state. Mu-protected via planUsageMu.
	lastPlanCapResumeAt time.Time
	planCapHaltActive   bool

	// Slice-5 H-22-bis item 4 auditor state. deliveryFlagTracker dedupes
	// [DELIVERY-GAP] alerts by message_queue row id.
	deliveryFlagMu      sync.Mutex
	deliveryFlagTracker map[int64]struct{}

	// Egress auditor state — see egress_audit.go for full rationale.
	egressMu          sync.Mutex
	egressBaseline    map[string]egressBaselineEntry
	egressFlagTracker map[string]struct{}
	egressPaneCapture egressPaneCaptureFn
}

// NewSystemMonitor builds the daemon-cadence engine. No config dependency:
// the EmmaConfig fields that used to drive Ollama (Model/OllamaURL/MaxConcurrent)
// are dead post-Z-9d.
func NewSystemMonitor(db *hub.DB) *SystemMonitor {
	return &SystemMonitor{
		db:              db,
		monitorInterval: defaultMonitorInterval,
		stopCh:          make(chan struct{}),
		flagHistory:     make(map[string]time.Time),
	}
}

// Start launches the daemon-cadence goroutines. Idempotent. Does NOT
// register a hub agent — SystemMonitor is unregistered infrastructure.
func (g *SystemMonitor) Start() error {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.running {
		return nil
	}

	// Seed lastMsgID with hub's current MAX so the first sentinel-poll
	// tick doesn't replay the entire backlog. Same Z-0 CL-first principle
	// brian/rain follow.
	msgs, err := g.db.GetRecentMessages(1)
	if err == nil && len(msgs) > 0 {
		g.lastMsgID = msgs[0].ID
	}

	g.running = true

	// Phase H slice 4 C6 (H-31): wire the panestate snapshot source if a
	// test hasn't already injected one. Default uses a hub-DB-backed
	// panestate.Manager + real tmux.CapturePane.
	g.initContextCapDefault()

	// Phase H slice 5 C1 (H-32+H-33): wire the production plan-usage
	// fetcher (keychain + /api/oauth/usage) when no test fake is
	// already injected. Non-darwin hosts publish PlanUsagePct=-1 and
	// skip polling forever per ErrUnsupportedPlatform contract.
	g.initPlanUsageDefault()

	// Phase J tail-4 (K-1-bis-deeper Axis A): seed in-memory
	// planCapHaltActive from hub.db halt_state so the post-restart
	// fire-path correctly recognizes continuous halt across process
	// restart.
	g.seedPlanCapHaltActiveFromDB()

	go g.healthLoop()
	go g.monitorLoop()
	go g.sentinelPollLoop()
	go g.wakeDispatchLoop()

	// Phase H slice 3 C7: schedule first _internal:docdrift wake if no
	// pending one exists from a prior boot. Re-arms on every fire via
	// runDocDriftScanAndReArm.
	g.bootstrapInternalDocDrift()

	// Boot-time replay: run the most recent N hub messages through the
	// sentinel so any active failures from the pre-restart window are
	// caught. Live in-process messages from this point onward arrive via
	// the OnMessage subscriber wired in cmd/bot-hq/main.go; cross-process
	// MCP-routed inserts arrive via sentinelPollLoop's tick.
	g.replayThroughSentinel()

	return nil
}

// Stop halts the daemon-cadence goroutines. No Ollama kill or agent-
// status update — SystemMonitor doesn't own either.
func (g *SystemMonitor) Stop() {
	g.mu.Lock()
	defer g.mu.Unlock()
	if !g.running {
		return
	}
	g.running = false
	close(g.stopCh)
}

// healthLoop runs the 30s cadence tier: roster-prune + context-cap +
// delivery-gap audit. Pre-Z-9d this also polled Ollama health; that path
// is retired (no more Ollama).
func (g *SystemMonitor) healthLoop() {
	ticker := time.NewTicker(healthInterval)
	defer ticker.Stop()
	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			g.runRosterPrune()
			g.checkContextCap(time.Now())
			g.auditDeliveryGap()
		}
	}
}

// pruneInterval is how often runRosterPrune fires (only one prune per
// pruneInterval window even though healthLoop ticks every 30s).
const pruneInterval = 1 * time.Hour

// pruneThreshold is the last_seen age beyond which an offline agent row is
// eligible for deletion. 24h ensures intermittent agents that go offline
// briefly are not pruned; only long-dead rows reclaim space.
const pruneThreshold = 24 * time.Hour

// runRosterPrune deletes offline agent rows older than pruneThreshold and
// emits a [ROSTER-PRUNE] notice for audit. Idempotent (no-op when nothing
// to prune). Live agents (online/working) are protected by the status
// filter inside PruneStaleOfflineAgents — never pruned regardless of age.
func (g *SystemMonitor) runRosterPrune() {
	now := time.Now()
	if !g.lastPruneAt.IsZero() && now.Sub(g.lastPruneAt) < pruneInterval {
		return
	}
	g.lastPruneAt = now
	ids, err := g.db.PruneStaleOfflineAgents(pruneThreshold)
	if err != nil {
		log.Printf("[system-monitor] roster prune failed: %v", err)
		return
	}
	if len(ids) == 0 {
		return
	}
	daemoncron.EmitRosterPrune(g.db, ids)
}

// replayBacklog is the number of recent hub messages SystemMonitor
// re-classifies at boot to catch active failures from the pre-restart window.
const replayBacklog = 50

// OnHubMessage is the OnMessage subscriber SystemMonitor registers with the hub.
// Pure dispatcher: skips own emits (FromAgent="system") + registered-agent
// emits (process-self-tool-call), runs SentinelMatch on everything else, and
// hands matched messages to dispatchSentinelHit.
//
// Wired post-Start in cmd/bot-hq/main.go via h.DB.OnMessage(...).
func (g *SystemMonitor) OnHubMessage(msg protocol.Message) {
	if msg.FromAgent == systemFromAgent {
		return // skip self to avoid feedback loops
	}
	d := SentinelMatch(msg)
	if !d.Match {
		return // default-ignore
	}
	// Source-filter: registered-agent hub_send is process-self-tool-call
	// (agent-prose by construction), not a crash-report channel. Real crash
	// reports arrive via out-of-band capture (stderr/tmux/PID-monitor),
	// since a panicking process can't emit a tool call from itself.
	//
	// Empirical anchor (hub.db FP-rate query 2026-04-28, Phase I msg #4778):
	// 14/14 always-flag hits from registered agents over months were
	// prose-mention FPs (panic 5, fatal 4, rate-limit 3, schema-constraint 1,
	// segfault 1) — zero observed real-event loss against 100% observed FP
	// suppression. Earlier H-22-bis source-filter (queueFailPattern only)
	// was conservative; data-supported extension to all sentinel patterns
	// landed Phase I W1b.
	if g.isFromRegisteredAgent(msg.FromAgent) {
		return
	}
	g.dispatchSentinelHit(msg, d)
}

// isFromRegisteredAgent reports whether the given agent ID is currently
// registered in the hub. Best-effort: a DB error returns false (fail-open).
func (g *SystemMonitor) isFromRegisteredAgent(id string) bool {
	if id == "" {
		return false
	}
	agents, err := g.db.ListAgents("")
	if err != nil {
		return false
	}
	for _, a := range agents {
		if a.ID == id {
			return true
		}
	}
	return false
}

// dispatchSentinelHit emits the appropriate hub message for a sentinel
// match. Always-flag matches go out as Type=MsgFlag (Discord-bound);
// pre-filter-only matches go out as Type=MsgUpdate observations to Rain.
// Both paths reuse shouldFlag's hysteresis + rate cap. Emits sign as
// "system" (Z-8b convention extended Z-9d).
func (g *SystemMonitor) dispatchSentinelHit(msg protocol.Message, d SentinelDecision) {
	now := time.Now()
	if d.AlwaysFlag {
		if !g.shouldFlag("sentinel:"+d.Pattern, now) {
			return
		}
		g.db.InsertMessage(protocol.Message{
			FromAgent: systemFromAgent,
			Type:      protocol.MsgFlag,
			Content:   fmt.Sprintf("Sentinel always-flag hit in msg #%d (from %s, pattern %s): %s", msg.ID, msg.FromAgent, d.Pattern, summarizeOutput(msg.Content, 200)),
		})
		return
	}
	if !g.shouldFlag("sentinel-obs:"+d.Pattern, now) {
		return
	}
	// H-22 dry-run gate: patterns in the tuning-gate period write to a
	// ledger file instead of pinging Rain via hub.
	if name, isDryRun := IsDryRunPattern(d.Pattern); isDryRun {
		AppendToDryRunLedger(name, fmt.Sprintf("msg #%d from %s | pattern %s | %s", msg.ID, msg.FromAgent, d.Pattern, summarizeOutput(msg.Content, 200)))
		return
	}
	g.db.InsertMessage(protocol.Message{
		FromAgent: systemFromAgent,
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   fmt.Sprintf("Sentinel match in msg #%d (from %s, pattern %s): %s", msg.ID, msg.FromAgent, d.Pattern, summarizeOutput(msg.Content, 200)),
	})
}

// sentinelPollLoop is the cross-process catch-up path for sentinel
// detection. Closes the H-22 acceptance gap discovered at slice 2 runtime
// test 3.
//
// Why this exists: db.OnMessage callbacks fire only for inserts made by
// *the same process* that registered the callback. Brian, Rain, coders,
// and the Subprocess all emit hub_send via the MCP server (separate
// process), so their inserts never traverse the TUI process's onMessages
// list. Pre-hotfix, SystemMonitor's only path to see such messages was
// the boot-time replayThroughSentinel window.
func (g *SystemMonitor) sentinelPollLoop() {
	ticker := time.NewTicker(sentinelPollInterval)
	defer ticker.Stop()
	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			g.pollSentinel()
			// Phase H slice 5 C1 (H-32+H-33): plan-usage check piggybacks
			// on the 5s sentinel tick. The 60s base cadence + 600s backoff
			// gates live inside checkPlanUsage.
			g.checkPlanUsage(time.Now())
		}
	}
}

// pollSentinel reads all DB messages newer than the watermark and feeds
// each through OnHubMessage. Updates the watermark to the max processed
// ID so the next tick is incremental.
func (g *SystemMonitor) pollSentinel() {
	g.sentinelMu.Lock()
	since := g.lastSentinelMsgID
	g.sentinelMu.Unlock()

	msgs, err := g.db.ReadMessages("", since, 200)
	if err != nil {
		return
	}

	maxID := since
	for _, m := range msgs {
		g.OnHubMessage(m)
		if m.ID > maxID {
			maxID = m.ID
		}
	}

	g.sentinelMu.Lock()
	if maxID > g.lastSentinelMsgID {
		g.lastSentinelMsgID = maxID
	}
	g.sentinelMu.Unlock()
}

// monitorLoop proactively runs Tier 1 health checks on a configurable interval.
func (g *SystemMonitor) monitorLoop() {
	ticker := time.NewTicker(g.monitorInterval)
	defer ticker.Stop()
	for {
		select {
		case <-g.stopCh:
			return
		case <-ticker.C:
			g.runHealthChecks()
			// Slice-5 H-22-bis item 4 (expensive-tier auditor): pane
			// content + cross-agent hub-msg scan once per 5min cadence.
			g.auditEgressGap()
		}
	}
}

// anomaly is a single detected condition with a stable key for hysteresis
// dedupe and a human-readable message for the flag body.
type anomaly struct {
	key, msg string
}

// checkAgentImbalance reports an offline-ratio anomaly if non-coder agents
// skew offline. Coders are excluded — they are spawn-and-die by design.
func checkAgentImbalance(agents []protocol.Agent) (anomaly, bool) {
	online, offline := 0, 0
	considered := 0
	for _, a := range agents {
		if a.Type == protocol.AgentCoder {
			continue
		}
		considered++
		if a.Status == protocol.StatusOnline || a.Status == protocol.StatusWorking {
			online++
		} else {
			offline++
		}
	}
	if offline > online && considered > 1 {
		return anomaly{
			key: "agent-imbalance",
			msg: fmt.Sprintf("Agent anomaly: %d online, %d offline (coders excluded)", online, offline),
		}, true
	}
	return anomaly{}, false
}

// pseudoFilesystemMounts lists mount-point prefixes whose capacity reading
// is a kernel artifact, not real disk pressure.
var pseudoFilesystemMounts = []string{
	"/dev",
	"/proc",
	"/sys",
	"/run",
	"/System/Volumes/VM",
	"/System/Volumes/Preboot",
	"/System/Volumes/Update",
	"/System/Volumes/iSCPreboot",
	"/System/Volumes/xarts",
	"/System/Volumes/Hardware",
	"/private/var/vm",
}

func isPseudoMount(mount string) bool {
	for _, prefix := range pseudoFilesystemMounts {
		if mount == prefix || strings.HasPrefix(mount, prefix+"/") {
			return true
		}
	}
	return false
}

// shouldSkipMonitor returns true when monitorLoop should no-op entirely.
// Precondition: no non-system, non-emma hub traffic in the last hour AND
// no agents currently in `working` status. Avoids alerting about an empty
// house. Filters both systemFromAgent (our own emits) and agentID (emma
// subprocess emits) since both are background noise from the monitor's
// perspective.
func (g *SystemMonitor) shouldSkipMonitor() bool {
	cutoff := time.Now().Add(-monitorPreconditionGap)
	msgs, err := g.db.GetRecentMessages(50)
	if err == nil {
		for _, m := range msgs {
			if m.FromAgent != systemFromAgent && m.FromAgent != agentID && m.Created.After(cutoff) {
				return false
			}
		}
	}

	agents, err := g.db.ListAgents("")
	if err == nil {
		for _, a := range agents {
			if a.Status == protocol.StatusWorking {
				return false
			}
		}
	}
	return true
}

// shouldFlag applies hysteresis (per-condition) and the shared rate cap.
// Returns true if this anomaly should be emitted now and records it.
func (g *SystemMonitor) shouldFlag(condition string, now time.Time) bool {
	g.flagMu.Lock()
	defer g.flagMu.Unlock()

	if last, ok := g.flagHistory[condition]; ok {
		if now.Sub(last) < flagHysteresisWindow {
			return false
		}
	}

	cutoff := now.Add(-time.Hour)
	pruned := g.flagWindow[:0]
	for _, t := range g.flagWindow {
		if t.After(cutoff) {
			pruned = append(pruned, t)
		}
	}
	g.flagWindow = pruned
	if len(g.flagWindow) >= flagRateCapPerHour {
		return false
	}

	g.flagHistory[condition] = now
	g.flagWindow = append(g.flagWindow, now)
	return true
}

// runHealthChecks executes Tier 1 checks and reports anomalies to Rain.
// Refinement A: hub-quiet is no longer a standalone flag — it's only
// useful inside a Tier 3 stall-detector conjunction.
// Refinement B: the whole loop no-ops when nobody is working.
func (g *SystemMonitor) runHealthChecks() {
	if g.shouldSkipMonitor() {
		return
	}

	var anomalies []anomaly
	projectDir := ProjectDir()

	// 1. Run go test in bot-hq project directory
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	testCmd := exec.CommandContext(ctx, "go", "test", "./...")
	testCmd.Dir = projectDir
	testOut, testErr := testCmd.CombinedOutput()
	cancel()
	if testErr != nil {
		anomalies = append(anomalies, anomaly{
			key: "go-test-fail",
			msg: fmt.Sprintf("go test FAIL: %s", summarizeOutput(string(testOut), 500)),
		})
	}

	// 2. System health: disk space
	ctx2, cancel2 := context.WithTimeout(context.Background(), 10*time.Second)
	dfCmd := exec.CommandContext(ctx2, "df", "-h")
	dfOut, _ := dfCmd.CombinedOutput()
	cancel2()
	for _, line := range strings.Split(string(dfOut), "\n") {
		fields := strings.Fields(line)
		if len(fields) < 6 {
			continue
		}
		pct := strings.TrimSuffix(fields[4], "%")
		var usage int
		if _, err := fmt.Sscanf(pct, "%d", &usage); err != nil || usage < 90 {
			continue
		}
		mount := fields[len(fields)-1]
		if isPseudoMount(mount) {
			continue
		}
		anomalies = append(anomalies, anomaly{
			key: "disk:" + mount,
			msg: fmt.Sprintf("Disk usage high: %s at %s%%", mount, fields[4]),
		})
	}

	// 3. System health: memory (macOS vm_stat)
	ctx3, cancel3 := context.WithTimeout(context.Background(), 10*time.Second)
	vmCmd := exec.CommandContext(ctx3, "vm_stat")
	vmOut, vmErr := vmCmd.CombinedOutput()
	cancel3()
	if vmErr != nil {
		anomalies = append(anomalies, anomaly{key: "vm-stat-failed", msg: "vm_stat failed"})
	} else {
		availablePages := 0
		for _, line := range strings.Split(string(vmOut), "\n") {
			var match string
			switch {
			case strings.HasPrefix(line, "Pages free:"):
				match = "Pages free:"
			case strings.HasPrefix(line, "Pages inactive:"):
				match = "Pages inactive:"
			case strings.HasPrefix(line, "Pages speculative:"):
				match = "Pages speculative:"
			case strings.HasPrefix(line, "Pages purgeable:"):
				match = "Pages purgeable:"
			}
			if match == "" {
				continue
			}
			parts := strings.Fields(line)
			if len(parts) < 3 {
				continue
			}
			pagesStr := strings.TrimSuffix(parts[len(parts)-1], ".")
			var pages int
			fmt.Sscanf(pagesStr, "%d", &pages)
			availablePages += pages
		}
		if availablePages > 0 && availablePages < 32768 {
			anomalies = append(anomalies, anomaly{
				key: "low-memory",
				msg: fmt.Sprintf("Low available memory: %d pages (free+inactive+speculative+purgeable)", availablePages),
			})
		}
	}

	// 4. Hub agent status
	agents, err := g.db.ListAgents("")
	if err == nil {
		if a, ok := checkAgentImbalance(agents); ok {
			anomalies = append(anomalies, a)
		}
	}

	// Apply hysteresis + rate cap, then emit the survivors as one bundled report.
	now := time.Now()
	var allowed []string
	for _, a := range anomalies {
		if g.shouldFlag(a.key, now) {
			allowed = append(allowed, a.msg)
		}
	}
	if len(allowed) > 0 {
		report := "Monitor check anomalies:\n- " + strings.Join(allowed, "\n- ")
		g.db.InsertMessage(protocol.Message{
			FromAgent: systemFromAgent,
			ToAgent:   "rain",
			Type:      protocol.MsgResult,
			Content:   report,
		})
	}
}

// summarizeOutput truncates output to maxLen, keeping the tail.
func summarizeOutput(s string, maxLen int) string {
	s = strings.TrimSpace(s)
	if len(s) <= maxLen {
		return s
	}
	return "..." + s[len(s)-maxLen:]
}

// ProjectDir returns the bot-hq project directory for health checks.
func ProjectDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, "Projects", "bot-hq")
}
