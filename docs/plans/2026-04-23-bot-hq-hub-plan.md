# Bot-HQ Hub Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a native Go multi-agent communication hub with terminal UI, MCP server, Gemini Live proxy, and Discord integration.

**Architecture:** Single Go binary (`bot-hq`) serving as both the hub terminal app and stdio MCP server. SQLite for shared state, Bubbletea for TUI, WebSocket for Live proxy, goroutines for concurrent dispatch.

**Tech Stack:** Go 1.26, Bubbletea/Lipgloss/Bubbles (TUI), modernc.org/sqlite (pure Go SQLite), mark3labs/mcp-go (MCP SDK), gorilla/websocket (WebSocket), discordgo (Discord)

**Design Doc:** `docs/plans/2026-04-23-bot-hq-hub-design.md`

---

## Phase 1: Project Foundation

### Task 1.1: Initialize Go Module

**Files:**
- Create: `cmd/bot-hq/main.go`
- Create: `go.mod`

**Step 1: Initialize Go module and install core dependencies**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq
go mod init github.com/gregoryerrl/bot-hq
go get github.com/charmbracelet/bubbletea@latest
go get github.com/charmbracelet/lipgloss@latest
go get github.com/charmbracelet/bubbles@latest
go get modernc.org/sqlite@latest
go get github.com/gorilla/websocket@latest
go get github.com/mark3labs/mcp-go@latest
go get github.com/bwmarrin/discordgo@latest
go get github.com/google/uuid@latest
go get github.com/BurntSushi/toml@latest
```

**Step 2: Create entry point**

Create `cmd/bot-hq/main.go`:
```go
package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) > 1 {
		switch os.Args[1] {
		case "mcp":
			fmt.Println("MCP server mode — not yet implemented")
			os.Exit(0)
		case "status":
			fmt.Println("Status — not yet implemented")
			os.Exit(0)
		case "version":
			fmt.Println("bot-hq v0.1.0")
			os.Exit(0)
		}
	}
	fmt.Println("Bot-HQ Hub — not yet implemented")
}
```

**Step 3: Verify it builds and runs**

Run: `go build -o bot-hq ./cmd/bot-hq && ./bot-hq version`
Expected: `bot-hq v0.1.0`

**Step 4: Commit**

```bash
git add cmd/ go.mod go.sum
git commit -m "feat(hub): initialize Go project with dependencies"
```

---

### Task 1.2: Protocol Types

**Files:**
- Create: `internal/protocol/types.go`
- Create: `internal/protocol/constants.go`
- Test: `internal/protocol/types_test.go`

**Step 1: Write the test**

Create `internal/protocol/types_test.go`:
```go
package protocol

import "testing"

func TestAgentTypeValid(t *testing.T) {
	valid := []AgentType{AgentCoder, AgentVoice, AgentBrain, AgentDiscord}
	for _, at := range valid {
		if !at.Valid() {
			t.Errorf("expected %s to be valid", at)
		}
	}
	if AgentType("invalid").Valid() {
		t.Error("expected 'invalid' to be invalid")
	}
}

func TestMessageTypeValid(t *testing.T) {
	valid := []MessageType{MsgHandshake, MsgQuestion, MsgResponse, MsgCommand, MsgUpdate, MsgResult, MsgError}
	for _, mt := range valid {
		if !mt.Valid() {
			t.Errorf("expected %s to be valid", mt)
		}
	}
}

func TestSessionModeValid(t *testing.T) {
	valid := []SessionMode{ModeBrainstorm, ModeImplement, ModeChat}
	for _, sm := range valid {
		if !sm.Valid() {
			t.Errorf("expected %s to be valid", sm)
		}
	}
}
```

**Step 2: Run test to verify it fails**

Run: `go test ./internal/protocol/ -v`
Expected: FAIL — types not defined

**Step 3: Write implementation**

Create `internal/protocol/types.go`:
```go
package protocol

import "time"

// Agent types
type AgentType string

const (
	AgentCoder   AgentType = "coder"
	AgentVoice   AgentType = "voice"
	AgentBrain   AgentType = "brain"
	AgentDiscord AgentType = "discord"
)

func (a AgentType) Valid() bool {
	switch a {
	case AgentCoder, AgentVoice, AgentBrain, AgentDiscord:
		return true
	}
	return false
}

// Agent statuses
type AgentStatus string

const (
	StatusOnline  AgentStatus = "online"
	StatusWorking AgentStatus = "working"
	StatusIdle    AgentStatus = "idle"
	StatusOffline AgentStatus = "offline"
)

// Message types
type MessageType string

const (
	MsgHandshake MessageType = "handshake"
	MsgQuestion  MessageType = "question"
	MsgResponse  MessageType = "response"
	MsgCommand   MessageType = "command"
	MsgUpdate    MessageType = "update"
	MsgResult    MessageType = "result"
	MsgError     MessageType = "error"
)

func (m MessageType) Valid() bool {
	switch m {
	case MsgHandshake, MsgQuestion, MsgResponse, MsgCommand, MsgUpdate, MsgResult, MsgError:
		return true
	}
	return false
}

// Session modes
type SessionMode string

const (
	ModeBrainstorm SessionMode = "brainstorm"
	ModeImplement  SessionMode = "implement"
	ModeChat       SessionMode = "chat"
)

func (s SessionMode) Valid() bool {
	switch s {
	case ModeBrainstorm, ModeImplement, ModeChat:
		return true
	}
	return false
}

// Session statuses
type SessionStatus string

const (
	SessionActive SessionStatus = "active"
	SessionPaused SessionStatus = "paused"
	SessionDone   SessionStatus = "done"
)

// Core structs

type Agent struct {
	ID         string      `json:"id"`
	Name       string      `json:"name"`
	Type       AgentType   `json:"type"`
	Status     AgentStatus `json:"status"`
	Project    string      `json:"project,omitempty"`
	Meta       string      `json:"meta,omitempty"`
	Registered time.Time   `json:"registered"`
	LastSeen   time.Time   `json:"last_seen"`
}

type Session struct {
	ID      string        `json:"id"`
	Mode    SessionMode   `json:"mode"`
	Purpose string        `json:"purpose"`
	Agents  []string      `json:"agents"`
	Status  SessionStatus `json:"status"`
	Created time.Time     `json:"created"`
	Updated time.Time     `json:"updated"`
}

