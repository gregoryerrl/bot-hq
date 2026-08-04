-- One row per duo-activity transition, so "the input unlocked while an agent was
-- still working" becomes a query instead of a recollection.
--
-- Context: `SessionActivity` is derived per session and pushed to the frontend to
-- gate the chat input. It was broadcast-only — `notify_session_activity` is a
-- fire-and-forget `event_tx.send` — so the state side of the timeline evaporated
-- the moment the UI consumed it. `messages` already persists what the agents SAID
-- and DID, with timestamps; there was simply nothing to join it against, so
-- "Brian emitted while the input was unlocked" could be reported by a user and
-- not reconstructed by anyone.
--
-- That gap produced a real, hard-to-pin report: the input unlocking mid-turn and
-- an agent surfacing output seconds later. The cause turned out to be that
-- `awaiting` outranks `busy` in the derive (a parked question must re-open the
-- input), which is correct — but nothing recorded it, so it stayed anecdotal.
--
-- The per-agent flags are NOT redundant with `state`. The derived state collapses
-- both agents into one `busy`, and `awaiting`/`paused` outrank it entirely, so
-- `state` alone cannot answer "was anyone actually working at that moment?" —
-- which is the whole question. A row is written on a change to EITHER the derived
-- state or a per-agent flag (mirroring exactly what the frontend receives): if
-- only state changes were recorded, a flag flip inside a stable `awaiting_user`
-- would leave the last row asserting `brian_busy = 1` long after Brian stopped,
-- and every query reading it would inherit that stale claim.
--
-- Volume is small by construction — the tracker already dedupes, emitting only on
-- an actual change — and bounded by the same 90-day sweep pattern used elsewhere
-- if it ever needs one.
CREATE TABLE activity_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    -- `SessionActivity::as_str`:
    -- 'idle' | 'busy' | 'awaiting_user' | 'cancelling' | 'paused'
    state       TEXT NOT NULL,
    -- Per-agent, at the instant of the transition. 1 = mid-turn.
    brian_busy  INTEGER NOT NULL,
    rain_busy   INTEGER NOT NULL
);

CREATE INDEX idx_activity_events_session
    ON activity_events (session_id, id DESC);
