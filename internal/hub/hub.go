package hub

import (
	"fmt"
	"sync"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// Hub is the central orchestrator that routes messages between agents.
// It holds the database, manages WebSocket client subscriptions, and
// dispatches messages via the DB's OnMessage callback.
type Hub struct {
	Config    Config
	DB        *DB
	wsClients map[string]chan protocol.Message // agent_id -> WebSocket channel
	mu        sync.RWMutex
}

// NewHub creates a new Hub, opens the database, and wires up the
// OnMessage callback for real-time dispatch.
func NewHub(cfg Config) (*Hub, error) {
	db, err := OpenDB(cfg.Hub.DBPath)
	if err != nil {
		return nil, fmt.Errorf("open db: %w", err)
	}

	h := &Hub{
		Config:    cfg,
		DB:        db,
		wsClients: make(map[string]chan protocol.Message),
	}

	db.OnMessage(h.dispatch)

	return h, nil
}

// Start begins any background goroutines the hub needs.
// Currently a no-op; the dispatch callback is registered in NewHub.
func (h *Hub) Start() error {
	return nil
}

// Stop closes the database and cleans up resources.
func (h *Hub) Stop() error {
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
	h.mu.RLock()
	defer h.mu.RUnlock()

	if msg.ToAgent == "" {
		// Broadcast to all WS clients
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

	// If not a WS client, it might be a coder agent (tmux target).
	// Tmux integration will be added in Task 4.2.
}
