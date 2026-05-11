// Package hub — Z-3d session-lifecycle queue.
//
// Bridges the bot-hq daemon process and per-agent MCP stdio subprocesses
// so a subprocess invocation of hub_session_open or hub_session_finalize
// can trigger daemon-side spawn machinery (brian.New + rain.New + Start
// for open; Stop + Discord archive for finalize).
//
// Why: the daemon's SessionOpenHook lives in the daemon process's
// memory; the MCP server runs as a per-agent stdio subprocess (per
// `.bot-hq-<agent>-mcp.json` config). Function pointers don't cross
// processes. Instead, the subprocess enqueues a row and polls for
// completion; the daemon's queue ticker drains pending rows, calls the
// hook, and writes the result back to the row. Subprocess parses the
// result and returns it to the MCP caller.
//
// Mirrors internal/hub/db_messages.go message_queue + drain-loop
// pattern. Same SQLite WAL serialization handles concurrent writers.
package hub

import (
	"database/sql"
	"fmt"
	"time"
)

// SessionLifecycleOp is a single queue row. Kind discriminates open vs
// finalize; the field set is the union of both ops (unused fields are
// zero-valued per op kind).
type SessionLifecycleOp struct {
	ID                int64
	Kind              string // "open" or "finalize"
	SessionID         string
	Project           string
	Scope             string
	PointerListJSON   string // []string serialized as JSON
	DiscordThreadID   string
	Force             bool
	Status            string // "pending" | "fired" | "failed"
	ResultJSON        string
	Created           time.Time
	ClaimedAt         time.Time
	FiredAt           time.Time
}

// EnqueueSessionOp inserts a pending row and returns its id. Subprocess
// callers use the returned id to poll for completion via GetSessionOp.
func (db *DB) EnqueueSessionOp(op SessionLifecycleOp) (int64, error) {
	if op.Kind != "open" && op.Kind != "finalize" {
		return 0, fmt.Errorf("EnqueueSessionOp: invalid kind %q", op.Kind)
	}
	forceInt := 0
	if op.Force {
		forceInt = 1
	}
	now := time.Now().UnixMilli()
	res, err := db.conn.Exec(`
		INSERT INTO session_lifecycle_queue
			(kind, session_id, project, scope, pointer_list_json, discord_thread_id, force, status, created)
		VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?)
	`, op.Kind, op.SessionID, op.Project, op.Scope, op.PointerListJSON, op.DiscordThreadID, forceInt, now)
	if err != nil {
		return 0, fmt.Errorf("enqueue session op: %w", err)
	}
	return res.LastInsertId()
}

// ClaimPendingSessionOps atomically marks up to `limit` pending rows
// with claimed_at=now() and returns them. Subsequent pollers won't
// re-pick the same rows (claimed_at != 0 acts as a soft lock).
//
// Returned ops have claimed_at populated; the caller is responsible for
// calling MarkSessionOpFired with the outcome.
func (db *DB) ClaimPendingSessionOps(limit int) ([]SessionLifecycleOp, error) {
	if limit <= 0 {
		limit = 10
	}
	now := time.Now().UnixMilli()
	tx, err := db.conn.Begin()
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()

	rows, err := tx.Query(`
		SELECT id, kind, session_id, project, scope, pointer_list_json,
		       discord_thread_id, force, status, result_json, created,
		       claimed_at, fired_at
		FROM session_lifecycle_queue
		WHERE status='pending' AND claimed_at=0
		ORDER BY id ASC
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	var ops []SessionLifecycleOp
	for rows.Next() {
		var op SessionLifecycleOp
		var forceInt int
		var created, claimedAt, firedAt int64
		if err := rows.Scan(&op.ID, &op.Kind, &op.SessionID, &op.Project, &op.Scope,
			&op.PointerListJSON, &op.DiscordThreadID, &forceInt, &op.Status,
			&op.ResultJSON, &created, &claimedAt, &firedAt); err != nil {
			rows.Close()
			return nil, err
		}
		op.Force = forceInt == 1
		op.Created = time.UnixMilli(created)
		if claimedAt > 0 {
			op.ClaimedAt = time.UnixMilli(claimedAt)
		}
		if firedAt > 0 {
			op.FiredAt = time.UnixMilli(firedAt)
		}
		ops = append(ops, op)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}

	// Stamp claimed_at on all returned rows in one UPDATE so subsequent
	// pollers see them as no-longer-claimable.
	for i := range ops {
		if _, err := tx.Exec(
			`UPDATE session_lifecycle_queue SET claimed_at=? WHERE id=?`,
			now, ops[i].ID,
		); err != nil {
			return nil, err
		}
		ops[i].ClaimedAt = time.UnixMilli(now)
	}

	if err := tx.Commit(); err != nil {
		return nil, err
	}
	return ops, nil
}

// GetSessionOp reads a single queue row by id. Used by subprocess
// pollers to check completion status.
func (db *DB) GetSessionOp(id int64) (SessionLifecycleOp, error) {
	var op SessionLifecycleOp
	var forceInt int
	var created, claimedAt, firedAt int64
	err := db.conn.QueryRow(`
		SELECT id, kind, session_id, project, scope, pointer_list_json,
		       discord_thread_id, force, status, result_json, created,
		       claimed_at, fired_at
		FROM session_lifecycle_queue
		WHERE id=?
	`, id).Scan(&op.ID, &op.Kind, &op.SessionID, &op.Project, &op.Scope,
		&op.PointerListJSON, &op.DiscordThreadID, &forceInt, &op.Status,
		&op.ResultJSON, &created, &claimedAt, &firedAt)
	if err != nil {
		if err == sql.ErrNoRows {
			return op, fmt.Errorf("session op %d not found", id)
		}
		return op, err
	}
	op.Force = forceInt == 1
	op.Created = time.UnixMilli(created)
	if claimedAt > 0 {
		op.ClaimedAt = time.UnixMilli(claimedAt)
	}
	if firedAt > 0 {
		op.FiredAt = time.UnixMilli(firedAt)
	}
	return op, nil
}

// MarkSessionOpFired finalizes a queue row with the daemon-hook outcome.
// status is "fired" (success) or "failed" (hook error); resultJSON is
// the JSON-encoded result blob the subprocess will parse.
func (db *DB) MarkSessionOpFired(id int64, status string, resultJSON string) error {
	if status != "fired" && status != "failed" {
		return fmt.Errorf("MarkSessionOpFired: invalid status %q", status)
	}
	now := time.Now().UnixMilli()
	_, err := db.conn.Exec(`
		UPDATE session_lifecycle_queue
		SET status=?, result_json=?, fired_at=?
		WHERE id=?
	`, status, resultJSON, now, id)
	if err != nil {
		return fmt.Errorf("mark session op fired: %w", err)
	}
	return nil
}
