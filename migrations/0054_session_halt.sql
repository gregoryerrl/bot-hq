-- rc3 D35 (second half): a halt is a SESSION state, not a tray row.
--
-- The user: "halt should be complete different, and not even remotely close to
-- parkable items in tray. It is now a session channel feature." The first cut
-- of D35 kept the durable `session_tray` row and merely hid it from the tray
-- surfaces — still tray-shaped underneath. These columns are the divorce: one
-- halt slot on the session itself, by construction ("in this way there can
-- never be 2 halts parked anymore"), declared by an agent and cleared by any
-- user response.
ALTER TABLE sessions ADD COLUMN halt_declared_by TEXT;
ALTER TABLE sessions ADD COLUMN halt_reason TEXT;
ALTER TABLE sessions ADD COLUMN halt_declared_at TEXT;

-- Close out any legacy pending halt ROWS so nothing is left half-claimed
-- between the two representations. History stays readable; nothing writes
-- kind='halt' rows any more.
UPDATE session_tray
SET status = 'answered',
    answered_at = datetime('now'),
    picked_option = '(superseded: halts are session state as of rc3 D35)'
WHERE kind = 'halt' AND status = 'pending';
