package hub

import (
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
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

// WakeSchedule is a row in the wake_schedule table — the persisted
// agentic time-trigger primitive landed in Phase H slice 3 C1 (#7).
//
// State machine (per design doc O6, locked at slice-3 design open):
//
//	pending → fired      (Emma tick-loop dispatched payload via hub_send)
//	pending → failed     (Emma hub_send returned error; drop, no retry per arch lean 4)
//	pending → cancelled  (caller invoked hub_cancel_wake before dispatch)
//
// No other transitions in v1. Future state additions ("retrying", etc.)
// require state-machine consistency review.
type WakeSchedule struct {
	ID          int64
	TargetAgent string
	FireAt      time.Time
	Payload     string
	CreatedBy   string
	CreatedAt   time.Time
	FiredAt     time.Time // zero if not yet fired
	FireStatus  string
}

// Wake-schedule status constants. The CHECK constraint on the table column
// pins these values; anything else is a migration-class change.
const (
	WakeStatusPending   = "pending"
	WakeStatusFired     = "fired"
	WakeStatusFailed    = "failed"
	WakeStatusCancelled = "cancelled"
)

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

// MessageExists reports whether a message with the given id exists in the
// messages table. Used by citeanchor.HubMsgChecker (Phase T T-1.9) to
// validate msg-id citations in scope-lock-docs.
func (db *DB) MessageExists(id int64) (bool, error) {
	var exists int
	err := db.conn.QueryRow("SELECT 1 FROM messages WHERE id = ? LIMIT 1", id).Scan(&exists)
	if err == sql.ErrNoRows {
		return false, nil
	}
	if err != nil {
		return false, fmt.Errorf("query message %d: %w", id, err)
	}
	return exists == 1, nil
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

// HaltStateTrio names the agents whose joint re-registration auto-clears an
// active context-cap halt_state. Causality-only: a halt set by Emma at T_h
// clears once every currently-registered trio member has last_seen > T_h.
// Members absent from the agents table (pruned/never-registered) are excluded
// from the comparison set so a partial trio can still produce a clear; an
// empty comparison set never clears (would be a false positive).
//
// Phase H slice 4 C6 (H-31). Slice 5 C1 (H-33) scopes the auto-clear to
// cause="context-cap" only — plan-cap clears organically via window-rollover
// or poll-shows-decay, not via re-register.
var HaltStateTrio = []string{"brian", "rain", "clive"}

// Halt cause constants. cause is the primary key of halt_state in the
// multi-row cause-keyed schema (Phase H slice 5 C1). Each cause can be
// independently active; IsHalted() reports whether any cause is active.
const (
	HaltCauseContextCap = "context-cap"
	HaltCausePlanCap    = "plan-cap"
)

// HaltState is one row of the cause-keyed halt_state table. Returned by
// GetHaltCause for production callers (e.g. trio-re-register clear path).
type HaltState struct {
	Cause  string
	SetAt  time.Time
	SetBy  string
	Reason string
}