type Message struct {
	ID        int64       `json:"id"`
	SessionID string      `json:"session_id,omitempty"`
	FromAgent string      `json:"from_agent"`
	ToAgent   string      `json:"to_agent,omitempty"`
	Type      MessageType `json:"type"`
	Content   string      `json:"content"`
	Created   time.Time   `json:"created"`
}
```

Create `internal/protocol/constants.go`:
```go
package protocol

const (
	DefaultPort   = 3847
	DefaultDBPath = "~/.bot-hq/hub.db"
	Version       = "0.1.0"
)
```

**Step 4: Run test to verify it passes**

Run: `go test ./internal/protocol/ -v`
Expected: PASS

**Step 5: Commit**

```bash
git add internal/protocol/
git commit -m "feat(hub): add protocol types, constants, and validation"
```

---

### Task 1.3: Config

**Files:**
- Create: `internal/hub/config.go`
- Test: `internal/hub/config_test.go`

**Step 1: Write the test**

Create `internal/hub/config_test.go`:
```go
package hub

import (
	"os"
	"path/filepath"
	"testing"
)

func TestDefaultConfig(t *testing.T) {
	cfg := DefaultConfig()
	if cfg.Hub.LivePort != 3847 {
		t.Errorf("expected port 3847, got %d", cfg.Hub.LivePort)
	}
	if cfg.Live.Voice != "Iapetus" {
		t.Errorf("expected voice Iapetus, got %s", cfg.Live.Voice)
	}
}

func TestLoadConfigCreatesDefault(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.toml")
	cfg, err := LoadConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Hub.LivePort != 3847 {
		t.Errorf("expected default port, got %d", cfg.Hub.LivePort)
	}
	// File should now exist
	if _, err := os.Stat(path); os.IsNotExist(err) {
		t.Error("config file should have been created")
	}
}

func TestLoadConfigReadsExisting(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.toml")
	os.WriteFile(path, []byte(`
[hub]
live_port = 9999

[live]
voice = "Charon"
`), 0644)

	cfg, err := LoadConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Hub.LivePort != 9999 {
		t.Errorf("expected port 9999, got %d", cfg.Hub.LivePort)
	}
	if cfg.Live.Voice != "Charon" {
		t.Errorf("expected voice Charon, got %s", cfg.Live.Voice)
	}
}
```

**Step 2: Run test to verify it fails**

Run: `go test ./internal/hub/ -v`
Expected: FAIL

**Step 3: Write implementation**

Create `internal/hub/config.go`:
```go
package hub

import (
	"os"
	"path/filepath"

	"github.com/BurntSushi/toml"
)

type Config struct {
	Hub     HubConfig     `toml:"hub"`
	Live    LiveConfig    `toml:"live"`
	Discord DiscordConfig `toml:"discord"`
	Brain   BrainConfig   `toml:"brain"`
}

type HubConfig struct {
	DBPath   string `toml:"db_path"`
	LivePort int    `toml:"live_port"`
}

type LiveConfig struct {
	Voice        string `toml:"voice"`
	GeminiAPIKey string `toml:"gemini_api_key"`
}

type DiscordConfig struct {
	Token     string `toml:"token"`
	ChannelID string `toml:"channel_id"`
}

type BrainConfig struct {
	AutoStart bool `toml:"auto_start"`
}

func DefaultConfig() Config {
	home, _ := os.UserHomeDir()
	return Config{
		Hub: HubConfig{
			DBPath:   filepath.Join(home, ".bot-hq", "hub.db"),
			LivePort: 3847,
		},
		Live: LiveConfig{
			Voice: "Iapetus",
		},
	}
}

