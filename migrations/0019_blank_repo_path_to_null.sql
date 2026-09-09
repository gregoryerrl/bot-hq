-- A session created with a blank-but-present repo path ('' or whitespace)
-- reads as "has a working repo" everywhere working_repo_path is consumed —
-- action_gate then hard-errors ("session has no working_repo_path") before
-- the approve prompt, and hook install / diff anchoring misfire on a phantom
-- path. Storage::create_session now normalizes blank → NULL at insert; this
-- repairs rows that predate the guard.
UPDATE sessions
SET working_repo_path = NULL
WHERE working_repo_path IS NOT NULL
  AND TRIM(working_repo_path) = '';
