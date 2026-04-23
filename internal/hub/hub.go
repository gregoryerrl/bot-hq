package hub

import (
	"encoding/json"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
)

// Hub is the central orchestrator that routes messages between agents.
// It holds the database, manages WebSocket client subscriptions, and
// dispatches messages via the DB's OnMessage callback.
type Hub struct {
	Config         Config
	DB             *DB
	wsClients      map[string]chan protocol.Message // agent_id -> WebSocket channel
	mu             sync.RWMutex
	lastPollID     int64
	stopPollCh     chan struct{}
	dispatchedIDs  map[int64]bool // tracks in-process dispatched message IDs
	dispatchedMu   sync.Mutex
	dispatchMu     sync.Map       // per-agent *sync.Mutex for tmux send serialization
}

// NewHub creates a new Hub, opens the database, and wires up the
// OnMessage callback for real-time dispatch.
func NewHub(cfg Config) (*Hub, error) {
	db, err := OpenDB(cfg.Hub.DBPath)
	if err != nil {
		return nil, fmt.Errorf("open db: %w", err)
	}

	h := &Hub{
		Config:        cfg,
		DB:            db,
		wsClients:     make(map[string]chan protocol.Message),
		stopPollCh:    make(chan struct{}),
		dispatchedIDs: make(map[int64]bool),
	}

	db.OnMessage(h.dispatch)

	return h, nil
}

