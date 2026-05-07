package webui

import (
	"encoding/json"
	"fmt"
	"log"
	"net/url"
	"sync"

	"github.com/gorilla/websocket"
)

const (
	geminiWSEndpoint = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
	geminiModel      = "models/gemini-3.1-flash-live-preview"
	defaultSystemInstruction = `You are Clive, the voice for bot-hq. Brian (id "brian") is the orchestrator and Rain (id "rain") is the QA watchdog. Use the available hub tools to read activity and message agents; for anything that needs running code or spawning sessions, ask Brian.`
)

// GeminiProxy manages a WebSocket connection to the Gemini Live API,
// forwarding audio and text between a browser client and Gemini.
type GeminiProxy struct {
	apiKey    string
	voice     string
	conn      *websocket.Conn
	mu        sync.Mutex
	connected bool
}

// NewGeminiProxy creates a new proxy with the given API key and voice name.
func NewGeminiProxy(apiKey, voice string) *GeminiProxy {
	if voice == "" {
		voice = "Iapetus"
	}
	return &GeminiProxy{
		apiKey: apiKey,
		voice:  voice,
	}
}

// hubToolDeclarations returns Gemini function declarations for hub tools.
func hubToolDeclarations() []map[string]interface{} {
	return []map[string]interface{}{
		{
			"name":        "hub_list_agents",
			"description": "List all agents registered in the bot-hq hub with their status",
			"parameters": map[string]interface{}{
				"type":       "OBJECT",
				"properties": map[string]interface{}{},
			},
		},
		{
			"name":        "hub_read_messages",
			"description": "Read recent messages from the bot-hq hub",
			"parameters": map[string]interface{}{
				"type": "OBJECT",
				"properties": map[string]interface{}{
					"limit": map[string]interface{}{
						"type":        "NUMBER",
						"description": "Max messages to return (default 20)",
					},
				},
			},
		},
		{
			"name":        "hub_send_message",
			"description": "Send a message to an agent or broadcast to the hub. Use this to talk to Brian, other agents, or broadcast commands.",
			"parameters": map[string]interface{}{
				"type": "OBJECT",
				"properties": map[string]interface{}{
					"to": map[string]interface{}{
						"type":        "STRING",
						"description": "Recipient agent ID (e.g. 'brian', 'user'). Leave empty for broadcast.",
					},
					"content": map[string]interface{}{
						"type":        "STRING",
						"description": "Message content",
					},
					"type": map[string]interface{}{
						"type":        "STRING",
						"description": "Message type: command, question, update",
					},
				},
				"required": []string{"content"},
			},
		},
		{
			"name":        "hub_list_sessions",
			"description": "List active sessions (Claude Code tmux sessions, brainstorming sessions, etc.)",
			"parameters": map[string]interface{}{
				"type":       "OBJECT",
				"properties": map[string]interface{}{},
			},
		},
		{
			"name":        "read_file",
			"description": "Read the current text content of a file under the bot-hq canonical store (~/.bot-hq/). If path is omitted, reads the file the user is currently viewing in the web UI.",
			"parameters": map[string]interface{}{
				"type": "OBJECT",
				"properties": map[string]interface{}{
					"path": map[string]interface{}{
						"type":        "STRING",
						"description": "Path relative to ~/.bot-hq/ (e.g. 'tasks.md' or 'projects/bcc-ad-manager/plans/2026-05-07-foo.md'). Optional; defaults to the user's current focus file.",
					},
				},
			},
		},
		{
			"name":        "propose_file_edit",
			"description": "Propose a full-content replacement for a file under the bot-hq canonical store. Pops a diff in the user's web UI for explicit approval — does NOT apply automatically. Use this for any planning, note-taking, decisions, or content the user asks you to capture. If path is omitted, proposes against the user's current focus file.",
			"parameters": map[string]interface{}{
				"type": "OBJECT",
				"properties": map[string]interface{}{
					"path": map[string]interface{}{
						"type":        "STRING",
						"description": "Path relative to ~/.bot-hq/ (e.g. 'tasks.md', 'projects/<p>/plans/<file>.md'). Optional; defaults to the user's current focus file.",
					},
					"content": map[string]interface{}{
						"type":        "STRING",
						"description": "Full new content of the file (not a diff — the entire post-edit text).",
					},
					"purpose": map[string]interface{}{
						"type":        "STRING",
						"description": "One-line description of why this edit is being proposed. Surfaced in the diff UI.",
					},
				},
				"required": []string{"content", "purpose"},
			},
		},
	}
}

