package hub

import (
	"encoding/json"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/gregoryerrl/bot-hq/internal/tmuxsink"
)

// Hub is the central orchestrator that routes messages between agents.
// It holds the database, manages WebSocket client subscriptions, and
// dispatches messages via the DB's OnMessage callback.
type Hub struct {
	Config         Config
	DB             *DB
	// RebuildGen is the rebuild generation assigned at this hub's startup.
	// Bumped once per NewHub call. Used to flag pre-rebuild stale agent
	// registrations leaking into post-rebuild state.
	RebuildGen     int64
	wsClients      map[string]chan protocol.Message // agent_id -> WebSocket channel
	mu             sync.RWMutex
	lastPollID     int64
	stopPollCh     chan struct{}
	dispatchedIDs  map[int64]bool // tracks in-process dispatched message IDs
	dispatchedMu   sync.Mutex

	// Phase I W2 I-7 Layer-2: per-agent tmuxsink.Sink registry. Sinks own
	// their own per-target sync.Mutex (replacing the old dispatchMu sync.Map);
	// sinkMu protects only the registry map for lazy get-or-create. All
	// pane-delivery semantics (capture → IsReady → send-or-enqueue → drain)
	// live in tmuxsink.Sink, shared with rain/brian self-paste paths.
	sinks   map[string]*tmuxsink.Sink
	sinkMu  sync.RWMutex
}

// NewHub creates a new Hub, opens the database, and wires up the
// OnMessage callback for real-time dispatch.
func NewHub(cfg Config) (*Hub, error) {
	db, err := OpenDB(cfg.Hub.DBPath)
	if err != nil {
		return nil, fmt.Errorf("open db: %w", err)
	}

	gen, err := db.IncrementRebuildGen()
	if err != nil {
		db.Close()
		return nil, fmt.Errorf("increment rebuild gen: %w", err)
	}

	h := &Hub{
		Config:        cfg,
		DB:            db,
		RebuildGen:    gen,
		wsClients:     make(map[string]chan protocol.Message),
		stopPollCh:    make(chan struct{}),
		dispatchedIDs: make(map[int64]bool),
		sinks:         make(map[string]*tmuxsink.Sink),
	}

	db.OnMessage(h.dispatch)

	return h, nil
}

// Start begins background goroutines including cross-process message polling.
func (h *Hub) Start() error {
	// Intentionally pre-seed lastPollID to the current max ID. The cross-process
	// poller dispatches messages it sees for the first time — tail-replaying
	// pre-restart messages here would re-fire Discord forwards and other
	// dispatch side effects on every hub restart. This is NOT the same pattern
	// as brian/rain agent inits (which were buggy and fixed in commit fb431f3);
	// agents need backlog replay for context recovery, the hub poller does not.
	// Do not apply ReadMessages tail-mode (commit a96ebcc) to this site.
	if msgs, err := h.DB.GetRecentMessages(1); err == nil && len(msgs) > 0 {
		h.lastPollID = msgs[len(msgs)-1].ID
	}
	go h.pollExternalMessages()
	go h.processMessageQueue()
	return nil
}

// Stop closes the database and cleans up resources.
func (h *Hub) Stop() error {
	close(h.stopPollCh)

	h.mu.Lock()
	for id, ch := range h.wsClients {
		close(ch)
		delete(h.wsClients, id)
	}
	h.mu.Unlock()

	if h.DB != nil {
		return h.DB.Close()
	}
	return nil
}

// RegisterWSClient registers a WebSocket client for message delivery
// and returns a channel on which dispatched messages will be sent.
func (h *Hub) RegisterWSClient(agentID string) chan protocol.Message {
	h.mu.Lock()
	defer h.mu.Unlock()

	if old, ok := h.wsClients[agentID]; ok {
		close(old)
	}
	ch := make(chan protocol.Message, 64)
	h.wsClients[agentID] = ch
	return ch
}

// UnregisterWSClient removes a WebSocket client and closes its channel.
func (h *Hub) UnregisterWSClient(agentID string) {
	h.mu.Lock()
	defer h.mu.Unlock()

	if ch, ok := h.wsClients[agentID]; ok {
		close(ch)
		delete(h.wsClients, agentID)
	}
}

