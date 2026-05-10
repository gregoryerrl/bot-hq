package hub

import (
	"database/sql"
	"errors"
	"fmt"
	"strings"
)

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

	CREATE TABLE IF NOT EXISTS wake_schedule (
		id            INTEGER PRIMARY KEY AUTOINCREMENT,
		target_agent  TEXT NOT NULL,
		fire_at       INTEGER NOT NULL,
		payload       TEXT NOT NULL DEFAULT '',
		created_by    TEXT NOT NULL,
		created_at    INTEGER NOT NULL,
		fired_at      INTEGER NOT NULL DEFAULT 0,
		fire_status   TEXT NOT NULL DEFAULT 'pending'
		             CHECK(fire_status IN ('pending','fired','failed','cancelled'))
	);

	CREATE TABLE IF NOT EXISTS session_ledger (
		agent_id    TEXT PRIMARY KEY,
		snap_text   TEXT NOT NULL,
		closed_at   INTEGER NOT NULL
	);

	CREATE TABLE IF NOT EXISTS halt_state (
		cause   TEXT PRIMARY KEY,
		set_at  INTEGER NOT NULL,
		set_by  TEXT NOT NULL,
		reason  TEXT NOT NULL
	);

	-- Phase P P-9 / phase-n.md:818: pending-actions queue. Tracks
	-- user-actionable items derived from [HR]-tagged hub messages so
	-- the webui can surface a sidebar badge + popover list. Schema:
	-- one row per pending action; status enum {pending, ack, dismissed}.
	CREATE TABLE IF NOT EXISTS pending_actions (
		id          INTEGER PRIMARY KEY AUTOINCREMENT,
		agent_id    TEXT NOT NULL,
		kind        TEXT NOT NULL,
		summary     TEXT NOT NULL,
		msg_id      INTEGER DEFAULT 0,
		status      TEXT NOT NULL DEFAULT 'pending',
		created     INTEGER NOT NULL,
		updated     INTEGER NOT NULL
	);
	CREATE INDEX IF NOT EXISTS idx_pending_actions_status ON pending_actions(status, created DESC);
	`
	if _, err := db.conn.Exec(schema); err != nil {
		return err
	}

	// Phase H slice 5 C1 (H-32+H-33): migrate single-row halt_state
	// (id=1, active flag) to multi-row cause-keyed schema. Idempotent —
	// no-op when already on the new schema.
	if err := db.migrateHaltStateMultiCause(); err != nil {
		return err
	}

	// Phase H slice 3 C1 (#7): wake_schedule index. Partial-index needs SQLite
	// >=3.8.0 (per design doc O4). modernc.org/sqlite ships a recent embedded
	// build, so the partial form is the expected branch — but we probe at
	// runtime and fall back to a full composite index on older engines so an
	// embedded-version downgrade doesn't break migration.
	if err := db.createWakeScheduleIndex(); err != nil {
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
	// Phase H slice 3 C3 (#2): last_seen_msg_id column on agents. Stores the
	// MAX(messages.id) at register-time so the agent can self-filter incoming
	// msg.ID <= last_seen_msg_id as silent boot-replay (post-rebuild context
	// bootstrap pathology). Distinct from last_seen timestamp.
	if err := db.addColumnIfMissing("agents", "last_seen_msg_id", "INTEGER NOT NULL DEFAULT 0"); err != nil {
		return err
	}
	// Phase-R-followup (f): current_task column on agents. Agent-side write
	// declares an active multi-step work-thread; emma-stale checker treats
	// non-empty current_task as intentional-idle and suppresses
	// stale-coder PMs for the agent until cleared.
	if err := db.addColumnIfMissing("agents", "current_task", "TEXT NOT NULL DEFAULT ''"); err != nil {
		return err
	}
	// Phase T T-1.1: per-agent model-config table per R51 PER-AGENT-MODEL-CONFIG-DISCIPLINE
	// + R52 HUB-DB-CONFIG-DISCIPLINE. Single-source-of-truth for cross-model
	// agent configuration (Brian-Claude + Rain-DeepSeek-V4-Pro + emma/clive/coder-
	// template-Claude). Reference-pointer secret-storage; actual secrets resolve
	// from env-var/keychain/.env at agent-spawn-time.
	if err := db.migrateAgentModelConfigs(); err != nil {
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

// createWakeScheduleIndex installs the index Emma's tick loop relies on for
// the `WHERE fire_status='pending' AND fire_at <= ?` scan. Partial indexes
// (the narrow, scan-friendly form) require SQLite >=3.8.0; we probe via
// `SELECT sqlite_version()` and fall back to a full composite index on older
// engines. Either form keeps the tick query off a table scan; the partial
// variant is just leaner because non-pending rows never enter the b-tree.
func (db *DB) createWakeScheduleIndex() error {
	var ver string
	if err := db.conn.QueryRow("SELECT sqlite_version()").Scan(&ver); err != nil {
		return fmt.Errorf("query sqlite_version: %w", err)
	}
	stmt := `CREATE INDEX IF NOT EXISTS idx_wake_schedule_pending_fire_at
		ON wake_schedule(fire_at) WHERE fire_status = 'pending'`
	if !sqliteSupportsPartialIndex(ver) {
		stmt = `CREATE INDEX IF NOT EXISTS idx_wake_schedule_status_fire_at
			ON wake_schedule(fire_status, fire_at)`
	}
	if _, err := db.conn.Exec(stmt); err != nil {
		return fmt.Errorf("create wake_schedule index: %w", err)
	}
	return nil
}

// sqliteSupportsPartialIndex reports whether ver (e.g. "3.45.1") is at or
// above the 3.8.0 partial-index threshold. Defensive parser — anything
// unparseable falls back to the safe (full-index) branch.
func sqliteSupportsPartialIndex(ver string) bool {
	parts := strings.SplitN(ver, ".", 3)
	if len(parts) < 2 {
		return false
	}
	var major, minor int
	if _, err := fmt.Sscanf(parts[0], "%d", &major); err != nil {
		return false
	}
	if _, err := fmt.Sscanf(parts[1], "%d", &minor); err != nil {
		return false
	}
	if major > 3 {
		return true
	}
	if major < 3 {
		return false
	}
	return minor >= 8
}

// migrateHaltStateMultiCause migrates the legacy single-row halt_state schema
// (id=1 CHECK, active flag) to the multi-row cause-keyed schema. Idempotent:
// detects the legacy "id" column via PRAGMA table_info and rewrites the table
// in-place. Fresh DBs already have the new schema and skip migration.
//
// Migration path:
//  1. PRAGMA table_info(halt_state) — if columns include "id" (legacy schema):
//     a. Read the active=1 row (if any) into local vars.
//     b. DROP TABLE halt_state.
//     c. Recreate with new schema.
//     d. If the legacy row was active, INSERT cause='context-cap' preserving
//     set_at/set_by/reason — slice-4 callers fired exclusively on context-cap.
//  2. Otherwise (already new schema or empty pragma) — no-op.
func (db *DB) migrateHaltStateMultiCause() error {
	rows, err := db.conn.Query("PRAGMA table_info(halt_state)")
	if err != nil {
		return fmt.Errorf("pragma table_info(halt_state): %w", err)
	}
	defer rows.Close()
	hasIDColumn := false
	for rows.Next() {
		var cid int
		var name, typ string
		var notnull, pk int
		var dflt sql.NullString
		if err := rows.Scan(&cid, &name, &typ, &notnull, &dflt, &pk); err != nil {
			return err
		}
		if name == "id" {
			hasIDColumn = true
		}
	}
	if err := rows.Err(); err != nil {
		return err
	}
	if !hasIDColumn {
		return nil
	}

	// Legacy schema present. Capture the active row before recreating.
	var active int
	var setBy, reason string
	var setAt int64
	err = db.conn.QueryRow(
		`SELECT active, set_by, set_at, reason FROM halt_state WHERE id = 1`,
	).Scan(&active, &setBy, &setAt, &reason)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return fmt.Errorf("read legacy halt_state: %w", err)
	}

	tx, err := db.conn.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err := tx.Exec(`DROP TABLE halt_state`); err != nil {
		return fmt.Errorf("drop legacy halt_state: %w", err)
	}
	if _, err := tx.Exec(`
		CREATE TABLE halt_state (
			cause   TEXT PRIMARY KEY,
			set_at  INTEGER NOT NULL,
			set_by  TEXT NOT NULL,
			reason  TEXT NOT NULL
		)`); err != nil {
		return fmt.Errorf("recreate halt_state: %w", err)
	}
	if active == 1 {
		if setBy == "" {
			setBy = "emma"
		}
		if _, err := tx.Exec(
			`INSERT INTO halt_state (cause, set_at, set_by, reason) VALUES (?, ?, ?, ?)`,
			HaltCauseContextCap, setAt, setBy, reason,
		); err != nil {
			return fmt.Errorf("seed migrated context-cap row: %w", err)
		}
	}
	return tx.Commit()
}