// Connect dials the Gemini Live WebSocket and sends the setup message.
func (g *GeminiProxy) Connect(systemInstruction string) error {
	g.mu.Lock()
	defer g.mu.Unlock()

	if g.connected {
		return fmt.Errorf("already connected")
	}

	if systemInstruction == "" {
		systemInstruction = defaultSystemInstruction
	}

	u, err := url.Parse(geminiWSEndpoint)
	if err != nil {
		return fmt.Errorf("parse gemini url: %w", err)
	}
	q := u.Query()
	q.Set("key", g.apiKey)
	u.RawQuery = q.Encode()

	conn, _, err := websocket.DefaultDialer.Dial(u.String(), nil)
	if err != nil {
		return fmt.Errorf("dial gemini: %w", err)
	}
	g.conn = conn

	// Send setup message
	setup := map[string]interface{}{
		"setup": map[string]interface{}{
			"model": geminiModel,
			"generationConfig": map[string]interface{}{
				"responseModalities": []string{"AUDIO"},
				"speechConfig": map[string]interface{}{
					"voiceConfig": map[string]interface{}{
						"prebuiltVoiceConfig": map[string]interface{}{
							"voiceName": g.voice,
						},
					},
				},
			},
			"systemInstruction": map[string]interface{}{
				"parts": []map[string]interface{}{
					{"text": systemInstruction},
				},
			},
			"tools": []map[string]interface{}{
				{"functionDeclarations": hubToolDeclarations()},
			},
			"inputAudioTranscription":  map[string]interface{}{},
			"outputAudioTranscription": map[string]interface{}{},
			"realtimeInputConfig": map[string]interface{}{
				"automaticActivityDetection": map[string]interface{}{
					"disabled":                  false,
					"startOfSpeechSensitivity":  "START_SENSITIVITY_HIGH",
					"endOfSpeechSensitivity":    "END_SENSITIVITY_LOW",
					"silenceDurationMs":         1000,
				},
			},
		},
	}

	if err := conn.WriteJSON(setup); err != nil {
		conn.Close()
		return fmt.Errorf("send setup: %w", err)
	}

	// Read the setup completion response
	_, _, err = conn.ReadMessage()
	if err != nil {
		conn.Close()
		return fmt.Errorf("read setup response: %w", err)
	}

	g.connected = true
	log.Printf("Gemini Live API connected (voice=%s)", g.voice)
	return nil
}

// SendAudio sends a base64-encoded PCM audio chunk to Gemini.
func (g *GeminiProxy) SendAudio(base64PCM string) error {
	g.mu.Lock()
	defer g.mu.Unlock()

	if !g.connected || g.conn == nil {
		return fmt.Errorf("not connected")
	}

	msg := map[string]interface{}{
		"realtimeInput": map[string]interface{}{
			"audio": map[string]interface{}{
				"mimeType": "audio/pcm;rate=16000",
				"data":     base64PCM,
			},
		},
	}

	return g.conn.WriteJSON(msg)
}

// SendText sends a text message to Gemini as client content.
func (g *GeminiProxy) SendText(text string) error {
	g.mu.Lock()
	defer g.mu.Unlock()

	if !g.connected || g.conn == nil {
		return fmt.Errorf("not connected")
	}

	msg := map[string]interface{}{
		"clientContent": map[string]interface{}{
			"turns": []map[string]interface{}{
				{
					"role": "user",
					"parts": []map[string]interface{}{
						{"text": text},
					},
				},
			},
			"turnComplete": true,
		},
	}

	return g.conn.WriteJSON(msg)
}

// SendToolResponse sends a function call response back to Gemini.
func (g *GeminiProxy) SendToolResponse(callID string, result map[string]interface{}) error {
	g.mu.Lock()
	defer g.mu.Unlock()

	if !g.connected || g.conn == nil {
		return fmt.Errorf("not connected")
	}

	msg := map[string]interface{}{
		"toolResponse": map[string]interface{}{
			"functionResponses": []map[string]interface{}{
				{
					"id":       callID,
					"response": result,
				},
			},
		},
	}

	return g.conn.WriteJSON(msg)
}

// ReadMessage reads and parses one JSON message from the Gemini WebSocket.
// The caller should not hold g.mu when calling this method, as ReadMessage
// blocks until a message is available.
func (g *GeminiProxy) ReadMessage() (map[string]interface{}, error) {
	g.mu.Lock()
	conn := g.conn
	g.mu.Unlock()

	if conn == nil {
		return nil, fmt.Errorf("not connected")
	}

	_, data, err := conn.ReadMessage()
	if err != nil {
		return nil, err
	}

	var msg map[string]interface{}
	if err := json.Unmarshal(data, &msg); err != nil {
		return nil, fmt.Errorf("unmarshal gemini message: %w", err)
	}

	return msg, nil
}

// Close shuts down the Gemini WebSocket connection.
func (g *GeminiProxy) Close() error {
	g.mu.Lock()
	defer g.mu.Unlock()

	if !g.connected || g.conn == nil {
		return nil
	}

	g.connected = false
	err := g.conn.Close()
	g.conn = nil
	log.Printf("Gemini Live API disconnected")
	return err
}
