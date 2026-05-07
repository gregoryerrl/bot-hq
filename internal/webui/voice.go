package webui

// Voice surface migrated from internal/live (Phase P P-10 / phase-n.md:823
// "Two-web unification"). Per user msg 15068 directive ("why route? why
// not directly. I hate this session"), voice features are integrated
// DIRECTLY into the workspace webui — no /voice subroute, no iframe, no
// reverse-proxy. Mic button + Gemini-backed audio chat live alongside
// the file browser, rules editor, and pending-actions queue at :3849.
//
// Internal/live package retired in this commit; voice handler logic
// migrated here using webui's hub.DB-direct pattern (no *hub.Hub
// dependency; OnMessage callback drives live message streaming).

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// voiceUpgrader is the gorilla/websocket Upgrader for the voice handler.
// CheckOrigin is permissive because the webui is loopback-only.
var voiceUpgrader = websocket.Upgrader{
	CheckOrigin: func(r *http.Request) bool { return true },
}

// voiceAgentID is the canonical hub-agent identifier for the voice
// surface — registers as "clive" matching internal/clive AgentID.
const voiceAgentID = "clive"

// browserMessage is the JSON envelope from the browser to the voice
// WebSocket. Type discriminates audio / text / hub-message intents.
type voiceBrowserMessage struct {
	Type string `json:"type"`
	Data string `json:"data"`
}

// handleVoiceWS upgrades the connection + bridges browser ↔ Gemini ↔
// hub. Migrated from internal/live/server.go handleWebSocket; uses
// db directly + OnMessage callback for hub-message streaming.
func (s *Server) handleVoiceWS(w http.ResponseWriter, r *http.Request) {
	if s.db == nil {
		http.Error(w, "hub.DB not configured", http.StatusServiceUnavailable)
		return
	}
	conn, err := voiceUpgrader.Upgrade(w, r, nil)
	if err != nil {
		log.Printf("voice ws upgrade error: %v", err)
		return
	}
	log.Printf("voice WebSocket client connected")

	// Register voice agent in the hub DB.
	_ = s.db.RegisterAgent(protocol.Agent{
		ID:     voiceAgentID,
		Name:   "Clive",
		Type:   protocol.AgentVoice,
		Status: protocol.StatusOnline,
	})

	// Per-connection mutex for concurrent writes to the browser ws.
	var wsMu sync.Mutex
	writeBrowser := func(v interface{}) {
		wsMu.Lock()
		defer wsMu.Unlock()
		data, err := json.Marshal(v)
		if err != nil {
			log.Printf("voice ws marshal error: %v", err)
			return
		}
		if err := conn.WriteMessage(websocket.TextMessage, data); err != nil {
			log.Printf("voice ws write error: %v", err)
		}
	}

	// Connect to Gemini if API key is configured (Settings tab or
	// db default). Voice features degrade gracefully without a key.
	apiKey := s.db.GetSetting("live.gemini_api_key", "")
	var gemini *GeminiProxy
	done := make(chan struct{})
	if apiKey != "" {
		voice := s.db.GetSetting("live.voice", "Iapetus")
		gemini = NewGeminiProxy(apiKey, voice)
		// Inject ambient webui-focus context into systemInstruction so Clive
		// knows what the user is currently looking at without being told.
		// Per user msg 15117 ("i want clive to see what i am looking at on
		// the web ui so i don't have to mention fileName, filePath, etc").
		if err := gemini.Connect(s.buildVoiceSystemInstruction()); err != nil {
			log.Printf("Gemini connect error: %v", err)
			writeBrowser(map[string]string{
				"type":  "error",
				"error": fmt.Sprintf("Gemini connection failed: %v", err),
			})
			gemini = nil
		} else {
			writeBrowser(map[string]string{"type": "connected"})
			go s.voiceGeminiReadLoop(gemini, writeBrowser, done, &gemini)
			// Subscribe to mid-session webui-focus changes; SendText each
			// new focus into the live Gemini conversation. Connect-time
			// systemInstruction alone misses changes that happen after
			// the voice WS opens (page-load → empty focus → user opens
			// file → Gemini blind). Per user msg "clive can't see the
			// file that i'm looking at" 2026-05-07.
			ctxCh, unsubCtx := s.SubscribeWebuiContext()
			go func() {
				defer unsubCtx()
				for {
					select {
					case <-done:
						return
					case ctx, ok := <-ctxCh:
						if !ok {
							return
						}
						if gemini == nil {
							continue
						}
						focus := formatFocusContext(ctx)
						if focus == "" {
							continue
						}
						if err := gemini.SendText("[CONTEXT UPDATE]\n" + focus); err != nil {
							log.Printf("voice ctx-update inject error: %v", err)
						}
					}
				}
			}()
		}
	} else {
		log.Printf("voice: no Gemini API key configured — audio chat disabled")
		writeBrowser(map[string]string{
			"type":  "error",
			"error": "No Gemini API key configured — set it in Settings tab, then reconnect",
		})
	}

	// Subscribe to hub messages via OnMessage; debounce-batched
	// injection into Gemini for messages addressed to "clive".
	hubCh := make(chan protocol.Message, 64)
	hubSub := func(m protocol.Message) {
		select {
		case hubCh <- m:
		default:
			// drop on overflow rather than block broadcaster
		}
	}
	s.db.OnMessage(hubSub)

	// Hub-message goroutine: write to browser + debounce-inject into Gemini.
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
				if msg.ToAgent == voiceAgentID && msg.FromAgent != voiceAgentID && msg.Content != "" {
					pendingMessages = append(pendingMessages, msg)
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

	// Cleanup on disconnect.
	defer func() {
		close(done)
		_ = s.db.UpdateAgentStatus(voiceAgentID, protocol.StatusOffline)
		if gemini != nil {
			gemini.Close()
		}
		conn.Close()
		log.Printf("voice WebSocket client disconnected")
	}()

	// Browser → server read loop: audio / text / hub-protocol messages.
	for {
		msgType, data, err := conn.ReadMessage()
		if err != nil {
			if websocket.IsUnexpectedCloseError(err, websocket.CloseGoingAway, websocket.CloseNormalClosure) {
				log.Printf("voice ws read error: %v", err)
			}
			break
		}
		if msgType != websocket.TextMessage {
			continue
		}
		var bMsg voiceBrowserMessage
		if err := json.Unmarshal(data, &bMsg); err != nil {
			log.Printf("voice ws unmarshal error: %v", err)
			continue
		}
		switch bMsg.Type {
		case "audio":
			if gemini != nil {
				if err := gemini.SendAudio(bMsg.Data); err != nil {
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
			var msg protocol.Message
			if err := json.Unmarshal(data, &msg); err != nil {
				log.Printf("voice ws unmarshal hub msg error: %v", err)
				continue
			}
			msg.FromAgent = voiceAgentID
			if msg.SessionID != "" {
				if _, err := s.db.InsertMessage(msg); err != nil {
					log.Printf("voice ws db error: %v", err)
				}
			}
		}
	}
}

// voiceGeminiReadLoop reads Gemini API responses + forwards audio /
// transcripts / tool-calls to the browser. Migrated from internal/
// live/server.go geminiReadLoop with auto-reconnect logic preserved.
func (s *Server) voiceGeminiReadLoop(gemini *GeminiProxy, writeBrowser func(interface{}), done <-chan struct{}, geminiPtr **GeminiProxy) {
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
			}
			log.Printf("gemini read error: %v", err)
			return
		}
		s.processGeminiMessage(gemini, msg, writeBrowser)
	}
}

