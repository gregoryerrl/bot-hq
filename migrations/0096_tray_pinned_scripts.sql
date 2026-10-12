-- 0096: the scripts a gated command RUNS, pinned by hash when the gate parked
-- (feedback #55): a JSON object `{"<path as written>": "<sha256>"}` of every
-- script that was readable at park. At approval each is re-hashed — a changed
-- or missing one refuses the run ("NOT RUN: the script changed after it was
-- parked") — and a script that was NOT readable at park (one the command
-- itself creates, a path only the command's `cd` resolves) is simply not
-- pinned and runs as it is, with a note (EYES d8c8c3a4: pinning the unreadable
-- refused every approval of such a command). NULL on rows parked before this
-- column existed, and on commands that run no script.
ALTER TABLE session_tray ADD COLUMN pinned_scripts TEXT;
