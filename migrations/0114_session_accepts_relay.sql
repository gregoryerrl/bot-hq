-- Plan Batch 6 (peer relay): a session that holds a controller receives
-- attributed peer messages (`send_session_message`) only after it opted in —
-- an agent with the narrower grant must not reach a full-control one unasked.
-- Sessions without a controller accept by default; this flag is read only for
-- controller sessions. Set by the user in Session Settings or by a controller
-- through `app_control admin`.
ALTER TABLE sessions ADD COLUMN accepts_relay INTEGER NOT NULL DEFAULT 0;
