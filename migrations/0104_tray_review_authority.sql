-- Review and user approval are separate authorities. Existing gates still
-- require the user's answer; only explicitly auto-allowed publishes opt out.
ALTER TABLE session_tray ADD COLUMN approval_required INTEGER NOT NULL DEFAULT 1
    CHECK (approval_required IN (0, 1));
ALTER TABLE session_tray ADD COLUMN reviewed_at TEXT;
