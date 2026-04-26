package hub

import (
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/snap"
	_ "modernc.org/sqlite"
)

// sqlIdentifierRE matches the SQLite identifier subset used by addColumnIfMissing.
// Compile-time literal table/column names always pass; dynamic input fails fast.
var sqlIdentifierRE = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)

// Checkpoint represents a persisted agent state snapshot.
type Checkpoint struct {
	AgentID string    `json:"agent_id"`
	Data    string    `json:"data"`
	Version int       `json:"version"`
	Created time.Time `json:"created"`
	Updated time.Time `json:"updated"`
}

// QueuedMessage represents a message waiting to be delivered to a busy agent.
type QueuedMessage struct {
	ID            int64
	MessageID     int64
	TargetAgent   string
	TmuxTarget    string
	FormattedText string
	Attempts      int
	MaxAttempts   int
	Status        string
	Created       time.Time
	LastAttempt   time.Time
}

type DB struct {
	conn       *sql.DB
	mu         sync.RWMutex
	onMessages []func(protocol.Message)
	// rebuildMu serializes IncrementRebuildGen's read-modify-write across
	// goroutines in this process. The generation itself lives in the settings
	// table — there is no in-memory cache, so cross-process readers (e.g. the
	// MCP subcommand process) always see the authoritative value.
	rebuildMu sync.Mutex
}

