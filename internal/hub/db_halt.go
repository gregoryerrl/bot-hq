package hub

import (
	"database/sql"
	"errors"
	"time"
)

// IsHalted reports whether any cause is currently active in halt_state.
// Backed by EXISTS so the predicate stays cheap regardless of how many
// causes accumulate.
func (db *DB) IsHalted() (bool, error) {
	var one int
	err := db.conn.QueryRow(`SELECT 1 FROM halt_state LIMIT 1`).Scan(&one)
	if errors.Is(err, sql.ErrNoRows) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return true, nil
}

// SetHaltActive upserts a halt_state row keyed by cause. Idempotent —
// repeated calls for the same cause refresh set_at/set_by/reason; callers
// wrap with hysteresis to avoid set_at thrash. Multiple causes coexist
// independently (e.g. plan-cap + context-cap can both be active).
func (db *DB) SetHaltActive(cause, reason, setBy string) error {
	if cause == "" {
		return errors.New("SetHaltActive: cause must not be empty")
	}
	now := time.Now().UnixMilli()
	_, err := db.conn.Exec(
		`INSERT INTO halt_state (cause, set_at, set_by, reason) VALUES (?, ?, ?, ?)
		 ON CONFLICT(cause) DO UPDATE SET
		   set_at = excluded.set_at,
		   set_by = excluded.set_by,
		   reason = excluded.reason`,
		cause, now, setBy, reason,
	)
	return err
}

// ClearHalt deletes the halt_state row for the given cause. Idempotent —
// missing cause is not an error. Other active causes are unaffected.
func (db *DB) ClearHalt(cause string) error {
	_, err := db.conn.Exec(`DELETE FROM halt_state WHERE cause = ?`, cause)
	return err
}

// ClearHaltManually unconditionally clears every halt_state row. Backs the
// hub_clear_halt MCP tool — explicit operator-initiated abort that dismisses
// all causes, no causality requirement (e.g. user wants to resume in current
// sessions rather than restart). Plan-cap and context-cap are both wiped.
func (db *DB) ClearHaltManually() error {
	_, err := db.conn.Exec(`DELETE FROM halt_state`)
	return err
}

// GetHaltCause returns the row for a specific cause. (true, ...) when the
// cause is active; (false, zero-value, nil) when not active.
func (db *DB) GetHaltCause(cause string) (HaltState, bool, error) {
	var h HaltState
	var setAt int64
	err := db.conn.QueryRow(
		`SELECT cause, set_at, set_by, reason FROM halt_state WHERE cause = ?`, cause,
	).Scan(&h.Cause, &setAt, &h.SetBy, &h.Reason)
	if errors.Is(err, sql.ErrNoRows) {
		return HaltState{}, false, nil
	}
	if err != nil {
		return HaltState{}, false, err
	}
	h.SetAt = time.UnixMilli(setAt)
	return h, true, nil
}

// ClearHaltIfTrioReregistered clears the context-cap halt iff every
// currently-registered trio member has last_seen > halt_state.set_at.
// Scoped to cause="context-cap" per slice-5 C1 (H-33) — plan-cap halts do
// not auto-clear on trio re-register; they clear organically via
// window-rollover or poll-shows-decay.
//
// Members missing from the agents table (e.g. pruned, never-registered) are
// excluded from the comparison set so a partial trio can still trigger a
// clear. An empty comparison set (no trio member registered at all) does not
// clear — that is not evidence of fresh-context re-arrival, just absence.
//
// Returns (true, nil) when the clear fired, (false, nil) when the
// context-cap halt was not active or the trio condition was unmet.
func (db *DB) ClearHaltIfTrioReregistered(trioIDs []string) (bool, error) {
	row, ok, err := db.GetHaltCause(HaltCauseContextCap)
	if err != nil {
		return false, err
	}
	if !ok {
		return false, nil
	}
	setAtMs := row.SetAt.UnixMilli()

	considered := 0
	advanced := 0
	for _, id := range trioIDs {
		var lastSeen int64
		err := db.conn.QueryRow(
			`SELECT last_seen FROM agents WHERE id = ?`, id,
		).Scan(&lastSeen)
		if errors.Is(err, sql.ErrNoRows) {
			continue // pruned/never-registered — exclude from comparison
		}
		if err != nil {
			return false, err
		}
		considered++
		if lastSeen > setAtMs {
			advanced++
		}
	}
	if considered == 0 || advanced != considered {
		return false, nil
	}
	if err := db.ClearHalt(HaltCauseContextCap); err != nil {
		return false, err
	}
	return true, nil
}
