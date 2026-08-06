-- ============================================================================
-- DRAFT — NOT A MIGRATION YET. DO NOT MOVE INTO migrations/ UNTIL APPROVED.
--
-- Target filename once approved: migrations/0044_session_participants.sql
--
-- WHY THIS FILE LIVES HERE AND NOT IN migrations/:
--   sqlx applies every file in migrations/ automatically at app start, so
--   creating it there IS arming the destructive step. And the immutable-artifact
--   pre-commit gate + sqlx's runtime checksums mean an applied migration can
--   never be revised. This must be reviewed, dry-run against a COPY, and
--   explicitly approved before it becomes migration 0044.
--
-- SCOPE: the session-focused architecture, batch B1 (see the session `plan`
-- doc). Introduces participants + cursors and rebuilds `messages`.
--
-- MEASURED INPUTS (bot-hq.db, 2026-08-06):
--   messages : 199,607 rows — brian 132,037 · rain 62,548 · user 5,022
--              ZERO 'emma' rows (the legacy CHECK value is dead weight)
--   sessions : 382 total — 12 with rain_enabled = 0 (solo)
-- ============================================================================

PRAGMA foreign_keys = OFF;   -- required for the table rebuild; restored at end

-- ---------------------------------------------------------------------------
-- 1. Participants — replaces the 11 paired brian_*/rain_* columns + rain_enabled
-- ---------------------------------------------------------------------------
CREATE TABLE session_participants (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id        TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    -- Stable per-session handle. Also the legacy bridge: 'brian' / 'rain' map
    -- 1:1 onto the old author strings during backfill.
    slug              TEXT    NOT NULL,
    display_name      TEXT    NOT NULL,
    -- Role preset this participant was created from ('hands' | 'eyes' | ...).
    -- Presets are seed data, not code — a new role is a row, not a variant.
    preset            TEXT,
    model_id          TEXT,
    -- 'claude_code' | 'native'. NOT a capability: native eligibility is a
    -- property of the model's credential (subscription OAuth is CLI-bound),
    -- so it is resolved as requested-runtime ∩ model-supported.
    runtime           TEXT    NOT NULL DEFAULT 'claude_code',
    -- JSON array of capability slugs. THE authorization source of truth,
    -- replacing `caller.agent != "brian"` name equality at the tool boundary.
    capabilities      TEXT    NOT NULL DEFAULT '[]',
    effort            TEXT,
    ultracode         INTEGER,
    claude_session_id TEXT,
    -- The composed system prompt this participant runs with, stored so it is
    -- inspectable (closes invisible-injection leak #6).
    prompt            TEXT,
    enabled           INTEGER NOT NULL DEFAULT 1,
    joined_at         TEXT    NOT NULL DEFAULT (datetime('now')),
    left_at           TEXT,
    UNIQUE (session_id, slug)
);

CREATE INDEX idx_participants_session ON session_participants (session_id, enabled);

-- ---------------------------------------------------------------------------
-- 2. Delivery cursors — delivery becomes an auditable fact, not a side-effect
-- ---------------------------------------------------------------------------
CREATE TABLE participant_cursors (
    participant_id       INTEGER PRIMARY KEY
                         REFERENCES session_participants(id) ON DELETE CASCADE,
    last_read_message_id INTEGER NOT NULL DEFAULT 0,
    updated_at           TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- Per-delivery outcome. A policy (convergence / hard-cap) may suppress
-- DELIVERY; it may never suppress the POST — so a suppressed message is still
-- a visible row here with a reason, instead of vanishing (plan R2).
CREATE TABLE participant_deliveries (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    participant_id INTEGER NOT NULL REFERENCES session_participants(id) ON DELETE CASCADE,
    message_id     INTEGER NOT NULL,
    delivered_at   TEXT,
    -- NULL = delivered; else 'convergence' | 'hard_cap' | 'awaiting' | 'closed'
    withheld_reason TEXT,
    UNIQUE (participant_id, message_id)
);

CREATE INDEX idx_deliveries_message ON participant_deliveries (message_id);

-- ---------------------------------------------------------------------------
-- 3. Seed participants from the existing paired columns
--    370 duo sessions → 2 rows each; 12 solo sessions → 1 row.
-- ---------------------------------------------------------------------------
INSERT INTO session_participants
    (session_id, slug, display_name, preset, model_id, effort, ultracode,
     claude_session_id, capabilities, joined_at)
SELECT
    s.id, 'brian', 'Brian', 'hands',
    COALESCE(s.brian_model_id, s.brian_model_at_spawn),
    s.brian_effort, s.brian_ultracode, s.brian_claude_session_id,
    -- HANDS preset, mirroring today's HANDS_ONLY_TOOLS + write access.
    json('["read_channel","post_channel","ask_user","park_approval",
           "route_gated_command","supersede_question","disposition_finding",
           "override_reviewer_block","halt","declare_working","run_terminal",
           "write_context_library","edit_files","run_bash","gated_bash",
           "close_session"]'),
    s.created_at
FROM sessions s;

INSERT INTO session_participants
    (session_id, slug, display_name, preset, model_id, effort, ultracode,
     claude_session_id, capabilities, enabled, joined_at)
SELECT
    s.id, 'rain', 'Rain', 'eyes',
    COALESCE(s.rain_model_id, s.rain_model_at_spawn),
    s.rain_effort, s.rain_ultracode, s.rain_claude_session_id,
    -- EYES preset: reviews, files findings, reads. No edit/user-facing verbs —
    -- exactly today's deny-list + EYES_ONLY_TOOLS, expressed as data.
    json('["read_channel","post_channel","file_finding","approve_finding",
           "run_bash"]'),
    s.rain_enabled,          -- solo sessions keep the row, disabled
    s.created_at
FROM sessions s;

INSERT INTO participant_cursors (participant_id, last_read_message_id)
SELECT id, 0 FROM session_participants;

-- ---------------------------------------------------------------------------
-- 4. Rebuild `messages` — SQLite cannot drop a CHECK constraint.
--    THE RISKIEST STEP: 199,607 rows, irreversible, no second attempt.
-- ---------------------------------------------------------------------------
CREATE TABLE messages_new (
    id             INTEGER PRIMARY KEY,          -- ids PRESERVED (since_id watermark)
    session_id     TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    -- NULL for user + system origins.
    participant_id INTEGER REFERENCES session_participants(id),
    -- 'participant' | 'user' | 'system'. `system` is how host-authored
    -- injections (apply-entry nudge, reconcile directive, idle nudge, phase
    -- notices) become visible rows instead of invisible stdin writes.
    origin         TEXT    NOT NULL,
    kind           TEXT    NOT NULL,
    content        TEXT    NOT NULL,
    -- JSON metadata that used to be invisible string mutation: phase envelope,
    -- blocking-findings banner, sender-role prefix, peer_ack-override tag.
    envelope       TEXT,
    created_at     TEXT    NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO messages_new (id, session_id, participant_id, origin, kind, content, created_at)
SELECT
    m.id,
    m.session_id,
    p.id,
    CASE WHEN m.author = 'user' THEN 'user' ELSE 'participant' END,
    m.kind,
    m.content,
    m.created_at
FROM messages m
LEFT JOIN session_participants p
       ON p.session_id = m.session_id AND p.slug = m.author;

-- ---- GUARDS ---------------------------------------------------------------
-- `RAISE(ABORT, …)` is ONLY legal inside a trigger program — used as a bare
-- statement it is a parse error, and `sqlite3` continues past parse errors with
-- exit 0, so the guards would silently no-op and the rebuild would proceed
-- unguarded. (Caught in the 2026-08-06 dry run; this is exactly the class of
-- error an immutable migration cannot survive.)
--
-- Portable alternative: a table whose CHECK can never be satisfied. Inserting
-- into it is conditional on the FAILURE predicate, so a violation raises a
-- constraint error and aborts; when the predicate is false no row is inserted
-- and nothing happens.
CREATE TABLE _migration_guard_0044 (
    failure TEXT CHECK (failure IS NULL)
);

-- GUARD 1: no rows lost — abort if the copy is short by even one row.
INSERT INTO _migration_guard_0044 (failure)
SELECT 'messages rebuild: row count mismatch'
WHERE (SELECT count(*) FROM messages_new) <> (SELECT count(*) FROM messages);

-- GUARD 2: every non-user row resolved to a participant.
-- Catches an author string with no matching participant (the 'emma' class).
INSERT INTO _migration_guard_0044 (failure)
SELECT 'messages rebuild: unmapped author rows'
WHERE EXISTS (
    SELECT 1 FROM messages_new
    WHERE origin = 'participant' AND participant_id IS NULL
);

-- GUARD 3: id continuity — the max id must survive, or `since_id` watermarks
-- and every stored message reference break silently.
INSERT INTO _migration_guard_0044 (failure)
SELECT 'messages rebuild: max id changed'
WHERE (SELECT COALESCE(max(id),0) FROM messages_new)
   <> (SELECT COALESCE(max(id),0) FROM messages);

DROP TABLE _migration_guard_0044;

DROP TABLE messages;
ALTER TABLE messages_new RENAME TO messages;

-- Recreate the three indexes the old table had; the author-keyed one becomes
-- participant-keyed.
CREATE INDEX idx_messages_session_time
    ON messages (session_id, created_at);
CREATE INDEX idx_messages_session_id
    ON messages (session_id, id);
CREATE INDEX idx_messages_session_participant_time
    ON messages (session_id, participant_id, created_at);

-- Restore the AUTOINCREMENT high-water mark so new ids continue past the old
-- max rather than colliding with archived references.
INSERT OR REPLACE INTO sqlite_sequence (name, seq)
SELECT 'messages', COALESCE(max(id), 0) FROM messages;

PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- DELIBERATELY NOT DONE HERE
--   * dropping sessions.{brian,rain}_* + rain_enabled — a second migration
--     after the code stops reading them, so this one stays revertible-by-restore
--     rather than revertible-by-nothing;
--   * agent_configs.agent_name CHECK — separate, low-traffic, no urgency;
--   * retrieval_events.agent / cancel_events / activity_events paired columns —
--     batch B3 rekeys them.
-- ---------------------------------------------------------------------------
