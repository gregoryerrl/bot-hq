package live

import (
	"embed"
	"encoding/json"
	"fmt"
	"io/fs"
	"log"
	"net/http"
	"sync"

	"github.com/gorilla/websocket"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

//go:embed web/*
var webFiles embed.FS

var upgrader = websocket.Upgrader{
	CheckOrigin: func(r *http.Request) bool { return true },
}

// Server serves the Bot-HQ Live web UI and handles WebSocket connections
// between the browser and the hub.
type Server struct {
	hub  *hub.Hub
	port int
	mu   sync.Mutex
}

// NewServer creates a new Live web server.
func NewServer(h *hub.Hub, port int) *Server {
	return &Server{hub: h, port: port}
}

// Start begins serving the embedded web UI and WebSocket endpoint.
func (s *Server) Start() error {
	staticFS, err := fs.Sub(webFiles, "web")
	if err != nil {
		return fmt.Errorf("embed sub: %w", err)
	}

	mux := http.NewServeMux()
	mux.Handle("/", http.FileServer(http.FS(staticFS)))
	mux.HandleFunc("/ws", s.handleWebSocket)

	addr := fmt.Sprintf(":%d", s.port)
	log.Printf("Bot-HQ Live serving at http://localhost%s", addr)

	go func() {
		if err := http.ListenAndServe(addr, mux); err != nil {
			log.Printf("Live server error: %v", err)
		}
	}()

	return nil
}

func (s *Server) handleWebSocket(w http.ResponseWriter, r *http.Request) {
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		log.Printf("ws upgrade error: %v", err)
		return
	}

	const agentID = "live"
	log.Printf("Live WebSocket client connected")

	// Register with the hub
	hubCh := s.hub.RegisterWSClient(agentID)

	// Channel to signal when the read loop exits
	done := make(chan struct{})

	// Write hub messages to the WebSocket client
	go func() {
		defer conn.Close()
		for {
			select {
			case msg, ok := <-hubCh:
				if !ok {
					return
				}
				data, err := json.Marshal(msg)
				if err != nil {
					log.Printf("ws marshal error: %v", err)
					continue
				}
				if err := conn.WriteMessage(websocket.TextMessage, data); err != nil {
					log.Printf("ws write error: %v", err)
					return
				}
			case <-done:
				return
			}
		}
	}()

	// Read messages from the WebSocket client
	defer func() {
		close(done)
		s.hub.UnregisterWSClient(agentID)
		conn.Close()
		log.Printf("Live WebSocket client disconnected")
	}()

	for {
		msgType, data, err := conn.ReadMessage()
		if err != nil {
			if websocket.IsUnexpectedCloseError(err, websocket.CloseGoingAway, websocket.CloseNormalClosure) {
				log.Printf("ws read error: %v", err)
			}
			break
		}

		if msgType == websocket.TextMessage {
			var msg protocol.Message
			if err := json.Unmarshal(data, &msg); err != nil {
				log.Printf("ws unmarshal error: %v", err)
				continue
			}
			msg.FromAgent = agentID
			log.Printf("Live WS received: type=%s content=%s", msg.Type, msg.Content)

			// Store in DB if it has a session
			if msg.SessionID != "" {
				if _, err := s.hub.DB.InsertMessage(msg); err != nil {
					log.Printf("ws db error: %v", err)
				}
			}
		} else if msgType == websocket.BinaryMessage {
			// Binary messages are audio chunks — will be forwarded to Gemini in Task 6.2
			log.Printf("Live WS received binary: %d bytes", len(data))
		}
	}
}
