package live

import (
	"embed"
	"encoding/json"
	"fmt"
	"io/fs"
	"log"
	"net/http"
	"strings"
	"sync"
	"time"

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
	log.Printf("Clive serving at http://localhost%s", addr)

	go func() {
		if err := http.ListenAndServe(addr, mux); err != nil {
			log.Printf("Clive server error: %v", err)
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
	log.Printf("Clive WebSocket client connected")

	// Register live agent in the hub DB
	s.hub.DB.RegisterAgent(protocol.Agent{
		ID:     agentID,
		Name:   "Clive",
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

	// Query API key from DB at connection time (user may have set it via Settings tab)
	var gemini *GeminiProxy
	apiKey := s.hub.DB.GetSetting("live.gemini_api_key", "")
	if apiKey == "" {
		// Fall back to config file / env var
		apiKey = s.hub.Config.Live.GeminiAPIKey
	}
	if apiKey != "" {
		voice := s.hub.DB.GetSetting("live.voice", s.hub.Config.Live.Voice)
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
			go s.geminiReadLoop(gemini, writeBrowser, done, &gemini)
		}
	} else {
		log.Printf("No Gemini API key configured — voice proxy disabled")
		writeBrowser(map[string]string{
			"type":  "error",
			"error": "No Gemini API key configured — set it in Settings tab, then reconnect",
		})
	}

	// Write hub messages to the WebSocket client and forward agent replies to Gemini.
	// Messages addressed to "live" are batched over a short window so Brian's
	// rapid-fire responses don't interrupt Clive mid-sentence.
	go func() {
		var pendingMessages []protocol.Message
		debounceTimer := time.NewTimer(0)
		if !debounceTimer.Stop() {
			<-debounceTimer.C
		}
		debounceActive := false

		flushToGemini := func() {
			debounceActive = false
			if len(pendingMessages) == 0 || gemini == nil {
				pendingMessages = nil
				return
			}
			// Combine all pending messages into one context injection
			var parts []string
			for _, m := range pendingMessages {
				parts = append(parts, fmt.Sprintf("[%s]: %s", m.FromAgent, m.Content))
			}
			combined := strings.Join(parts, "\n\n")
			contextText := fmt.Sprintf("%s\n\nRelay this to the user conversationally — summarize if it's long.", combined)
			if err := gemini.SendText(contextText); err != nil {
				log.Printf("gemini context inject error: %v", err)
			}
			pendingMessages = nil
		}

		for {
			select {
			case msg, ok := <-hubCh:
				if !ok {
					debounceTimer.Stop()
					return
				}
				writeBrowser(msg)
				// Only inject messages explicitly addressed to "live" (Clive)
				if msg.ToAgent == "live" && msg.FromAgent != "live" && msg.Content != "" {
					pendingMessages = append(pendingMessages, msg)
					// Reset debounce timer — wait 3s for more messages before flushing
					if debounceActive {
						if !debounceTimer.Stop() {
							<-debounceTimer.C
						}
					}
					debounceTimer.Reset(3 * time.Second)
					debounceActive = true
				}
			case <-debounceTimer.C:
				flushToGemini()
			case <-done:
				debounceTimer.Stop()
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
		log.Printf("Clive WebSocket client disconnected")
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
						// Don't spam logs when connection is closing
						if !strings.Contains(err.Error(), "close sent") {
							log.Printf("gemini send audio error: %v", err)
						}
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
				log.Printf("Clive WS received: type=%s content=%s", msg.Type, msg.Content)

				if msg.SessionID != "" {
					if _, err := s.hub.DB.InsertMessage(msg); err != nil {
						log.Printf("ws db error: %v", err)
					}
				}
			}
		}
	}
}

// transcriptToMessage converts a voice transcript into a hub message.
func transcriptToMessage(role, text string) protocol.Message {
	if role == "user" {
		return protocol.Message{
			FromAgent: "live",
			Type:      protocol.MsgCommand,
			Content:   text,
		}
	}
	return protocol.Message{
		FromAgent: "live",
		Type:      protocol.MsgResponse,
		Content:   text,
	}
}

// executeHubTool runs a hub tool call from Gemini and returns the result.
func (s *Server) executeHubTool(name string, args map[string]interface{}) map[string]interface{} {
	switch name {
	case "hub_list_agents":
		agents, err := s.hub.DB.ListAgents("")
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		var result []map[string]string
		for _, a := range agents {
			result = append(result, map[string]string{
				"id":      a.ID,
				"name":    a.Name,
				"type":    string(a.Type),
				"status":  string(a.Status),
				"project": a.Project,
			})
		}
		return map[string]interface{}{"agents": result}

	case "hub_read_messages":
		limit := 20
		if l, ok := args["limit"].(float64); ok && l > 0 {
			limit = int(l)
		}
		msgs, err := s.hub.DB.GetRecentMessages(limit)
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		var result []map[string]string
		for _, m := range msgs {
			result = append(result, map[string]string{
				"from":    m.FromAgent,
				"to":      m.ToAgent,
				"type":    string(m.Type),
				"content": m.Content,
			})
		}
		return map[string]interface{}{"messages": result}

	case "hub_send_message":
		content, _ := args["content"].(string)
		to, _ := args["to"].(string)
		msgType, _ := args["type"].(string)
		if msgType == "" {
			msgType = "command"
		}
		if content == "" {
			return map[string]interface{}{"error": "content is required"}
		}
		msg := protocol.Message{
			FromAgent: "live",
			ToAgent:   to,
			Type:      protocol.MessageType(msgType),
			Content:   content,
		}
		id, err := s.hub.DB.InsertMessage(msg)
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		return map[string]interface{}{"status": "sent", "message_id": id}

	case "hub_list_sessions":
		sessions, err := s.hub.DB.ListSessions("")
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		var result []map[string]string
		for _, sess := range sessions {
			result = append(result, map[string]string{
				"id":      sess.ID,
				"mode":    string(sess.Mode),
				"purpose": sess.Purpose,
				"status":  string(sess.Status),
			})
		}
		return map[string]interface{}{"sessions": result}

	default:
		return map[string]interface{}{"error": fmt.Sprintf("unknown tool: %s", name)}
	}
}

// geminiReadLoop reads messages from Gemini and forwards them to the browser.
// If the Gemini connection drops, it auto-reconnects and updates the gemini pointer.
func (s *Server) geminiReadLoop(gemini *GeminiProxy, writeBrowser func(interface{}), done <-chan struct{}, geminiPtr **GeminiProxy) {
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
				// Auto-reconnect
				writeBrowser(map[string]string{"type": "error", "error": "Reconnecting to Gemini..."})
				gemini.Close()

				apiKey := s.hub.DB.GetSetting("live.gemini_api_key", "")
				if apiKey == "" {
					apiKey = s.hub.Config.Live.GeminiAPIKey
				}
				voice := s.hub.DB.GetSetting("live.voice", s.hub.Config.Live.Voice)
				newGemini := NewGeminiProxy(apiKey, voice)
				if err := newGemini.Connect(""); err != nil {
					log.Printf("gemini reconnect failed: %v", err)
					writeBrowser(map[string]string{"type": "error", "error": fmt.Sprintf("Reconnect failed: %v", err)})
					*geminiPtr = nil
					return
				}
				log.Printf("Gemini auto-reconnected")
				writeBrowser(map[string]string{"type": "connected"})
				*geminiPtr = newGemini
				gemini = newGemini
				continue
			}
		}

		// Handle tool calls from Gemini
		if tc, ok := msg["toolCall"].(map[string]interface{}); ok {
			if calls, ok := tc["functionCalls"].([]interface{}); ok {
				for _, c := range calls {
					call, ok := c.(map[string]interface{})
					if !ok {
						continue
					}
					callID, _ := call["id"].(string)
					fnName, _ := call["name"].(string)
					args, _ := call["args"].(map[string]interface{})
					log.Printf("Gemini tool call: %s(%v)", fnName, args)

					result := s.executeHubTool(fnName, args)
					if err := gemini.SendToolResponse(callID, result); err != nil {
						log.Printf("tool response error: %v", err)
					}
				}
			}
			continue
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
				// Don't insert voice transcriptions into the hub — Clive decides
				// what to send via hub_send_message tool calls. Broadcasting raw
				// voice here causes Brian to pick it up AND Clive to send a
				// separate tool call, resulting in duplicate processing.
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