func OpenDB(path string) (*DB, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return nil, err
	}

	// Pre-create the DB file with restrictive permissions if it doesn't exist
	if _, err := os.Stat(path); os.IsNotExist(err) {
		f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY, 0600)
		if err != nil {
			return nil, fmt.Errorf("create db file: %w", err)
		}
		f.Close()
	}

	conn, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, err
	}

	// Force single connection so PRAGMAs apply to all operations.
	// SQLite only allows one writer at a time anyway; extra pool connections
	// would not inherit PRAGMA settings and cause SQLITE_BUSY under contention.
	conn.SetMaxOpenConns(1)

	// Apply PRAGMAs explicitly — modernc.org/sqlite ignores DSN parameters
	pragmas := []string{
		"PRAGMA journal_mode=WAL",
		"PRAGMA busy_timeout=5000",
		"PRAGMA synchronous=NORMAL",
	}
	for _, p := range pragmas {
		if _, err := conn.Exec(p); err != nil {
			conn.Close()
			return nil, fmt.Errorf("pragma %q: %w", p, err)
		}
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

// OnMessage registers a callback that fires whenever a message is inserted.
// Multiple callbacks can be registered; they are all called in order.
func (db *DB) OnMessage(fn func(protocol.Message)) {
	db.mu.Lock()
	defer db.mu.Unlock()
	db.onMessages = append(db.onMessages, fn)
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

	CREATE TABLE IF NOT EXISTS claude_sessions (
		id          TEXT PRIMARY KEY,
		project     TEXT NOT NULL,
		tmux_target TEXT NOT NULL,
		pid         INTEGER DEFAULT 0,
		mode        TEXT NOT NULL,
		status      TEXT NOT NULL,
		last_output TEXT DEFAULT '',
		started     INTEGER NOT NULL,
		ended       INTEGER DEFAULT 0
	);

	CREATE TABLE IF NOT EXISTS settings (
		key         TEXT PRIMARY KEY,
		value       TEXT NOT NULL,
		updated     INTEGER NOT NULL
	);

	CREATE TABLE IF NOT EXISTS message_queue (
		id              INTEGER PRIMARY KEY AUTOINCREMENT,
		message_id      INTEGER NOT NULL,
		target_agent    TEXT NOT NULL,
		tmux_target     TEXT NOT NULL,
		formatted_text  TEXT NOT NULL,
		attempts        INTEGER DEFAULT 0,
		max_attempts    INTEGER DEFAULT 30,
		status          TEXT DEFAULT 'pending',
		created         TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
		last_attempt    TIMESTAMP
	);

	CREATE INDEX IF NOT EXISTS idx_messages_to ON messages(to_agent, id);
	CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, id);
	CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created);
	CREATE TABLE IF NOT EXISTS checkpoints (
		agent_id    TEXT PRIMARY KEY,
		data        TEXT NOT NULL,
		version     INTEGER NOT NULL DEFAULT 1,
		created     INTEGER NOT NULL,
		updated     INTEGER NOT NULL
	);

	CREATE TABLE IF NOT EXISTS issues (
		id          TEXT PRIMARY KEY,
		reporter    TEXT NOT NULL,
		severity    TEXT NOT NULL CHECK(severity IN ('low','medium','high','critical')),
		title       TEXT NOT NULL,
		description TEXT,
		file_path   TEXT,
		line_number INTEGER,
		status      TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','in_progress','fixed','wontfix','duplicate')),
		assigned_to TEXT,
		resolution  TEXT,
		created     INTEGER NOT NULL,
		updated     INTEGER NOT NULL
	);
	CREATE INDEX IF NOT EXISTS idx_issues_status ON issues(status);
	CREATE INDEX IF NOT EXISTS idx_issues_severity ON issues(severity);
	CREATE INDEX IF NOT EXISTS idx_issues_reporter ON issues(reporter);

	CREATE INDEX IF NOT EXISTS idx_mq_status ON message_queue(status);
	CREATE INDEX IF NOT EXISTS idx_mq_target ON message_queue(target_agent);
	`
	if _, err := db.conn.Exec(schema); err != nil {
		return err
	}

	// Phase G v1 #20: rebuild_gen column on agents. Guarded ALTER for
	// idempotent migration on existing DBs.
	if err := db.addColumnIfMissing("agents", "rebuild_gen", "INTEGER NOT NULL DEFAULT 0"); err != nil {
		return err
	}
	// Phase G v1 slice 2 C2: snap_json column on messages. Stores the
	// serialized SNAP footer extracted by the send-path hook (slice 2 C3).
	// Empty string for messages with no SNAP block or with malformed blocks
	// (parse-error policy: log+warn, do not fail the send).
	if err := db.addColumnIfMissing("messages", "snap_json", "TEXT NOT NULL DEFAULT ''"); err != nil {
		return err
	}
	return nil
}

// addColumnIfMissing applies an ALTER TABLE only when the named column is
// absent on the given table. Lets the migration block run unchanged on
// fresh DBs (CREATE TABLE already includes nothing extra) and on upgraded
// DBs (ALTER adds the column once, subsequent runs are no-ops).
//
// Identifier guard: table and column must match `^[A-Za-z_][A-Za-z0-9_]*$`.
// All current call sites pass compile-time literals, so this only fires if
// a future contributor introduces dynamic identifiers — at which point we
// want a hard failure, not a silent SQL-injection vector. decl is not
// validated (free-form SQL type/constraint clause is the design intent).
func (db *DB) addColumnIfMissing(table, column, decl string) error {
	if !sqlIdentifierRE.MatchString(table) {
		return fmt.Errorf("addColumnIfMissing: invalid table name %q", table)
	}
	if !sqlIdentifierRE.MatchString(column) {
		return fmt.Errorf("addColumnIfMissing: invalid column name %q", column)
	}
	pragma := fmt.Sprintf("PRAGMA table_info(%s)", table)
	rows, err := db.conn.Query(pragma)
	if err != nil {
		return fmt.Errorf("pragma table_info(%s): %w", table, err)
	}
	defer rows.Close()
	for rows.Next() {
		var cid int
		var name, typ string
		var notnull, pk int
		var dflt sql.NullString
		if err := rows.Scan(&cid, &name, &typ, &notnull, &dflt, &pk); err != nil {
			return err
		}
		if name == column {
			return nil // already present
		}
	}
	if err := rows.Err(); err != nil {
		return err
	}
	stmt := fmt.Sprintf("ALTER TABLE %s ADD COLUMN %s %s", table, column, decl)
	if _, err := db.conn.Exec(stmt); err != nil {
		return fmt.Errorf("alter %s add %s: %w", table, column, err)
	}
	return nil
}

// IncrementRebuildGen reads hub_rebuild_gen from settings, increments it by
// one, persists, and returns the new generation number. Called once at hub
// startup (NewHub).
//
// Concurrency: serialized by rebuildMu so a hypothetical double-call from
// two goroutines in this process doesn't double-bump. Cross-process callers
// rely on SQLite's busy_timeout for serialization at the storage layer.
func (db *DB) IncrementRebuildGen() (int64, error) {
	db.rebuildMu.Lock()
	defer db.rebuildMu.Unlock()

	var current int64
	row := db.conn.QueryRow(`SELECT value FROM settings WHERE key = ?`, "hub_rebuild_gen")
	var raw string
	switch err := row.Scan(&raw); err {
	case nil:
		fmt.Sscanf(raw, "%d", &current)
	case sql.ErrNoRows:
		current = 0
	default:
		return 0, err
	}

	next := current + 1
	now := time.Now().UnixMilli()
	if _, err := db.conn.Exec(
		`INSERT INTO settings (key, value, updated) VALUES (?, ?, ?)
		 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated = excluded.updated`,
		"hub_rebuild_gen", fmt.Sprintf("%d", next), now,
	); err != nil {
		return 0, err
	}
	return next, nil
}

// CurrentRebuildGen reads hub_rebuild_gen from the settings table. Returns 0
// if the row is absent (pre-feature, or hub never started yet).
//
// Reads from settings on every call so cross-process callers (e.g. the MCP
// subcommand process, which never calls IncrementRebuildGen itself) see the
// authoritative gen, not a stale per-process cache.
func (db *DB) CurrentRebuildGen() int64 {
	var raw string
	row := db.conn.QueryRow(`SELECT value FROM settings WHERE key = ?`, "hub_rebuild_gen")
	if err := row.Scan(&raw); err != nil {
		return 0
	}
	var gen int64
	fmt.Sscanf(raw, "%d", &gen)
	return gen
}

// --- Agents ---

func (db *DB) RegisterAgent(agent protocol.Agent) error {
	now := time.Now().UnixMilli()
	if agent.Registered.IsZero() {
		agent.Registered = time.Now()
	}
	gen := db.CurrentRebuildGen()
	_, err := db.conn.Exec(
		`INSERT OR REPLACE INTO agents (id, name, type, status, project, meta, registered, last_seen, rebuild_gen)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		agent.ID, agent.Name, string(agent.Type), string(agent.Status),
		agent.Project, agent.Meta, agent.Registered.UnixMilli(), now, gen,
	)
	return err
}

