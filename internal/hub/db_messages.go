package hub

import (
	"database/sql"
	"encoding/json"
	"errors"
	"log"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/snap"
)

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

// GetMessagesFromAgent returns hub messages emitted by the given agent
// (from_agent = fromAgent) since sinceID, in chronological order.
// Slice-5 H-22-bis item 4 helper for the egress-gap auditor: it needs to
// know whether the agent has produced any hub messages in a window,
// regardless of routing target. Mirrors ReadMessages' shape so callers
// can iterate uniformly.
func (db *DB) GetMessagesFromAgent(fromAgent string, sinceID int64, limit int) ([]protocol.Message, error) {
	if limit <= 0 {
		limit = 50
	}
	rows, err := db.conn.Query(
		`SELECT id, session_id, from_agent, to_agent, type, content, created
		 FROM messages
		 WHERE from_agent = ? AND id > ?
		 ORDER BY id ASC
		 LIMIT ?`,
		fromAgent, sinceID, limit,
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
	return msgs, rows.Err()
}

// GetMessageByID returns the hub message with the given ID. Returns
// (zero-value, false, nil) if no message with that ID exists. Used by
// the K-13 R12-pre-commit gate (toolgate package) to verify a
// commit-message-cited peer-greenflag-msg-id resolves to a real peer
// message.
//
// Phase K K-13.
func (db *DB) GetMessageByID(id int64) (protocol.Message, bool, error) {
	var m protocol.Message
	var typ string
	var created int64
	err := db.conn.QueryRow(
		`SELECT id, session_id, from_agent, to_agent, type, content, created
		 FROM messages
		 WHERE id = ?`,
		id,
	).Scan(&m.ID, &m.SessionID, &m.FromAgent, &m.ToAgent, &typ, &m.Content, &created)
	if err == sql.ErrNoRows {
		return protocol.Message{}, false, nil
	}
	if err != nil {
		return protocol.Message{}, false, err
	}
	m.Type = protocol.MessageType(typ)
	m.Created = time.UnixMilli(created)
	return m, true, nil
}

// HasRecentMessageFrom reports whether any message from the given agent
// has a created timestamp at or after `since`. Used by Emma's stale-coder
// detection (Phase I I-6) as a defense-in-depth backstop against the
// LastSeen-write-failure-race class — even if UpdateAgentLastSeen silently
// failed, a recent hub message from the agent proves they were active in
// the window. Cheap query: indexed by created via the messages table's
// implicit btree on the id PRIMARY KEY (id is monotonic with created in
// practice).
func (db *DB) HasRecentMessageFrom(fromAgent string, since time.Time) (bool, error) {
	sinceMillis := since.UnixMilli()
	var count int
	err := db.conn.QueryRow(
		`SELECT 1 FROM messages WHERE from_agent = ? AND created >= ? LIMIT 1`,
		fromAgent, sinceMillis,
	).Scan(&count)
	if err == sql.ErrNoRows {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return true, nil
}

// GetLatestMessageFrom returns the most recent message from the given
// agent, or (Message{}, false, nil) if no message exists. Used by Emma's
// stale-coder detection (Phase I I-6 C1) to check whether the latest user
// message is a HALT directive — if so, the duo is intentionally idle and
// stale-checks must suppress until user emits a non-HALT directive.
func (db *DB) GetLatestMessageFrom(fromAgent string) (protocol.Message, bool, error) {
	var m protocol.Message
	var typ string
	var created int64
	err := db.conn.QueryRow(
		`SELECT id, session_id, from_agent, to_agent, type, content, created
		 FROM messages
		 WHERE from_agent = ?
		 ORDER BY id DESC
		 LIMIT 1`,
		fromAgent,
	).Scan(&m.ID, &m.SessionID, &m.FromAgent, &m.ToAgent, &typ, &m.Content, &created)
	if err == sql.ErrNoRows {
		return protocol.Message{}, false, nil
	}
	if err != nil {
		return protocol.Message{}, false, err
	}
	m.Type = protocol.MessageType(typ)
	m.Created = time.UnixMilli(created)
	return m, true, nil
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