// FormatTmuxMessage formats a message for display in a tmux pane.
// Returns "[from_agent] content" — the string that would be typed into tmux.
func (h *Hub) FormatTmuxMessage(targetSession string, msg protocol.Message) string {
	return fmt.Sprintf("[%s] %s", msg.FromAgent, msg.Content)
}

// dispatch is the OnMessage callback. It routes messages to targets:
//   - If ToAgent is a registered WS client, send to its channel.
//   - If ToAgent is empty, broadcast to all WS clients.
//   - If ToAgent is a coder agent (not a WS client), it will be handled
//     by tmux integration (Task 4.2). For now we skip silently.
func (h *Hub) dispatch(msg protocol.Message) {
	// Mark as dispatched in-process so poller skips it
	if msg.ID > 0 {
		h.dispatchedMu.Lock()
		h.dispatchedIDs[msg.ID] = true
		h.dispatchedMu.Unlock()
	}

	h.mu.RLock()
	defer h.mu.RUnlock()

	if msg.ToAgent == "" || msg.Type == protocol.MsgFlag {
		// Broadcast to all WS clients (flags always broadcast for Discord/UI visibility)
		for _, ch := range h.wsClients {
			select {
			case ch <- msg:
			default:
				// Channel full, skip to avoid blocking
			}
		}
		return
	}

	// Targeted delivery
	if ch, ok := h.wsClients[msg.ToAgent]; ok {
		select {
		case ch <- msg:
		default:
			// Channel full, skip to avoid blocking
		}
		return
	}

	// Try to dispatch to coder agent via tmux
	go h.dispatchToTmux(msg)
}

// pollExternalMessages periodically checks the DB for messages inserted
// by other processes (e.g. MCP) and dispatches them to WS clients.
func (h *Hub) pollExternalMessages() {
	ticker := time.NewTicker(500 * time.Millisecond)
	defer ticker.Stop()

	for {
		select {
		case <-h.stopPollCh:
			return
		case <-ticker.C:
			msgs, err := h.DB.ReadMessages("", h.lastPollID, 50)
			if err != nil || len(msgs) == 0 {
				continue
			}
			for _, msg := range msgs {
				if msg.ID > h.lastPollID {
					h.lastPollID = msg.ID
				}
				// Skip messages already dispatched in-process
				h.dispatchedMu.Lock()
				if h.dispatchedIDs[msg.ID] {
					delete(h.dispatchedIDs, msg.ID)
					h.dispatchedMu.Unlock()
					continue
				}
				h.dispatchedMu.Unlock()
				h.dispatchToClients(msg)
			}

			// Prune stale entries — IDs at or below lastPollID will never
			// appear in a future poll, so keeping them leaks memory.
			h.dispatchedMu.Lock()
			for id := range h.dispatchedIDs {
				if id <= h.lastPollID {
					delete(h.dispatchedIDs, id)
				}
			}
			h.dispatchedMu.Unlock()
		}
	}
}

// dispatchToClients routes a message to WS clients without triggering
// another DB insert (used by the poller to avoid loops).
func (h *Hub) dispatchToClients(msg protocol.Message) {
	h.mu.RLock()
	defer h.mu.RUnlock()

	if msg.ToAgent == "" || msg.Type == protocol.MsgFlag {
		for _, ch := range h.wsClients {
			select {
			case ch <- msg:
			default:
			}
		}
		return
	}

	if ch, ok := h.wsClients[msg.ToAgent]; ok {
		select {
		case ch <- msg:
		default:
		}
	}
}

// getSink returns the tmuxsink.Sink for the given agent, lazily creating
// it on first access. Sinks own per-target sync.Mutex internally; sinkMu
// protects only the map's get-or-create. Phase I W2 I-7 Layer-2 — replaces
// the old getAgentMu / dispatchMu sync.Map.
func (h *Hub) getSink(agentID, target string) *tmuxsink.Sink {
	h.sinkMu.RLock()
	if s, ok := h.sinks[agentID]; ok {
		h.sinkMu.RUnlock()
		return s
	}
	h.sinkMu.RUnlock()

	h.sinkMu.Lock()
	defer h.sinkMu.Unlock()
	// Double-check after acquiring write lock.
	if s, ok := h.sinks[agentID]; ok {
		return s
	}
	s := tmuxsink.New(sinkStore{db: h.DB}, agentID, target)
	h.sinks[agentID] = s
	return s
}