func (db *DB) GetAgent(id string) (protocol.Agent, error) {
	var a protocol.Agent
	var typ, status string
	var registered, lastSeen int64
	err := db.conn.QueryRow(
		`SELECT id, name, type, status, project, meta, registered, last_seen, rebuild_gen FROM agents WHERE id = ?`, id,
	).Scan(&a.ID, &a.Name, &typ, &status, &a.Project, &a.Meta, &registered, &lastSeen, &a.RebuildGen)
	if err != nil {
		return a, err
	}
	a.Type = protocol.AgentType(typ)
	a.Status = protocol.AgentStatus(status)
	a.Registered = time.UnixMilli(registered)
	a.LastSeen = time.UnixMilli(lastSeen)
	return a, nil
}

func (db *DB) UpdateAgentStatus(id string, status protocol.AgentStatus, project ...string) error {
	now := time.Now().UnixMilli()
	if len(project) > 0 && project[0] != "" {
		_, err := db.conn.Exec(
			`UPDATE agents SET status = ?, project = ?, last_seen = ? WHERE id = ?`,
			string(status), project[0], now, id,
		)
		return err
	}
	_, err := db.conn.Exec(
		`UPDATE agents SET status = ?, last_seen = ? WHERE id = ?`,
		string(status), now, id,
	)
	return err
}

