-- One row per peer forward that was DISCARDED, so a duo that goes half-deaf can
-- be diagnosed instead of argued about.
--
-- Context: `route_forward` has several early-returns, and until now every one of
-- them was a bare `debug!` — and there is no log sink configured, so a dropped
-- forward left no trace for the user, the sender, or the receiver. That produced
-- a whole session in which one agent repeatedly and correctly reported never
-- having seen plans the other had definitely written, and neither could tell
-- whether the reviewer was careless or the transport was lossy.
--
-- DROPS ONLY, deliberately:
--   * delivered forwards are the hot path, and `RouterDeps` is intentionally
--     lock-free (see its `open_blocking` note — a per-forward storage acquire is
--     exactly what that field exists to avoid). Drops are rare by definition, so
--     a write here costs nothing in the common case.
--   * `awaiting` and pause no longer drop at all — they HOLD and replay, so
--     there is nothing to record.
--   * `peer_ack` suppression is intentional AND already visible to the agent
--     that asked for it.
--
-- So a row here always means: a message was lost, and this is why.
CREATE TABLE forward_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL,
    occurred_at  TEXT NOT NULL,
    from_agent   TEXT NOT NULL,
    to_agent     TEXT NOT NULL,
    -- 'hard_cap'    — L2 runaway breaker (VOLLEY_HARD_CAP consecutive forwards
    --                 with no user message)
    -- 'convergence' — repetition breaker (Jaccard >= threshold, streak reached)
    -- 'no_peer'     — invariant breach: no sender for the peer
    reason       TEXT NOT NULL,
    -- Full length, so truncation in the preview is never mistaken for a short
    -- message.
    body_len     INTEGER NOT NULL,
    -- Enough to recognise WHICH message was lost without storing whole turns.
    body_preview TEXT NOT NULL
);

CREATE INDEX idx_forward_events_session
    ON forward_events (session_id, id DESC);
