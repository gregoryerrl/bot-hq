package hub

import (
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// GetLastSessionSnap returns the snap_text last stored via StoreSessionClose
// for the given agent, or "" when no row exists. Phase H slice 4 C4 (H-15).
func (db *DB) GetLastSessionSnap(agentID string) (string, error) {
	var snap sql.NullString
	err := db.conn.QueryRow(
		`SELECT snap_text FROM session_ledger WHERE agent_id = ?`, agentID,
	).Scan(&snap)
	if errors.Is(err, sql.ErrNoRows) {
		return "", nil
	}
	if err != nil {
		return "", err
	}
	return snap.String, nil
}

// StoreSessionClose upserts the agent's final SNAP into session_ledger.
// Best-effort, voluntary-only — agents call this before graceful idle so the
// next register (typically post-rebuild fresh-context) can pre-load it as
// cold-start context. Phase H slice 4 C4 (H-15).
func (db *DB) StoreSessionClose(agentID, snapText string) error {
	if agentID == "" {
		return errors.New("StoreSessionClose: agent_id must not be empty")
	}
	now := time.Now().UnixMilli()
	_, err := db.conn.Exec(
		`INSERT INTO session_ledger (agent_id, snap_text, closed_at)
		 VALUES (?, ?, ?)
		 ON CONFLICT(agent_id) DO UPDATE SET
		   snap_text = excluded.snap_text,
		   closed_at = excluded.closed_at`,
		agentID, snapText, now,
	)
	return err
}

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

// UpdateSessionStatus transitions the sessions-table row to a new
// status (active → done on finalize). Idempotent on no-op; ignored
// when the session-id is absent (Z-3 hub_session_open writes a row
// during open, but legacy paths may not — best-effort).
func (db *DB) UpdateSessionStatus(sessionID, status string) error {
	_, err := db.conn.Exec(
		`UPDATE sessions SET status = ?, updated = ? WHERE id = ?`,
		status, time.Now().UnixMilli(), sessionID,
	)
	return err
}
