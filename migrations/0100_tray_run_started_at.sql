-- 0100: when bot-hq STARTED running an approved gated command (EYES 2577e626,
-- s-42bf34b3). Set by one atomic claim before any run — the immediate run at
-- approval and the scheduler's deferred run of a windowed gate (0098) — so the
-- two paths can never both run the command: the scheduler skips a claimed row,
-- and a claim that finds the stamp already set runs nothing. A row claimed with
-- no result and no refusal when bot-hq boots is a run the previous process was
-- cut off in; it is reported, never re-run. NULL = not started.
ALTER TABLE session_tray ADD COLUMN run_started_at TEXT;
