-- ============================================================================
-- 0044 — session participants: the session-focused architecture's schema.
--
-- Introduces user-owned `roles`, `session_participants` (with the invite-time
-- capability snapshot and turn state), `participant_cursors`,
-- `participant_deliveries`, and rebuilds `messages` to drop the
-- agent-name CHECK constraint.
--
-- Design:  docs/plans/2026-08-06-session-focused-redesign-design.md
-- Runbook: docs/plans/2026-08-06-session-participants-runbook.md
--
-- APPROVED AND ARMED 2026-08-06 after: four dry runs against copies of the live
-- database (the last against its exact current state — 201,034 messages, author
-- preserved on every row, 0 unmapped, integrity + FK clean); guard-firing tests
-- on three deliberately broken variants, each leaving the original table
-- intact; and a boot check
-- (`storage::participants::tests::every_legacy_message_query_still_works_after_0044`)
-- proving the legacy query paths survive.
--
-- WHAT MAKES THIS SAFE TO APPLY TO A LIVE DATABASE:
--   * `author` is RETAINED and still populated, so every query that has not
--     migrated yet keeps working the moment this applies. The CHECK constraint —
--     the only reason the table needed rebuilding — is what goes.
--   * `origin` is nullable for the same reason: it would otherwise be NOT NULL
--     with no default and every legacy `insert_message` would fail on insert.
--   * Six guards run BEFORE `DROP TABLE messages`, so a failed migration is a
--     no-op rather than a half-rebuild.
--   * A follow-up migration drops `author` and makes `origin` NOT NULL once the
--     cutover grep audit proves nothing reads the legacy column.
--
-- PRECONDITION: free disk ≈ 2× the database size. This rebuilds the largest
-- table, so the copy and the original coexist until the DROP.
-- ============================================================================

PRAGMA foreign_keys = OFF;   -- required for the table rebuild; restored at end