// NewTmuxSinkStore returns a tmuxsink.Store adapter wrapping the given DB.
// External packages (rain, brian) call this to construct a Sink without
// importing hub's private sinkStore type. Phase I W2 Layer-2 (c) — keeps
// the adapter struct unexported while exposing the canonical factory.
func NewTmuxSinkStore(db *DB) tmuxsink.Store {
	return sinkStore{db: db}
}

// sinkStore is the thin adapter wrapping *DB to satisfy tmuxsink.Store.
// Lives in package hub so tmuxsink stays free of hub imports (cycle break).
type sinkStore struct {
	db *DB
}

func (s sinkStore) EnqueueMessage(messageID int64, targetAgent, tmuxTarget, formattedText string) error {
	return s.db.EnqueueMessage(messageID, targetAgent, tmuxTarget, formattedText)
}

func (s sinkStore) PendingForAgent(agentID string) ([]tmuxsink.QueuedItem, error) {
	rows, err := s.db.GetPendingMessagesForAgent(agentID)
	if err != nil {
		return nil, err
	}
	out := make([]tmuxsink.QueuedItem, len(rows))
	for i, r := range rows {
		out[i] = tmuxsink.QueuedItem{
			ID:            r.ID,
			MessageID:     r.MessageID,
			FormattedText: r.FormattedText,
			Attempts:      r.Attempts,
		}
	}
	return out, nil
}

func (s sinkStore) UpdateQueueStatus(id int64, status string, attempts int) error {
	return s.db.UpdateQueueStatus(id, status, attempts)
}

// dispatchDecisionEnvVar gates the dispatchToTmux JSONL instrumentation.
// Empty/unset = no-op (production default). Set to any non-empty value to
// enable per-call decision logging at ~/.bot-hq/diag/dispatch-decisions.jsonl
// (or $BOT_HQ_HOME/diag/... when overridden).
//
// Slice-5 H-22-bis instrumentation: writes one JSON record per
// dispatchToTmux invocation capturing isReady decision + last-line bytes
// + action taken. Cross-correlated with retry-exhaust events to pin
// dispatch-classification failure modes.
const dispatchDecisionEnvVar = "BOT_HQ_DIAG_DISPATCH"

// dispatchDecisionRecord is the JSONL row shape for dispatch-decisions.jsonl.
// Field names are stable wire format — change only with a corresponding
// reader update in any analysis tooling.
//
// Decision = ready (the isReady return). Outcome = what happened next
// (the actual action result, including SendKeys / Enqueue errors). Decision
// + outcome is the diagnostic unit — a clean "ready=true / outcome=sent"
// entry that still produces a retry-exhaust event implicates the
// paste-buffer-interleave hypothesis; a "send_keys_err" with a non-empty
// err disambiguates a transport failure.
//
// Field rename history: "at_prompt" → "ready" in hub-hotfix-isReady-inversion
// (2026-04-28). Old logs use "at_prompt"; new logs use "ready".
//
// TODO: log rotation when retention > 7d. Slice-5 diagnostic window is
// days-to-weeks so v1 is no-rotation; mark for follow-up if instrumentation
// stays past slice-5 close-out.
type dispatchDecisionRecord struct {
	TS         string `json:"ts"`
	MsgID      int64  `json:"msg_id"`
	ToAgent    string `json:"to_agent"`
	TmuxTarget string `json:"tmux_target,omitempty"`
	Ready      bool   `json:"ready"`
	LastLine   string `json:"last_line,omitempty"`
	Outcome    string `json:"outcome"`
	Err        string `json:"err,omitempty"`
}