func LoadConfig(path string) (Config, error) {
	cfg := DefaultConfig()

	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			// Create default config
			if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
				return cfg, err
			}
			f, err := os.Create(path)
			if err != nil {
				return cfg, err
			}
			defer f.Close()
			if err := toml.NewEncoder(f).Encode(cfg); err != nil {
				return cfg, err
			}
			return cfg, nil
		}
		return cfg, err
	}

	if _, err := toml.Decode(string(data), &cfg); err != nil {
		return cfg, err
	}

	// Override with env vars if set
	if key := os.Getenv("BOT_HQ_GEMINI_KEY"); key != "" {
		cfg.Live.GeminiAPIKey = key
	}
	if token := os.Getenv("BOT_HQ_DISCORD_TOKEN"); token != "" {
		cfg.Discord.Token = token
	}

	return cfg, nil
}
```

**Step 4: Run test to verify it passes**

Run: `go test ./internal/hub/ -v`
Expected: PASS

**Step 5: Commit**

```bash
git add internal/hub/
git commit -m "feat(hub): add TOML config with defaults and env overrides"
```

---

## Phase 2: Database Layer

### Task 2.1: SQLite Database

**Files:**
- Create: `internal/hub/db.go`
- Test: `internal/hub/db_test.go`

**Step 1: Write the test**

Create `internal/hub/db_test.go`:
```go
package hub

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func setupTestDB(t *testing.T) *DB {
	t.Helper()
	path := filepath.Join(t.TempDir(), "test.db")
	db, err := OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

func TestRegisterAndGetAgent(t *testing.T) {
	db := setupTestDB(t)

	agent := protocol.Agent{
		ID:     "claude-abc",
		Name:   "Claude ABC",
		Type:   protocol.AgentCoder,
		Status: protocol.StatusOnline,
		Project: "/projects/test",
	}

	if err := db.RegisterAgent(agent); err != nil {
		t.Fatal(err)
	}

	got, err := db.GetAgent("claude-abc")
	if err != nil {
		t.Fatal(err)
	}
	if got.Name != "Claude ABC" {
		t.Errorf("expected name 'Claude ABC', got %q", got.Name)
	}
	if got.Type != protocol.AgentCoder {
		t.Errorf("expected type coder, got %s", got.Type)
	}
}

func TestListAgents(t *testing.T) {
	db := setupTestDB(t)

	db.RegisterAgent(protocol.Agent{ID: "a1", Name: "A1", Type: protocol.AgentCoder, Status: protocol.StatusOnline})
	db.RegisterAgent(protocol.Agent{ID: "a2", Name: "A2", Type: protocol.AgentVoice, Status: protocol.StatusOnline})
	db.RegisterAgent(protocol.Agent{ID: "a3", Name: "A3", Type: protocol.AgentCoder, Status: protocol.StatusOffline})

	agents, err := db.ListAgents("")
	if err != nil {
		t.Fatal(err)
	}
	if len(agents) != 3 {
		t.Errorf("expected 3 agents, got %d", len(agents))
	}

	online, err := db.ListAgents("online")
	if err != nil {
		t.Fatal(err)
	}
	if len(online) != 2 {
		t.Errorf("expected 2 online agents, got %d", len(online))
	}
}

func TestInsertAndReadMessages(t *testing.T) {
	db := setupTestDB(t)

	db.RegisterAgent(protocol.Agent{ID: "sender", Name: "S", Type: protocol.AgentCoder, Status: protocol.StatusOnline})
	db.RegisterAgent(protocol.Agent{ID: "receiver", Name: "R", Type: protocol.AgentVoice, Status: protocol.StatusOnline})

	id, err := db.InsertMessage(protocol.Message{
		FromAgent: "sender",
		ToAgent:   "receiver",
		Type:      protocol.MsgQuestion,
		Content:   "Hello?",
		Created:   time.Now(),
	})
	if err != nil {
		t.Fatal(err)
	}
	if id <= 0 {
		t.Errorf("expected positive message ID, got %d", id)
	}

	msgs, err := db.ReadMessages("receiver", 0, 50)
	if err != nil {
		t.Fatal(err)
	}
	if len(msgs) != 1 {
		t.Fatalf("expected 1 message, got %d", len(msgs))
	}
	if msgs[0].Content != "Hello?" {
		t.Errorf("expected 'Hello?', got %q", msgs[0].Content)
	}
}

func TestCreateAndGetSession(t *testing.T) {
	db := setupTestDB(t)

	sess := protocol.Session{
		ID:      "sess-1",
		Mode:    protocol.ModeBrainstorm,
		Purpose: "fix login bug",
		Agents:  []string{"claude-abc", "live"},
		Status:  protocol.SessionActive,
	}

	if err := db.CreateSession(sess); err != nil {
		t.Fatal(err)
	}

	got, err := db.GetSession("sess-1")
	if err != nil {
		t.Fatal(err)
	}
	if got.Purpose != "fix login bug" {
		t.Errorf("expected purpose 'fix login bug', got %q", got.Purpose)
	}
	if len(got.Agents) != 2 {
		t.Errorf("expected 2 agents, got %d", len(got.Agents))
	}
}

func TestUpdateHook(t *testing.T) {
	db := setupTestDB(t)

	received := make(chan protocol.Message, 1)
	db.OnMessage(func(msg protocol.Message) {
		received <- msg
	})

	db.InsertMessage(protocol.Message{
		FromAgent: "test",
		ToAgent:   "other",
		Type:      protocol.MsgUpdate,
		Content:   "hook test",
		Created:   time.Now(),
	})

	select {
	case msg := <-received:
		if msg.Content != "hook test" {
			t.Errorf("expected 'hook test', got %q", msg.Content)
		}
	case <-time.After(2 * time.Second):
		t.Error("timed out waiting for update hook")
	}
}
```

**Step 2: Run test to verify it fails**

Run: `go test ./internal/hub/ -run TestRegister -v`
Expected: FAIL — DB type not defined

**Step 3: Write implementation**

Create `internal/hub/db.go`:
```go
package hub

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	_ "modernc.org/sqlite"
)

type DB struct {
	conn       *sql.DB
	mu         sync.RWMutex
	onMessage  func(protocol.Message)
}

func OpenDB(path string) (*DB, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return nil, err
	}

	conn, err := sql.Open("sqlite", path+"?_journal_mode=WAL&_busy_timeout=5000&_synchronous=NORMAL")
	if err != nil {
		return nil, err
	}

	db := &DB{conn: conn}
	if err := db.migrate(); err != nil {
		conn.Close()
		return nil, err
	}

	return db, nil
}

func (db *DB) Close() error {
	return db.conn.Close()
}

func (db *DB) OnMessage(fn func(protocol.Message)) {
	db.mu.Lock()
	defer db.mu.Unlock()
	db.onMessage = fn
}

func (db *DB) migrate() error {
	schema := `
	CREATE TABLE IF NOT EXISTS agents (
		id          TEXT PRIMARY KEY,
		name        TEXT NOT NULL,
		type        TEXT NOT NULL,
		status      TEXT NOT NULL,
		project     TEXT DEFAULT '',
		meta        TEXT DEFAULT '',
		registered  INTEGER NOT NULL,
		last_seen   INTEGER NOT NULL
	);

	CREATE TABLE IF NOT EXISTS sessions (
		id          TEXT PRIMARY KEY,
		mode        TEXT NOT NULL,
		purpose     TEXT NOT NULL,
		agents      TEXT NOT NULL,
		status      TEXT NOT NULL,
		created     INTEGER NOT NULL,
		updated     INTEGER NOT NULL
	);

	CREATE TABLE IF NOT EXISTS messages (
		id          INTEGER PRIMARY KEY AUTOINCREMENT,
		session_id  TEXT DEFAULT '',
		from_agent  TEXT NOT NULL,
		to_agent    TEXT DEFAULT '',
		type        TEXT NOT NULL,
		content     TEXT NOT NULL,
		created     INTEGER NOT NULL
	);

	CREATE INDEX IF NOT EXISTS idx_messages_to ON messages(to_agent, id);
	CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, id);
	CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created);
	`
	_, err := db.conn.Exec(schema)
	return err
}

// --- Agents ---