func (db *DB) UnregisterAgent(id string) error {
	return db.UpdateAgentStatus(id, protocol.StatusOffline)
}

// staleCoderCutoff is the last_seen age beyond which an online coder row
// is treated as a ghost regardless of claude_session state. Tuned wide
// enough to never sweep a healthy coder (active sessions update last_seen
// per MCP tool call via UpdateAgentLastSeen) but tight enough to keep
// Emma's anomaly checks from flagging long-stale rows on every restart.
const staleCoderCutoff = 7 * 24 * time.Hour

// ReconcileCoderGhosts flips ghost coder agents back to offline. Two
// cleanup paths share one query for atomicity and a single count return:
//
//  1. Coders whose paired claude_session row is already "stopped" — the
//     bug-#4-era leak path that this function originally targeted.
//  2. Coders whose last_seen is older than staleCoderCutoff — covers
//     pre-existing stale rows from before the bug-#4 fix landed and any
//     future drift where a coder dies without its session marker being
//     written.
//
// Idempotent. Wired post-OpenDB at boot. Returns the count of rows
// flipped so callers can log it.
//
// Pure DB scope by design — does NOT use tmux discovery. The discovery-
// based reconciliation lives in claudeList and conflates registration
// with cleanup; this is the cleanup-only path with no IO contract.
func (db *DB) ReconcileCoderGhosts() (int, error) {
	staleCutoffMillis := time.Now().Add(-staleCoderCutoff).UnixMilli()
	res, err := db.conn.Exec(`
		UPDATE agents SET status = ?
		WHERE type = ? AND status = ?
		  AND (
		    id IN (SELECT id FROM claude_sessions WHERE status = ?)
		    OR last_seen < ?
		  )
	`,
		string(protocol.StatusOffline),
		string(protocol.AgentCoder),
		string(protocol.StatusOnline),
		"stopped",
		staleCutoffMillis,
	)
	if err != nil {
		return 0, err
	}
	n, _ := res.RowsAffected()
	return int(n), nil
}

// UpdateAgentLastSeen touches only the last_seen timestamp, leaving status
// and project intact. Used by the MCP middleware in internal/mcp/tools.go to
// auto-refresh activity recency on every tool call without disturbing status
// transitions.
//
// Phase F prerequisite: heartbeat goroutine (when added) calls this on a
// timer for agents that don't initiate MCP calls (e.g. dormant coders
// awaiting input). See docs/plans/phase-e.md §6.
func (db *DB) UpdateAgentLastSeen(id string) error {
	now := time.Now().UnixMilli()
	_, err := db.conn.Exec(
		`UPDATE agents SET last_seen = ? WHERE id = ?`,
		now, id,
	)
	return err
}

// DeleteAgent permanently removes an agent record from the database.
func (db *DB) DeleteAgent(id string) error {
	_, err := db.conn.Exec(`DELETE FROM agents WHERE id = ?`, id)
	return err
}