// diagDir returns the directory for diagnostic output files. Honors
// BOT_HQ_HOME for test isolation, mirroring the convention used by
// internal/gemma sentinelsDir.
func diagDir() (string, error) {
	if h := os.Getenv("BOT_HQ_HOME"); h != "" {
		return filepath.Join(h, "diag"), nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".bot-hq", "diag"), nil
}

// recordDispatchDecision appends a dispatchToTmux decision record to the
// diag JSONL file when the env gate is set. Best-effort: errors swallowed
// because diagnostic instrumentation must not crash the dispatch path.
func recordDispatchDecision(rec dispatchDecisionRecord) {
	if os.Getenv(dispatchDecisionEnvVar) == "" {
		return
	}
	dir, err := diagDir()
	if err != nil {
		return
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return
	}
	rec.TS = time.Now().UTC().Format(time.RFC3339Nano)
	data, err := json.Marshal(rec)
	if err != nil {
		return
	}
	f, err := os.OpenFile(filepath.Join(dir, "dispatch-decisions.jsonl"), os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return
	}
	defer f.Close()
	f.Write(data)
	f.Write([]byte("\n"))
}

// retryExhaustFromAgent is the synthetic from_agent used by the bridge
// emit at retry-exhaust. Must be a non-registered agent ID so Emma's
// source-filter (which suppresses queueFailPattern hits from registered
// agents to defeat prose false-positives) does not also suppress the
// real bridge emit.
const retryExhaustFromAgent = "hub"

// emitRetryExhaustAlert inserts a synthetic protocol.Message that mirrors
// the hub's retry-exhaust log line, so Emma's sentinel pipeline (which
// reads protocol.Message content, not stdout) can detect real exhaust
// events. Without this bridge the queueFailPattern sentinel is
// architecturally disconnected from its emit source — see the slice-5
// H-22-bis diagnosis. Best-effort: insert errors are swallowed so a DB
// hiccup does not crash the queue worker.
func (h *Hub) emitRetryExhaustAlert(messageID int64, targetAgent string, attempts int) {
	content := fmt.Sprintf("[queue] Message %d to %s failed after %d attempts", messageID, targetAgent, attempts)
	h.DB.InsertMessage(protocol.Message{
		FromAgent: retryExhaustFromAgent,
		Type:      protocol.MsgUpdate,
		Content:   content,
	})
}

// IsReady, busy-marker glyphs/prefixes, and lastLineSummary moved to
// internal/tmuxsink package in Phase I W2 I-7 Layer-2 (commit 407113d).
// Hub callers use tmuxsink.IsReady and tmuxsink.LastLineSummary directly,
// or route through Sink.Deliver which encapsulates capture + IsReady +
// send-or-enqueue.

// dispatchToTmux sends a message to a coder agent's tmux session.
// It looks up the agent's tmux target, then routes through tmuxsink.Sink
// which handles isReady-check + send-or-enqueue + drain-on-ready. The
// returned Decision is recorded as a JSONL diag record.
//
// Phase I W2 I-7 Layer-2 refactor: prior implementation inlined
// CapturePane + isReady + SendKeys + drainQueue under a per-agent Mutex.
// Now delegates to Sink (which owns its per-target Mutex internally and
// shares the same delivery path with rain/brian self-paste calls — single
// source of truth for at-least-once tmux delivery semantics).
func (h *Hub) dispatchToTmux(msg protocol.Message) {
	agent, err := h.DB.GetAgent(msg.ToAgent)
	if err != nil {
		recordDispatchDecision(dispatchDecisionRecord{
			MsgID:   msg.ID,
			ToAgent: msg.ToAgent,
			Outcome: "no_target",
			Err:     err.Error(),
		})
		return
	}

	var meta struct {
		TmuxTarget string `json:"tmux_target"`
	}
	if agent.Meta != "" {
		json.Unmarshal([]byte(agent.Meta), &meta)
	}

	tmuxTarget := meta.TmuxTarget
	if tmuxTarget == "" {
		sessions, err := h.DB.ListClaudeSessions("")
		if err == nil {
			for _, s := range sessions {
				if s.Status == "running" && s.ID == msg.ToAgent {
					tmuxTarget = s.TmuxTarget
					break
				}
			}
		}
	}

	if tmuxTarget == "" {
		recordDispatchDecision(dispatchDecisionRecord{
			MsgID:   msg.ID,
			ToAgent: msg.ToAgent,
			Outcome: "no_target",
		})
		return
	}

	text := h.FormatTmuxMessage(tmuxTarget, msg)
	sink := h.getSink(msg.ToAgent, tmuxTarget)
	dec := sink.Deliver(msg.ID, text)

	rec := dispatchDecisionRecord{
		MsgID:      msg.ID,
		ToAgent:    msg.ToAgent,
		TmuxTarget: tmuxTarget,
		Ready:      dec.Ready,
		LastLine:   dec.LastLine,
		Outcome:    dec.Outcome,
	}
	if dec.Err != nil {
		rec.Err = dec.Err.Error()
	}
	recordDispatchDecision(rec)

	switch dec.Outcome {
	case "enqueue_err":
		log.Printf("[dispatch] Failed to enqueue message %d for %s: %v", msg.ID, msg.ToAgent, dec.Err)
	case "queued":
		log.Printf("[dispatch] Agent %s busy, queued message %d", msg.ToAgent, msg.ID)
	}
}