func (db *DB) RegisterAgent(agent protocol.Agent) error {
	now := time.Now().UnixMilli()
	if agent.Registered.IsZero() {
		agent.Registered = time.Now()
	}
	_, err := db.conn.Exec(
		`INSERT OR REPLACE INTO agents (id, name, type, status, project, meta, registered, last_seen)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
		agent.ID, agent.Name, string(agent.Type), string(agent.Status),
		agent.Project, agent.Meta, agent.Registered.UnixMilli(), now,
	)
	return err
}

func (db *DB) GetAgent(id string) (protocol.Agent, error) {
	var a protocol.Agent
	var typ, status string
	var registered, lastSeen int64
	err := db.conn.QueryRow(
		`SELECT id, name, type, status, project, meta, registered, last_seen FROM agents WHERE id = ?`, id,
	).Scan(&a.ID, &a.Name, &typ, &status, &a.Project, &a.Meta, &registered, &lastSeen)
	if err != nil {
		return a, err
	}
	a.Type = protocol.AgentType(typ)
	a.Status = protocol.AgentStatus(status)
	a.Registered = time.UnixMilli(registered)
	a.LastSeen = time.UnixMilli(lastSeen)
	return a, nil
}

func (db *DB) UpdateAgentStatus(id string, status protocol.AgentStatus) error {
	_, err := db.conn.Exec(
		`UPDATE agents SET status = ?, last_seen = ? WHERE id = ?`,
		string(status), time.Now().UnixMilli(), id,
	)
	return err
}

func (db *DB) UnregisterAgent(id string) error {
	return db.UpdateAgentStatus(id, protocol.StatusOffline)
}

func (db *DB) ListAgents(statusFilter string) ([]protocol.Agent, error) {
	var rows *sql.Rows
	var err error
	if statusFilter != "" {
		rows, err = db.conn.Query(
			`SELECT id, name, type, status, project, meta, registered, last_seen FROM agents WHERE status = ? ORDER BY last_seen DESC`, statusFilter,
		)
	} else {
		rows, err = db.conn.Query(
			`SELECT id, name, type, status, project, meta, registered, last_seen FROM agents ORDER BY last_seen DESC`,
		)
	}
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var agents []protocol.Agent
	for rows.Next() {
		var a protocol.Agent
		var typ, status string
		var registered, lastSeen int64
		if err := rows.Scan(&a.ID, &a.Name, &typ, &status, &a.Project, &a.Meta, &registered, &lastSeen); err != nil {
			return nil, err
		}
		a.Type = protocol.AgentType(typ)
		a.Status = protocol.AgentStatus(status)
		a.Registered = time.UnixMilli(registered)
		a.LastSeen = time.UnixMilli(lastSeen)
		agents = append(agents, a)
	}
	return agents, nil
}

// --- Messages ---

func (db *DB) InsertMessage(msg protocol.Message) (int64, error) {
	if msg.Created.IsZero() {
		msg.Created = time.Now()
	}
	result, err := db.conn.Exec(
		`INSERT INTO messages (session_id, from_agent, to_agent, type, content, created)
		 VALUES (?, ?, ?, ?, ?, ?)`,
		msg.SessionID, msg.FromAgent, msg.ToAgent, string(msg.Type), msg.Content, msg.Created.UnixMilli(),
	)
	if err != nil {
		return 0, err
	}

	id, err := result.LastInsertId()
	if err != nil {
		return 0, err
	}

	msg.ID = id

	// Fire update hook
	db.mu.RLock()
	fn := db.onMessage
	db.mu.RUnlock()
	if fn != nil {
		go fn(msg)
	}

	return id, nil
}

func (db *DB) ReadMessages(agentID string, sinceID int64, limit int) ([]protocol.Message, error) {
	if limit <= 0 {
		limit = 50
	}
	rows, err := db.conn.Query(
		`SELECT id, session_id, from_agent, to_agent, type, content, created
		 FROM messages
		 WHERE (to_agent = ? OR to_agent = '') AND id > ?
		 ORDER BY id ASC
		 LIMIT ?`,
		agentID, sinceID, limit,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var msgs []protocol.Message
	for rows.Next() {
		var m protocol.Message
		var typ string
		var created int64
		if err := rows.Scan(&m.ID, &m.SessionID, &m.FromAgent, &m.ToAgent, &typ, &m.Content, &created); err != nil {
			return nil, err
		}
		m.Type = protocol.MessageType(typ)
		m.Created = time.UnixMilli(created)
		msgs = append(msgs, m)
	}
	return msgs, nil
}

func (db *DB) GetRecentMessages(limit int) ([]protocol.Message, error) {
	if limit <= 0 {
		limit = 100
	}
	rows, err := db.conn.Query(
		`SELECT id, session_id, from_agent, to_agent, type, content, created
		 FROM messages ORDER BY id DESC LIMIT ?`, limit,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var msgs []protocol.Message
	for rows.Next() {
		var m protocol.Message
		var typ string
		var created int64
		if err := rows.Scan(&m.ID, &m.SessionID, &m.FromAgent, &m.ToAgent, &typ, &m.Content, &created); err != nil {
			return nil, err
		}
		m.Type = protocol.MessageType(typ)
		m.Created = time.UnixMilli(created)
		msgs = append(msgs, m)
	}
	// Reverse to chronological order
	for i, j := 0, len(msgs)-1; i < j; i, j = i+1, j-1 {
		msgs[i], msgs[j] = msgs[j], msgs[i]
	}
	return msgs, nil
}

// --- Sessions ---

func (db *DB) CreateSession(sess protocol.Session) error {
	now := time.Now().UnixMilli()
	if sess.Created.IsZero() {
		sess.Created = time.Now()
	}
	agentsJSON, err := json.Marshal(sess.Agents)
	if err != nil {
		return fmt.Errorf("marshal agents: %w", err)
	}
	_, err = db.conn.Exec(
		`INSERT INTO sessions (id, mode, purpose, agents, status, created, updated)
		 VALUES (?, ?, ?, ?, ?, ?, ?)`,
		sess.ID, string(sess.Mode), sess.Purpose, string(agentsJSON),
		string(sess.Status), sess.Created.UnixMilli(), now,
	)
	return err
}

func (db *DB) GetSession(id string) (protocol.Session, error) {
	var s protocol.Session
	var mode, status, agentsJSON string
	var created, updated int64
	err := db.conn.QueryRow(
		`SELECT id, mode, purpose, agents, status, created, updated FROM sessions WHERE id = ?`, id,
	).Scan(&s.ID, &mode, &s.Purpose, &agentsJSON, &status, &created, &updated)
	if err != nil {
		return s, err
	}
	s.Mode = protocol.SessionMode(mode)
	s.Status = protocol.SessionStatus(status)
	s.Created = time.UnixMilli(created)
	s.Updated = time.UnixMilli(updated)
	if err := json.Unmarshal([]byte(agentsJSON), &s.Agents); err != nil {
		return s, err
	}
	return s, nil
}

func (db *DB) ListSessions(statusFilter string) ([]protocol.Session, error) {
	var rows *sql.Rows
	var err error
	if statusFilter != "" {
		rows, err = db.conn.Query(
			`SELECT id, mode, purpose, agents, status, created, updated FROM sessions WHERE status = ? ORDER BY updated DESC`, statusFilter,
		)
	} else {
		rows, err = db.conn.Query(
			`SELECT id, mode, purpose, agents, status, created, updated FROM sessions ORDER BY updated DESC`,
		)
	}
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var sessions []protocol.Session
	for rows.Next() {
		var s protocol.Session
		var mode, status, agentsJSON string
		var created, updated int64
		if err := rows.Scan(&s.ID, &mode, &s.Purpose, &agentsJSON, &status, &created, &updated); err != nil {
			return nil, err
		}
		s.Mode = protocol.SessionMode(mode)
		s.Status = protocol.SessionStatus(status)
		s.Created = time.UnixMilli(created)
		s.Updated = time.UnixMilli(updated)
		json.Unmarshal([]byte(agentsJSON), &s.Agents)
		sessions = append(sessions, s)
	}
	return sessions, nil
}

func (db *DB) JoinSession(sessionID, agentID string) error {
	sess, err := db.GetSession(sessionID)
	if err != nil {
		return err
	}
	for _, a := range sess.Agents {
		if a == agentID {
			return nil // already in session
		}
	}
	sess.Agents = append(sess.Agents, agentID)
	agentsJSON, _ := json.Marshal(sess.Agents)
	_, err = db.conn.Exec(
		`UPDATE sessions SET agents = ?, updated = ? WHERE id = ?`,
		string(agentsJSON), time.Now().UnixMilli(), sessionID,
	)
	return err
}
```

**Step 4: Run tests to verify they pass**

Run: `go test ./internal/hub/ -v`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add internal/hub/db.go internal/hub/db_test.go
git commit -m "feat(hub): add SQLite database layer with agents, sessions, messages"
```

---

## Phase 3: MCP Server

### Task 3.1: MCP Server with Hub Tools

**Files:**
- Create: `internal/mcp/server.go`
- Create: `internal/mcp/tools.go`
- Test: `internal/mcp/server_test.go`

**Step 1: Write the test**

Create `internal/mcp/server_test.go`:
```go
package mcp

import (
	"path/filepath"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

func TestToolsRegistered(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	tools := BuildTools(db)
	expectedTools := []string{
		"hub_register", "hub_unregister", "hub_send", "hub_read",
		"hub_agents", "hub_sessions", "hub_session_create",
		"hub_session_join", "hub_status", "hub_spawn",
	}

	toolNames := make(map[string]bool)
	for _, tool := range tools {
		toolNames[tool.Name] = true
	}

	for _, name := range expectedTools {
		if !toolNames[name] {
			t.Errorf("missing tool: %s", name)
		}
	}
}

func TestHubRegisterTool(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	tools := BuildTools(db)
	var registerTool *ToolHandler
	for i := range tools {
		if tools[i].Name == "hub_register" {
			registerTool = &tools[i]
			break
		}
	}
	if registerTool == nil {
		t.Fatal("hub_register tool not found")
	}

	result, err := registerTool.Handler(map[string]interface{}{
		"id":   "test-agent",
		"name": "Test Agent",
		"type": "coder",
	})
	if err != nil {
		t.Fatal(err)
	}

	// Verify agent was registered
	agent, err := db.GetAgent("test-agent")
	if err != nil {
		t.Fatal(err)
	}
	if agent.Name != "Test Agent" {
		t.Errorf("expected 'Test Agent', got %q", agent.Name)
	}
	_ = result
}

func TestHubSendAndReadTools(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	tools := BuildTools(db)
	toolMap := make(map[string]*ToolHandler)
	for i := range tools {
		toolMap[tools[i].Name] = &tools[i]
	}

	// Register two agents
	toolMap["hub_register"].Handler(map[string]interface{}{
		"id": "agent-a", "name": "Agent A", "type": "coder",
	})
	toolMap["hub_register"].Handler(map[string]interface{}{
		"id": "agent-b", "name": "Agent B", "type": "voice",
	})

	// Send message from A to B
	toolMap["hub_send"].Handler(map[string]interface{}{
		"from": "agent-a",
		"to":   "agent-b",
		"type": "question",
		"content": "Hello from A",
	})

	// Read messages for B
	result, err := toolMap["hub_read"].Handler(map[string]interface{}{
		"agent_id": "agent-b",
		"since_id": float64(0),
	})
	if err != nil {
		t.Fatal(err)
	}

	msgs, ok := result.([]map[string]interface{})
	if !ok {
		t.Fatalf("expected []map, got %T", result)
	}
	if len(msgs) != 1 {
		t.Fatalf("expected 1 message, got %d", len(msgs))
	}
	if msgs[0]["content"] != "Hello from A" {
		t.Errorf("expected 'Hello from A', got %v", msgs[0]["content"])
	}
}
```

**Step 2: Run test to verify it fails**

Run: `go test ./internal/mcp/ -v`
Expected: FAIL

**Step 3: Write implementation**

Create `internal/mcp/tools.go`:
```go
package mcp

import (
	"fmt"
	"os/exec"
	"time"

	"github.com/google/uuid"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

type ToolHandler struct {
	Name        string
	Description string
	Schema      map[string]interface{}
	Handler     func(args map[string]interface{}) (interface{}, error)
}

func getString(args map[string]interface{}, key string) string {
	v, ok := args[key]
	if !ok {
		return ""
	}
	s, _ := v.(string)
	return s
}

func getFloat(args map[string]interface{}, key string) float64 {
	v, ok := args[key]
	if !ok {
		return 0
	}
	f, _ := v.(float64)
	return f
}

func BuildTools(db *hub.DB) []ToolHandler {
	return []ToolHandler{
		{
			Name:        "hub_register",
			Description: "Register as an agent on the Bot-HQ hub",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"id":      map[string]interface{}{"type": "string", "description": "Unique agent ID"},
					"name":    map[string]interface{}{"type": "string", "description": "Display name"},
					"type":    map[string]interface{}{"type": "string", "description": "Agent type: coder, voice, brain, discord"},
					"project": map[string]interface{}{"type": "string", "description": "Current project path (optional)"},
				},
				"required": []string{"id", "name", "type"},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				agent := protocol.Agent{
					ID:      getString(args, "id"),
					Name:    getString(args, "name"),
					Type:    protocol.AgentType(getString(args, "type")),
					Status:  protocol.StatusOnline,
					Project: getString(args, "project"),
				}
				if err := db.RegisterAgent(agent); err != nil {
					return nil, err
				}
				return map[string]interface{}{"registered": agent.ID}, nil
			},
		},
		{
			Name:        "hub_unregister",
			Description: "Unregister from the Bot-HQ hub (go offline)",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"id": map[string]interface{}{"type": "string", "description": "Agent ID to unregister"},
				},
				"required": []string{"id"},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				if err := db.UnregisterAgent(getString(args, "id")); err != nil {
					return nil, err
				}
				return map[string]interface{}{"unregistered": getString(args, "id")}, nil
			},
		},
		{
			Name:        "hub_send",
			Description: "Send a message to another agent or session on the hub",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"from":       map[string]interface{}{"type": "string", "description": "Sender agent ID"},
					"to":         map[string]interface{}{"type": "string", "description": "Target agent ID (optional, broadcast if omitted)"},
					"session_id": map[string]interface{}{"type": "string", "description": "Session ID (optional)"},
					"type":       map[string]interface{}{"type": "string", "description": "Message type: question, response, command, update, result, error"},
					"content":    map[string]interface{}{"type": "string", "description": "Message content"},
				},
				"required": []string{"from", "type", "content"},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				msg := protocol.Message{
					SessionID: getString(args, "session_id"),
					FromAgent: getString(args, "from"),
					ToAgent:   getString(args, "to"),
					Type:      protocol.MessageType(getString(args, "type")),
					Content:   getString(args, "content"),
					Created:   time.Now(),
				}
				id, err := db.InsertMessage(msg)
				if err != nil {
					return nil, err
				}
				// Update last_seen for sender
				db.UpdateAgentStatus(msg.FromAgent, protocol.StatusOnline)
				return map[string]interface{}{"message_id": id}, nil
			},
		},
		{
			Name:        "hub_read",
			Description: "Read new messages addressed to you",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"agent_id": map[string]interface{}{"type": "string", "description": "Your agent ID"},
					"since_id": map[string]interface{}{"type": "number", "description": "Read messages after this ID (0 for all)"},
					"limit":    map[string]interface{}{"type": "number", "description": "Max messages to return (default 50)"},
				},
				"required": []string{"agent_id"},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				sinceID := int64(getFloat(args, "since_id"))
				limit := int(getFloat(args, "limit"))
				if limit <= 0 {
					limit = 50
				}
				msgs, err := db.ReadMessages(getString(args, "agent_id"), sinceID, limit)
				if err != nil {
					return nil, err
				}
				result := make([]map[string]interface{}, len(msgs))
				for i, m := range msgs {
					result[i] = map[string]interface{}{
						"id":         m.ID,
						"from":       m.FromAgent,
						"to":         m.ToAgent,
						"type":       string(m.Type),
						"content":    m.Content,
						"session_id": m.SessionID,
						"created":    m.Created.Format(time.RFC3339),
					}
				}
				return result, nil
			},
		},
		{
			Name:        "hub_agents",
			Description: "List agents registered on the hub",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"status": map[string]interface{}{"type": "string", "description": "Filter by status: online, working, idle, offline (optional)"},
				},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				agents, err := db.ListAgents(getString(args, "status"))
				if err != nil {
					return nil, err
				}
				result := make([]map[string]interface{}, len(agents))
				for i, a := range agents {
					result[i] = map[string]interface{}{
						"id":       a.ID,
						"name":     a.Name,
						"type":     string(a.Type),
						"status":   string(a.Status),
						"project":  a.Project,
						"last_seen": a.LastSeen.Format(time.RFC3339),
					}
				}
				return result, nil
			},
		},
		{
			Name:        "hub_sessions",
			Description: "List active sessions on the hub",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"status": map[string]interface{}{"type": "string", "description": "Filter by status: active, paused, done (optional)"},
				},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				sessions, err := db.ListSessions(getString(args, "status"))
				if err != nil {
					return nil, err
				}
				result := make([]map[string]interface{}, len(sessions))
				for i, s := range sessions {
					result[i] = map[string]interface{}{
						"id":      s.ID,
						"mode":    string(s.Mode),
						"purpose": s.Purpose,
						"agents":  s.Agents,
						"status":  string(s.Status),
					}
				}
				return result, nil
			},
		},
		{
			Name:        "hub_session_create",
			Description: "Create a new collaboration session",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"mode":    map[string]interface{}{"type": "string", "description": "Session mode: brainstorm, implement, chat"},
					"purpose": map[string]interface{}{"type": "string", "description": "What this session is about"},
					"agents":  map[string]interface{}{"type": "string", "description": "Comma-separated agent IDs to include"},
				},
				"required": []string{"mode", "purpose"},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				id := uuid.New().String()[:8]
				sess := protocol.Session{
					ID:      id,
					Mode:    protocol.SessionMode(getString(args, "mode")),
					Purpose: getString(args, "purpose"),
					Status:  protocol.SessionActive,
				}
				agentsStr := getString(args, "agents")
				if agentsStr != "" {
					for _, a := range splitComma(agentsStr) {
						sess.Agents = append(sess.Agents, a)
					}
				}
				if err := db.CreateSession(sess); err != nil {
					return nil, err
				}
				return map[string]interface{}{"session_id": id, "status": "active"}, nil
			},
		},
		{
			Name:        "hub_session_join",
			Description: "Join an existing session",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"session_id": map[string]interface{}{"type": "string", "description": "Session ID to join"},
					"agent_id":   map[string]interface{}{"type": "string", "description": "Your agent ID"},
				},
				"required": []string{"session_id", "agent_id"},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				if err := db.JoinSession(getString(args, "session_id"), getString(args, "agent_id")); err != nil {
					return nil, err
				}
				return map[string]interface{}{"joined": getString(args, "session_id")}, nil
			},
		},
		{
			Name:        "hub_status",
			Description: "Update your agent status",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"id":      map[string]interface{}{"type": "string", "description": "Your agent ID"},
					"status":  map[string]interface{}{"type": "string", "description": "New status: online, working, idle"},
					"project": map[string]interface{}{"type": "string", "description": "Current project (optional)"},
				},
				"required": []string{"id", "status"},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				status := protocol.AgentStatus(getString(args, "status"))
				if err := db.UpdateAgentStatus(getString(args, "id"), status); err != nil {
					return nil, err
				}
				return map[string]interface{}{"updated": getString(args, "id"), "status": string(status)}, nil
			},
		},
		{
			Name:        "hub_spawn",
			Description: "Spawn a new Claude Code session in tmux",
			Schema: map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"project": map[string]interface{}{"type": "string", "description": "Project directory path"},
					"prompt":  map[string]interface{}{"type": "string", "description": "Initial prompt to send to Claude Code (optional)"},
				},
				"required": []string{"project"},
			},
			Handler: func(args map[string]interface{}) (interface{}, error) {
				project := getString(args, "project")
				sessionName := fmt.Sprintf("claude-%s", uuid.New().String()[:6])

				// Create tmux session
				cmd := exec.Command("tmux", "new-session", "-d", "-s", sessionName, "-c", project)
				if err := cmd.Run(); err != nil {
					return nil, fmt.Errorf("failed to create tmux session: %w", err)
				}

				// Launch claude in the session
				cmd = exec.Command("tmux", "send-keys", "-t", sessionName, "claude", "Enter")
				if err := cmd.Run(); err != nil {
					return nil, fmt.Errorf("failed to start claude: %w", err)
				}

				// Send initial prompt if provided
				prompt := getString(args, "prompt")
				if prompt != "" {
					time.Sleep(3 * time.Second) // Wait for Claude to start
					cmd = exec.Command("tmux", "send-keys", "-t", sessionName, prompt, "Enter")
					cmd.Run()
				}

				return map[string]interface{}{
					"session":  sessionName,
					"project":  project,
					"status":   "spawned",
				}, nil
			},
		},
	}
}