-- ---------------------------------------------------------------------------
-- 1. Roles — user-owned templates. Supersedes `agent_configs`, whose
--    `agent_name` PK is CHECK-constrained to ('emma','brian','rain') — the same
--    closed-enum problem one layer down.
-- ---------------------------------------------------------------------------
CREATE TABLE roles (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    slug               TEXT    NOT NULL UNIQUE,
    display_name       TEXT    NOT NULL,
    -- Layer 3 ONLY: the user's free-text identity/voice/priorities. Layers 1
    -- (core rules) and 2 (capability-derived rules) are composed at spawn and
    -- are deliberately NOT stored here — a role must not be able to author
    -- rules that contradict its own capability set.
    description_prompt TEXT,
    -- JSON array of capability slugs. GRANTS ONLY; deny-lists and spawn flags
    -- are derived from what is absent, so the two can never contradict.
    capabilities       TEXT    NOT NULL DEFAULT '[]',
    -- 'active' (in the turn rotation) | 'observer' (reads, never wakes) |
    -- 'on_demand' (skipped in rotation, wakes only when addressed).
    participation_mode TEXT    NOT NULL DEFAULT 'active',
    -- Desired runtime; resolved as desired ∩ model-supported. NOT a capability:
    -- native eligibility is a credential constraint, not a permission.
    desired_runtime    TEXT,
    default_model_id   TEXT,
    -- 1 = seeded by bot-hq. Seeds are still user-editable; the flag exists so
    -- the UI can offer "restore defaults", not to lock them.
    builtin            INTEGER NOT NULL DEFAULT 0,
    created_at         TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at         TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- Seed the user's two existing roles. Capability sets map 1:1 onto today's
-- HANDS_ONLY_TOOLS / EYES_ONLY_TOOLS + access, so a default session behaves
-- exactly as the duo does now (design Constraint 0 — behavioural parity).
INSERT INTO roles (slug, display_name, capabilities, participation_mode, builtin, description_prompt)
VALUES
  ('hands', 'HANDS',
   json('["read_channel","post_channel","ask_user","park_approval",
          "route_gated_command","supersede_question","disposition_finding",
          "override_reviewer_block","halt","declare_working","run_terminal",
          "write_context_library","edit_files","run_bash","gated_bash",
          "close_session"]'),
   'active', 1, NULL),
  ('eyes', 'EYES',
   json('["read_channel","post_channel","file_finding","approve_finding",
          "run_bash"]'),
   'active', 1, NULL);

-- ---------------------------------------------------------------------------
-- 2. Participants — replaces the 11 paired brian_*/rain_* columns + rain_enabled
-- ---------------------------------------------------------------------------
CREATE TABLE session_participants (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id         TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    -- Stable per-session handle; also the legacy bridge, since 'brian'/'rain'
    -- map 1:1 onto the old author strings during backfill.
    slug               TEXT    NOT NULL,
    display_name       TEXT    NOT NULL,
    role_id            INTEGER REFERENCES roles(id),
    model_id           TEXT,
    runtime            TEXT    NOT NULL DEFAULT 'claude_code',

    -- ---- INVITE-TIME SNAPSHOT ------------------------------------------
    -- Deliberately duplicated from the role: editing a role must never widen a
    -- LIVE participant's permissions mid-turn. role_id says which template;
    -- these say what this participant actually runs with. (Same pattern as
    -- *_model_at_spawn and session-policies/<sid>.yaml.)
    capabilities       TEXT    NOT NULL DEFAULT '[]',
    participation_mode TEXT    NOT NULL DEFAULT 'active',
    -- The fully composed prompt (core rules + capability-derived + description),
    -- stored so what the agent actually ran with is inspectable.
    prompt             TEXT,

    -- ---- TURN STATE ----------------------------------------------------
    -- Position in the fixed cycle. Observers/on-demand are excluded from the
    -- rotation; their position is retained but unused if promoted later.
    turn_position      INTEGER NOT NULL DEFAULT 0,
    -- Consensus halt: 1 = this participant declared "done" for the current
    -- round. ANY substantive output resets every participant's vote to 0, so a
    -- stale vote cannot accumulate into a false arrival.
    done_vote          INTEGER NOT NULL DEFAULT 0,

    effort             TEXT,
    ultracode          INTEGER,
    claude_session_id  TEXT,
    enabled            INTEGER NOT NULL DEFAULT 1,
    joined_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    left_at            TEXT,
    UNIQUE (session_id, slug)
);

CREATE INDEX idx_participants_session ON session_participants (session_id, enabled);
CREATE INDEX idx_participants_turn    ON session_participants (session_id, turn_position);

-- Per-session turn cursor. NULL holder = nobody's turn (idle / awaiting user).
ALTER TABLE sessions ADD COLUMN current_turn_participant_id INTEGER;
ALTER TABLE sessions ADD COLUMN round_number INTEGER NOT NULL DEFAULT 0;

-- ---------------------------------------------------------------------------
-- 3. Channel: cursors + delivery records
-- ---------------------------------------------------------------------------
CREATE TABLE participant_cursors (
    participant_id       INTEGER PRIMARY KEY
                         REFERENCES session_participants(id) ON DELETE CASCADE,
    last_read_message_id INTEGER NOT NULL DEFAULT 0,
    updated_at           TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- Per-delivery outcome. A policy (spin detection, round cap) may suppress
-- DELIVERY; it may NEVER suppress the POST — so a withheld message is still a
-- visible row with a reason. Suppressing the post is precisely what makes
-- today's losses invisible.
CREATE TABLE participant_deliveries (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    participant_id  INTEGER NOT NULL REFERENCES session_participants(id) ON DELETE CASCADE,
    message_id      INTEGER NOT NULL,
    delivered_at    TEXT,
    -- NULL = delivered; else 'spin' | 'round_cap' | 'awaiting' | 'closed'
    withheld_reason TEXT,
    UNIQUE (participant_id, message_id)
);

CREATE INDEX idx_deliveries_message ON participant_deliveries (message_id);

-- ---------------------------------------------------------------------------
-- 4. Seed participants from the existing paired columns.
--    370 duo sessions → 2 rows each; 12 solo sessions → 1 enabled + 1 disabled.
--    turn_position 0 = HANDS acts first, so the reviewer always sees the
--    executor's work before responding.
-- ---------------------------------------------------------------------------
INSERT INTO session_participants
    (session_id, slug, display_name, role_id, model_id, effort, ultracode,
     claude_session_id, capabilities, participation_mode, turn_position, joined_at)
SELECT
    s.id, 'brian', 'Brian',
    (SELECT id FROM roles WHERE slug = 'hands'),
    COALESCE(s.brian_model_id, s.brian_model_at_spawn),
    s.brian_effort, s.brian_ultracode, s.brian_claude_session_id,
    (SELECT capabilities FROM roles WHERE slug = 'hands'),
    (SELECT participation_mode FROM roles WHERE slug = 'hands'),
    0,
    s.created_at
FROM sessions s;

INSERT INTO session_participants
    (session_id, slug, display_name, role_id, model_id, effort, ultracode,
     claude_session_id, capabilities, participation_mode, turn_position,
     enabled, joined_at)
SELECT
    s.id, 'rain', 'Rain',
    (SELECT id FROM roles WHERE slug = 'eyes'),
    COALESCE(s.rain_model_id, s.rain_model_at_spawn),
    s.rain_effort, s.rain_ultracode, s.rain_claude_session_id,
    (SELECT capabilities FROM roles WHERE slug = 'eyes'),
    (SELECT participation_mode FROM roles WHERE slug = 'eyes'),
    1,
    s.rain_enabled,          -- solo sessions keep the row, disabled
    s.created_at
FROM sessions s;

INSERT INTO participant_cursors (participant_id, last_read_message_id)
SELECT id, 0 FROM session_participants;

-- ---------------------------------------------------------------------------
-- 5. Rebuild `messages` — SQLite cannot drop a CHECK constraint.
--    THE RISKIEST STEP: ~199.6k rows, irreversible, no second attempt.
-- ---------------------------------------------------------------------------
CREATE TABLE messages_new (
    id             INTEGER PRIMARY KEY,          -- ids PRESERVED (since_id watermark)
    session_id     TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    -- NULL for user + system origins.
    participant_id INTEGER REFERENCES session_participants(id),
    -- 'participant' | 'user' | 'system'. `system` is how host-authored
    -- injections (apply-entry nudge, reconcile directive, idle nudge, phase
    -- notices) become visible rows instead of invisible stdin writes.
    --
    -- NULLABLE during the transition, deliberately. It would otherwise be
    -- NOT NULL with no default, and every legacy `insert_message` — which
    -- knows nothing about `origin` — would fail on insert, so the app would
    -- not boot at all. Backfilled for existing rows below; new rows written by
    -- pre-B3b code leave it NULL, and readers fall back to `author`. The
    -- follow-up migration that drops `author` makes this NOT NULL.
    origin         TEXT,
    kind           TEXT    NOT NULL,
    content        TEXT    NOT NULL,
    -- JSON metadata that used to be invisible string mutation: phase envelope,
    -- blocking-findings banner, sender-role prefix, peer_ack-override tag.
    envelope       TEXT,
    created_at     TEXT    NOT NULL DEFAULT (datetime('now')),

    -- ---- LEGACY, TRANSITIONAL (revision 3) -----------------------------
    -- The old author string, retained and still populated. The CHECK
    -- constraint is gone — which was the only reason this table needed
    -- rebuilding — but the column stays so that EVERY existing query keeps
    -- working the moment this migration applies.
    --
    -- Why: dropping it here would mean the app cannot boot until all 153
    -- `Author::` references and 31 `insert_message` call sites move in one
    -- unreviewable commit. Keeping it makes the SCHEMA change big-bang (one
    -- migration, as chosen) while letting the CODE migrate in reviewable,
    -- gate-green slices. Same pattern already used for
    -- `sessions.{brian,rain}_*`.
    --
    -- Dropped by a follow-up migration once the cutover grep audit proves
    -- nothing reads it. Until then it is written alongside participant_id,
    -- and `participant_id` is the source of truth for anything new.
    author         TEXT
);

INSERT INTO messages_new
    (id, session_id, participant_id, origin, kind, content, created_at, author)
SELECT
    m.id,
    m.session_id,
    p.id,
    CASE WHEN m.author = 'user' THEN 'user' ELSE 'participant' END,
    m.kind,
    m.content,
    m.created_at,
    m.author
FROM messages m
LEFT JOIN session_participants p
       ON p.session_id = m.session_id AND p.slug = m.author;

-- ---- GUARDS ---------------------------------------------------------------
-- `RAISE(ABORT, …)` is ONLY legal inside a trigger program — used as a bare
-- statement it is a parse error, and `sqlite3` continues past parse errors with
-- exit 0, so the guards would silently no-op and the rebuild would proceed
-- unguarded. (Caught in the 2026-08-06 dry run; exactly the class of error an
-- immutable migration cannot survive.)
--
-- Portable alternative: a table whose CHECK can never be satisfied. Inserting
-- into it is conditional on the FAILURE predicate, so a violation raises a
-- constraint error and aborts; when the predicate is false no row is inserted
-- and nothing happens.
--
-- ORDERING IS LOAD-BEARING: every guard runs BEFORE `DROP TABLE messages`, so a
-- failed migration leaves the original table intact rather than half-rebuilt.
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

-- GUARD 4 (revision 2): every session has its participants, and every
-- participant resolved a role. Catches a seeding step that silently matched
-- nothing — which would produce an empty roster and a session nobody can act in.
INSERT INTO _migration_guard_0044 (failure)
SELECT 'participant seeding: session/participant count mismatch'
WHERE (SELECT count(*) FROM session_participants)
   <> (SELECT count(*) * 2 FROM sessions);

INSERT INTO _migration_guard_0044 (failure)
SELECT 'participant seeding: participant with no role'
WHERE EXISTS (SELECT 1 FROM session_participants WHERE role_id IS NULL);

-- GUARD 6 (revision 3): the legacy `author` column survived the copy intact.
-- If it did not, every existing query keeps compiling and silently reads NULL —
-- the quietest possible failure, and the one this transitional column exists to
-- prevent.
INSERT INTO _migration_guard_0044 (failure)
SELECT 'messages rebuild: author column not preserved'
WHERE (SELECT count(*) FROM messages_new WHERE author IS NOT NULL)
   <> (SELECT count(*) FROM messages WHERE author IS NOT NULL);

-- GUARD 5 (revision 2): exactly one participant per session at turn_position 0,
-- or the turn cycle has no defined start.
INSERT INTO _migration_guard_0044 (failure)
SELECT 'turn seeding: session without exactly one position-0 participant'
WHERE EXISTS (
    SELECT 1 FROM sessions s
    WHERE (SELECT count(*) FROM session_participants p
           WHERE p.session_id = s.id AND p.turn_position = 0) <> 1
);

DROP TABLE _migration_guard_0044;

DROP TABLE messages;
ALTER TABLE messages_new RENAME TO messages;

-- Recreate the three indexes the old table had, and add the participant-keyed
-- one. The author-keyed index is retained for as long as the column is —
-- dropping it while queries still use `author` would silently make the message
-- pane's per-agent reads a table scan on ~200k rows.
CREATE INDEX idx_messages_session_time
    ON messages (session_id, created_at);
CREATE INDEX idx_messages_session_id
    ON messages (session_id, id);
CREATE INDEX idx_messages_session_author_time
    ON messages (session_id, author, created_at);
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
--   * dropping agent_configs — superseded by `roles`, but removed only once
--     nothing reads it;
--   * retrieval_events.agent / cancel_events / activity_events paired columns —
--     batch B3 rekeys them.
-- ---------------------------------------------------------------------------
