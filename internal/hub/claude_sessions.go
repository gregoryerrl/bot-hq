package hub

import (
	"time"
)

// ClaudeSession represents a tracked Claude Code session running in tmux.
type ClaudeSession struct {
	ID         string    `json:"id"`
	Project    string    `json:"project"`
	TmuxTarget string    `json:"tmux_target"`
	PID        int       `json:"pid"`
	Mode       string    `json:"mode"`   // "managed" or "attached"
	Status     string    `json:"status"` // "running", "stopped", "busy"
	LastOutput string    `json:"last_output,omitempty"`
	Started    time.Time `json:"started"`
	Ended      time.Time `json:"ended,omitempty"`
}

// InsertClaudeSession creates a new Claude session record.
func (db *DB) InsertClaudeSession(sess ClaudeSession) error {
	if sess.Started.IsZero() {
		sess.Started = time.Now()
	}
	_, err := db.conn.Exec(
		`INSERT OR REPLACE INTO claude_sessions (id, project, tmux_target, pid, mode, status, last_output, started, ended)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		sess.ID, sess.Project, sess.TmuxTarget, sess.PID,
		sess.Mode, sess.Status, sess.LastOutput,
		sess.Started.UnixMilli(), sess.Ended.UnixMilli(),
	)
	return err
}

// GetClaudeSession retrieves a session by ID.
func (db *DB) GetClaudeSession(id string) (ClaudeSession, error) {
	var s ClaudeSession
	var started, ended int64
	err := db.conn.QueryRow(
		`SELECT id, project, tmux_target, pid, mode, status, last_output, started, ended
		 FROM claude_sessions WHERE id = ?`, id,
	).Scan(&s.ID, &s.Project, &s.TmuxTarget, &s.PID, &s.Mode, &s.Status, &s.LastOutput, &started, &ended)
	if err != nil {
		return s, err
	}
	s.Started = time.UnixMilli(started)
	if ended > 0 {
		s.Ended = time.UnixMilli(ended)
	}
	return s, nil
}

// ListClaudeSessions returns all sessions, optionally filtered by status.
func (db *DB) ListClaudeSessions(statusFilter string) ([]ClaudeSession, error) {
	var query string
	var args []any
	if statusFilter != "" {
		query = `SELECT id, project, tmux_target, pid, mode, status, last_output, started, ended
		         FROM claude_sessions WHERE status = ? ORDER BY started DESC`
		args = []any{statusFilter}
	} else {
		query = `SELECT id, project, tmux_target, pid, mode, status, last_output, started, ended
		         FROM claude_sessions ORDER BY started DESC`
	}

	rows, err := db.conn.Query(query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var sessions []ClaudeSession
	for rows.Next() {
		var s ClaudeSession
		var started, ended int64
		if err := rows.Scan(&s.ID, &s.Project, &s.TmuxTarget, &s.PID, &s.Mode, &s.Status, &s.LastOutput, &started, &ended); err != nil {
			return nil, err
		}
		s.Started = time.UnixMilli(started)
		if ended > 0 {
			s.Ended = time.UnixMilli(ended)
		}
		sessions = append(sessions, s)
	}
	return sessions, rows.Err()
}

// UpdateClaudeSessionStatus updates the status and optionally the output.
func (db *DB) UpdateClaudeSessionStatus(id, status, output string) error {
	if output != "" {
		_, err := db.conn.Exec(
			`UPDATE claude_sessions SET status = ?, last_output = ? WHERE id = ?`,
			status, output, id,
		)
		return err
	}
	_, err := db.conn.Exec(
		`UPDATE claude_sessions SET status = ? WHERE id = ?`,
		status, id,
	)
	return err
}

// StopClaudeSession marks a session as stopped.
func (db *DB) StopClaudeSession(id string) error {
	_, err := db.conn.Exec(
		`UPDATE claude_sessions SET status = 'stopped', ended = ? WHERE id = ?`,
		time.Now().UnixMilli(), id,
	)
	return err
}

// FindClaudeSessionByTarget finds a session by its tmux target.
func (db *DB) FindClaudeSessionByTarget(target string) (ClaudeSession, error) {
	var s ClaudeSession
	var started, ended int64
	err := db.conn.QueryRow(
		`SELECT id, project, tmux_target, pid, mode, status, last_output, started, ended
		 FROM claude_sessions WHERE tmux_target = ?`, target,
	).Scan(&s.ID, &s.Project, &s.TmuxTarget, &s.PID, &s.Mode, &s.Status, &s.LastOutput, &started, &ended)
	if err != nil {
		return s, err
	}
	s.Started = time.UnixMilli(started)
	if ended > 0 {
		s.Ended = time.UnixMilli(ended)
	}
	return s, nil
}