func splitComma(s string) []string {
	var result []string
	for _, part := range split(s, ',') {
		trimmed := trim(part)
		if trimmed != "" {
			result = append(result, trimmed)
		}
	}
	return result
}

func split(s string, sep byte) []string {
	var parts []string
	start := 0
	for i := 0; i < len(s); i++ {
		if s[i] == sep {
			parts = append(parts, s[start:i])
			start = i + 1
		}
	}
	parts = append(parts, s[start:])
	return parts
}

func trim(s string) string {
	start, end := 0, len(s)
	for start < end && s[start] == ' ' {
		start++
	}
	for end > start && s[end-1] == ' ' {
		end--
	}
	return s[start:end]
}
```

Create `internal/mcp/server.go`:
```go
package mcp

import (
	"context"
	"fmt"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	mcpsdk "github.com/mark3labs/mcp-go/server"
	"github.com/mark3labs/mcp-go/mcp"
)

func RunStdioServer(db *hub.DB) error {
	s := mcpsdk.NewMCPServer(
		"bot-hq",
		protocol.Version,
	)

	tools := BuildTools(db)
	for _, tool := range tools {
		t := tool // capture
		mcpTool := mcp.NewTool(t.Name,
			mcp.WithDescription(t.Description),
			mcp.WithString("args", mcp.Description("JSON-encoded arguments")),
		)
		s.AddTool(mcpTool, func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
			args, _ := req.Params.Arguments["args"].(map[string]interface{})
			if args == nil {
				args = req.Params.Arguments
			}
			result, err := t.Handler(args)
			if err != nil {
				return mcp.NewToolResultError(err.Error()), nil
			}
			return mcp.NewToolResultText(fmt.Sprintf("%v", toJSON(result))), nil
		})
	}

	return mcpsdk.ServeStdio(s)
}
```

Note: The MCP server integration with `mcp-go` may need adjustments based on the exact API. The implementer should check `github.com/mark3labs/mcp-go` docs and adjust the tool registration accordingly. The key principle is: each tool handler wraps a `ToolHandler` from `tools.go`.

**Step 4: Run tests to verify they pass**

Run: `go test ./internal/mcp/ -v`
Expected: PASS

**Step 5: Commit**

```bash
git add internal/mcp/
git commit -m "feat(hub): add MCP server with all hub tools"
```

---

## Phase 4: Hub Core + Dispatcher

### Task 4.1: Hub Core with Notification Dispatch

**Files:**
- Create: `internal/hub/hub.go`
- Test: `internal/hub/hub_test.go`

**Step 1: Write the test**

Create `internal/hub/hub_test.go` (append to existing test file):
```go
// Add to existing hub_test.go or create new file

