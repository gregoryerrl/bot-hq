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

// browserMessage is the JSON structure sent from the browser client.
type browserMessage struct {
	Type string `json:"type"`
	Data string `json:"data"`
}

func (s *Server) handleWebSocket(w http.ResponseWriter, r *http.Request) {
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		log.Printf("ws upgrade error: %v", err)
		return
	}

	const agentID = "live"
	log.Printf("Live WebSocket client connected")

	// Register live agent in the hub DB
	s.hub.DB.RegisterAgent(protocol.Agent{
		ID:     agentID,
		Name:   "Live",
		Type:   protocol.AgentVoice,
		Status: protocol.StatusOnline,
	})

	// Register with the hub
	hubCh := s.hub.RegisterWSClient(agentID)

	// Channel to signal when the read loop exits
	done := make(chan struct{})

	// Mutex for writing to the browser WebSocket (shared between goroutines)
	var wsMu sync.Mutex

	// Helper to send JSON to the browser
	writeBrowser := func(v interface{}) {
		wsMu.Lock()
		defer wsMu.Unlock()
		data, err := json.Marshal(v)
		if err != nil {
			log.Printf("ws marshal error: %v", err)
			return
		}
		if err := conn.WriteMessage(websocket.TextMessage, data); err != nil {
			log.Printf("ws write error: %v", err)
		}
	}

	// Connect to Gemini if API key is configured
	var gemini *GeminiProxy
	apiKey := s.hub.Config.Live.GeminiAPIKey
	if apiKey != "" {
		voice := s.hub.Config.Live.Voice
		gemini = NewGeminiProxy(apiKey, voice)
		if err := gemini.Connect(""); err != nil {
			log.Printf("Gemini connect error: %v", err)
			writeBrowser(map[string]string{
				"type":  "error",
				"error": fmt.Sprintf("Gemini connection failed: %v", err),
			})
			gemini = nil
		} else {
			// Send connected confirmation to browser
			writeBrowser(map[string]string{"type": "connected"})

			// Start Gemini read loop — forwards Gemini responses to the browser
			go s.geminiReadLoop(gemini, writeBrowser, done)
		}
	} else {
		log.Printf("No Gemini API key configured — voice proxy disabled")
		writeBrowser(map[string]string{
			"type":  "error",
			"error": "No Gemini API key configured — voice disabled",
		})
	}

	// Write hub messages to the WebSocket client
	go func() {
		for {
			select {
			case msg, ok := <-hubCh:
				if !ok {
					return
				}
				writeBrowser(msg)
			case <-done:
				return
			}
		}
	}()

	// Read messages from the WebSocket client
	defer func() {
		close(done)
		s.hub.UnregisterWSClient(agentID)
		s.hub.DB.UpdateAgentStatus(agentID, protocol.StatusOffline)
		if gemini != nil {
			gemini.Close()
		}
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
			// Try to parse as browser message (audio/text for Gemini)
			var bMsg browserMessage
			if err := json.Unmarshal(data, &bMsg); err != nil {
				log.Printf("ws unmarshal error: %v", err)
				continue
			}

			switch bMsg.Type {
			case "audio":
				if gemini != nil {
					if err := gemini.SendAudio(bMsg.Data); err != nil {
						log.Printf("gemini send audio error: %v", err)
					}
				}
			case "text":
				if gemini != nil {
					if err := gemini.SendText(bMsg.Data); err != nil {
						log.Printf("gemini send text error: %v", err)
					}
				}
			default:
				// Fall back to hub protocol message
				var msg protocol.Message
				if err := json.Unmarshal(data, &msg); err != nil {
					log.Printf("ws unmarshal hub msg error: %v", err)
					continue
				}
				msg.FromAgent = agentID
				log.Printf("Live WS received: type=%s content=%s", msg.Type, msg.Content)

				if msg.SessionID != "" {
					if _, err := s.hub.DB.InsertMessage(msg); err != nil {
						log.Printf("ws db error: %v", err)
					}
				}
			}
		}
	}
}

// geminiReadLoop reads messages from Gemini and forwards them to the browser.
func (s *Server) geminiReadLoop(gemini *GeminiProxy, writeBrowser func(interface{}), done <-chan struct{}) {
	for {
		select {
		case <-done:
			return
		default:
		}

		msg, err := gemini.ReadMessage()
		if err != nil {
			select {
			case <-done:
				return
			default:
				log.Printf("gemini read error: %v", err)
				return
			}
		}

		sc, ok := msg["serverContent"]
		if !ok {
			continue
		}
		serverContent, ok := sc.(map[string]interface{})
		if !ok {
			continue
		}

		// Handle modelTurn parts (audio and text)
		if mt, ok := serverContent["modelTurn"].(map[string]interface{}); ok {
			if parts, ok := mt["parts"].([]interface{}); ok {
				for _, p := range parts {
					part, ok := p.(map[string]interface{})
					if !ok {
						continue
					}
					// Audio data
					if inlineData, ok := part["inlineData"].(map[string]interface{}); ok {
						if audioData, ok := inlineData["data"].(string); ok {
							writeBrowser(map[string]string{
								"type": "audio",
								"data": audioData,
							})
						}
					}
					// Text data
					if text, ok := part["text"].(string); ok && text != "" {
						writeBrowser(map[string]interface{}{
							"type": "transcript",
							"role": "assistant",
							"text": text,
						})
					}
				}
			}
		}

		// Handle input transcription (user speech-to-text)
		if it, ok := serverContent["inputTranscription"].(map[string]interface{}); ok {
			if text, ok := it["text"].(string); ok && text != "" {
				writeBrowser(map[string]interface{}{
					"type": "transcript",
					"role": "user",
					"text": text,
				})
			}
		}

		// Handle output transcription (assistant speech-to-text)
		if ot, ok := serverContent["outputTranscription"].(map[string]interface{}); ok {
			if text, ok := ot["text"].(string); ok && text != "" {
				writeBrowser(map[string]interface{}{
					"type": "transcript",
					"role": "assistant",
					"text": text,
				})
			}
		}

		// Handle turn complete
		if tc, ok := serverContent["turnComplete"].(bool); ok && tc {
			writeBrowser(map[string]string{"type": "turn_complete"})
		}

		// Handle interrupted
		if interrupted, ok := serverContent["interrupted"].(bool); ok && interrupted {
			writeBrowser(map[string]string{"type": "interrupted"})
		}
	}
}
