package hub

// Pending-actions queue per Phase P P-9 / phase-n.md:818. Backs the
// webui sidebar widget showing user-actionable items derived from
// [HR]-tagged hub messages. Schema: pending_actions table at
// internal/hub/db.go:262 migration block.
//
// Status semantics:
//   - "pending": newly-created; visible in sidebar badge count
//   - "ack":     user clicked ack; hidden from default queries but kept
//                for audit trail (List with includeAcked=true returns)
//   - "dismissed": optional future state for explicit hide (not yet wired)
//
// Persistence: rows survive bot-hq daemon restart per fork (d-i)
// "cross-restart persisted" pick at user msg user-fork-pick proceed
// → all-Brian-leans (msg 15028 surface).

import (
	"database/sql"
	"fmt"
	"time"
)

// PendingAction is one queue entry. Persisted in pending_actions
// table; serialized as JSON for the GET /api/pending-actions
// endpoint.
type PendingAction struct {
	ID      int64  `json:"id"`
	AgentID string `json:"agent_id"`        // who emitted the source message
	Kind    string `json:"kind"`            // hr-broadcast / hub-flag / etc
	Summary string `json:"summary"`         // truncated content snippet
	MsgID   int64  `json:"msg_id,omitempty"` // source hub message id (0 if synthetic)
	Status  string `json:"status"`          // pending / ack / dismissed
	Created int64  `json:"created"`         // unix milliseconds
	Updated int64  `json:"updated"`         // unix milliseconds
}

// pendingSummaryMaxLen caps the stored summary length so a single
// huge [HR] payload doesn't bloat the queue table.
const pendingSummaryMaxLen = 280

// InsertPendingAction creates a new queue row. Returns the assigned
// id. Auto-truncates summary at pendingSummaryMaxLen.
func (db *DB) InsertPendingAction(agentID, kind, summary string, msgID int64) (int64, error) {
	if len(summary) > pendingSummaryMaxLen {
		summary = summary[:pendingSummaryMaxLen] + "…"
	}
	now := time.Now().UnixMilli()
	res, err := db.conn.Exec(
		`INSERT INTO pending_actions(agent_id, kind, summary, msg_id, status, created, updated)
		 VALUES(?, ?, ?, ?, 'pending', ?, ?)`,
		agentID, kind, summary, msgID, now, now,
	)
	if err != nil {
		return 0, fmt.Errorf("insert pending_action: %w", err)
	}
	id, err := res.LastInsertId()
	if err != nil {
		return 0, fmt.Errorf("last insert id: %w", err)
	}
	return id, nil
}

// ListPendingActions returns queue entries with status='pending'
// (or all when includeAcked=true) in reverse-chronological order.
// limit caps return; <=0 → 50 default; >500 → 500 hard cap.
func (db *DB) ListPendingActions(limit int, includeAcked bool) ([]PendingAction, error) {
	if limit <= 0 {
		limit = 50
	}
	if limit > 500 {
		limit = 500
	}
	var rows *sql.Rows
	var err error
	if includeAcked {
		rows, err = db.conn.Query(
			`SELECT id, agent_id, kind, summary, msg_id, status, created, updated
			 FROM pending_actions
			 ORDER BY created DESC, id DESC
			 LIMIT ?`,
			limit,
		)
	} else {
		rows, err = db.conn.Query(
			`SELECT id, agent_id, kind, summary, msg_id, status, created, updated
			 FROM pending_actions
			 WHERE status = 'pending'
			 ORDER BY created DESC, id DESC
			 LIMIT ?`,
			limit,
		)
	}
	if err != nil {
		return nil, fmt.Errorf("list pending_actions: %w", err)
	}
	defer rows.Close()
	var out []PendingAction
	for rows.Next() {
		var p PendingAction
		if err := rows.Scan(&p.ID, &p.AgentID, &p.Kind, &p.Summary, &p.MsgID, &p.Status, &p.Created, &p.Updated); err != nil {
			return nil, err
		}
		out = append(out, p)
	}
	return out, rows.Err()
}

// CountPendingActions returns the count of status='pending' rows —
// drives the sidebar badge.
func (db *DB) CountPendingActions() (int, error) {
	var n int
	err := db.conn.QueryRow(`SELECT COUNT(*) FROM pending_actions WHERE status = 'pending'`).Scan(&n)
	if err != nil {
		return 0, fmt.Errorf("count pending_actions: %w", err)
	}
	return n, nil
}

// AckPendingAction transitions a row from pending → ack. Returns
// (true, nil) if a row was updated; (false, nil) if the id didn't
// exist or wasn't in pending status.
func (db *DB) AckPendingAction(id int64) (bool, error) {
	now := time.Now().UnixMilli()
	res, err := db.conn.Exec(
		`UPDATE pending_actions SET status = 'ack', updated = ? WHERE id = ? AND status = 'pending'`,
		now, id,
	)
	if err != nil {
		return false, fmt.Errorf("ack pending_action: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	return n > 0, nil
}
