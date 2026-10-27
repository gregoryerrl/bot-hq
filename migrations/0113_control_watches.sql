-- Plan 5c-2 (Driver watch/wake): a controller's standing subscriptions over
-- the control-event journal. One row per (controller participant, caller
-- request id). `targets` is a JSON list of session ids or NULL for "every
-- other session that holds no controller"; `kinds` a JSON list or NULL for
-- the attention kinds. `cursor` is the last journal id the controller
-- acknowledged (explicitly, with `ack` — a read never moves it);
-- `pending_since` is the journal id of the first unacknowledged matching
-- event (`pending_kind` its kind, kept here so a boot can tell an attention
-- event from chatter after the row itself was swept), or NULL while nothing
-- is waiting; `woke_for` / `woke_at` record the one wake CLAIMED for that
-- pending event, `wake_attempt_at` the lease of the one delivery attempt in
-- flight (a resume takes it only when the previous attempt's lease lapsed),
-- and `wake_delivered_at` when the ring itself consumed the wake — a claim is
-- intent, not delivery, and an undelivered claim is resumed, so a watch wakes
-- its controller at most once per acknowledgement and never loses a wake to
-- a crash in between.
CREATE TABLE control_watches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    participant_id INTEGER NOT NULL REFERENCES session_participants(id),
    slug TEXT NOT NULL,
    request_id TEXT NOT NULL,
    targets TEXT,
    kinds TEXT,
    wake INTEGER NOT NULL DEFAULT 0,
    cursor INTEGER NOT NULL,
    pending_since INTEGER,
    pending_kind TEXT,
    woke_for INTEGER,
    woke_at TEXT,
    wake_attempt_at TEXT,
    wake_delivered_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(participant_id, request_id)
);
CREATE INDEX control_watches_session ON control_watches(session_id);
