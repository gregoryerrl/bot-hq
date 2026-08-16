-- 0060_retire_the_agent_names.sql — rc3 D10's hard retirement, schema half.
--
-- The names Brian and Rain were retired from the PRODUCT on 2026-08-12 (D10):
-- participants render as `ROLE · Model` and slugs are role-derived. The schema
-- was left alone, so 15 columns across three tables and one CHECK constraint
-- still carried them. The user's reason for finishing the job is not tidiness:
-- the Context Library and these column names are what load into an agent's
-- context window, and a live schema naming two agents that do not exist is a
-- standing invitation to hallucinate state that matches it.
--
-- Ordered cheap-to-expensive. SQLite here is 3.51, so DROP COLUMN and RENAME
-- COLUMN are both available; only the CHECK and the DEFAULT need a rebuild.
--
-- Pre-flight: `sessions` carries exactly two indexes — the `id` autoindex and
-- `idx_sessions_active (archived, closed_at)`. None of the dropped columns is
-- indexed, so SQLite will not refuse the drops. Verified against the live
-- database before this file was written.

-- ── 1. sessions: eight columns with no reader AND no writer ────────────────
--
-- Established twice over, by two participants independently: `core/session.rs`
-- states it in prose ("Those columns are left in place and UNREAD"), spawn
-- takes every one of these values off the participant row instead, and a
-- field-access grep finds no consumer. Their last writers were retired in
-- `5decdcf`, so these have been inert since.
ALTER TABLE sessions DROP COLUMN brian_effort;
ALTER TABLE sessions DROP COLUMN rain_effort;
ALTER TABLE sessions DROP COLUMN brian_ultracode;
ALTER TABLE sessions DROP COLUMN rain_ultracode;
ALTER TABLE sessions DROP COLUMN brian_model_id;
ALTER TABLE sessions DROP COLUMN rain_model_id;
ALTER TABLE sessions DROP COLUMN brian_claude_session_id;
ALTER TABLE sessions DROP COLUMN rain_claude_session_id;

-- ── 2. sessions: rain_enabled, dropped rather than renamed ─────────────────
--
-- It was written from `seeded > 1` at a single site, which makes it a CACHED
-- COUNT of the roster — a second source of truth free to disagree with the rows
-- it summarises. That is the defect behind round 2's B3, where a boolean that
-- could not carry a count made rosters of 2, 3 and 8 produce identical
-- sessions. Renaming it would have kept the liability and moved the name.
--
-- Its one consumer (`SessionInfo`, which the dashboard reads to mark a solo
-- session) is now computed in SQL from `session_participants`, so the roster is
-- the only place the answer lives.
ALTER TABLE sessions DROP COLUMN rain_enabled;

-- ── 3. sessions: the two live columns, renamed to what they mean ───────────
--
-- These record the model each SPAWN SLOT ran with, frozen at spawn for the chat
-- header. They are slot-indexed already — `set_session_spawn_model_slot` maps
-- slot 0 and slot 1 onto them positionally — so the names were the only thing
-- claiming otherwise.
--
-- The two-column shape is a REAL LIMIT, not just a naming one, and it survives
-- this migration: a session may run up to MAX_SESSION_PARTICIPANTS (8), and a
-- third participant's spawn model is recorded nowhere. That is stated in
-- `spawn_model_slots_round_trip_and_slot_two_is_a_silent_no_op` rather than
-- fixed here, because widening it is a data-model change and this file is a
-- rename.
ALTER TABLE sessions RENAME COLUMN brian_model_at_spawn TO slot0_model_at_spawn;
ALTER TABLE sessions RENAME COLUMN rain_model_at_spawn  TO slot1_model_at_spawn;

-- ── 4. cancel_events ───────────────────────────────────────────────────────
--
-- Live: written by `record_cancel_event`, read back by its query. The Rust side
-- moves in the SAME commit as this migration — unlike activity_events below,
-- it carries no alias bridge, so column and struct must land together.
ALTER TABLE cancel_events RENAME COLUMN brian_interrupt_queued TO slot0_interrupt_queued;
ALTER TABLE cancel_events RENAME COLUMN rain_interrupt_queued  TO slot1_interrupt_queued;

-- ── 5. activity_events ─────────────────────────────────────────────────────
--
-- Also live. Its Rust side was renamed one commit EARLIER and bridged with
-- `brian_busy AS slot0_busy` in the shared column const, because renaming a SQL
-- literal ahead of its column turned four tests red — the schema and its
-- readers have to move together. This migration is what lets that alias go.
ALTER TABLE activity_events RENAME COLUMN brian_busy TO slot0_busy;
ALTER TABLE activity_events RENAME COLUMN rain_busy  TO slot1_busy;

-- ── 6. agent_configs: the only real CHECK in the database naming them ──────
--
-- `agent_name TEXT PRIMARY KEY CHECK (agent_name IN ('emma','brian','rain'))`,
-- from `0001_init.sql`, which is immutable — so this is a rebuild, and the
-- twelve-step form is required because SQLite cannot drop a CHECK in place.
--
-- The constraint rejects every slug a current session can produce, so the
-- per-spawn `get_agent_config(agent_name)` lookup could only ever miss and fall
-- through to `default_agent_config`. The seeded `emma`/`brian`/`rain` rows go
-- with it: they name two agents that no longer exist and one removed in 0017.
--
-- Precedent: `messages.author` carried the same `IN ('user','emma','brian',
-- 'rain')` CHECK and was already rebuilt without it; that column now holds
-- `hands`, `eyes`, `eyes-2`, `advisor`.
CREATE TABLE agent_configs_new (
    agent_name     TEXT PRIMARY KEY,
    provider       TEXT NOT NULL DEFAULT 'anthropic',
    model_name     TEXT NOT NULL,
    base_url       TEXT,
    auth_token     TEXT,
    updated_at     TEXT NOT NULL DEFAULT (datetime('now')),
    native         INTEGER NOT NULL DEFAULT 0,
    context_window INTEGER
);
-- Nothing is carried over: every existing row is one of the three retired
-- names, and a row keyed by a name no role answers to cannot be read back.
DROP TABLE agent_configs;
ALTER TABLE agent_configs_new RENAME TO agent_configs;

-- ── 7. participant_cursors: the one column default PROVEN to have fired ────
--
-- Round-3 F8. Migration 0059 backfilled 85 zone-less rows here and bound both
-- INSERT sites to `now_utc()`, which fixed the data and the writers — but left
-- the `datetime('now')` DEFAULT installed. Round 2's stated reason for leaving
-- the other fifteen defaults was that they have never fired; this is the one
-- that did, so it is the one that gets removed. A future INSERT that omits the
-- bind would silently re-contaminate the column, and the existing guard only
-- inspects a row it just wrote.
--
-- A rebuild rather than an ALTER: SQLite cannot drop a column default in place.
CREATE TABLE participant_cursors_new (
    participant_id       INTEGER PRIMARY KEY
                         REFERENCES session_participants(id) ON DELETE CASCADE,
    last_read_message_id INTEGER NOT NULL DEFAULT 0,
    -- No DEFAULT. Both INSERT sites bind `now_utc()` explicitly; a caller that
    -- forgets now fails loudly instead of writing SQLite's zone-less shape.
    updated_at           TEXT    NOT NULL
);
INSERT INTO participant_cursors_new (participant_id, last_read_message_id, updated_at)
    SELECT participant_id, last_read_message_id, updated_at FROM participant_cursors;
DROP TABLE participant_cursors;
ALTER TABLE participant_cursors_new RENAME TO participant_cursors;
