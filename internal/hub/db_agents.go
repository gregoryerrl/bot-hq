package hub

import (
	"database/sql"
	"errors"
	"log"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

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

// RegisterAgentWithWatermark registers an agent and atomically captures the
// current MAX(messages.id) as the agent's replay-cutoff watermark. Returns
// the watermark so the caller (typically hub_register) can include it in the
// tool response, letting the agent silently discard incoming msg.ID <=
// watermark as boot-replay of pre-bounce history.
//
// Phase H slice 3 C3 (#2): post-rebuild context-bootstrap-replay pathology —
// without this, agents bounced via rebuild see the inbound hub event flood as
// N replayed broadcasts and cannot distinguish "fresh real-time" from
// "boot-replay of resolved history." The watermark is the agent-side filter.
//
// Atomicity: insert-or-replace + SELECT MAX + UPDATE happen in one tx.
// SQLite write serialization (MaxOpenConns=1 + busy_timeout) plus the tx
// boundary guarantee no concurrent message INSERT slips between the SELECT
// and the watermark UPDATE — the returned watermark equals the highest
// committed msg.id that existed at register-commit-time.
func (db *DB) RegisterAgentWithWatermark(agent protocol.Agent) (int64, string, error) {
	if agent.Registered.IsZero() {
		agent.Registered = time.Now()
	}
	now := time.Now().UnixMilli()
	gen := db.CurrentRebuildGen()

	tx, err := db.conn.Begin()
	if err != nil {
		return 0, "", err
	}
	defer tx.Rollback()

	if _, err := tx.Exec(
		`INSERT OR REPLACE INTO agents (id, name, type, status, project, meta, registered, last_seen, rebuild_gen, last_seen_msg_id)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0)`,
		agent.ID, agent.Name, string(agent.Type), string(agent.Status),
		agent.Project, agent.Meta, agent.Registered.UnixMilli(), now, gen,
	); err != nil {
		return 0, "", err
	}

	var maxID sql.NullInt64
	if err := tx.QueryRow(`SELECT MAX(id) FROM messages`).Scan(&maxID); err != nil {
		return 0, "", err
	}
	watermark := maxID.Int64 // 0 when messages table is empty (NULL → 0)

	if _, err := tx.Exec(
		`UPDATE agents SET last_seen_msg_id = ? WHERE id = ?`,
		watermark, agent.ID,
	); err != nil {
		return 0, "", err
	}

	// Phase H slice 4 C4 (H-15): fetch this agent's last voluntarily-stored
	// SNAP from session_ledger so the cold-start caller can pre-load it as
	// boot context. Empty string when no prior close ever fired (first
	// registration, or rebuild-mid-flight skipped the voluntary call).
	var snap sql.NullString
	if err := tx.QueryRow(
		`SELECT snap_text FROM session_ledger WHERE agent_id = ?`, agent.ID,
	).Scan(&snap); err != nil && !errors.Is(err, sql.ErrNoRows) {
		return 0, "", err
	}

	if err := tx.Commit(); err != nil {
		return 0, "", err
	}

	// Phase H slice 4 C6 (H-31): causality-only halt-state auto-clear. After
	// commit, check whether this register advances the trio past the active
	// halt's set_at; if every currently-registered trio member's last_seen is
	// now past set_at, clear. Best-effort — clear errors are logged but never
	// fail the register.
	if cleared, cerr := db.ClearHaltIfTrioReregistered(HaltStateTrio); cerr != nil {
		log.Printf("[halt-state] auto-clear check failed: %v", cerr)
	} else if cleared {
		log.Printf("[halt-state] auto-cleared after %s re-register completed trio", agent.ID)
	}

	return watermark, snap.String, nil
}

func (db *DB) GetAgent(id string) (protocol.Agent, error) {
	var a protocol.Agent
	var typ, status string
	var registered, lastSeen int64
	err := db.conn.QueryRow(
		`SELECT id, name, type, status, project, meta, registered, last_seen, rebuild_gen, current_task FROM agents WHERE id = ?`, id,
	).Scan(&a.ID, &a.Name, &typ, &status, &a.Project, &a.Meta, &registered, &lastSeen, &a.RebuildGen, &a.CurrentTask)
	if err != nil {
		return a, err
	}
	a.Type = protocol.AgentType(typ)
	a.Status = protocol.AgentStatus(status)
	a.Registered = time.UnixMilli(registered)
	a.LastSeen = time.UnixMilli(lastSeen)
	return a, nil
}

// SetAgentCurrentTask writes the agent's current_task field. Empty
// string clears the field (intentional-idle declaration ends).
// Phase-R-followup (f): emma-stale checker treats non-empty
// current_task as intentional-idle and suppresses stale-coder PMs.
func (db *DB) SetAgentCurrentTask(agentID, task string) error {
	_, err := db.conn.Exec(
		`UPDATE agents SET current_task = ? WHERE id = ?`,
		task, agentID,
	)
	return err
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

// PruneStaleOfflineAgents deletes agent rows whose status is 'offline' and
// last_seen older than threshold, returning the IDs removed for audit. Live
// agents (status='online' or 'working') are protected by the status filter
// — only confirmed-offline rows are eligible.
//
// Phase H slice 3 H-25 (Emma roster hygiene). Distinct from
// ReconcileCoderGhosts (which only flips status, never deletes); this path
// reclaims long-dead rows so the agents table doesn't accumulate forever.
func (db *DB) PruneStaleOfflineAgents(threshold time.Duration) ([]string, error) {
	cutoffMillis := time.Now().Add(-threshold).UnixMilli()
	rows, err := db.conn.Query(
		`SELECT id FROM agents WHERE status = ? AND last_seen < ?`,
		string(protocol.StatusOffline), cutoffMillis,
	)
	if err != nil {
		return nil, err
	}
	var ids []string
	for rows.Next() {
		var id string
		if err := rows.Scan(&id); err != nil {
			rows.Close()
			return nil, err
		}
		ids = append(ids, id)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if len(ids) == 0 {
		return nil, nil
	}
	if _, err := db.conn.Exec(
		`DELETE FROM agents WHERE status = ? AND last_seen < ?`,
		string(protocol.StatusOffline), cutoffMillis,
	); err != nil {
		return nil, err
	}
	return ids, nil
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
			`SELECT id, name, type, status, project, meta, registered, last_seen, rebuild_gen, current_task FROM agents WHERE status = ? ORDER BY last_seen DESC`, statusFilter,
		)
	} else {
		rows, err = db.conn.Query(
			`SELECT id, name, type, status, project, meta, registered, last_seen, rebuild_gen, current_task FROM agents ORDER BY last_seen DESC`,
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
		if err := rows.Scan(&a.ID, &a.Name, &typ, &status, &a.Project, &a.Meta, &registered, &lastSeen, &a.RebuildGen, &a.CurrentTask); err != nil {
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