func TestHubDispatchToTmux(t *testing.T) {
	// This test verifies the hub can format dispatch commands
	// Actual tmux execution is tested in integration tests
	h := &Hub{}
	cmd := h.FormatTmuxMessage("claude-abc", protocol.Message{
		FromAgent: "live",
		Type:      protocol.MsgResponse,
		Content:   "JWT with refresh tokens",
	})
	if cmd == "" {
		t.Error("expected non-empty tmux command")
	}
}
```

**Step 2-5: Implement hub.go with dispatch logic, test, commit**

Create `internal/hub/hub.go` — the central hub struct that ties together DB, dispatch, and WebSocket. It:
- Holds the DB reference
- Sets up the `OnMessage` callback
- Routes messages to the right agent (tmux for Claude Code, WebSocket for Live)
- Manages agent lifecycle

```bash
git commit -m "feat(hub): add hub core with notification dispatcher"
```

---

### Task 4.2: Tmux Integration

**Files:**
- Create: `internal/tmux/tmux.go`
- Test: `internal/tmux/tmux_test.go`

Implement Go wrappers for: `send-keys`, `capture-pane`, `list-sessions`, `new-session`, `kill-session`. Test with actual tmux commands.

```bash
git commit -m "feat(hub): add tmux integration helpers"
```

---

## Phase 5: Terminal UI (Bubbletea)

### Task 5.1: App Shell with Tabs

**Files:**
- Create: `internal/ui/app.go`
- Create: `internal/ui/styles.go`

Implement the root Bubbletea model with tab switching (Tab key or number keys 1-4). Define Lipgloss styles for all colors (green, blue, purple, orange, red, gray, yellow).

```bash
git commit -m "feat(hub): add Bubbletea app shell with tab navigation"
```

### Task 5.2: Hub Tab — Message Feed

**Files:**
- Create: `internal/ui/hub_tab.go`

Scrollable message feed with color-coded entries. Command input at bottom. Subscribe to DB updates via hub's OnMessage callback.

```bash
git commit -m "feat(hub): add hub tab with color-coded message feed"
```

### Task 5.3: Agents Tab

**Files:**
- Create: `internal/ui/agents_tab.go`

Agent list with status dots (● green, ○ gray). Shows type, status, project, last activity.

```bash
git commit -m "feat(hub): add agents tab with status indicators"
```

### Task 5.4: Sessions Tab

**Files:**
- Create: `internal/ui/sessions_tab.go`

Session list with mode, agents, purpose, status. Selecting a session filters the hub tab.

```bash
git commit -m "feat(hub): add sessions tab"
```

### Task 5.5: Settings Tab

**Files:**
- Create: `internal/ui/settings_tab.go`

Editable config display. Reads/writes `~/.bot-hq/config.toml`.

```bash
git commit -m "feat(hub): add settings tab"
```

### Task 5.6: Wire Up Main Entry Point

**Files:**
- Modify: `cmd/bot-hq/main.go`

Wire together: open DB, start hub, start Bubbletea TUI, start WebSocket server, handle `mcp` subcommand.

```bash
git commit -m "feat(hub): wire up main entry point with all components"
```

---

## Phase 6: Bot-HQ Live (Web UI + Gemini Proxy)

### Task 6.1: WebSocket Server + Static File Serving

**Files:**
- Create: `internal/live/server.go`
- Create: `web/live/index.html`
- Create: `web/live/app.js`
- Create: `web/live/style.css`

HTTP server on `localhost:3847` that:
- Serves embedded static files (`go:embed`)
- Upgrades `/ws` to WebSocket for audio streaming
- Only connects to Gemini when a client connects

```bash
git commit -m "feat(hub): add Live web server with embedded static files"
```

### Task 6.2: Gemini Live API Proxy

**Files:**
- Create: `internal/live/gemini.go`

WebSocket proxy:
- Browser connects → hub dials Gemini Live API
- Forwards audio PCM between browser and Gemini
- Handles transcription events, tool calls
- Disconnects Gemini when browser disconnects

```bash
git commit -m "feat(hub): add Gemini Live API WebSocket proxy"
```

### Task 6.3: Live Web UI

**Files:**
- Modify: `web/live/index.html`
- Modify: `web/live/app.js`

Minimal voice UI:
- Mic capture via `getUserMedia` + `AudioWorklet`
- PCM encoding and WebSocket send
- Audio playback from Gemini responses
- Status indicator (connecting, listening, speaking)

**Note:** The user will test this themselves. Build it, ensure no build errors.

```bash
git commit -m "feat(hub): add Live web UI with audio capture and playback"
```

---

## Phase 7: Discord Integration

### Task 7.1: Discord Bot

**Files:**
- Create: `internal/discord/bot.go`
- Test: `internal/discord/bot_test.go`

Discord bot using `discordgo`:
- Connects with bot token from config
- Listens for messages in configured channel
- Routes to hub as messages (from_agent: "discord", content: message text)
- Sends hub responses back to Discord channel
- Registers as agent on hub

**QA:** Test using the Discord MCP tools to verify messages appear in the channel.

```bash
git commit -m "feat(hub): add Discord bot integration"
```

---

## Phase 8: /bot-hq Claude Code Skill

### Task 8.1: Create Global Skill

**Files:**
- Create: `~/.claude/skills/bot-hq/SKILL.md`

The skill tells Claude Code:
1. You're in a hub session — use `hub_send` for all communication
2. Brainstorm mode: send every question through the hub
3. Implement mode: work silently, notify on completion
4. Respect stop commands
5. Keep messages concise (they may be spoken)

```bash
# No git commit — this is installed in ~/.claude/skills/
```

---

## Phase 9: Integration + QA

### Task 9.1: Build Binary

Run:
```bash
go build -o bot-hq ./cmd/bot-hq
./bot-hq version
```

Expected: `bot-hq v0.1.0`

### Task 9.2: End-to-End Test — MCP Server

1. Start hub: `./bot-hq &`
2. In another terminal: `echo '{}' | ./bot-hq mcp` — verify it starts and accepts MCP messages
3. Verify `~/.bot-hq/hub.db` is created

### Task 9.3: End-to-End Test — Claude Code Integration

1. Add to Claude Code config:
   ```json
   { "mcpServers": { "bot-hq": { "command": "./bot-hq", "args": ["mcp"] } } }
   ```
2. Start Claude Code, verify `hub_register`, `hub_send`, `hub_read` tools are available
3. Register, send a message, read it back

### Task 9.4: End-to-End Test — Discord

1. Start hub with Discord config
2. Send message in #bot channel
3. Verify it appears in hub feed
4. Send message from hub to Discord
5. Verify it appears in #bot channel

### Task 9.5: Install Binary

```bash
go build -o bot-hq ./cmd/bot-hq
cp bot-hq /usr/local/bin/
```

### Task 9.6: Final Commit

```bash
git add -A
git commit -m "feat(hub): complete bot-hq hub v0.1.0"
```

---

## Phase Summary

| Phase | What | QA |
|-------|------|-----|
| 1 | Go project, protocol types, config | Unit tests |
| 2 | SQLite database layer | Unit tests (agents, messages, sessions, update_hook) |
| 3 | MCP server with 10 tools | Unit tests (tool handlers) |
| 4 | Hub core + tmux dispatch | Unit tests + manual tmux test |
| 5 | Bubbletea TUI (4 tabs) | Manual visual inspection |
| 6 | Live web server + Gemini proxy | User tests browser UI |
| 7 | Discord bot | Test via Discord MCP tools |
| 8 | /bot-hq Claude Code skill | Test in Claude Code session |
| 9 | Integration + build + install | End-to-end tests |