func (db *DB) ListAgents(statusFilter string) ([]protocol.Agent, error) {
	var rows *sql.Rows
	var err error
	if statusFilter != "" {
		rows, err = db.conn.Query(
			`SELECT id, name, type, status, project, meta, registered, last_seen, rebuild_gen FROM agents WHERE status = ? ORDER BY last_seen DESC`, statusFilter,
		)
	} else {
		rows, err = db.conn.Query(
			`SELECT id, name, type, status, project, meta, registered, last_seen, rebuild_gen FROM agents ORDER BY last_seen DESC`,
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
		if err := rows.Scan(&a.ID, &a.Name, &typ, &status, &a.Project, &a.Meta, &registered, &lastSeen, &a.RebuildGen); err != nil {
			return nil, err
		}
		a.Type = protocol.AgentType(typ)
		a.Status = protocol.AgentStatus(status)
		a.Registered = time.UnixMilli(registered)
		a.LastSeen = time.UnixMilli(lastSeen)
		agents = append(agents, a)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	return agents, nil
}

// --- Messages ---

// extractSnapJSON parses a SNAP footer from msg content and returns the
// canonical JSON form. Empty string is returned when no SNAP block is
// present (the common case for non-orchestrator messages) or when the
// block is malformed.
//
// Parse-error policy: log+warn + empty. We do not fail the insert on a
// malformed SNAP — the message itself is still useful, the structured
// indexable form just isn't available. Logging surfaces footer-convention
// drift (per Rain's pre-dispatch refinement #1) without polluting storage
// with raw fragments.
func extractSnapJSON(fromAgent, content string) string {
	s, err := snap.Parse(content)
	if err != nil {
		if errors.Is(err, snap.ErrNoSNAPBlock) {
			return "" // common case, not a drift signal
		}
		log.Printf("[snap] warn: parse failed for message from %s: %v", fromAgent, err)
		return ""
	}
	out, err := json.Marshal(s)
	if err != nil {
		log.Printf("[snap] warn: marshal failed for message from %s: %v", fromAgent, err)
		return ""
	}
	return string(out)
}

func (db *DB) InsertMessage(msg protocol.Message) (int64, error) {
	if msg.Created.IsZero() {
		msg.Created = time.Now()
	}
	snapJSON := extractSnapJSON(msg.FromAgent, msg.Content)
	result, err := db.conn.Exec(
		`INSERT INTO messages (session_id, from_agent, to_agent, type, content, created, snap_json)
		 VALUES (?, ?, ?, ?, ?, ?, ?)`,
		msg.SessionID, msg.FromAgent, msg.ToAgent, string(msg.Type), msg.Content, msg.Created.UnixMilli(), snapJSON,
	)
	if err != nil {
		return 0, err
	}

	id, err := result.LastInsertId()
	if err != nil {
		return 0, err
	}

	msg.ID = id

	// Fire update hooks
	db.mu.RLock()
	fns := make([]func(protocol.Message), len(db.onMessages))
	copy(fns, db.onMessages)
	db.mu.RUnlock()
	for _, fn := range fns {
		go fn(msg)
	}

	return id, nil
}

// ReadMessages returns hub messages for an agent (or all if agentID="").
// sinceID<=0 returns the latest N rows in chronological order (tail mode for
// fresh-start callers). sinceID>0 returns rows with id>sinceID in chronological
// order (incremental polling). Stable contract relied on by brian/rain pollers.
func (db *DB) ReadMessages(agentID string, sinceID int64, limit int) ([]protocol.Message, error) {
	if limit <= 0 {
		limit = 50
	}
	tail := sinceID <= 0
	var rows *sql.Rows
	var err error
	if agentID == "" {
		if tail {
			rows, err = db.conn.Query(
				`SELECT id, session_id, from_agent, to_agent, type, content, created
				 FROM messages
				 ORDER BY id DESC
				 LIMIT ?`,
				limit,
			)
		} else {
			rows, err = db.conn.Query(
				`SELECT id, session_id, from_agent, to_agent, type, content, created
				 FROM messages
				 WHERE id > ?
				 ORDER BY id ASC
				 LIMIT ?`,
				sinceID, limit,
			)
		}
	} else {
		if tail {
			rows, err = db.conn.Query(
				`SELECT id, session_id, from_agent, to_agent, type, content, created
				 FROM messages
				 WHERE (to_agent = ? OR to_agent = '')
				 ORDER BY id DESC
				 LIMIT ?`,
				agentID, limit,
			)
		} else {
			rows, err = db.conn.Query(
				`SELECT id, session_id, from_agent, to_agent, type, content, created
				 FROM messages
				 WHERE (to_agent = ? OR to_agent = '') AND id > ?
				 ORDER BY id ASC
				 LIMIT ?`,
				agentID, sinceID, limit,
			)
		}
	}
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
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if tail {
		for i, j := 0, len(msgs)-1; i < j; i, j = i+1, j-1 {
			msgs[i], msgs[j] = msgs[j], msgs[i]
		}
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
	if err := rows.Err(); err != nil {
		return nil, err
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
		if err := json.Unmarshal([]byte(agentsJSON), &s.Agents); err != nil {
			return nil, err
		}
		sessions = append(sessions, s)
	}
	if err := rows.Err(); err != nil {
		return nil, err
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
	agentsJSON, err := json.Marshal(sess.Agents)
	if err != nil {
		return fmt.Errorf("marshal agents: %w", err)
	}
	_, err = db.conn.Exec(
		`UPDATE sessions SET agents = ?, updated = ? WHERE id = ?`,
		string(agentsJSON), time.Now().UnixMilli(), sessionID,
	)
	return err
}

// --- Settings ---

// GetSetting returns the value for a setting key, or defaultVal if not found.
func (db *DB) GetSetting(key, defaultVal string) string {
	var val string
	err := db.conn.QueryRow(`SELECT value FROM settings WHERE key = ?`, key).Scan(&val)
	if err != nil {
		return defaultVal
	}
	return val
}

// SetSetting upserts a setting key-value pair.
func (db *DB) SetSetting(key, value string) error {
	_, err := db.conn.Exec(
		`INSERT INTO settings (key, value, updated) VALUES (?, ?, ?)
		 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated = excluded.updated`,
		key, value, time.Now().UnixMilli(),
	)
	return err
}

// GetAllSettings returns all settings as a map.
func (db *DB) GetAllSettings() (map[string]string, error) {
	rows, err := db.conn.Query(`SELECT key, value FROM settings ORDER BY key`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	settings := make(map[string]string)
	for rows.Next() {
		var k, v string
		if err := rows.Scan(&k, &v); err != nil {
			return nil, err
		}
		settings[k] = v
	}
	return settings, rows.Err()
}

// --- Message Queue ---

// EnqueueMessage adds a message to the retry queue for later delivery.
func (db *DB) EnqueueMessage(messageID int64, targetAgent, tmuxTarget, formattedText string) error {
	_, err := db.conn.Exec(
		`INSERT INTO message_queue (message_id, target_agent, tmux_target, formatted_text)
		 VALUES (?, ?, ?, ?)`,
		messageID, targetAgent, tmuxTarget, formattedText,
	)
	return err
}

// GetPendingMessages returns all pending queued messages ordered by creation time.
func (db *DB) GetPendingMessages() ([]QueuedMessage, error) {
	rows, err := db.conn.Query(
		`SELECT id, message_id, target_agent, tmux_target, formatted_text, attempts, max_attempts, status, created, last_attempt
		 FROM message_queue WHERE status = 'pending' ORDER BY created ASC`,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var msgs []QueuedMessage
	for rows.Next() {
		var qm QueuedMessage
		var created string
		var lastAttempt sql.NullString
		if err := rows.Scan(&qm.ID, &qm.MessageID, &qm.TargetAgent, &qm.TmuxTarget,
			&qm.FormattedText, &qm.Attempts, &qm.MaxAttempts, &qm.Status,
			&created, &lastAttempt); err != nil {
			return nil, err
		}
		qm.Created, _ = time.Parse(time.DateTime, created)
		if lastAttempt.Valid {
			qm.LastAttempt, _ = time.Parse(time.DateTime, lastAttempt.String)
		}
		msgs = append(msgs, qm)
	}
	return msgs, rows.Err()
}

// GetPendingMessagesForAgent returns pending queued messages for a specific agent.
func (db *DB) GetPendingMessagesForAgent(agentID string) ([]QueuedMessage, error) {
	rows, err := db.conn.Query(
		`SELECT id, message_id, target_agent, tmux_target, formatted_text, attempts, max_attempts, status, created, last_attempt
		 FROM message_queue WHERE status = 'pending' AND target_agent = ? ORDER BY created ASC`, agentID,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var msgs []QueuedMessage
	for rows.Next() {
		var qm QueuedMessage
		var created string
		var lastAttempt sql.NullString
		if err := rows.Scan(&qm.ID, &qm.MessageID, &qm.TargetAgent, &qm.TmuxTarget,
			&qm.FormattedText, &qm.Attempts, &qm.MaxAttempts, &qm.Status,
			&created, &lastAttempt); err != nil {
			return nil, err
		}
		qm.Created, _ = time.Parse(time.DateTime, created)
		if lastAttempt.Valid {
			qm.LastAttempt, _ = time.Parse(time.DateTime, lastAttempt.String)
		}
		msgs = append(msgs, qm)
	}
	return msgs, rows.Err()
}

// UpdateQueueStatus updates the status and attempt count of a queued message.
func (db *DB) UpdateQueueStatus(id int64, status string, attempts int) error {
	_, err := db.conn.Exec(
		`UPDATE message_queue SET status = ?, attempts = ?, last_attempt = CURRENT_TIMESTAMP WHERE id = ?`,
		status, attempts, id,
	)
	return err
}

// CleanDeliveredMessages removes delivered queue entries older than the given duration.
func (db *DB) CleanDeliveredMessages(olderThan time.Duration) error {
	cutoff := time.Now().Add(-olderThan).Format(time.DateTime)
	_, err := db.conn.Exec(
		`DELETE FROM message_queue WHERE status = 'delivered' AND created < ?`,
		cutoff,
	)
	return err
}

// --- Checkpoints ---

// SaveCheckpoint upserts a checkpoint for an agent. data must be a valid JSON string.
func (db *DB) SaveCheckpoint(agentID, data string) error {
	if !json.Valid([]byte(data)) {
		return fmt.Errorf("invalid JSON data")
	}
	now := time.Now().UnixMilli()
	_, err := db.conn.Exec(
		`INSERT INTO checkpoints (agent_id, data, version, created, updated) VALUES (?, ?, 1, ?, ?)
		 ON CONFLICT(agent_id) DO UPDATE SET data=excluded.data, version=version+1, updated=excluded.updated`,
		agentID, data, now, now,
	)
	return err
}

// GetCheckpoint retrieves the checkpoint for an agent.
func (db *DB) GetCheckpoint(agentID string) (Checkpoint, error) {
	var cp Checkpoint
	var created, updated int64
	err := db.conn.QueryRow(
		`SELECT agent_id, data, version, created, updated FROM checkpoints WHERE agent_id = ?`, agentID,
	).Scan(&cp.AgentID, &cp.Data, &cp.Version, &created, &updated)
	if err != nil {
		return cp, err
	}
	cp.Created = time.UnixMilli(created)
	cp.Updated = time.UnixMilli(updated)
	return cp, nil
}

// DeleteCheckpoint removes a checkpoint for an agent.
func (db *DB) DeleteCheckpoint(agentID string) error {
	_, err := db.conn.Exec(`DELETE FROM checkpoints WHERE agent_id = ?`, agentID)
	return err
}

// --- Issues ---

// CreateIssue inserts a new issue and returns it as a map.
func (db *DB) CreateIssue(id, reporter, severity, title, description, filePath string, lineNumber *int) (map[string]interface{}, error) {
	now := time.Now().UnixMilli()
	var lineNumArg interface{}
	if lineNumber != nil {
		lineNumArg = *lineNumber
	}
	_, err := db.conn.Exec(
		`INSERT INTO issues (id, reporter, severity, title, description, file_path, line_number, status, created, updated)
		 VALUES (?, ?, ?, ?, ?, ?, ?, 'open', ?, ?)`,
		id, reporter, severity, title, description, filePath, lineNumArg, now, now,
	)
	if err != nil {
		return nil, err
	}
	issue := map[string]interface{}{
		"id":          id,
		"reporter":    reporter,
		"severity":    severity,
		"title":       title,
		"description": description,
		"file_path":   filePath,
		"status":      "open",
		"assigned_to": "",
		"resolution":  "",
		"created":     now,
		"updated":     now,
	}
	if lineNumber != nil {
		issue["line_number"] = *lineNumber
	} else {
		issue["line_number"] = nil
	}
	return issue, nil
}

// ListIssues queries issues with optional filters.
func (db *DB) ListIssues(status, severity, reporter string) ([]map[string]interface{}, error) {
	query := `SELECT id, reporter, severity, title, description, file_path, line_number, status, assigned_to, resolution, created, updated FROM issues`
	var conditions []string
	var args []interface{}

	if status != "" {
		conditions = append(conditions, "status = ?")
		args = append(args, status)
	}
	if severity != "" {
		conditions = append(conditions, "severity = ?")
		args = append(args, severity)
	}
	if reporter != "" {
		conditions = append(conditions, "reporter = ?")
		args = append(args, reporter)
	}

	if len(conditions) > 0 {
		query += " WHERE " + strings.Join(conditions, " AND ")
	}
	query += " ORDER BY created DESC"

	rows, err := db.conn.Query(query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var issues []map[string]interface{}
	for rows.Next() {
		var id, rep, sev, title, st string
		var desc, fp, assignedTo, resolution sql.NullString
		var lineNum sql.NullInt64
		var created, updated int64
		if err := rows.Scan(&id, &rep, &sev, &title, &desc, &fp, &lineNum, &st, &assignedTo, &resolution, &created, &updated); err != nil {
			return nil, err
		}
		issue := map[string]interface{}{
			"id":          id,
			"reporter":    rep,
			"severity":    sev,
			"title":       title,
			"description": desc.String,
			"file_path":   fp.String,
			"status":      st,
			"assigned_to": assignedTo.String,
			"resolution":  resolution.String,
			"created":     created,
			"updated":     updated,
		}
		if lineNum.Valid {
			issue["line_number"] = lineNum.Int64
		} else {
			issue["line_number"] = nil
		}
		issues = append(issues, issue)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	return issues, nil
}

// UpdateIssue updates specified fields on an issue and returns the updated issue.
func (db *DB) UpdateIssue(id, status, assignedTo, resolution string) (map[string]interface{}, error) {
	now := time.Now().UnixMilli()
	var sets []string
	var args []interface{}

	if status != "" {
		sets = append(sets, "status = ?")
		args = append(args, status)
	}
	if assignedTo != "" {
		sets = append(sets, "assigned_to = ?")
		args = append(args, assignedTo)
	}
	if resolution != "" {
		sets = append(sets, "resolution = ?")
		args = append(args, resolution)
	}

	if len(sets) == 0 {
		return nil, fmt.Errorf("no fields to update")
	}

	sets = append(sets, "updated = ?")
	args = append(args, now)
	args = append(args, id)

	query := "UPDATE issues SET " + strings.Join(sets, ", ") + " WHERE id = ?"
	result, err := db.conn.Exec(query, args...)
	if err != nil {
		return nil, err
	}
	affected, _ := result.RowsAffected()
	if affected == 0 {
		return nil, fmt.Errorf("issue not found: %s", id)
	}

	// Read back the updated issue
	var rep, sev, title, st string
	var desc, fp, assignTo, res sql.NullString
	var lineNum sql.NullInt64
	var created, updated int64
	err = db.conn.QueryRow(
		`SELECT id, reporter, severity, title, description, file_path, line_number, status, assigned_to, resolution, created, updated FROM issues WHERE id = ?`, id,
	).Scan(&id, &rep, &sev, &title, &desc, &fp, &lineNum, &st, &assignTo, &res, &created, &updated)
	if err != nil {
		return nil, err
	}
	issue := map[string]interface{}{
		"id":          id,
		"reporter":    rep,
		"severity":    sev,
		"title":       title,
		"description": desc.String,
		"file_path":   fp.String,
		"status":      st,
		"assigned_to": assignTo.String,
		"resolution":  res.String,
		"created":     created,
		"updated":     updated,
	}
	if lineNum.Valid {
		issue["line_number"] = lineNum.Int64
	} else {
		issue["line_number"] = nil
	}
	return issue, nil
}