// Start begins background goroutines including cross-process message polling.
func (h *Hub) Start() error {
	// Seed lastPollID to current max so we only dispatch new messages
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

	if msg.ToAgent == "" {
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

// getAgentMu returns a per-agent mutex for serializing tmux sends.
// Both dispatchToTmux and processMessageQueue use this to prevent
// interleaved input on the same tmux pane.
func (h *Hub) getAgentMu(agentID string) *sync.Mutex {
	mu, _ := h.dispatchMu.LoadOrStore(agentID, &sync.Mutex{})
	return mu.(*sync.Mutex)
}

// isAtPrompt checks if the tmux pane output indicates the agent is at a prompt.
// Empty output is treated as "at prompt" — intentional.
// CapturePane errors are handled by the caller before reaching here,
// so empty output means the pane exists but has no visible content (idle).
func (h *Hub) isAtPrompt(paneOutput string) bool {
	lines := strings.Split(strings.TrimSpace(paneOutput), "\n")
	if len(lines) == 0 {
		return false
	}
	lastLine := strings.TrimSpace(lines[len(lines)-1])
	return lastLine == "" || strings.HasSuffix(lastLine, "❯") ||
		strings.HasSuffix(lastLine, ">") || strings.HasSuffix(lastLine, "$")
}

// dispatchToTmux sends a message to a coder agent's tmux session.
// It looks up the agent's tmux target, checks if Claude is at a prompt,
// and sends the message text as keystrokes. If the agent is busy, the
// message is queued for retry delivery.
func (h *Hub) dispatchToTmux(msg protocol.Message) {
	agent, err := h.DB.GetAgent(msg.ToAgent)
	if err != nil {
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
		return
	}

	// Lock per-agent mutex to prevent interleaved tmux input
	// from concurrent dispatchToTmux and processMessageQueue calls.
	mu := h.getAgentMu(msg.ToAgent)
	mu.Lock()
	defer mu.Unlock()

	output, err := tmuxpkg.CapturePane(tmuxTarget, 5)
	if err != nil {
		return
	}

	text := h.FormatTmuxMessage(tmuxTarget, msg)

	if h.isAtPrompt(output) {
		tmuxpkg.SendKeys(tmuxTarget, text, true)
		// Drain any previously queued messages for this agent
		// Note: drainQueue is called under the same lock — do not re-lock inside it.
		h.drainQueue(tmuxTarget, msg.ToAgent)
	} else {
		// Agent is busy — queue for retry.
		// Delivery is at-least-once: if we crash between SendKeys and
		// UpdateQueueStatus, the message stays "pending" and may be re-sent.
		if err := h.DB.EnqueueMessage(msg.ID, msg.ToAgent, tmuxTarget, text); err != nil {
			log.Printf("[dispatch] Failed to enqueue message %d for %s: %v", msg.ID, msg.ToAgent, err)
			return
		}
		log.Printf("[dispatch] Agent %s busy, queued message %d", msg.ToAgent, msg.ID)
	}
}

// drainQueue delivers any previously queued messages to an agent that is now at a prompt.
// Called from dispatchToTmux which already holds the per-agent mutex — do NOT lock here.
func (h *Hub) drainQueue(tmuxTarget, agentID string) {
	pending, err := h.DB.GetPendingMessagesForAgent(agentID)
	if err != nil {
		return
	}
	for _, qm := range pending {
		// Re-check prompt before each send
		output, err := tmuxpkg.CapturePane(tmuxTarget, 5)
		if err != nil {
			break
		}
		if !h.isAtPrompt(output) {
			break
		}
		if err := tmuxpkg.SendKeys(tmuxTarget, qm.FormattedText, true); err != nil {
			log.Printf("[queue] Failed to send queued message %d: %v", qm.ID, err)
			break
		}
		// SendKeys already sleeps 500ms for bracketed paste — no extra delay needed.
		h.DB.UpdateQueueStatus(qm.ID, "delivered", qm.Attempts+1)
		log.Printf("[queue] Delivered queued message %d to %s", qm.MessageID, agentID)
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

			// Group by target agent
			byAgent := make(map[string][]QueuedMessage)
			for _, qm := range pending {
				byAgent[qm.TargetAgent] = append(byAgent[qm.TargetAgent], qm)
			}

			for agentID, messages := range byAgent {
				tmuxTarget := messages[0].TmuxTarget

				// Lock per-agent mutex to prevent interleaved sends
				// with concurrent dispatchToTmux calls.
				mu := h.getAgentMu(agentID)
				mu.Lock()

				output, err := tmuxpkg.CapturePane(tmuxTarget, 5)
				if err != nil {
					for _, qm := range messages {
						if qm.Attempts >= qm.MaxAttempts {
							h.DB.UpdateQueueStatus(qm.ID, "failed", qm.Attempts+1)
							log.Printf("[queue] Message %d to %s failed after %d attempts", qm.MessageID, agentID, qm.Attempts)
						} else {
							h.DB.UpdateQueueStatus(qm.ID, "pending", qm.Attempts+1)
						}
					}
					mu.Unlock()
					continue
				}

				if !h.isAtPrompt(output) {
					for _, qm := range messages {
						if qm.Attempts >= qm.MaxAttempts {
							h.DB.UpdateQueueStatus(qm.ID, "failed", qm.Attempts+1)
							log.Printf("[queue] Message %d to %s failed after %d attempts", qm.MessageID, agentID, qm.Attempts)
						} else {
							h.DB.UpdateQueueStatus(qm.ID, "pending", qm.Attempts+1)
						}
					}
					mu.Unlock()
					continue
				}

				// Agent is at prompt — deliver queued messages in order
				for _, qm := range messages {
					output, err = tmuxpkg.CapturePane(tmuxTarget, 5)
					if err != nil || !h.isAtPrompt(output) {
						break
					}
					if err := tmuxpkg.SendKeys(tmuxTarget, qm.FormattedText, true); err != nil {
						log.Printf("[queue] Failed to send queued message %d: %v", qm.ID, err)
						break
					}
					// SendKeys already sleeps 500ms for bracketed paste — no extra delay needed.
					h.DB.UpdateQueueStatus(qm.ID, "delivered", qm.Attempts+1)
					log.Printf("[queue] Delivered queued message %d to %s", qm.MessageID, agentID)
				}

				mu.Unlock()
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
