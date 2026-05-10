package hub

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"strings"
	"time"
)

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
		var lastAttempt sql.NullTime
		if err := rows.Scan(&qm.ID, &qm.MessageID, &qm.TargetAgent, &qm.TmuxTarget,
			&qm.FormattedText, &qm.Attempts, &qm.MaxAttempts, &qm.Status,
			&qm.Created, &lastAttempt); err != nil {
			return nil, err
		}
		if lastAttempt.Valid {
			qm.LastAttempt = lastAttempt.Time
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
		var lastAttempt sql.NullTime
		if err := rows.Scan(&qm.ID, &qm.MessageID, &qm.TargetAgent, &qm.TmuxTarget,
			&qm.FormattedText, &qm.Attempts, &qm.MaxAttempts, &qm.Status,
			&qm.Created, &lastAttempt); err != nil {
			return nil, err
		}
		if lastAttempt.Valid {
			qm.LastAttempt = lastAttempt.Time
		}
		msgs = append(msgs, qm)
	}
	return msgs, rows.Err()
}

// SetQueueRowCreatedForTest overrides the `created` column on a queued
// message row, identified by message_id. Test-only helper: lets tests
// age a row past deliveryGapAge / egress thresholds without sleeping.
// Lives on the production type because callers in other packages'
// `_test.go` files need it, but the `ForTest` suffix is the call-site
// signal — never invoke from production paths.
func (db *DB) SetQueueRowCreatedForTest(messageID int64, t time.Time) error {
	_, err := db.conn.Exec(
		`UPDATE message_queue SET created = ? WHERE message_id = ?`,
		t.UTC().Format(time.DateTime), messageID,
	)
	return err
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

// --- Wake schedule (Phase H slice 3 C1 #7) ---

// HasPendingWakeForTarget reports whether any wake_schedule row exists for
// targetAgent in fire_status='pending'. Used by C7 (#6 H-23 periodic
// invoker) bootstrap to avoid double-scheduling _internal:docdrift across
// rebuilds when a prior boot's pending wake is still in the table.
func (db *DB) HasPendingWakeForTarget(target string) (bool, error) {
	var one int
	err := db.conn.QueryRow(
		`SELECT 1 FROM wake_schedule WHERE target_agent = ? AND fire_status = 'pending' LIMIT 1`,
		target,
	).Scan(&one)
	if err == sql.ErrNoRows {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return true, nil
}

// InsertWakeSchedule persists a pending wake row and returns the assigned
// row id. Caller is responsible for ISO 8601 parsing — this layer takes a
// time.Time. Status is always 'pending' on insert; transitions go through
// MarkWakeFired / MarkWakeFailed / CancelWake.
func (db *DB) InsertWakeSchedule(targetAgent, createdBy, payload string, fireAt time.Time) (int64, error) {
	res, err := db.conn.Exec(
		`INSERT INTO wake_schedule (target_agent, fire_at, payload, created_by, created_at, fire_status)
		 VALUES (?, ?, ?, ?, ?, 'pending')`,
		targetAgent, fireAt.UnixMilli(), payload, createdBy, time.Now().UnixMilli(),
	)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// ListPendingWakes returns wake rows whose fire_at is at or before the given
// instant and whose status is still pending. Caller (Emma tick loop) iterates
// the result and dispatches each via hub_send. Bounded scan via the index
// installed in migrate().
func (db *DB) ListPendingWakes(asOf time.Time) ([]WakeSchedule, error) {
	rows, err := db.conn.Query(
		`SELECT id, target_agent, fire_at, payload, created_by, created_at, fired_at, fire_status
		 FROM wake_schedule
		 WHERE fire_status = 'pending' AND fire_at <= ?
		 ORDER BY fire_at ASC, id ASC`,
		asOf.UnixMilli(),
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []WakeSchedule
	for rows.Next() {
		var w WakeSchedule
		var fireAt, createdAt, firedAt int64
		if err := rows.Scan(&w.ID, &w.TargetAgent, &fireAt, &w.Payload, &w.CreatedBy, &createdAt, &firedAt, &w.FireStatus); err != nil {
			return nil, err
		}
		w.FireAt = time.UnixMilli(fireAt)
		w.CreatedAt = time.UnixMilli(createdAt)
		if firedAt > 0 {
			w.FiredAt = time.UnixMilli(firedAt)
		}
		out = append(out, w)
	}
	return out, rows.Err()
}

// GetWakeSchedule fetches a single row by id. sql.ErrNoRows when absent.
func (db *DB) GetWakeSchedule(id int64) (WakeSchedule, error) {
	var w WakeSchedule
	var fireAt, createdAt, firedAt int64
	err := db.conn.QueryRow(
		`SELECT id, target_agent, fire_at, payload, created_by, created_at, fired_at, fire_status
		 FROM wake_schedule WHERE id = ?`, id,
	).Scan(&w.ID, &w.TargetAgent, &fireAt, &w.Payload, &w.CreatedBy, &createdAt, &firedAt, &w.FireStatus)
	if err != nil {
		return w, err
	}
	w.FireAt = time.UnixMilli(fireAt)
	w.CreatedAt = time.UnixMilli(createdAt)
	if firedAt > 0 {
		w.FiredAt = time.UnixMilli(firedAt)
	}
	return w, nil
}

// markWakeTerminal flips a pending row to a terminal state (fired/failed/
// cancelled). Pending-only WHERE clause makes the call idempotent — a
// concurrent transition (e.g. cancel racing fire) leaves the second caller
// with RowsAffected==0 instead of corrupting the state machine. Returns
// whether this call performed the transition.
func (db *DB) markWakeTerminal(id int64, status string, firedAt int64) (bool, error) {
	res, err := db.conn.Exec(
		`UPDATE wake_schedule SET fire_status = ?, fired_at = ?
		 WHERE id = ? AND fire_status = 'pending'`,
		status, firedAt, id,
	)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	return n > 0, nil
}

// MarkWakeFired records a successful dispatch. Sets fired_at = now and flips
// status to 'fired'. Returns true if the row was pending; false if it had
// already moved to a terminal state (cancel race).
func (db *DB) MarkWakeFired(id int64) (bool, error) {
	return db.markWakeTerminal(id, WakeStatusFired, time.Now().UnixMilli())
}

// MarkWakeFailed records a hub_send failure. fired_at is left at zero so the
// row distinguishes "tried and failed" (status=failed, fired_at=0) from
// "successfully fired" (status=fired, fired_at>0).
func (db *DB) MarkWakeFailed(id int64) (bool, error) {
	return db.markWakeTerminal(id, WakeStatusFailed, 0)
}

// CancelWake transitions a pending wake to 'cancelled'. Idempotent on rows
// that have already left the pending state — returns (false, nil) so the MCP
// tool can report status=already_terminal without surfacing an error. A
// missing id surfaces as sql.ErrNoRows.
func (db *DB) CancelWake(id int64) (bool, error) {
	if _, err := db.GetWakeSchedule(id); err != nil {
		return false, err
	}
	return db.markWakeTerminal(id, WakeStatusCancelled, 0)
}

// CancelPendingWakesForTargetByPayloadPrefix cancels every pending wake for
// the given target whose payload starts with the supplied prefix. Returns
// the number of rows transitioned to 'cancelled'.
//
// Phase J post-rebuild fix (2026-04-29): Emma's emitPlanCapResume calls this
// at auto-clear time to prevent the accumulated-wake-spam observed when
// maxUtil oscillates around halt-threshold (each oscillation cycle scheduled
// a +5h+1min RESUME wake; ~30min of jitter accumulated 200+ pending rows
// that all fired at fire_at). Once Emma emits RESUME via the auto-clear
// path, any pending future RESUME wakes for the same agents are redundant
// belt-and-suspenders that just re-spam the agent's pane.
func (db *DB) CancelPendingWakesForTargetByPayloadPrefix(target, prefix string) (int, error) {
	res, err := db.conn.Exec(
		`UPDATE wake_schedule SET fire_status = ? WHERE fire_status = ? AND target_agent = ? AND payload LIKE ?`,
		WakeStatusCancelled, WakeStatusPending, target, prefix+"%",
	)
	if err != nil {
		return 0, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return 0, err
	}
	return int(n), nil
}
