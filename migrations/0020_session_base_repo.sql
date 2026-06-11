-- Per-session git worktrees: when a session runs in an isolated worktree,
-- `working_repo_path` points at the WORKTREE (the path agents actually run
-- in — action_gate, hooks, diff anchoring all keep working unchanged) and
-- `base_repo_path` remembers the user's main repo (needed for
-- `git worktree add/remove` and shared-hooks install). NULL = the session
-- runs directly in `working_repo_path` (no worktree) — all pre-existing rows.
ALTER TABLE sessions ADD COLUMN base_repo_path TEXT;