// processMessageQueue periodically checks for pending queued messages
// and attempts to deliver them when the target agent becomes idle.
// Retry interval: 3s. Max attempts: 30 (configurable per message, default 30 = ~90s).
// Delivery semantics: at-least-once. If a crash occurs between SendKeys and
// UpdateQueueStatus("delivered"), the message stays "pending" and may be re-sent.
func (h *Hub) processMessageQueue() {
	ticker := time.NewTicker(3 * time.Second)
	defer ticker.Stop()

	var cleanupCounter int

	for {
		select {
		case <-h.stopPollCh:
			return
		case <-ticker.C:
			pending, err := h.DB.GetPendingMessages()
			if err != nil {
				continue
			}
			if len(pending) == 0 {
				continue
			}

			// Group by target agent.
			byAgent := make(map[string][]QueuedMessage)
			for _, qm := range pending {
				byAgent[qm.TargetAgent] = append(byAgent[qm.TargetAgent], qm)
			}

			for agentID, messages := range byAgent {
				tmuxTarget := messages[0].TmuxTarget

				// Probe pane state independently of Sink's lock so we can
				// preserve attempt-tracking semantics for busy panes:
				// every tick where the pane is busy = +1 attempt against
				// every still-pending message; exhaust-at-max emits the
				// retry-exhaust alert. Sink.Drain handles the ready-pane
				// path (delivering each row in order, stopping on busy).
				output, err := tmuxpkg.CapturePane(tmuxTarget, 5)
				paneAvailable := err == nil && tmuxsink.IsReady(output)

				if !paneAvailable {
					for _, qm := range messages {
						if qm.Attempts >= qm.MaxAttempts {
							h.DB.UpdateQueueStatus(qm.ID, "failed", qm.Attempts+1)
							log.Printf("[queue] Message %d to %s failed after %d attempts", qm.MessageID, agentID, qm.Attempts)
							h.emitRetryExhaustAlert(qm.MessageID, agentID, qm.Attempts)
						} else {
							h.DB.UpdateQueueStatus(qm.ID, "pending", qm.Attempts+1)
						}
					}
					continue
				}

				// Pane ready — delegate to Sink. Drain delivers each row
				// in order, stopping at first busy/error. Successfully
				// delivered rows are marked "delivered" by Sink; any
				// remaining rows survive to the next tick. Errors are
				// returned but non-fatal — same pending rows retry.
				sink := h.getSink(agentID, tmuxTarget)
				if delivered, err := sink.Drain(); err != nil {
					log.Printf("[queue] sink.Drain error for %s after %d delivered: %v", agentID, delivered, err)
				}
			}

			// Cleanup old delivered messages every ~100 ticks (~5 min at 3s interval)
			cleanupCounter++
			if cleanupCounter >= 100 {
				h.DB.CleanDeliveredMessages(1 * time.Hour)
				cleanupCounter = 0
			}
		}
	}
}