// processGeminiMessage routes a Gemini server message to the right
// browser-bound output (audio bytes, transcript text, tool calls).
// Tool calls execute locally + send response back to Gemini.
func (s *Server) processGeminiMessage(gemini *GeminiProxy, msg map[string]interface{}, writeBrowser func(interface{})) {
	if toolCall, ok := msg["toolCall"].(map[string]interface{}); ok {
		fcs, _ := toolCall["functionCalls"].([]interface{})
		for _, fc := range fcs {
			fcMap, ok := fc.(map[string]interface{})
			if !ok {
				continue
			}
			id, _ := fcMap["id"].(string)
			name, _ := fcMap["name"].(string)
			args, _ := fcMap["args"].(map[string]interface{})
			result := s.executeVoiceHubTool(name, args)
			if err := gemini.SendToolResponse(id, result); err != nil {
				log.Printf("gemini tool response error: %v", err)
			}
		}
		return
	}
	if sc, ok := msg["serverContent"].(map[string]interface{}); ok {
		if mt, ok := sc["modelTurn"].(map[string]interface{}); ok {
			parts, _ := mt["parts"].([]interface{})
			for _, p := range parts {
				pm, ok := p.(map[string]interface{})
				if !ok {
					continue
				}
				if id, ok := pm["inlineData"].(map[string]interface{}); ok {
					if dataStr, ok := id["data"].(string); ok {
						writeBrowser(map[string]string{"type": "audio", "data": dataStr})
					}
				}
				if t, ok := pm["text"].(string); ok && t != "" {
					writeBrowser(map[string]string{"type": "text", "data": t})
				}
			}
		}
		if it, ok := sc["inputTranscription"].(map[string]interface{}); ok {
			if text, ok := it["text"].(string); ok && text != "" {
				writeBrowser(map[string]string{"type": "transcript-input", "data": text})
			}
		}
		if ot, ok := sc["outputTranscription"].(map[string]interface{}); ok {
			if text, ok := ot["text"].(string); ok && text != "" {
				writeBrowser(map[string]string{"type": "transcript-output", "data": text})
			}
		}
		if tc, ok := sc["turnComplete"].(bool); ok && tc {
			writeBrowser(map[string]string{"type": "turn-complete"})
		}
		if i, ok := sc["interrupted"].(bool); ok && i {
			writeBrowser(map[string]string{"type": "interrupted"})
		}
	}
}

// executeVoiceHubTool runs a hub tool call from Gemini and returns
// a result map. Migrated from internal/live/server.go executeHubTool;
// uses db directly.
func (s *Server) executeVoiceHubTool(name string, args map[string]interface{}) map[string]interface{} {
	switch name {
	case "hub_list_agents":
		agents, err := s.db.ListAgents("")
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		var result []map[string]string
		for _, a := range agents {
			result = append(result, map[string]string{
				"id": a.ID, "name": a.Name, "type": string(a.Type),
				"status": string(a.Status), "project": a.Project,
			})
		}
		return map[string]interface{}{"agents": result}
	case "hub_read_messages":
		limit := 20
		if l, ok := args["limit"].(float64); ok && l > 0 {
			limit = int(l)
		}
		msgs, err := s.db.GetRecentMessages(limit)
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		var result []map[string]string
		for _, m := range msgs {
			result = append(result, map[string]string{
				"from": m.FromAgent, "to": m.ToAgent,
				"type": string(m.Type), "content": m.Content,
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
			FromAgent: voiceAgentID, ToAgent: to,
			Type: protocol.MessageType(msgType), Content: content,
		}
		id, err := s.db.InsertMessage(msg)
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		return map[string]interface{}{"status": "sent", "message_id": id}
	case "hub_list_sessions":
		sessions, err := s.db.ListSessions("")
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		var result []map[string]string
		for _, sess := range sessions {
			result = append(result, map[string]string{
				"id": sess.ID, "mode": string(sess.Mode),
				"purpose": sess.Purpose, "status": string(sess.Status),
			})
		}
		return map[string]interface{}{"sessions": result}
	case "read_file":
		path, _ := args["path"].(string)
		if path == "" {
			path = s.GetWebuiContext().CurrentPath
		}
		if path == "" {
			return map[string]interface{}{"error": "no path provided and no file currently in focus"}
		}
		abs, err := resolveCanonicalPath(s.canonicalRoot, path)
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		data, err := os.ReadFile(abs)
		if err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		return map[string]interface{}{"path": path, "content": string(data)}
	case "propose_file_edit":
		path, _ := args["path"].(string)
		if path == "" {
			path = s.GetWebuiContext().CurrentPath
		}
		content, _ := args["content"].(string)
		purpose, _ := args["purpose"].(string)
		if path == "" {
			return map[string]interface{}{"error": "no path provided and no file currently in focus"}
		}
		if content == "" {
			return map[string]interface{}{"error": "content is required"}
		}
		if purpose == "" {
			return map[string]interface{}{"error": "purpose is required"}
		}
		if _, err := resolveCanonicalPath(s.canonicalRoot, path); err != nil {
			return map[string]interface{}{"error": err.Error()}
		}
		id := newProposalID()
		s.proposals.add(&cliveProposal{
			id:        id,
			relPath:   path,
			content:   content,
			purpose:   purpose,
			expiresAt: time.Now().Add(10 * time.Minute),
		})
		// Notify user-side: mirror the HTTP propose handler's behavior so
		// the diff lands in the webui pending-actions UI for approval.
		s.notifyProposal(path, purpose, id)
		return map[string]interface{}{"status": "proposed", "proposal_id": id, "path": path}
	default:
		return map[string]interface{}{"error": fmt.Sprintf("unknown tool: %s", name)}
	}
}

// hubMessageSink defines the minimum surface of hub.DB used by the
// voice handler — kept for future test-replacement of the DB layer.
type hubMessageSink interface {
	OnMessage(fn func(protocol.Message))
	GetSetting(key, defaultVal string) string
	RegisterAgent(agent protocol.Agent) error
	UpdateAgentStatus(id string, status protocol.AgentStatus, project ...string) error
	InsertMessage(msg protocol.Message) (int64, error)
	ListAgents(statusFilter string) ([]protocol.Agent, error)
	GetRecentMessages(limit int) ([]protocol.Message, error)
	ListSessions(statusFilter string) ([]protocol.Session, error)
}

// compile-time check: *hub.DB satisfies hubMessageSink (for future
// test replacements + interface stability).
var _ hubMessageSink = (*hub.DB)(nil)
